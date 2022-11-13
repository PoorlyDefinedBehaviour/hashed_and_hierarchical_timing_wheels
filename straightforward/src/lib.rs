use std::{
    ops::Sub,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

pub struct Registry {
    timers: Mutex<Vec<Timer>>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        let registry = Arc::new(Self {
            timers: Mutex::new(Vec::new()),
        });
        let registry_clone = Arc::downgrade(&registry);
        std::thread::spawn(move || per_tick_bookkeeping(registry_clone));
        registry
    }

    pub fn start_timer(
        &self,
        id: u64,
        interval: Duration,
        expire_action: impl FnOnce() + Send + Sync + 'static,
    ) {
        let mut timers = self.timers.lock().unwrap();
        timers.push(Timer {
            id,
            interval,
            expire_action: Box::new(expire_action),
        });
    }

    pub fn stop_timer(&self, id: u64) {
        let mut timers = self.timers.lock().unwrap();

        for i in 0..timers.len() {
            if timers[i].id == id {
                let _ = timers.remove(i);
                break;
            }
        }
    }

    pub fn expire_timers(&self) {
        let mut timers = self.timers.lock().unwrap();

        let mut to_remove = vec![];

        for (i, timer) in timers.iter_mut().enumerate() {
            timer.interval = timer.interval.sub(Duration::from_secs(1));
            if timer.interval.is_zero() {
                to_remove.push(i);
            }
        }

        for i in to_remove.into_iter() {
            let timer = timers.remove(i);
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
                registry.expire_timers();
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

type ExpireAction = dyn FnOnce() + Send + Sync;

pub struct Timer {
    id: u64,
    interval: Duration,
    expire_action: Box<ExpireAction>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    pub fn simple() {
        let registry = Registry::new();

        registry.start_timer(0, Duration::from_secs(1), || {
            println!("expired 1 sec");
        });

        registry.start_timer(1, Duration::from_secs(3), || {
            println!("expired 3 sec");
        });

        std::thread::sleep(Duration::from_secs(5));
    }
}
