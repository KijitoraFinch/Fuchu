// fuchu: Very simple and minimalistic async runtime

pub mod custom_futures;

use crossbeam::channel;
use futures::{future::BoxFuture, task::ArcWake};
use std::{
    cell::RefCell,
    future::Future,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

thread_local! {
    static REACTOR: RefCell<Option<ReactorHandle>> = RefCell::new(None);
}

pub fn reactor() -> ReactorHandle {
    // get the reactor handle from the thread-local storage
    // this will panic if called outside of a runtime context
    REACTOR.with(|reactor| reactor.borrow().as_ref().unwrap().clone())
}

pub struct EnterGuard {
    _private: (),
}

impl Drop for EnterGuard {
    fn drop(&mut self) {
        REACTOR.with(|reactor| reactor.borrow_mut().take());
    }
}

pub enum RegistryRequest {
    RegisterFd {
        fd: std::os::fd::RawFd,
        waker: std::task::Waker,
    },
    UnregisterFd {
        fd: std::os::fd::RawFd,
    },
    RegisterTimer {
        timer: std::time::Instant,
        id: usize,
        waker: std::task::Waker,
    },
    UnregisterTimer {
        id: usize,
    },
}

pub struct Task {
    future: Mutex<BoxFuture<'static, ()>>,
    executor_sender: channel::Sender<Arc<Self>>,
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &std::sync::Arc<Self>) {
        let task = arc_self.clone();
        if let Err(_) = arc_self.executor_sender.send(task) {
            // Executorが終了している場合は何もしない
            return;
        }
    }
}

impl Task {
    pub fn new<F>(future: F, executor_sender: channel::Sender<Arc<Self>>) -> Arc<Self>
    where
        F: futures::Future<Output = ()> + Send + 'static,
    {
        Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            executor_sender,
        })
    }
}

pub struct Runtime {
    reactor_handle: ReactorHandle,
    executor_handle: ExecutorHandle,
    reactor_thread: JoinHandle<()>,
    executor_thread: JoinHandle<()>,
}

impl Runtime {
    pub fn spawner(&self) -> Spawner {
        Spawner::new(self.executor_handle.clone())
    }

    pub fn enter(&self) -> EnterGuard {
        REACTOR.with(|reactor| *reactor.borrow_mut() = Some(self.reactor_handle.clone()));
        EnterGuard { _private: () }
    }

    pub fn block_on<F: Future + Send + 'static>(&self, future: F) -> F::Output
    where
        F::Output: Send + 'static,
    {
        let _guard = self.enter();
        let (sender, receiver) = std::sync::mpsc::channel();
        self.spawner().spawn(async move {
            let result = future.await;
            sender.send(result).unwrap();
        });
        receiver.recv().unwrap()
    }
}

pub struct RuntimeBuilder {
    // TODO: nanika
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        RuntimeBuilder {}
    }
    pub fn build(self) -> Runtime {
        let (executor_sender, executor_receiver) = channel::unbounded();
        let executor = Executor::new(executor_receiver);
        let executor_handle = ExecutorHandle::new(executor_sender.clone());

        let (mut reactor, reactor_sender) = Reactor::new();
        let reactor_handle = ReactorHandle::new(reactor_sender);

        let executor_thread_reactor_handle = reactor_handle.clone();
        let executor_thread = std::thread::spawn(move || {
            executor.run(executor_thread_reactor_handle);
        });

        let reactor_thread = std::thread::spawn(move || {
            reactor.run();
        });

        Runtime {
            reactor_handle,
            executor_handle,
            reactor_thread,
            executor_thread,
        }
    }
}

pub struct Reactor {
    waker_lookup: std::collections::HashMap<std::os::fd::RawFd, std::task::Waker>,
    timeout_lookup: std::collections::BTreeMap<std::time::Instant, std::task::Waker>,
    registry_channel: crossbeam::channel::Receiver<RegistryRequest>,
}

impl Reactor {
    pub fn new() -> (Self, channel::Sender<RegistryRequest>) {
        let (sender, receiver) = channel::unbounded();
        (
            Reactor {
                waker_lookup: std::collections::HashMap::new(),
                timeout_lookup: std::collections::BTreeMap::new(),
                registry_channel: receiver,
            },
            sender,
        )
    }

