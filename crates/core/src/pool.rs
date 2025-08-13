use std::sync::Arc;

use dashmap::DashSet;
use rand::Rng;
use tokio::sync::{Mutex, Notify};

use nomad_types::Signal;

#[derive(Clone, Default)]
pub struct SignalPool {
    set: Arc<DashSet<Arc<Signal>>>,
    list: Arc<Mutex<Vec<Arc<Signal>>>>,
    notify: Arc<Notify>,
}

impl SignalPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a signal into the pool, returning true if not duplicated
    pub async fn insert(&self, signal: Signal) -> bool {
        let signal = Arc::new(signal);
        if self.set.insert(signal.clone()) {
            // Duplicates
            return false;
        }

        // TODO: evict oldest signals when pool is too big
        let mut list = self.list.lock().await;
        if list.is_empty() {
            // If we're pushing the first item, notify pending sample calls
            // Sample call will wait for the lock before getting a value
            self.notify.notify_waiters();
        }
        list.push(signal);
        true
    }

    /// Sample and remove a random signal from the pool
    pub async fn sample(&self) -> Signal {
        if self.set.is_empty() {
            // Wait for a signal to be pushed before sampling
            self.notify.notified().await;
        }

        // Select random signal from the list
        let mut list = self.list.lock().await;
        let idx = if list.len() == 1 {
            0
        } else {
            let mut rng = rand::rng();
            rng.random_range(0..list.len())
        };

        // Get the signal and from the pool
        let signal = list.swap_remove(idx);
        self.set.remove(&signal);

        Arc::into_inner(signal).unwrap()
    }
}
