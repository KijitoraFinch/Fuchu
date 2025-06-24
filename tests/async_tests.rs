use fuchu::Runtime;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

/// A simple future that yields control a specified number of times before completing
struct YieldingFuture {
    yields_remaining: usize,
}

impl YieldingFuture {
    fn new(yields: usize) -> Self {
        Self {
            yields_remaining: yields,
        }
    }
}

impl Future for YieldingFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.yields_remaining == 0 {
            Poll::Ready(())
        } else {
            self.yields_remaining -= 1;
            // Wake immediately to be polled again
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// A future that never completes unless explicitly woken
struct ManualWakeFuture {
    completed: Arc<Mutex<bool>>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl ManualWakeFuture {
    fn new() -> (Self, ManualWakeHandle) {
        let completed = Arc::new(Mutex::new(false));
        let waker = Arc::new(Mutex::new(None));

        let future = Self {
            completed: completed.clone(),
            waker: waker.clone(),
        };

        let handle = ManualWakeHandle { completed, waker };

        (future, handle)
    }
}

impl Future for ManualWakeFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let completed = *self.completed.lock().unwrap();
        if completed {
            Poll::Ready(())
        } else {
            // Store the waker
            *self.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

struct ManualWakeHandle {
    completed: Arc<Mutex<bool>>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl ManualWakeHandle {
    fn complete(&self) {
        *self.completed.lock().unwrap() = true;
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

#[test]
fn test_yielding_future() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let counter_clone = counter.clone();

    runtime.spawner().spawn(async move {
        // This future will yield 5 times before completing
        YieldingFuture::new(5).await;
        *counter_clone.lock().unwrap() = 42;
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(100));
    assert_eq!(*counter.lock().unwrap(), 42);

    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_multiple_yielding_futures() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let spawner = runtime.spawner().clone();

    // Spawn multiple tasks that yield different numbers of times
    for i in 1..=5 {
        let counter_clone = counter.clone();
        spawner.spawn(async move {
            YieldingFuture::new(i).await;
            let mut count = counter_clone.lock().unwrap();
            *count += i;
        });
    }

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(200));

    // Sum of 1+2+3+4+5 = 15
    assert_eq!(*counter.lock().unwrap(), 15);

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_manual_wake_future() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let counter_clone = counter.clone();

    let (future, handle) = ManualWakeFuture::new();

    runtime.spawner().spawn(async move {
        future.await;
        *counter_clone.lock().unwrap() = 100;
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Task should be pending
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 0);

    // Complete the future
    handle.complete();

    // Now the task should complete
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 100);

    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_concurrent_manual_wake_futures() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let spawner = runtime.spawner().clone();

    let mut handles = Vec::new();

    // Create multiple manual wake futures
    for i in 1..=3 {
        let (future, handle) = ManualWakeFuture::new();
        handles.push(handle);

        let counter_clone = counter.clone();
        spawner.spawn(async move {
            future.await;
            let mut count = counter_clone.lock().unwrap();
            *count += i * 10;
        });
    }

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // All tasks should be pending
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 0);

    // Complete futures one by one
    handles[1].complete(); // Should add 20
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 20);

    handles[0].complete(); // Should add 10
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 30);

    handles[2].complete(); // Should add 30
    thread::sleep(Duration::from_millis(50));
    assert_eq!(*counter.lock().unwrap(), 60);

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_complex_async_composition() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let results = Arc::new(Mutex::new(Vec::new()));
    let spawner = runtime.spawner().clone();

    let results_clone = results.clone();
    spawner.spawn(async move {
        // First yield a few times
        YieldingFuture::new(3).await;
        results_clone.lock().unwrap().push("first");

        // Then yield some more
        YieldingFuture::new(2).await;
        results_clone.lock().unwrap().push("second");
    });

    let results_clone = results.clone();
    let (future, handle) = ManualWakeFuture::new();
    spawner.spawn(async move {
        future.await;
        results_clone.lock().unwrap().push("manual");
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Give time for the first task to complete
    thread::sleep(Duration::from_millis(100));

    {
        let res = results.lock().unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], "first");
        assert_eq!(res[1], "second");
    }

    // Complete the manual future
    handle.complete();
    thread::sleep(Duration::from_millis(50));

    {
        let res = results.lock().unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[2], "manual");
    }

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}
