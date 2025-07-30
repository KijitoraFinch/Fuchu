use crossbeam::channel;
use futures::{future::BoxFuture, task::ArcWake};
use std::{
    sync::{Arc, Mutex},
};

pub struct Task {
    pub(crate) future: Mutex<BoxFuture<'static, ()>>,
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
