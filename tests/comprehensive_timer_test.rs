use fuchu::{RuntimeBuilder, reactor};
use futures::channel::oneshot;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};

#[test]
fn test_different_ids_same_instant_comprehensive() {
    let runtime = RuntimeBuilder::new().build();
    let _guard = runtime.enter();
    
    // Create multiple wakers for the same exact instant
    let instant = Instant::now() + Duration::from_millis(100);
    let mut timer_ids = Vec::new();
    
    // Register 5 timers with the exact same instant
    for i in 0..5 {
        let (sender, _receiver) = oneshot::channel::<()>();
        let waker = futures::task::waker(Arc::new(TestWaker::new(sender, i)));
        let timer_id = reactor().register_timer(instant, waker);
        timer_ids.push(timer_id);
        println!("Registered timer {} with ID {}", i, timer_id);
    }
    
    // Verify all timer IDs are different
    for i in 0..timer_ids.len() {
        for j in (i+1)..timer_ids.len() {
            assert_ne!(timer_ids[i], timer_ids[j], 
                "Timer IDs {} and {} should be different", timer_ids[i], timer_ids[j]);
        }
    }
    
    // Unregister some timers
    reactor().unregister_timer(timer_ids[0]);
    reactor().unregister_timer(timer_ids[2]);
    reactor().unregister_timer(timer_ids[4]);
    
    println!("Successfully unregistered timers with IDs: {}, {}, {}", 
             timer_ids[0], timer_ids[2], timer_ids[4]);
    
    // This test verifies that:
    // 1. Multiple timers can be registered with the same instant
    // 2. Each timer gets a unique ID 
    // 3. Timers can be unregistered by ID without panicking
}

// Enhanced test helper
struct TestWaker {
    _sender: oneshot::Sender<()>,
    id: usize,
}

impl TestWaker {
    fn new(sender: oneshot::Sender<()>, id: usize) -> Self {
        Self { _sender: sender, id }
    }
}

impl futures::task::ArcWake for TestWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        println!("Timer {} woken up", arc_self.id);
    }
}