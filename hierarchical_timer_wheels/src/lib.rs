#![feature(binary_heap_retain)]
#![feature(drain_filter)]

use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

const SECONDS_IN_A_MINUTE: u32 = 60;
const MINUTES_IN_A_HOUR: u32 = 60;
const HOURS_IN_A_DAY: u32 = 24;

struct DoublyLinkedList<T> {
    dummy_head: *mut Node<T>,
    dummy_tail: *mut Node<T>,
}

impl<T> Default for DoublyLinkedList<T> {
    fn default() -> Self {
        DoublyLinkedList::new()
    }
}

unsafe impl<T> Send for DoublyLinkedList<T> where T: Send {}
unsafe impl<T> Sync for DoublyLinkedList<T> where T: Sync {}

impl<T> std::fmt::Debug for DoublyLinkedList<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            let mut current = (*self.dummy_head).next;

            write!(f, "DoublyLinkedList(")?;

            let mut i = 0;

            while current != self.dummy_tail {
                if i > 0 {
                    write!(f, " -> {:?}", (*current).value.as_ref().unwrap())?;
                } else {
                    write!(f, "{:?}", (*current).value.as_ref().unwrap())?;
                }

                current = (*current).next;

                i += 1;
            }

            writeln!(f, ")")?;
        }

        Ok(())
    }
}

impl<T> DoublyLinkedList<T> {
    fn new() -> Self {
        let mut list = Self {
            dummy_head: Box::into_raw(Box::new(Node {
                value: None,
                previous: std::ptr::null_mut(),
                next: std::ptr::null_mut(),
            })),
            dummy_tail: Box::into_raw(Box::new(Node {
                value: None,
                previous: std::ptr::null_mut(),
                next: std::ptr::null_mut(),
            })),
        };

        unsafe {
            (*list.dummy_head).next = list.dummy_tail;
            (*list.dummy_tail).previous = list.dummy_head;
        }

        list
    }

    fn head(&self) -> *mut Node<T> {
        unsafe { (*self.dummy_head).next }
    }

    fn remove(&mut self, node: *mut Node<T>) -> Box<Node<T>> {
        unsafe {
            let previous = (*node).previous;
            let next = (*node).next;
            (*previous).next = next;
            (*next).previous = previous;
            Box::from_raw(node)
        }
    }

    fn push_back(&mut self, value: T) -> *mut Node<T> {
        unsafe {
            let node = Box::into_raw(Box::new(Node {
                value: Some(value),
                previous: std::ptr::null_mut(),
                next: std::ptr::null_mut(),
            }));

            let previous = (*self.dummy_tail).previous;
            (*node).previous = previous;
            (*previous).next = node;
            (*self.dummy_tail).previous = node;
            (*node).next = self.dummy_tail;

            node
        }
    }

    fn iter_mut(&mut self) -> IterMut<T> {
        IterMut::new(self.head())
    }
}

struct IterMut<T> {
    current: *mut Node<T>,
}

impl<T> IterMut<T> {
    fn new(head: *mut Node<T>) -> Self {
        Self { current: head }
    }
}

impl<T> Iterator for IterMut<T> {
    type Item = *mut Node<T>;

    fn next(&mut self) -> Option<*mut Node<T>> {
        unsafe {
            if (*self.current).is_head() || (*self.current).is_tail() {
                None
            } else {
                let node = self.current;
                self.current = (*self.current).next;
                Some(node)
            }
        }
    }
}

impl<T> Drop for DoublyLinkedList<T> {
    fn drop(&mut self) {
        unsafe {
            let mut current = (*self.dummy_head).next;

            while current != self.dummy_tail {
                let node = current;
                current = (*current).next;
                let _ = Box::from_raw(node);
            }
        }
    }
}

struct Node<T> {
    value: Option<T>,
    previous: *mut Node<T>,
    next: *mut Node<T>,
}

impl<T> Node<T> {
    fn is_head(&self) -> bool {
        self.previous.is_null()
    }

    fn is_tail(&self) -> bool {
        self.next.is_null()
    }
}

pub struct Registry {
    state: Mutex<State>,
}

pub struct State {
    clocks: Clocks,
    buckets: Buckets,
}

struct Clocks {
    /// The current second.
    second: u32,
    /// The current minute.
    minute: u32,
    /// The current hour.
    hour: u32,
}

impl Clocks {
    fn new() -> Self {
        Self {
            second: 0,
            minute: 0,
            hour: 0,
        }
    }
}

struct Buckets {
    seconds: [DoublyLinkedList<Timer>; 60],
    minutes: [DoublyLinkedList<Timer>; 60],
    hours: [DoublyLinkedList<Timer>; 24],
}

