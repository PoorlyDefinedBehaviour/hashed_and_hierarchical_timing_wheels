#![feature(binary_heap_retain)]
#![feature(drain_filter)]

use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

struct DoublyLinkedList<T> {
    dummy_head: *mut Node<T>,
    dummy_tail: *mut Node<T>,
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

    fn is_empty(&self) -> bool {
        unsafe { (*self.dummy_head).next == self.dummy_tail }
    }

    fn head(&self) -> *mut Node<T> {
        unsafe { (*self.dummy_head).next }
    }

    fn remove(&mut self, node: *mut Node<T>) {
        unsafe {
            let previous = (*node).previous;
            let next = (*node).next;
            (*previous).next = next;
            (*next).previous = previous;
            let _ = Box::from_raw(node);
        }
    }

    fn insert_after(&mut self, node: *mut Node<T>, value: T) {
        let new_node = Box::into_raw(Box::new(Node {
            value: Some(value),
            previous: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
        }));

        unsafe {
            let next = (*node).next;

            (*new_node).previous = node;
            (*node).next = new_node;

            (*next).previous = node;
            (*new_node).next = next;
        }
    }

    fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut::new(self)
    }
}

struct IterMut<'a, T> {
    current: *mut Node<T>,
    _list: &'a mut DoublyLinkedList<T>,
}

impl<'a, T> IterMut<'a, T> {
    fn new(list: &'a mut DoublyLinkedList<T>) -> Self {
        Self {
            current: list.head(),
            _list: list,
        }
    }
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = *mut Node<T>;

    fn next(&mut self) -> Option<*mut Node<T>> {
        if self.current.is_null() {
            None
        } else {
            let node = self.current;
            unsafe { self.current = (*self.current).next };
            Some(node)
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

pub struct Registry {
    state: Mutex<State>,
}

pub struct State {
    next_timer_id: usize,
    current_time: u64,
    buckets: Vec<DoublyLinkedList<Timer>>,
}

const NUM_BUCKETS: usize = 256;

fn lowest_8_bits(n: u32) -> u32 {
    n & 0xFF
}

fn highest_24_bits(n: u32) -> u32 {
    n & 0xFFFFFF00
}

impl Registry {
    pub fn new() -> Arc<Self> {
        let mut buckets = Vec::new();
        buckets.resize_with(NUM_BUCKETS, DoublyLinkedList::new);

        let registry = Arc::new(Self {
            state: Mutex::new(State {
                next_timer_id: 0,
                current_time: 0,
                buckets,
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

        let expires_in_as_seconds = expires_in.as_secs() as u32;

        let highest_24_bits = highest_24_bits(expires_in_as_seconds);
        let lowest_8_bits = lowest_8_bits(expires_in_as_seconds);

        // TODO: if the number of seconds that the time should wait before expiring
        // is greater than the number of buckets, the timer should go to a overflow list.
        let bucket_position =
            (state.current_time + lowest_8_bits as u64) as usize % state.buckets.len();

        let bucket = &mut state.buckets[bucket_position];

        insert_node_in_list(
            bucket,
            Timer {
                id: timer_id,
                highest_24_bits,
                expire_action: Some(Box::new(expire_action)),
            },
        );

        TimerHandle {
            bucket_position,
            timer_id,
        }
    }

    pub fn stop_timer(&self, timer_handle: &TimerHandle) {
        let mut state = self.state.lock().unwrap();

        let bucket = &mut state.buckets[timer_handle.bucket_position];

        let mut node_to_remove = None;

        for node in bucket.iter_mut() {
            unsafe {
                if (*node).value.as_ref().unwrap().id == timer_handle.timer_id {
                    node_to_remove = Some(node);
                    break;
                }
            }
        }

        if let Some(node) = node_to_remove {
            bucket.remove(node);
        }
    }

    pub fn expire_timers(&self) {
        let mut state = self.state.lock().unwrap();

        state.current_time = (state.current_time + 1) % state.buckets.len() as u64;

        let bucket_index = state.current_time as usize;

        let current_time_highest_24_bits = highest_24_bits(state.current_time as u32);

        let bucket = &mut state.buckets[bucket_index];

        unsafe {
            let mut current = bucket.head();

            while current != bucket.dummy_tail {
                let timer = (*current).value.as_mut().unwrap();

                if timer.highest_24_bits != current_time_highest_24_bits {
                    break;
                }

                let node = current;
                current = (*current).next;

                let f = (timer.expire_action.take()).unwrap();

                (f)();

                bucket.remove(node);
            }
        }
    }
}

fn insert_node_in_list(list: &mut DoublyLinkedList<Timer>, timer: Timer) {
    let node = find_node_to_insert_timer_after(list, timer.highest_24_bits);
    list.insert_after(node, timer);
}

fn find_node_to_insert_timer_after(
    list: &mut DoublyLinkedList<Timer>,
    highest_24_bits: u32,
) -> *mut Node<Timer> {
    if list.is_empty() {
        list.dummy_head
    } else {
        for node in list.iter_mut() {
            unsafe {
                let node_highest_24_bits = (*node).value.as_ref().unwrap().highest_24_bits;
                match node_highest_24_bits.cmp(&highest_24_bits) {
                    std::cmp::Ordering::Less => { /* no-op */ }
                    std::cmp::Ordering::Equal => return node,
                    std::cmp::Ordering::Greater => return (*node).previous,
                }
            }
        }

        unreachable!()
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
    highest_24_bits: u32,
    expire_action: Option<Box<ExpireAction>>,
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

        std::thread::sleep(Duration::from_secs(5));
    }
}
