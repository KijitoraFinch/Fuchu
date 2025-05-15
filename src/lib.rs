// Fuchu: Very simple and minimalistic async runtime

use core::task;
use crossbeam::channel;
use futures::task::ArcWake;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::future::Future;  // Missing import

/// Main runtime for executing async tasks
pub struct Runtime {
    executor: Executor,
    spawner: Spawner,
}

impl Runtime {
    /// Create a new runtime instance
    pub fn new() -> Self {
        let (executor, scheduling_sender) = Executor::new();
        let spawner = Spawner::new(scheduling_sender);
        Self { executor, spawner }
    }

    /// Run the runtime, processing tasks until completion
    pub fn run(&self) {
        self.executor.run();
    }

    /// Get a reference to the spawner for creating new tasks
    pub fn spawner(&self) -> &Spawner {
        &self.spawner
    }
}

/// Represents an async task in the runtime
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
        Task {
            id,
            task_future: Mutex::new(task_future),
            scheduling_sender,
        }
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let task = arc_self.clone();
        match task.scheduling_sender.send(task.clone()) {
            Ok(_) => {
                // Successfully sent the task to the executor
            }
            Err(_) => {
                // Handle the error case
            }
        }
    }
}

/// Executes tasks in the runtime
struct Executor {
    task_queue: crossbeam::channel::Receiver<Arc<Task>>,
}

impl Executor {
    fn new() -> (Self, crossbeam::channel::Sender<Arc<Task>>) {
        let (scheduling_sender, task_queue) = crossbeam::channel::unbounded();
        (Executor { task_queue }, scheduling_sender)
    }

    fn run(&self) {
        loop {
            match self.task_queue.recv() {
                Ok(task) => {
                    let mut task_future = task.task_future.lock().unwrap();
                    let waker = futures::task::waker(Arc::clone(&task));
                    let context = &mut std::task::Context::from_waker(&waker);
                    match task_future.as_mut().poll(context) {
                        std::task::Poll::Ready(_) => {
                            // Task completed
                            if cfg!(debug_assertions) {
                                println!("Task {} completed", task.id);
                            }
                        }
                        std::task::Poll::Pending => {
                            // Task is still pending, continue waiting
                            if cfg!(debug_assertions) {
                                println!("Task {} is still pending", task.id);
                            }
                        }
                    }
                }
                Err(_) => {
                    // Handle the error case
                    eprintln!("Failed to receive task from queue");
                }
            }
        }
    }
}

pub struct Spawner {
    scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
    uniq_counter: usize,
}

impl Spawner {
    fn new(scheduling_sender: crossbeam::channel::Sender<Arc<Task>>) -> Self {
        Spawner {
            scheduling_sender,
            uniq_counter: 0,
        }
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task::new(
            self.uniq_counter,
            Box::pin(future),
            self.scheduling_sender.clone(),
        ));
        self.scheduling_sender.send(task).unwrap();
        self.uniq_counter += 1;
    }
}
