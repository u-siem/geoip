//! A shared, atomically replaceable [`MmdbGeo`].

use crate::mmdb::MmdbGeo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// The live GeoIP databases, shared by every process worker.
///
/// Readers hold their own `Arc` snapshot and compare [`GeoHandle::generation`]
/// (one atomic load) once per batch; the lock is taken only to pick up a new
/// snapshot, so lookups never contend. A swap never blocks on lookups in
/// flight: old snapshots stay valid until their last reader drops them.
#[derive(Debug)]
pub struct GeoHandle {
    current: RwLock<(u64, Arc<MmdbGeo>)>,
    generation: AtomicU64,
}

impl Default for GeoHandle {
    fn default() -> Self {
        GeoHandle::new(MmdbGeo::empty())
    }
}

impl GeoHandle {
    pub fn new(geo: MmdbGeo) -> Self {
        GeoHandle {
            current: RwLock::new((0, Arc::new(geo))),
            generation: AtomicU64::new(0),
        }
    }

    /// Bumped by every [`GeoHandle::swap`].
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The current databases and their generation, read together.
    pub fn snapshot(&self) -> (u64, Arc<MmdbGeo>) {
        let guard = self.current.read().unwrap_or_else(|p| p.into_inner());
        (guard.0, guard.1.clone())
    }

    /// Replaces the databases; returns the new generation.
    pub fn swap(&self, geo: MmdbGeo) -> u64 {
        self.update(|_| geo)
    }

    /// Applies `edit` to the current databases and swaps in the result,
    /// so two concurrent installs (one city, one ASN) cannot lose either.
    pub fn update(&self, edit: impl FnOnce(&MmdbGeo) -> MmdbGeo) -> u64 {
        let mut guard = self.current.write().unwrap_or_else(|p| p.into_inner());
        let next = edit(&guard.1);
        let generation = guard.0 + 1;
        *guard = (generation, Arc::new(next));
        self.generation.store(generation, Ordering::Release);
        generation
    }
}
