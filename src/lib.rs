// Fuchu: Very simple and minimalistic async runtime

use core::task;
use crossbeam::channel;
use futures::task::ArcWake;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub(crate) struct Runtime {
    executor: Executor,
    spawner: Spawner,
}

impl Runtime {
    pub(crate) fn new() -> Self {
        let (executor, scheduling_sender) = Executor::new();
        let spawner = Spawner::new(scheduling_sender);
        Self { executor, spawner }
    }

    pub(crate) fn run(&self) {
        self.executor.run();
    }

    pub(crate) fn spawner(&self) -> &Spawner {
        &self.spawner
    }
}

pub(crate) struct Task {
    pub(crate) id: usize,
    pub(crate) task_future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
    pub(crate) scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
}

impl Task {
    pub(crate) fn new(
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

#[derive(Debug)]
pub(crate) struct Executor {
    task_queue: crossbeam::channel::Receiver<Arc<Task>>,
}

impl Executor {
    pub(crate) fn new() -> (Self, crossbeam::channel::Sender<Arc<Task>>) {
        let (scheduling_sender, task_queue) = crossbeam::channel::unbounded();
        (Executor { task_queue }, scheduling_sender)
    }

    pub(crate) fn run(&self) {
        loop {
            match self.task_queue.recv() {
                Ok(task) => {
                    let mut task_future = task.task_future.lock().unwrap();
                    let waker = futures::task::waker(Arc::clone(&task));
                    let context = &mut std::task::Context::from_waker(&waker);
                    if let future = task_future.as_mut() {
                        let _ = future.poll(context);
                    }
                }
                Err(_) => {}
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Spawner {
    scheduling_sender: crossbeam::channel::Sender<Arc<Task>>,
}

impl Spawner {
    pub(crate) fn new(scheduling_sender: crossbeam::channel::Sender<Arc<Task>>) -> Self {
        Spawner { scheduling_sender }
    }

    pub(crate) fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task::new(
            0,
            Box::pin(future),
            self.scheduling_sender.clone(),
        ));
        self.scheduling_sender.send(task).unwrap();
    }
}
