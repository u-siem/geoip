//! GeoIP and ASN enrichment for uSIEM events, from MaxMind DB (`.mmdb`)
//! files: MaxMind GeoIP2/GeoLite2 or DB-IP Lite.
//!
//! - [`MmdbFile::open`] validates one file (kind, tree, probe lookups).
//! - [`MmdbGeo`] queries a City and an optional ASN database together and
//!   implements usiem's [`GeoIpDataset`](usiem::processing::enrichment::GeoIpDataset).
//! - [`GeoHandle`] holds the live databases and swaps them atomically.
//! - [`GeoEnricher`] is the per-worker step that writes
//!   `source|destination.geo.*` and `.as.*`, skipping non-public addresses.
//!
//! Everything is synchronous; a lookup is a memory-mapped tree walk.

mod enrich;
mod handle;
mod mmdb;

pub use enrich::{DEFAULT_CACHE, GeoEnricher};
pub use handle::GeoHandle;
pub use mmdb::{DatasetMeta, GeoError, Kind, MmdbFile, MmdbGeo};