impl Buckets {
    fn new() -> Self {
        Self {
            seconds: [(); 60].map(|_| DoublyLinkedList::new()),
            minutes: [(); 60].map(|_| DoublyLinkedList::new()),
            hours: [(); 24].map(|_| DoublyLinkedList::new()),
        }
    }
}

impl Registry {
    pub fn new() -> Arc<Self> {
        let registry = Arc::new(Self {
            state: Mutex::new(State {
                clocks: Clocks::new(),
                buckets: Buckets::new(),
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

        let expires_in_as_seconds = expires_in.as_secs() as u32;

        let (seconds, minutes, hours) = time_components(expires_in_as_seconds);

        let timer = Timer {
            seconds,
            minutes,
            hours,
            expire_action: Some(Box::new(expire_action)),
        };

        let node = if timer.hours > 0 {
            let index = timer.hours as usize;
            state.buckets.hours[index].push_back(timer)
        } else if timer.minutes > 0 {
            let index = timer.minutes as usize;
            state.buckets.minutes[index].push_back(timer)
        } else {
            let index = timer.seconds as usize;
            state.buckets.seconds[index].push_back(timer)
        };

        TimerHandle { node }
    }

    pub fn stop_timer(&self, timer_handle: &TimerHandle) {
        let mut state = self.state.lock().unwrap();

        let timer = unsafe { (*timer_handle.node).value.as_ref().unwrap() };
        if timer.hours > 0 {
            state.buckets.hours[timer.hours as usize].remove(timer_handle.node);
        } else if timer.minutes > 0 {
            state.buckets.minutes[timer.minutes as usize].remove(timer_handle.node);
        } else {
            state.buckets.seconds[timer.seconds as usize].remove(timer_handle.node);
        }
    }

    pub fn expire_timers(&self) {
        let mut state = self.state.lock().unwrap();

        let index = state.clocks.second as usize;
        let iter = state.buckets.seconds[index].iter_mut();
        for node in iter {
            let node = state.buckets.seconds[index].remove(node);
            let timer = node.value.unwrap();
            timer.expire_action.unwrap()();
        }

        state.clocks.second = (state.clocks.second + 1) % SECONDS_IN_A_MINUTE;
        // If 1 minute has not passed yet.
        if state.clocks.second > 0 {
            return;
        }

        state.clocks.minute = (state.clocks.minute + 1) % MINUTES_IN_A_HOUR;
        let index = state.clocks.minute as usize;
        let iter = state.buckets.minutes[index].iter_mut();
        for node in iter {
            let node = state.buckets.minutes[index].remove(node);
            let timer = node.value.unwrap();

            // Timer has expired.
            if timer.seconds == 0 {
                timer.expire_action.unwrap()();
            } else {
                // The timer will expire in the future so we schedule it again
                // but in a different bucket.
                let index = timer.seconds as usize;
                state.buckets.seconds[index].push_back(timer);
            }
        }

        // If 1 hour has not passed yet.
        if state.clocks.minute > 0 {
            return;
        }

        state.clocks.hour = (state.clocks.hour + 1) % HOURS_IN_A_DAY;
        let index = state.clocks.hour as usize;
        let iter = state.buckets.hours[index].iter_mut();
        for node in iter {
            let node = state.buckets.minutes[index].remove(node);
            let timer = node.value.unwrap();

            // Timer has expired.
            if timer.minutes == 0 && timer.seconds == 0 {
                timer.expire_action.unwrap()();
            } else if timer.minutes > 0 {
                let index = timer.minutes as usize;
                state.buckets.minutes[index].push_back(timer);
            } else {
                let index = timer.seconds as usize;
                state.buckets.seconds[index].push_back(timer);
            }
        }
    }
}

fn time_components(secs: u32) -> (u32, u32, u32) {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    (seconds, minutes, hours)
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
    seconds: u32,
    minutes: u32,
    hours: u32,
    expire_action: Option<Box<ExpireAction>>,
}

/// Can be used to interact with a Timer after it has been registered.
/// Could be used to cancel a timer for example.
pub struct TimerHandle {
    /// Node pointing to the timer in the bucket.
    node: *mut Node<Timer>,
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn simple() {
        let registry = Registry::new();

        let start = Instant::now();
        registry.start_timer(Duration::from_secs(1), move || {
            println!("expired 1 sec. time={:?}", start.elapsed());
        });

        let start = Instant::now();
        registry.start_timer(Duration::from_secs(3), move || {
            println!("expired 3 sec. time={:?}", start.elapsed());
        });

        registry.start_timer(Duration::from_secs(1), move || {
            println!("expired 1 sec 2. time={:?}", start.elapsed());
        });

        registry.start_timer(Duration::from_secs(61), move || {
            println!("expired 61 sec. time={:?}", start.elapsed());
        });

        std::thread::sleep(Duration::from_secs(120));
    }
}