    pub fn run(&mut self) {
        let mut poll = mio::Poll::new().unwrap();
        let mut events = mio::Events::with_capacity(1024);

        loop {
            // `registry_channel`にたまっているリクエストをすべて処理する
            // `try_recv`はブロックしない
            while let Ok(request) = self.registry_channel.try_recv() {
                match request {
                    RegistryRequest::RegisterFd { fd, waker } => {
                        let mut source = mio::unix::SourceFd(&fd);
                        let token = mio::Token(fd as usize);
                        poll.registry()
                            .register(
                                &mut source,
                                token,
                                mio::Interest::READABLE | mio::Interest::WRITABLE,
                            )
                            .or_else(|e| {
                                if e.kind() == std::io::ErrorKind::AlreadyExists {
                                    Ok(())
                                } else {
                                    Err(e)
                                }
                            })
                            .unwrap();
                        self.waker_lookup.insert(fd, waker);
                    }
                    RegistryRequest::UnregisterFd { fd } => {
                        let mut source = mio::unix::SourceFd(&fd);
                        let token = mio::Token(fd as usize);
                        // The fd may be closed by the time we process this, so ignore the result.
                        let _ = poll.registry().deregister(&mut source);
                        self.waker_lookup.remove(&fd);
                    }
                    RegistryRequest::RegisterTimer { timer, id, waker } => {
                        self.timeout_lookup.insert(timer, waker);
                    }
                    RegistryRequest::UnregisterTimer { id } => {
                        todo!("なんとかする");
                    }
                }
            }

            poll.poll(&mut events, Some(Duration::from_millis(0)))
                .unwrap();

            let now = Instant::now();
            // Process the timers before processing the events.
            let mut expired_timers = Vec::new();
            for (timer, waker) in self.timeout_lookup.range(..=now) {
                expired_timers.push((timer.clone(), waker.clone()));
            }
            for (timer, waker) in expired_timers {
                self.timeout_lookup.remove(&timer);
                waker.wake();
            }

            // Process the events.
            for event in events.iter() {
                let fd = event.token().0 as std::os::fd::RawFd;
                if let Some(waker) = self.waker_lookup.get(&fd) {
                    if event.is_readable() || event.is_writable() {
                        waker.wake_by_ref();
                    }
                }
            }
        }
    }
}

pub struct ReactorHandle {
    sender: channel::Sender<RegistryRequest>,
}

impl Clone for ReactorHandle {
    fn clone(&self) -> Self {
        ReactorHandle {
            sender: self.sender.clone(),
        }
    }
}

impl ReactorHandle {
    pub fn new(sender: channel::Sender<RegistryRequest>) -> Self {
        ReactorHandle { sender }
    }

    pub fn register(&self, fd: std::os::fd::RawFd, waker: std::task::Waker) {
        let request = RegistryRequest::RegisterFd { fd, waker };
        self.sender
            .send(request)
            .expect("Failed to send register request");
    }

    pub fn unregister(&self, fd: std::os::fd::RawFd) {
        let request = RegistryRequest::UnregisterFd { fd };
        self.sender
            .send(request)
            .expect("Failed to send unregister request");
    }

    pub fn register_timer(&self, timer: std::time::Instant, waker: std::task::Waker) {
        let request = RegistryRequest::RegisterTimer {
            timer,
            id: 0, // TODO: Implement ID management for timers
            waker,
        };
        self.sender
            .send(request)
            .expect("Failed to send register timer request");
    }

    pub fn unregister_timer(&self, id: usize) {
        let request = RegistryRequest::UnregisterTimer { id };
        self.sender
            .send(request)
            .expect("Failed to send unregister timer request");
    }
}

pub struct Executor {
    task_queue: crossbeam::channel::Receiver<Arc<Task>>,
}

impl Executor {
    pub fn new(task_queue: crossbeam::channel::Receiver<Arc<Task>>) -> Self {
        Executor { task_queue }
    }

    pub fn run(&self, reactor_handle: ReactorHandle) {
        // set the reactor handle in the thread-local storage
        // so that it can be accessed by the futures spawned in this executor
        let _guard = REACTOR.with(|reactor| {
            *reactor.borrow_mut() = Some(reactor_handle.clone());
            EnterGuard { _private: () }
        });

        while let Ok(task) = self.task_queue.recv() {
            let mut future = task.future.lock().unwrap();

            // Imagine The Future.
            let waker = futures::task::waker_ref(&task);
            let mut context = std::task::Context::from_waker(&*waker);
            future.as_mut().poll(&mut context); // discarded. (because we can ensure that the result of _Task_ is Ready(()) or Pending)
        }
    }
}

#[derive(Clone)]
pub struct ExecutorHandle {
    sender: channel::Sender<Arc<Task>>,
}

impl ExecutorHandle {
    pub fn new(sender: channel::Sender<Arc<Task>>) -> Self {
        ExecutorHandle { sender }
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: futures::Future<Output = ()> + Send + 'static,
    {
        let task = Task::new(future, self.sender.clone());
        self.sender.send(task.clone()).expect("Failed to send task");
    }
}

#[derive(Clone)]
pub struct Spawner {
    executor_handle: ExecutorHandle,
}

impl Spawner {
    pub fn new(executor_handle: ExecutorHandle) -> Self {
        Spawner { executor_handle }
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: futures::Future<Output = ()> + Send + 'static,
    {
        self.executor_handle.spawn(future);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_handle_spawn() {
        let (sender, receiver) = channel::unbounded::<Arc<Task>>();
        let handle = ExecutorHandle::new(sender);

        handle.spawn(async {});

        // Check that a task was sent to the channel
        assert!(receiver.try_recv().is_ok());
    }
}
