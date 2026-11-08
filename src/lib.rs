//! # GeoIP and ASN enrichment for uSIEM
//!
//! This crate adds country, city, coordinates and autonomous system fields to
//! [uSIEM](https://github.com/u-siem/usiem) events, from MaxMind DB
//! (`.mmdb`) files: MaxMind GeoIP2 and GeoLite2, or DB-IP Lite.
//!
//! - [`MmdbFile::open`] memory-maps one file and validates it: its
//!   [`Kind`], the whole search tree and data section, and a few probe
//!   lookups.
//! - [`MmdbGeo`] queries a City and an optional ASN database together, and
//!   implements uSIEM's [`GeoIpDataset`](usiem::processing::enrichment::GeoIpDataset).
//! - [`GeoHandle`] holds the live databases and replaces them atomically.
//! - [`GeoEnricher`] is the per-worker step that writes
//!   `source|destination.geo.*` and `.as.*`, skipping non-public addresses.
//!
//! Everything is synchronous and needs no runtime: a lookup is a walk of a
//! memory-mapped tree, so it can run inline on an ingest worker thread while
//! another thread installs new database files.
//!
//! ## Example
//!
//! ```
//! use std::sync::Arc;
//! use usiem::event::{CoreEvent, FieldValue, Origin, RawData, Timestamp};
//! use usiem_geoip::{GeoEnricher, GeoHandle, Kind, MmdbFile, MmdbGeo};
//!
//! // Validate once, when a file arrives.
//! let city = Arc::new(MmdbFile::open("tests/data/GeoIP2-City-Test.mmdb", Kind::City)?);
//! let asn = Arc::new(MmdbFile::open("tests/data/GeoLite2-ASN-Test.mmdb", Kind::Asn)?);
//! let handle = Arc::new(GeoHandle::new(MmdbGeo::empty().with(city).with(asn)));
//!
//! // One enricher per worker thread.
//! let mut enricher = GeoEnricher::new(handle.clone());
//! let now = Timestamp::now();
//! let mut event = CoreEvent::new(now, now, Origin::from_hostname("fw"), RawData::Text("".into()));
//! event.add_field("source.ip".into(), FieldValue::Str("89.160.20.128".into()));
//!
//! enricher.refresh(); // once per batch: picks up swapped databases
//! assert!(enricher.enrich(&mut event));
//! assert_eq!(
//!     event.get_field("source.geo.country_iso_code"),
//!     Some(&FieldValue::Str("SE".into()))
//! );
//! assert_eq!(event.get_field("source.as.number"), Some(&FieldValue::Int(29518)));
//! # Ok::<(), usiem_geoip::GeoError>(())
//! ```
//!
//! ## The memory map contract
//!
//! [`MmdbFile`] memory-maps its file. **The file must not be modified or
//! truncated while it is open**, by this process or any other: the map would
//! change underneath live references, which is undefined behaviour and in
//! practice a crash (`SIGBUS`) or garbage answers. Install every new version
//! under a fresh name, swap it in through [`GeoHandle`], and only ever delete
//! old files. Deleting is safe on Unix; on Windows the delete can fail while
//! the file is still mapped.
//!
//! ## Data licenses
//!
//! No database ships with this crate. DB-IP Lite is licensed CC BY 4.0 and
//! needs the attribution "IP Geolocation by DB-IP" (<https://db-ip.com>);
//! GeoLite2 needs a MaxMind account and has its own EULA.

mod enrich;
mod handle;
mod mmdb;

pub use enrich::{DEFAULT_CACHE, GeoEnricher};
pub use handle::GeoHandle;
pub use mmdb::{DatasetMeta, GeoError, Kind, MmdbFile, MmdbGeo};
