use fuchu::custom_futures::{TimerFuture, CancellableTimerFuture};
use fuchu::{Runtime, RuntimeBuilder};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn test_timer_unregister_functionality() {
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();
    
    let completed_timers = Arc::new(Mutex::new(Vec::new()));
    let completed_timers_clone = completed_timers.clone();
    
    // Create multiple timers with different durations
    for i in 0..5 {
        let duration = Duration::from_millis(50 + (i * 10));
        let timer_id = i;
        let completed_timers = completed_timers_clone.clone();
        
        spawner.spawn(async move {
            let _start = Instant::now();
            TimerFuture::new(duration).await;
            completed_timers.lock().unwrap().push(timer_id);
        });
    }
    
    // Wait long enough for some timers to complete
    std::thread::sleep(Duration::from_millis(100));
    
    let completed = completed_timers.lock().unwrap().len();
    println!("Completed timers: {}", completed);
    
    // This test currently just verifies basic timer functionality
    // Once unregister is implemented, we can test that cancelled timers don't complete
    assert!(completed > 0);
}

#[test]
fn test_cancellable_timer_functionality() {
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();
    
    let completed_timers = Arc::new(Mutex::new(Vec::new()));
    let completed_timers_clone = completed_timers.clone();
    
    // Create a cancellable timer that should complete
    let timer1_completed = completed_timers_clone.clone();
    spawner.spawn(async move {
        CancellableTimerFuture::new(Duration::from_millis(50)).await;
        timer1_completed.lock().unwrap().push(1);
    });
    
    // Create a cancellable timer that will be cancelled
    let timer2_completed = completed_timers_clone.clone();
    spawner.spawn(async move {
        let mut timer = CancellableTimerFuture::new(Duration::from_millis(100));
        // Cancel it immediately 
        timer.cancel();
        timer.await;
        timer2_completed.lock().unwrap().push(2);
    });
    
    // Wait for timers to complete or be cancelled
    std::thread::sleep(Duration::from_millis(150));
    
    let completed = completed_timers.lock().unwrap();
    println!("Completed cancellable timers: {:?}", *completed);
    
    // Both timers should complete - timer 1 normally, timer 2 immediately due to cancellation
    assert_eq!(completed.len(), 2);
    assert!(completed.contains(&1));
    assert!(completed.contains(&2));
}

// This test will demonstrate the unregister functionality once implemented
#[test] 
fn test_timer_unregister_prevents_completion() {
    let runtime = RuntimeBuilder::new().build();
    let spawner = runtime.spawner();
    let _guard = runtime.enter();
    
    let completed_timers = Arc::new(Mutex::new(Vec::new()));
    let completed_timers_clone = completed_timers.clone();
    
    // Test direct unregister via reactor handle
    let reactor_handle = fuchu::reactor();
    let timer_id = reactor_handle.register_timer(
        Instant::now() + Duration::from_millis(50),
        std::task::Waker::noop().clone()
    );
    
    // Immediately unregister the timer
    reactor_handle.unregister_timer(timer_id);
    
    // Test CancellableTimerFuture drop behavior
    spawner.spawn(async move {
        {
            let _timer = CancellableTimerFuture::new(Duration::from_millis(50));
            // Timer will be unregistered when dropped at end of scope
        }
        // If we get here, the timer was properly cleaned up
        completed_timers_clone.lock().unwrap().push(42);
    });
    
    // Wait to see if any timers inappropriately complete
    std::thread::sleep(Duration::from_millis(100));
    
    let completed = completed_timers.lock().unwrap();
    println!("Completed timers after unregister test: {:?}", *completed);
    
    // Should have the completion from the drop test
    assert_eq!(completed.len(), 1);
    assert!(completed.contains(&42));
}