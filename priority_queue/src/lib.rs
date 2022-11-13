#![feature(binary_heap_retain)]

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

pub struct Registry {
    timers: Mutex<BinaryHeap<Reverse<Timer>>>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        let registry = Arc::new(Self {
            timers: Mutex::new(BinaryHeap::new()),
        });
        let registry_clone = Arc::downgrade(&registry);
        std::thread::spawn(move || per_tick_bookkeeping(registry_clone));
        registry
    }

    pub fn start_timer(
        &self,
        id: u64,
        expires_at: Instant,
        expire_action: impl FnOnce() + Send + Sync + 'static,
    ) {
        let mut timers = self.timers.lock().unwrap();
        timers.push(Reverse(Timer {
            id,
            expires_at,
            expire_action: Box::new(expire_action),
        }));
    }

    pub fn stop_timer(&self, id: u64) {
        let mut timers = self.timers.lock().unwrap();
        timers.retain(|Reverse(timer)| timer.id != id);
    }

    pub fn expire_timers(&self, current_time: Instant) {
        let mut timers = self.timers.lock().unwrap();

        while let Some(Reverse(timer)) = timers.peek() && timer.expires_at <= current_time{
          let Reverse(timer) = timers.pop().unwrap();
          (timer.expire_action)();
        }
    }
}

pub fn per_tick_bookkeeping(registry: Weak<Registry>) {
    loop {
        match registry.upgrade() {
            None => {
                return;
            }
            Some(registry) => {
                registry.expire_timers(Instant::now());
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

type ExpireAction = dyn FnOnce() + Send + Sync;

pub struct Timer {
    id: u64,
    expires_at: Instant,
    expire_action: Box<ExpireAction>,
}

impl PartialEq for Timer {
    fn eq(&self, other: &Timer) -> bool {
        self.id == other.id
    }
}

impl Eq for Timer {}

impl PartialOrd for Timer {
    fn partial_cmp(&self, other: &Timer) -> Option<Ordering> {
        Some(self.expires_at.cmp(&other.expires_at))
    }
}

impl Ord for Timer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.expires_at.cmp(&other.expires_at)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    pub fn simple() {
        let registry = Registry::new();

        registry.start_timer(0, Instant::now() + Duration::from_secs(1), || {
            println!("expired 1 sec");
        });

        registry.start_timer(1, Instant::now() + Duration::from_secs(3), || {
            println!("expired 3 sec");
        });

        std::thread::sleep(Duration::from_secs(5));
    }
}
