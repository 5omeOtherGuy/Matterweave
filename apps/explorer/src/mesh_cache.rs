//! Byte-budgeted, pin-aware CPU cache of derived meshes.
//!
//! Shared by the distance-tile stream ([`crate::landscape_tiles`]) and the
//! water stream ([`crate::landscape_water`]). Both key a derived mesh by its
//! generator identity and both need a key the window dropped to survive long
//! enough that a camera hovering on a boundary is an upload, not a
//! regeneration. A cache is generic over the key, so a second copy of the
//! eviction and pin rules is not written per mesh kind.
//!
//! Eviction is least recently used, with the key as a deterministic tie-break,
//! so the same sequence of requests and hits always evicts the same entry. A
//! pinned entry survives the trim for a bounded number of frames; only when
//! every entry is pinned does the earliest-expiring pin get evicted, so the
//! cache can always return under its byte budget.

use matterweave_core::Mesh;
use std::{collections::BTreeMap, sync::Arc};

/// Allocated payload bytes of one mesh: the vertices and indices the cache
/// actually holds, not the growth slack.
pub fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<matterweave_core::Vertex>()
        + mesh.indices.capacity() * std::mem::size_of::<u32>()
}

struct CacheEntry {
    mesh: Arc<Mesh>,
    bytes: usize,
    last_use: u64,
}

/// Byte-budgeted CPU cache of derived meshes.
pub struct MeshCache<K: Ord> {
    entries: BTreeMap<K, CacheEntry>,
    bytes: usize,
    budget: usize,
    clock: u64,
    /// Frame through which a key may not be evicted while unpinned entries
    /// remain. Bounded by the caller to the hysteresis cap.
    pinned_until: BTreeMap<K, u64>,
}

impl<K: Ord + Copy> MeshCache<K> {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            budget,
            clock: 0,
            pinned_until: BTreeMap::new(),
        }
    }

    /// Drop pins that have expired.
    pub fn begin_frame(&mut self, frame: u64) {
        self.pinned_until.retain(|_, until| *until > frame);
    }

    /// Keep `key` in the cache through `until` (exclusive) even under byte
    /// pressure, while unpinned entries remain.
    pub fn pin(&mut self, key: K, until: u64) {
        self.pinned_until.insert(key, until);
    }

    /// The cached mesh for `key`, marking it as most recently used.
    pub fn get(&mut self, key: &K) -> Option<Arc<Mesh>> {
        let clock = self.clock;
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.last_use = clock;
        Some(Arc::clone(&entry.mesh))
    }

    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Insert a mesh and return the stored handle. The handle stays valid even
    /// if the entry is trimmed immediately, so the caller can still upload it.
    pub fn insert(&mut self, key: K, mut mesh: Mesh) -> Arc<Mesh> {
        // Measure the payload the cache actually holds, not the growth slack.
        mesh.vertices.shrink_to_fit();
        mesh.indices.shrink_to_fit();
        let mesh = Arc::new(mesh);
        let bytes = mesh_bytes(&mesh);
        if let Some(old) = self.entries.insert(
            key,
            CacheEntry {
                mesh: Arc::clone(&mesh),
                bytes,
                last_use: self.clock,
            },
        ) {
            self.bytes -= old.bytes;
        }
        self.clock += 1;
        self.bytes += bytes;
        self.trim();
        mesh
    }

    /// Trim to the budget. Unpinned entries go first, least recently used;
    /// only when every entry is pinned does a pin (the earliest expiring) get
    /// evicted, so the cache can always return under its byte budget.
    fn trim(&mut self) {
        while self.bytes > self.budget {
            let unpinned = self
                .entries
                .iter()
                .filter(|(key, _)| !self.pinned_until.contains_key(key))
                .min_by_key(|(key, entry)| (entry.last_use, **key))
                .map(|(key, _)| *key);
            let victim = unpinned.or_else(|| {
                self.pinned_until
                    .iter()
                    .filter(|(key, _)| self.entries.contains_key(key))
                    .min_by_key(|(key, until)| (**until, **key))
                    .map(|(key, _)| *key)
            });
            let Some(key) = victim else {
                break;
            };
            self.pinned_until.remove(&key);
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_core::Vertex;

    fn test_mesh(vertices: usize) -> Mesh {
        Mesh {
            vertices: vec![
                Vertex {
                    position: [0.0; 3],
                    normal: [0.0, 1.0, 0.0],
                    color: [1.0; 3],
                };
                vertices
            ],
            indices: vec![0; vertices],
            revision: 0,
        }
    }

    #[test]
    fn a_pin_survives_a_trim_until_it_expires() {
        let one = mesh_bytes(&test_mesh(100));
        let mut cache = MeshCache::new(one * 2);
        let pinned = cache.insert(1u32, test_mesh(100));
        cache.insert(2, test_mesh(100));
        cache.pin(1, 30);
        cache.insert(3, test_mesh(100));
        assert!(cache.bytes() <= one * 2, "the byte budget still bounds it");
        assert!(cache.contains(&1), "the pinned mesh survived the trim");
        assert!(Arc::ptr_eq(&pinned, &cache.get(&1).unwrap()));
        assert!(!cache.contains(&2), "the unpinned LRU entry was evicted");
        cache.begin_frame(31);
        cache.get(&3).expect("3 is cached");
        cache.insert(4, test_mesh(100));
        assert!(
            !cache.contains(&1),
            "an expired pin must not block eviction"
        );
    }

    #[test]
    fn the_cache_serves_a_hit_without_rebuilding_and_evicts_lru() {
        let one = mesh_bytes(&test_mesh(100));
        let mut cache = MeshCache::new(one * 2);
        let first = cache.insert(1u32, test_mesh(100));
        cache.insert(2, test_mesh(100));
        // Touch key 1 so key 2 is the least recently used entry.
        let hit = cache.get(&1).expect("1 is cached");
        assert!(
            Arc::ptr_eq(&first, &hit),
            "a hit must reuse the stored mesh, not rebuild it"
        );
        let third = cache.insert(3, test_mesh(100));
        assert!(cache.bytes() <= one * 2, "the byte budget bounds the cache");
        assert!(cache.contains(&1) && cache.contains(&3));
        assert!(!cache.contains(&2), "the least recently used entry left");
        assert!(Arc::ptr_eq(&third, &cache.get(&3).unwrap()));
    }
}
