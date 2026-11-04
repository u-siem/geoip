# usiem-geoip

GeoIP and ASN enrichment for [uSIEM](../usiem) events, from MaxMind DB (`.mmdb`) files. It works with MaxMind GeoIP2 and GeoLite2, and with DB-IP Lite.

```rust
use std::sync::Arc;
use usiem_geoip::{GeoEnricher, GeoHandle, Kind, MmdbFile, MmdbGeo};

// Validate once, when a file arrives: kind, full tree, sample lookups.
let city = Arc::new(MmdbFile::open("dbip-city-lite.mmdb", Kind::City)?);
let asn = Arc::new(MmdbFile::open("dbip-asn-lite.mmdb", Kind::Asn)?);
let handle = Arc::new(GeoHandle::new(MmdbGeo::empty().with(city).with(asn)));

// One enricher per worker thread.
let mut enricher = GeoEnricher::new(handle.clone());
enricher.refresh();            // once per batch: picks up swapped databases
enricher.enrich(&mut event);   // writes source|destination.geo.* and .as.*
```

- **`MmdbFile`**: a memory-mapped, validated database. `open` runs the full check. `open_installed` checks only the kind and is meant for files that already passed `open`. Never modify a file while it is open; install new versions under new names.
- **`MmdbGeo`**: a City database plus an optional ASN database. It implements uSIEM's `NetworkDataset<GeoIp>` and `GeoIpDataset`. IPv4-mapped IPv6 addresses are looked up as IPv4.
- **`GeoHandle`**: the live databases. `swap` and `update` replace them atomically. Readers pay one atomic load per batch.
- **`GeoEnricher`**: the per-worker step.
  - Writes ECS fields: `geo.country_iso_code`, `geo.country_name`, `geo.city_name`, `geo.location.lat`, `geo.location.lon`, `as.number` and `as.organization.name`.
  - Skips non-public addresses (`usiem::processing::lookup::is_non_public_addr`) and keeps fields that are already set.
  - Caches up to 4,096 addresses.

Everything is synchronous, with no runtime, so lookups can run inline in an ingest pipeline's worker threads while a background job replaces the database files.

## Data

DB-IP Lite databases are licensed CC BY 4.0, so attribute them: "IP Geolocation by DB-IP" (<https://db-ip.com>). GeoLite2 needs a MaxMind account and has its own EULA.

The test databases in `tests/data` come from [maxmind/MaxMind-DB](https://github.com/maxmind/MaxMind-DB) and are licensed MIT or Apache-2.0.
