use fuchu::{Interest, Runtime};
use std::future::Future;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;

// Helper futures for I/O operations
struct ReadableFuture {
    fd: i32,
    registered: bool,
}

impl ReadableFuture {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            registered: false,
        }
    }
}

impl Future for ReadableFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.registered {
            if let Err(e) =
                fuchu::Reactor::send_registration(self.fd, Interest::Readable, cx.waker().clone())
            {
                return Poll::Ready(Err(e));
            }
            self.registered = true;
        }
        Poll::Pending
    }
}

struct WritableFuture {
    fd: i32,
    registered: bool,
}

impl WritableFuture {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            registered: false,
        }
    }
}

impl Future for WritableFuture {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.registered {
            if let Err(e) =
                fuchu::Reactor::send_registration(self.fd, Interest::Writable, cx.waker().clone())
            {
                return Poll::Ready(Err(e));
            }
            self.registered = true;
        }
        Poll::Pending
    }
}

#[test]
fn test_echo_server() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let socket_path = temp_file.path().to_path_buf();
    drop(temp_file);

    let server_result = Arc::new(Mutex::new(Vec::new()));
    let server_result_clone = server_result.clone();

    // Echo server
    let socket_path_server = socket_path.clone();
    spawner.spawn(async move {
        let listener = UnixListener::bind(&socket_path_server).expect("Failed to bind Unix socket");

        // Handle multiple connections
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().expect("Failed to accept connection");
            let fd = stream.as_raw_fd();
            let result_clone = server_result_clone.clone();

            // Spawn a task for each connection
            spawner.spawn(async move {
                // Wait for data to be available
                ReadableFuture::new(fd)
                    .await
                    .expect("Failed to wait for readable");

                // Read the data
                let mut buffer = [0u8; 1024];
                let n = stream
                    .read(&mut buffer)
                    .expect("Failed to read from stream");
                let received = String::from_utf8_lossy(&buffer[..n]);

                // Echo it back
                WritableFuture::new(fd)
                    .await
                    .expect("Failed to wait for writable");
                stream.write_all(&buffer[..n]).expect("Failed to echo data");

                result_clone.lock().unwrap().push(received.to_string());
            });
        }
    });

    let client_results = Arc::new(Mutex::new(Vec::new()));

    // Multiple clients
    for i in 0..3 {
        let socket_path_client = socket_path.clone();
        let client_results_clone = client_results.clone();

        spawner.spawn(async move {
            // Give server time to start
            thread::sleep(Duration::from_millis(50));

            let mut stream =
                UnixStream::connect(&socket_path_client).expect("Failed to connect to Unix socket");
            let fd = stream.as_raw_fd();

            let message = format!("Hello from client {}", i);

            // Send message
            WritableFuture::new(fd)
                .await
                .expect("Failed to wait for writable");
            stream
                .write_all(message.as_bytes())
                .expect("Failed to write message");

            // Read echo response
            ReadableFuture::new(fd)
                .await
                .expect("Failed to wait for readable");
            let mut buffer = [0u8; 1024];
            let n = stream.read(&mut buffer).expect("Failed to read response");
            let response = String::from_utf8_lossy(&buffer[..n]);

            client_results_clone
                .lock()
                .unwrap()
                .push(response.to_string());
        });
    }

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Wait for all operations to complete
    thread::sleep(Duration::from_millis(1000));

    // Verify server received all messages
    let server_messages = server_result.lock().unwrap();
    assert_eq!(server_messages.len(), 3);

    // Verify clients received echoed messages
    let client_responses = client_results.lock().unwrap();
    assert_eq!(client_responses.len(), 3);

    // Check that messages were echoed correctly
    let mut server_sorted: Vec<String> = server_messages.clone();
    let mut client_sorted: Vec<String> = client_responses.clone();
    server_sorted.sort();
    client_sorted.sort();

    assert_eq!(server_sorted, client_sorted);

    drop(spawner);
    let _ = std::fs::remove_file(&socket_path);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_concurrent_io_and_compute_tasks() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let socket_path = temp_file.path().to_path_buf();
    drop(temp_file);

    let results = Arc::new(Mutex::new(Vec::new()));

    // I/O task
    let socket_path_server = socket_path.clone();
    let results_io = results.clone();
    spawner.spawn(async move {
        let listener = UnixListener::bind(&socket_path_server).expect("Failed to bind Unix socket");

        let (mut stream, _) = listener.accept().expect("Failed to accept connection");
        let fd = stream.as_raw_fd();

        ReadableFuture::new(fd)
            .await
            .expect("Failed to wait for readable");

        let mut buffer = [0u8; 1024];
        let n = stream.read(&mut buffer).expect("Failed to read");
        let received = String::from_utf8_lossy(&buffer[..n]);

        results_io.lock().unwrap().push(format!("IO: {}", received));
    });

    // Compute-intensive task
    let results_compute = results.clone();
    spawner.spawn(async move {
        let mut sum = 0u64;
        for i in 0..1000000 {
            sum += i;
            // Yield occasionally to allow other tasks to run
            if i % 100000 == 0 {
                // Simple yield by creating a ready future
                std::future::ready(()).await;
            }
        }
        results_compute
            .lock()
            .unwrap()
            .push(format!("Compute: {}", sum));
    });

    // Client task
    let socket_path_client = socket_path.clone();
    spawner.spawn(async move {
        thread::sleep(Duration::from_millis(100));

        let mut stream = UnixStream::connect(&socket_path_client).expect("Failed to connect");
        let fd = stream.as_raw_fd();

        WritableFuture::new(fd)
            .await
            .expect("Failed to wait for writable");
        stream
            .write_all(b"Mixed workload test")
            .expect("Failed to write");
    });

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    thread::sleep(Duration::from_millis(1000));

    let results_vec = results.lock().unwrap();
    assert_eq!(results_vec.len(), 2);

    // Check that both tasks completed
    let has_io = results_vec.iter().any(|s| s.starts_with("IO:"));
    let has_compute = results_vec.iter().any(|s| s.starts_with("Compute:"));

    assert!(has_io, "I/O task should have completed");
    assert!(has_compute, "Compute task should have completed");

    drop(spawner);
    let _ = std::fs::remove_file(&socket_path);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_runtime_performance_characteristics() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let task_count = 1000;
    let completed_tasks = Arc::new(Mutex::new(0));

    let start_time = Instant::now();

    // Spawn many lightweight tasks
    for i in 0..task_count {
        let completed_clone = completed_tasks.clone();
        spawner.spawn(async move {
            // Simulate some work with yields
            for _ in 0..10 {
                std::future::ready(()).await;
            }

            let mut count = completed_clone.lock().unwrap();
            *count += 1;

            if i % 100 == 0 {
                println!("Completed {} tasks", *count);
            }
        });
    }

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Wait for all tasks to complete
    let mut last_count = 0;
    loop {
        thread::sleep(Duration::from_millis(100));
        let current_count = *completed_tasks.lock().unwrap();

        if current_count == task_count {
            break;
        }

        // Check for progress to avoid infinite loop
        if current_count == last_count {
            // No progress for a while, something might be wrong
            if start_time.elapsed() > Duration::from_secs(10) {
                panic!("Tasks are not completing within reasonable time");
            }
        }
        last_count = current_count;
    }

    let elapsed = start_time.elapsed();
    println!("Completed {} tasks in {:?}", task_count, elapsed);

    // Should complete within reasonable time (this is quite generous)
    assert!(
        elapsed < Duration::from_secs(5),
        "Tasks took too long to complete"
    );

    drop(spawner);
    assert!(runtime_handle.join().is_ok());
}

