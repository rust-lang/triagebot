use std::{collections::VecDeque, sync::Arc};

pub trait EstimatedSize {
    fn estimated_size(&self) -> usize;
}

/// Simple LRU cache.
///
/// Evicts the Least Recently Used entry when space is needed.
pub struct LeastRecentlyUsedCache<K, V> {
    size: usize,
    capacity: usize,
    entries: VecDeque<(K, Arc<V>)>,
}

impl<K, V> LeastRecentlyUsedCache<K, V> {
    /// Creates the cache with a maxmimum capacity
    pub fn new(capacity: usize) -> Self {
        LeastRecentlyUsedCache {
            size: 0,
            capacity,
            entries: VecDeque::default(),
        }
    }
}

#[cfg(test)]
impl<K, V> Default for LeastRecentlyUsedCache<K, V> {
    fn default() -> Self {
        Self::new(1 * 1024 * 1024)
    }
}

impl<K: PartialEq<K>, V: EstimatedSize> LeastRecentlyUsedCache<K, V> {
    /// Get a handle to the value.
    ///
    /// Also move the entry in the cache to the first place.
    pub(crate) fn get(&mut self, key: &K) -> Option<Arc<V>> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            // Move previously cached entry to the front
            let entry = self.entries.remove(pos).unwrap();
            self.entries.push_front(entry);
            Some(self.entries[0].1.clone())
        } else {
            None
        }
    }

    /// Inserts a new value to the cache
    pub(crate) fn put(&mut self, key: K, value: Arc<V>) -> Arc<V> {
        let estimated_size = value.estimated_size();

        if estimated_size > self.capacity {
            // Entry is too large, don't cache, return as is
            return value;
        }

        // Remove duplicate or last entry when necessary
        let removed = if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(pos)
        } else if self.size + estimated_size >= self.capacity {
            self.entries.pop_back()
        } else {
            None
        };
        if let Some(removed) = removed {
            self.size -= removed.1.estimated_size();
        }

        // Add entry the front of the list and return it
        self.size += estimated_size;
        self.entries.push_front((key, value.clone()));
        value
    }

    /// Removes a value from the cache
    pub(crate) fn prune(&mut self, key: &K) -> bool {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            let entry = self.entries.remove(pos).unwrap();
            self.size -= entry.1.estimated_size();
            true
        } else {
            false
        }
    }
}
