use crate::reactor::ReactorHandle;
use crate::task::Task;
use crossbeam::channel;
use futures::Future;
use std::sync::Arc;

pub struct Executor {
    task_queue: crossbeam::channel::Receiver<Arc<Task>>,
}

impl Executor {
    pub fn new(task_queue: crossbeam::channel::Receiver<Arc<Task>>) -> Self {
        Executor { task_queue }
    }

    pub fn run(&self, reactor_handle: ReactorHandle) {
        crate::reactor_handle_to_thread_local(reactor_handle);

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
