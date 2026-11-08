//! A shared, atomically replaceable [`MmdbGeo`].

use crate::mmdb::MmdbGeo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

/// The live GeoIP databases, shared by every worker.
///
/// Readers hold their own `Arc` snapshot and compare [`GeoHandle::generation`]
/// (one atomic load) once per batch; the lock is taken only to pick up a new
/// snapshot, so lookups never contend. A swap never blocks on lookups in
/// flight: old snapshots stay valid until their last reader drops them.
///
/// A panic inside an [`update`](GeoHandle::update) closure leaves the
/// current databases in place, and the handle stays usable.
#[derive(Debug)]
pub struct GeoHandle {
    current: RwLock<(u64, Arc<MmdbGeo>)>,
    generation: AtomicU64,
}

impl Default for GeoHandle {
    /// A handle with no databases ([`MmdbGeo::empty`]), at generation 0.
    fn default() -> Self {
        GeoHandle::new(MmdbGeo::empty())
    }
}

impl GeoHandle {
    /// A handle serving `geo`, at generation 0.
    pub fn new(geo: MmdbGeo) -> Self {
        GeoHandle {
            current: RwLock::new((0, Arc::new(geo))),
            generation: AtomicU64::new(0),
        }
    }

    /// Bumped by every [`GeoHandle::swap`] and [`GeoHandle::update`].
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The current databases and their generation, read together.
    pub fn snapshot(&self) -> (u64, Arc<MmdbGeo>) {
        let guard = self.current.read().unwrap_or_else(PoisonError::into_inner);
        (guard.0, guard.1.clone())
    }

    /// Replaces the databases; returns the new generation.
    pub fn swap(&self, geo: MmdbGeo) -> u64 {
        self.update(|_| geo)
    }

    /// Applies `edit` to the current databases and swaps in the result,
    /// so two concurrent installs (one City, one ASN) cannot lose either.
    /// Returns the new generation.
    ///
    /// `edit` runs under the write lock: keep it cheap (the
    /// [`MmdbGeo::with`] and [`MmdbGeo::without`] copies are), and do not
    /// call back into this handle from it, which would deadlock.
    pub fn update(&self, edit: impl FnOnce(&MmdbGeo) -> MmdbGeo) -> u64 {
        let mut guard = self.current.write().unwrap_or_else(PoisonError::into_inner);
        let next = edit(&guard.1);
        let generation = guard.0 + 1;
        *guard = (generation, Arc::new(next));
        self.generation.store(generation, Ordering::Release);
        generation
    }
}
