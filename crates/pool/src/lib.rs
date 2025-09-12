use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use scc::{Bag, HashCache};
use tokio::sync::Notify;

use nomad_types::SignalPayload;

/// Concurrent, lock-free, and unordered signal pool.
///
/// Shared between the gossip layer and the main worker thread, signals are
/// inserted and then randomly processed by the node.
#[derive(Clone)]
pub struct SignalPool {
    /// Cache containing hashes of signals for rejecting duplicates
    cache: Arc<HashCache<u64, ()>>,
    /// Concurrent, lock-free, and unordered container.
    bag: Arc<Bag<SignalPayload>>,
    /// Notify handle for awaiting first signals
    notify: Arc<Notify>,
    /// Maximum size bag is allowed to grow to
    max_size: usize,
}

impl SignalPool {
    /// Create a new signal pool with a given maximum number of signals to store
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: HashCache::with_capacity(0, max_size * 8).into(),
            bag: Bag::new().into(),
            notify: Default::default(),
            max_size,
        }
    }

    /// Insert a signal into the pool, returning true if not duplicated
    pub async fn insert(&self, signal: SignalPayload) -> bool {
        // Hash signal and insert into cache
        let hasher = &mut std::hash::DefaultHasher::new();
        signal.hash(hasher);
        if self.cache.put_async(hasher.finish(), ()).await.is_err() {
            return false;
        }

        let notify = self.bag.is_empty();
        self.bag.push(signal);

        if notify {
            self.notify.notify_waiters();
        }

        // Discard random signal
        if self.bag.len() > self.max_size {
            self.bag.pop();
        }

        true
    }

    /// Sample and remove a random signal from the pool, waiting if no items are available
    pub async fn sample(&self) -> SignalPayload {
        if self.bag.is_empty() {
            self.notify.notified().await;
        }
        self.bag.pop().unwrap()
    }
}
