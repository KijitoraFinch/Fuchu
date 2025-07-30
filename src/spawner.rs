use crate::executor::ExecutorHandle;

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
