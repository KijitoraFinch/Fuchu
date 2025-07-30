use crate::{
    executor::{Executor, ExecutorHandle},
    reactor::{Reactor, ReactorHandle},
    spawner::Spawner,
};
use crossbeam::channel;
use std::{
    future::Future,
    thread::JoinHandle,
};

pub struct Runtime {
    reactor_handle: ReactorHandle,
    executor_handle: ExecutorHandle,
    reactor_thread: JoinHandle<()>, // TODO: this should be used
    executor_thread: JoinHandle<()>, // TODO: this should be used
}

impl Runtime {
    pub fn spawner(&self) -> Spawner {
        Spawner::new(self.executor_handle.clone())
    }

    pub fn enter(&self) -> crate::EnterGuard {
        crate::reactor_handle_to_thread_local(self.reactor_handle.clone());
        crate::EnterGuard { _private: () }
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

pub struct RuntimeBuilder {}

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
