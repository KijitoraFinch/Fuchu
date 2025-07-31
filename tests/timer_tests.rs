use fuchu::custom_futures::TimerFuture;
use fuchu::{runtime::RuntimeBuilder, reactor};
use futures::channel::oneshot;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};

#[test]
fn test_multiple_timers_same_instant() {
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();
    
    let counter = Arc::new(Mutex::new(0));
    let (sender, receiver) = oneshot::channel::<()>();
    let sender = Arc::new(Mutex::new(Some(sender)));
    
    // Create multiple timers with the same duration (and likely same instant)
    for i in 0..5 {
        let counter = counter.clone();
        let sender = sender.clone();
        spawner.spawn(async move {
            TimerFuture::new(Duration::from_millis(50)).await;
            let mut count = counter.lock().unwrap();
            *count += 1;
            println!("Timer {} completed, total count: {}", i, *count);
            
            // Send completion signal when all timers are done
            if *count == 5 {
                if let Some(s) = sender.lock().unwrap().take() {
                    s.send(()).unwrap();
                }
            }
        });
    }
    
    // Wait for all timers to complete
    runtime.block_on(receiver).unwrap();
    
    // Verify all timers completed
    assert_eq!(*counter.lock().unwrap(), 5);
}

#[test]
fn test_timer_unregister_functionality() {
    let runtime = RuntimeBuilder::new().build();
    let _guard = runtime.enter();
    
    // Create multiple wakers (using dummy ones for testing)
    let (sender1, _receiver1) = oneshot::channel::<()>();
    let (sender2, _receiver2) = oneshot::channel::<()>();
    
    let deadline = Instant::now() + Duration::from_millis(100);
    
    // Register multiple timers with the same instant
    let timer1_id = {
        let waker1 = futures::task::waker(Arc::new(TestWaker::new(sender1)));
        reactor().register_timer(deadline, waker1)
    };
    
    let timer2_id = {
        let waker2 = futures::task::waker(Arc::new(TestWaker::new(sender2)));
        reactor().register_timer(deadline, waker2)
    };
    
    // Verify different IDs were assigned
    assert_ne!(timer1_id, timer2_id);
    
    // Unregister one timer
    reactor().unregister_timer(timer1_id);
    
    // The unregistration should succeed without panicking
    // (More comprehensive testing would require access to reactor internals)
}

// Helper struct for testing wakers
struct TestWaker {
    _sender: oneshot::Sender<()>,
}

impl TestWaker {
    fn new(sender: oneshot::Sender<()>) -> Self {
        Self { _sender: sender }
    }
}

impl futures::task::ArcWake for TestWaker {
    fn wake_by_ref(_arc_self: &Arc<Self>) {
        // Do nothing for test
    }
}