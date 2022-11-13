#![feature(binary_heap_retain)]
#![feature(drain_filter)]

use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

pub struct Registry {
    num_buckets: usize,
    state: Mutex<State>,
}

pub struct State {
    next_timer_id: usize,
    current_time: u64,
    timers: Vec<Vec<Timer>>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        let num_buckets = 100000;

        let mut timers = Vec::new();
        timers.resize_with(num_buckets, Vec::new);

        let registry = Arc::new(Self {
            num_buckets,
            state: Mutex::new(State {
                next_timer_id: 0,
                current_time: 0,
                timers,
            }),
        });
        let registry_clone = Arc::downgrade(&registry);
        std::thread::spawn(move || per_tick_bookkeeping(registry_clone));
        registry
    }

    pub fn start_timer(
        &self,
        expires_in: Duration,
        expire_action: impl FnOnce() + Send + Sync + 'static,
    ) -> TimerHandle {
        let mut state = self.state.lock().unwrap();

        let timer_id = state.next_timer_id;
        state.next_timer_id = state.next_timer_id.saturating_add(1);

        let expires_in_as_seconds = expires_in.as_secs();
        // TODO: if the number of seconds that the time should wait before expiring
        // is greater than the number of buckets, the timer should go to a overflow list.
        let bucket_position =
            (state.current_time + expires_in_as_seconds) as usize % self.num_buckets;

        state.timers[bucket_position].push(Timer {
            id: timer_id,
            expire_action: Box::new(expire_action),
        });

        TimerHandle {
            bucket_position,
            timer_id,
        }
    }

    pub fn stop_timer(&self, timer_handle: &TimerHandle) {
        let mut state = self.state.lock().unwrap();

        // TODO: this is slow but that's okay for now.
        let index = state.timers[timer_handle.bucket_position]
            .iter()
            .position(|timer| timer.id == timer_handle.timer_id);

        if let Some(index) = index {
            state.timers[timer_handle.bucket_position].remove(index);
        }
    }

    pub fn expire_timers(&self) {
        let mut state = self.state.lock().unwrap();

        state.current_time = (state.current_time + 1) % self.num_buckets as u64;

        let bucket_index = state.current_time as usize;

        let bucket = std::mem::take(&mut state.timers[bucket_index]);

        for timer in bucket.into_iter() {
            (timer.expire_action)();
        }
    }
}

pub fn per_tick_bookkeeping(registry: Weak<Registry>) {
    loop {
        std::thread::sleep(Duration::from_secs(1));

        match registry.upgrade() {
            None => {
                return;
            }
            Some(registry) => {
                registry.expire_timers();
            }
        }
    }
}

type ExpireAction = dyn FnOnce() + Send + Sync;

pub struct Timer {
    id: usize,
    expire_action: Box<ExpireAction>,
}

/// Can be used to interact with a Timer after it has been registered.
/// Could be used to cancel a timer for example.
pub struct TimerHandle {
    /// The position of the bucket that the timer has been added to.
    bucket_position: usize,
    /// The timer identifier.
    timer_id: usize,
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    pub fn simple() {
        let registry = Registry::new();

        let start = Instant::now();
        registry.start_timer(Duration::from_secs(1), move || {
            println!("expired 1 sec. time={:?}", start.elapsed());
        });

        let start = Instant::now();
        registry.start_timer(Duration::from_secs(3), move || {
            println!("expired 3 sec. time={:?}", start.elapsed());
        });

        std::thread::sleep(Duration::from_secs(5));
    }
}
