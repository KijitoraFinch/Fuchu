use Fuchu::{Interest, Runtime};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn test_runtime_creation() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    // Just ensure we can create a runtime without panicking
    drop(runtime);
}

#[test]
fn test_single_task_execution() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let counter_clone = counter.clone();

    runtime.spawner().spawn(async move {
        *counter_clone.lock().unwrap() = 42;
    });

    // Run the runtime in a separate thread with a timeout
    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Give some time for the task to complete
    thread::sleep(Duration::from_millis(100));

    assert_eq!(*counter.lock().unwrap(), 42);

    // The runtime should have terminated by now since all tasks completed
    // and the spawner was dropped
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_multiple_tasks() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));

    let spawner = runtime.spawner().clone();

    // Spawn multiple tasks
    for i in 0..10 {
        let counter_clone = counter.clone();
        spawner.spawn(async move {
            let mut count = counter_clone.lock().unwrap();
            *count += i;
        });
    }

    // Run runtime in separate thread
    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Wait for tasks to complete
    thread::sleep(Duration::from_millis(200));

    // Sum of 0..10 is 45
    assert_eq!(*counter.lock().unwrap(), 45);

    drop(spawner); // Drop the spawner to allow runtime to terminate
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_spawner_clone() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));

    let spawner1 = runtime.spawner().clone();
    let spawner2 = spawner1.clone();

    let counter1 = counter.clone();
    let counter2 = counter.clone();

    spawner1.spawn(async move {
        *counter1.lock().unwrap() += 1;
    });

    spawner2.spawn(async move {
        *counter2.lock().unwrap() += 2;
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(100));

    assert_eq!(*counter.lock().unwrap(), 3);

    // Drop spawners to allow runtime to terminate
    drop(spawner1);
    drop(spawner2);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_runtime_terminates_when_no_tasks() {
    let runtime = Runtime::new().expect("Failed to create runtime");

    let start = Instant::now();
    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Runtime should terminate quickly since there are no tasks
    // and the spawner is dropped immediately
    assert!(runtime_handle.join().is_ok());
    let elapsed = start.elapsed();

    // Should terminate within a reasonable time
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn test_nested_task_spawning() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let counter = Arc::new(Mutex::new(0));
    let spawner = runtime.spawner().clone();

    let counter_clone = counter.clone();
    let spawner_clone = spawner.clone();

    spawner.spawn(async move {
        *counter_clone.lock().unwrap() += 1;

        // Spawn another task from within this task
        let counter_inner = counter_clone.clone();
        spawner_clone.spawn(async move {
            *counter_inner.lock().unwrap() += 10;
        });
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(200));

    assert_eq!(*counter.lock().unwrap(), 11);

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}
