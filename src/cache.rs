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

        // Always remove an existing entry with the same key
        self.prune(&key);

        if estimated_size >= self.capacity / 2 {
            // Entry is too large, don't cache, return as is
            return value;
        }

        // Continuously evict LRU entries until there is enough capacity
        while !self.entries.is_empty()
            && self.size + estimated_size > self.capacity
            && let Some(removed) = self.entries.pop_back()
        {
            self.size -= removed.1.estimated_size();
        }
        debug_assert!(self.size + estimated_size <= self.capacity);

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

    /// Current size
    #[cfg(test)]
    fn size(&self) -> usize {
        self.size
    }
}

#[cfg(test)]
mod lru_tests {
    use super::*;

    struct Blob(usize);

    impl EstimatedSize for Blob {
        fn estimated_size(&self) -> usize {
            self.0
        }
    }

    #[test]
    fn mixed() {
        const CAPACITY: usize = 100usize;

        let mut cache = LeastRecentlyUsedCache::new(CAPACITY);

        cache.put("large-1".to_string(), Arc::new(Blob(40)));
        for index in 1..=5 {
            cache.put(format!("small-{index}"), Arc::new(Blob(10)));
        }
        cache.put("large-2".to_string(), Arc::new(Blob(40)));

        assert!(
            cache.size() <= CAPACITY,
            "cache size ({}) bigger than capacity ({CAPACITY})",
            cache.size()
        );
    }

    #[test]
    fn small_than_large() {
        const CAPACITY: usize = 100usize;

        let mut cache = LeastRecentlyUsedCache::new(CAPACITY);

        for index in 1..=5 {
            cache.put(format!("small-{index}"), Arc::new(Blob(10)));
        }
        cache.put("large-1".to_string(), Arc::new(Blob(40)));
        cache.put("large-2".to_string(), Arc::new(Blob(40)));

        assert!(
            cache.size() <= CAPACITY,
            "cache size ({}) bigger than capacity ({CAPACITY})",
            cache.size()
        );
    }

    #[test]
    fn larger_and_larger() {
        const CAPACITY: usize = 100usize;

        let mut cache = LeastRecentlyUsedCache::new(CAPACITY);

        for index in 1..=25 {
            let size = index * 2;
            cache.put(format!("blob-{size}"), Arc::new(Blob(size)));
            assert!(
                cache.size() <= CAPACITY,
                "cache size ({}) bigger than capacity ({CAPACITY})",
                cache.size()
            );
        }
    }

    #[test]
    fn more_than_half_capacity() {
        let mut cache = LeastRecentlyUsedCache::new(100);

        cache.put("large-1".to_string(), Arc::new(Blob(55)));
        cache.put("large-2".to_string(), Arc::new(Blob(50)));

        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn small_than_more_than_half_capacity() {
        let mut cache = LeastRecentlyUsedCache::new(100);

        cache.put("blob".to_string(), Arc::new(Blob(10)));
        cache.put("blob".to_string(), Arc::new(Blob(50)));

        assert_eq!(cache.size(), 0);
    }
}
