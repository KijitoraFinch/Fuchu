use crossbeam::channel;
use std::{
    collections::{BTreeMap, HashMap},
    os::fd::RawFd,
    sync::{atomic::{AtomicUsize, Ordering}, Arc},
    task::Waker,
    time::{Duration, Instant},
};

pub enum RegistryRequest {
    RegisterFd {
        fd: RawFd,
        waker: Waker,
    },
    UnregisterFd {
        fd: RawFd,
    },
    RegisterTimer {
        timer: Instant,
        id: usize,
        waker: Waker,
    },
    UnregisterTimer {
        id: usize,
    },
}

pub struct Reactor {
    waker_lookup: HashMap<RawFd, Waker>,
    timeout_lookup: BTreeMap<Instant, Vec<(usize, Waker)>>,
    registry_channel: channel::Receiver<RegistryRequest>,
}

impl Reactor {
    pub fn new() -> (Self, channel::Sender<RegistryRequest>) {
        let (sender, receiver) = channel::unbounded();
        (
            Reactor {
                waker_lookup: HashMap::new(),
                timeout_lookup: BTreeMap::new(),
                registry_channel: receiver,
            },
            sender,
        )
    }

    pub fn run(&mut self) {
        let mut poll = mio::Poll::new().unwrap();
        let mut events = mio::Events::with_capacity(1024);

        loop {
            while let Ok(request) = self.registry_channel.try_recv() {
                match request {
                    RegistryRequest::RegisterFd { fd, waker } => {
                        let mut source = mio::unix::SourceFd(&fd);
                        let token = mio::Token(fd as usize);
                        poll.registry()
                            .register(
                                &mut source,
                                token,
                                mio::Interest::READABLE | mio::Interest::WRITABLE,
                            )
                            .or_else(|e| {
                                if e.kind() == std::io::ErrorKind::AlreadyExists {
                                    Ok(())
                                } else {
                                    Err(e)
                                }
                            })
                            .unwrap();
                        self.waker_lookup.insert(fd, waker);
                    }
                    RegistryRequest::UnregisterFd { fd } => {
                        let mut source = mio::unix::SourceFd(&fd);
                        // The fd may be closed by the time we process this, so ignore the result.
                        let _ = poll.registry().deregister(&mut source);
                        self.waker_lookup.remove(&fd);
                    }
                    RegistryRequest::RegisterTimer { timer, id, waker } => {
                        self.timeout_lookup
                            .entry(timer)
                            .or_insert_with(Vec::new)
                            .push((id, waker));
                    }
                    RegistryRequest::UnregisterTimer { id } => {
                        let mut entries_to_remove = Vec::new();
                        for (instant, timers) in &mut self.timeout_lookup {
                            timers.retain(|(timer_id, _)| *timer_id != id);
                            if timers.is_empty() {
                                entries_to_remove.push(*instant);
                            }
                        }
                        for instant in entries_to_remove {
                            self.timeout_lookup.remove(&instant);
                        }
                    }
                }
            }

            poll.poll(&mut events, Some(Duration::from_millis(0)))
                .unwrap();

            let now = Instant::now();
            let mut expired_timers = Vec::new();
            for (timer, timer_list) in self.timeout_lookup.range(..=now) {
                for (id, waker) in timer_list {
                    expired_timers.push((timer.clone(), *id, waker.clone()));
                }
            }
            let mut entries_to_remove = Vec::new();
            for (timer, _timer_list) in &mut self.timeout_lookup {
                if *timer <= now {
                    entries_to_remove.push(*timer);
                }
            }
            for timer in entries_to_remove {
                self.timeout_lookup.remove(&timer);
            }
            for (_, _, waker) in expired_timers {
                waker.wake();
            }

            for event in events.iter() {
                let fd = event.token().0 as RawFd;
                if let Some(waker) = self.waker_lookup.get(&fd) {
                    if event.is_readable() || event.is_writable() {
                        waker.wake_by_ref();
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct ReactorHandle {
    sender: channel::Sender<RegistryRequest>,
    timer_id_counter: Arc<AtomicUsize>,
}

impl ReactorHandle {
    pub fn new(sender: channel::Sender<RegistryRequest>) -> Self {
        ReactorHandle {
            sender,
            timer_id_counter: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn register(&self, fd: RawFd, waker: Waker) {
        let request = RegistryRequest::RegisterFd { fd, waker };
        self.sender
            .send(request)
            .expect("Failed to send register request");
    }

    pub fn unregister(&self, fd: RawFd) {
        let request = RegistryRequest::UnregisterFd { fd };
        self.sender
            .send(request)
            .expect("Failed to send unregister request");
    }

    pub fn register_timer(&self, timer: Instant, waker: Waker) -> usize {
        let id = self.timer_id_counter.fetch_add(1, Ordering::SeqCst);
        let request = RegistryRequest::RegisterTimer { timer, id, waker };
        self.sender
            .send(request)
            .expect("Failed to send register timer request");
        id
    }

    pub fn unregister_timer(&self, id: usize) {
        let request = RegistryRequest::UnregisterTimer { id };
        self.sender
            .send(request)
            .expect("Failed to send unregister timer request");
    }
}
