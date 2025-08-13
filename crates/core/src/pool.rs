use std::sync::Arc;

use dashmap::DashSet;
use nomad_types::Signal;
use rand::Rng;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct SignalPool {
    set: Arc<DashSet<Arc<Signal>>>,
    list: Arc<Mutex<Vec<Arc<Signal>>>>,
}

impl SignalPool {
    /// Insert a signal into the pool, returning true if not duplicated
    pub async fn insert(&self, signal: Signal) -> bool {
        let signal = Arc::new(signal);
        if self.set.insert(signal.clone()) {
            false
        } else {
            self.list.lock().await.push(signal);
            true
        }
    }

    /// Sample and remove a random signal from the pool
    pub async fn sample(&self) -> Signal {
        let mut rng = rand::rng();
        let mut list = self.list.lock().await;
        let idx = rng.random_range(0..list.len());
        let signal = list.swap_remove(idx);
        self.set.remove(&signal);
        Arc::into_inner(signal).unwrap()
    }
}