#[test]
fn test_graceful_shutdown() {
    let runtime = Runtime::new().expect("Failed to create runtime");
    let spawner = runtime.spawner().clone();

    let shutdown_order = Arc::new(Mutex::new(Vec::new()));

    // Task that completes quickly
    let order_clone = shutdown_order.clone();
    spawner.spawn(async move {
        order_clone.lock().unwrap().push("quick_task");
    });

    // Task that takes a bit longer
    let order_clone = shutdown_order.clone();
    spawner.spawn(async move {
        for _ in 0..100 {
            std::future::ready(()).await;
        }
        order_clone.lock().unwrap().push("slow_task");
    });

    let start_time = Instant::now();

    let runtime_handle = thread::spawn(move || {
        runtime.run();
    });

    // Drop the spawner to signal shutdown
    drop(spawner);

    // Runtime should shutdown gracefully
    assert!(runtime_handle.join().is_ok());

    let elapsed = start_time.elapsed();
    let final_order = shutdown_order.lock().unwrap();

    // Both tasks should have completed
    assert_eq!(final_order.len(), 2);
    assert!(final_order.contains(&"quick_task".to_string()));
    assert!(final_order.contains(&"slow_task".to_string()));

    // Shutdown should be reasonably fast
    assert!(elapsed < Duration::from_secs(2));

    println!("Runtime shutdown gracefully in {:?}", elapsed);
}
