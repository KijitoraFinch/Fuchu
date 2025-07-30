pub mod custom_futures;
pub mod executor;
pub mod reactor;
pub mod runtime;
pub mod spawner;
pub mod task;

use crate::reactor::ReactorHandle;
use std::cell::RefCell;

thread_local! {
    static REACTOR: RefCell<Option<ReactorHandle>> = RefCell::new(None);
}

pub fn reactor() -> ReactorHandle {
    REACTOR.with(|reactor| reactor.borrow().as_ref().unwrap().clone())
}

pub(crate) fn reactor_handle_to_thread_local(handle: ReactorHandle) {
    REACTOR.with(|reactor| *reactor.borrow_mut() = Some(handle));
}

pub struct EnterGuard {
    _private: (),
}

impl Drop for EnterGuard {
    fn drop(&mut self) {
        REACTOR.with(|reactor| reactor.borrow_mut().take());
    }
}