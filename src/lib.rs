// Fuchu: Very simple and minimalistic async runtime

use core::{panic, task};
use crossbeam::channel;
use futures::task::ArcWake;
use nix::sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags, EpollTimeout};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{io, thread};

// --- Global and Thread-Local Definitions ---

/// Global task counter
static TASK_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A message sent to the Reactor to register a new I/O source.
type RegistrationMsg = (RawFd, Interest, task::Waker);

thread_local! {
    /// Thread-local sender to register I/O sources with the Reactor.
    static REGISTRATION_SENDER: RefCell<crossbeam::channel::Sender<RegistrationMsg>> = {
        // This is a placeholder. The actual sender is set when the Runtime is created.
        let (sender, _) = crossbeam::channel::unbounded();
        RefCell::new(sender)
    };
}

/// A registry mapping file descriptors to wakers.
type WakerRegistry = Arc<Mutex<HashMap<RawFd, task::Waker>>>;

// --- Public Runtime API ---

/// Main runtime for executing async tasks.
pub struct Runtime {
    executor: Executor,
    spawner: Spawner,
    _reactor_handle: thread::JoinHandle<()>,
}

impl Runtime {
    /// Create a new runtime instance.
    /// This will also spawn a background thread for the I/O reactor.
    pub fn new() -> io::Result<Self> {
        // 1. Create the reactor and get its registration sender.
        let (reactor, registration_sender) = Reactor::new()?;

        // 2. Spawn the reactor on a new thread.
        let _reactor_handle = thread::spawn(move || {
            if let Err(e) = reactor.run() {
                eprintln!("Reactor thread failed: {}", e);
            }
        });

        // 3. Set the thread-local sender for this main thread.
        REGISTRATION_SENDER.with(|cell| {
            *cell.borrow_mut() = registration_sender;
        });

        // 4. Create the executor and spawner.
        let (executor, scheduling_sender) = Executor::new();
        let spawner = Spawner::new(scheduling_sender);

        Ok(Self {
            executor,
            spawner,
            _reactor_handle,
        })
    }

    /// Run the runtime's executor.
    /// This function will block until all tasks are completed and all spawners are dropped.
    pub fn run(&self) {
        self.executor.run();
    }

    /// Get a reference to the spawner for creating new tasks.
    pub fn spawner(&self) -> &Spawner {
        &self.spawner
    }
}

/// Spawns new futures onto the runtime.
#[derive(Clone)]
pub struct Spawner {
    scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
}

impl Spawner {
    fn new(scheduling_sender: crossbeam::channel::Sender<Arc<Task>>) -> Self {
        Spawner { scheduling_sender }
    }

    /// Spawns a new future to be run on the executor.
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task_id = TASK_COUNTER.fetch_add(1, Ordering::SeqCst);
        let task_future = Box::pin(future);
        let arc_task = Arc::new(Task::new(
            task_id,
            task_future,
            self.scheduling_sender.clone(),
        ));

        // Send the task to the executor to be polled.
        self.scheduling_sender
            .send(arc_task)
            .expect("Failed to send task to executor");
    }
}

// --- Core Runtime Components ---

/// An asynchronous task.
struct Task {
    id: usize,
    task_future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
    scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
}

impl Task {
    fn new(
        id: usize,
        task_future: Pin<Box<dyn Future<Output = ()> + Send>>,
        scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
    ) -> Self {
        Self {
            id,
            task_future: Mutex::new(task_future),
            scheduling_sender,
        }
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        // When woken, the task is sent back to the executor's queue to be polled again.
        arc_self
            .scheduling_sender
            .send(arc_self.clone())
            .expect("Failed to send task to executor");
    }
}

/// Executes tasks by polling futures.
struct Executor {
    task_queue: crossbeam::channel::Receiver<Arc<Task>>,
}

impl Executor {
    fn new() -> (Self, crossbeam::channel::Sender<Arc<Task>>) {
        let (scheduling_sender, task_queue) = channel::unbounded();
        (Executor { task_queue }, scheduling_sender)
    }

    fn run(&self) {
        // This loop continues as long as there are active Senders for the task_queue.
        // It will automatically terminate when all tasks are done and spawners are dropped.
        while let Ok(task) = self.task_queue.recv() {
            let mut task_future = task.task_future.lock().unwrap();
            let waker = futures::task::waker(Arc::clone(&task));
            let mut context = std::task::Context::from_waker(&waker);

            match task_future.as_mut().poll(&mut context) {
                task::Poll::Ready(_) => {
                    if cfg!(debug_assertions) {
                        println!("Task {} completed", task.id);
                    }
                }
                task::Poll::Pending => {
                    if cfg!(debug_assertions) {
                        println!("Task {} is pending", task.id);
                    }
                    // The future is now responsible for waking the waker when it's ready.
                }
            }
        }
        println!("All tasks completed. Executor shutting down.");
    }
}

/// The I/O reactor that listens for events using epoll.
struct Reactor {
    epoll_instance: Epoll,
    registry: WakerRegistry,
    registration_receiver: crossbeam::channel::Receiver<RegistrationMsg>,
}

impl Reactor {
    fn new() -> io::Result<(Self, channel::Sender<RegistrationMsg>)> {
        let epoll_instance = Epoll::new(EpollCreateFlags::empty())?;
        let (registration_sender, registration_receiver) = channel::unbounded();
        let registry = Arc::new(Mutex::new(HashMap::new()));
        Ok((
            Self {
                epoll_instance,
                registry,
                registration_receiver,
            },
            registration_sender,
        ))
    }

    fn run(&self) -> io::Result<()> {
        let mut events = vec![EpollEvent::empty(); 1024];
        loop {
            // Wait for I/O events with a timeout to allow checking for new registrations.
            let num_events = self.epoll_instance.wait(&mut events, 100u16)?;

            // Wake any tasks that have pending I/O.
            for i in 0..num_events {
                let event = events[i];
                let fd = event.data() as RawFd;
                if let Some(waker) = self.registry.lock().unwrap().get(&fd) {
                    waker.wake_by_ref();
                }
            }

            // Process any new I/O source registrations.
            while let Ok((fd, interest, waker)) = self.registration_receiver.try_recv() {
                self.register(fd, interest, waker)?;
            }
        }
    }

    fn register(&self, fd: RawFd, interest: Interest, waker: task::Waker) -> io::Result<()> {
        let flags = match interest {
            Interest::Readable => EpollFlags::EPOLLIN,
            Interest::Writable => EpollFlags::EPOLLOUT,
            Interest::Both => EpollFlags::EPOLLIN | EpollFlags::EPOLLOUT,
        };
        // Register the file descriptor with epoll.
        let event = EpollEvent::new(flags, fd as u64);
        self.epoll_instance.add(fd, event)?;
    }

    fn send_registration(fd: RawFd, interest: Interest, waker: task::Waker) -> io::Result<()> {
        REGISTRATION_SENDER.with(|cell| {
            let sender = cell.borrow();
            sender
                .send((fd, interest, waker))
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to send registration"))
        })
    }
}

/// Represents the I/O interest for a file descriptor.
pub enum Interest {
    Readable,
    Writable,
    Both,
}
