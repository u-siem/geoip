# usiem-geoip

[![CI](https://github.com/u-siem/geoip/actions/workflows/ci.yml/badge.svg)](https://github.com/u-siem/geoip/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

GeoIP and ASN enrichment for [uSIEM](https://github.com/u-siem/usiem) events, from MaxMind DB (`.mmdb`) files. It works with MaxMind GeoIP2 and GeoLite2, and with DB-IP Lite.

```toml
[dependencies]
usiem-geoip = { git = "https://github.com/u-siem/geoip", tag = "0.1.0" }
```

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

- **`MmdbFile`**: a memory-mapped, validated database. `open` runs the full check. `open_installed` checks only the kind and is meant for files that already passed `open`.
- **`MmdbGeo`**: a City database plus an optional ASN database. It implements uSIEM's `NetworkDataset<GeoIp>` and `GeoIpDataset`. IPv4-mapped IPv6 addresses are looked up as IPv4.
- **`GeoHandle`**: the live databases. `swap` and `update` replace them atomically. Readers pay one atomic load per batch.
- **`GeoEnricher`**: the per-worker step.
  - Writes ECS fields: `geo.country_iso_code`, `geo.country_name`, `geo.city_name`, `geo.location.lat`, `geo.location.lon`, `as.number` and `as.organization.name`.
  - Skips non-public addresses (`usiem::processing::lookup::is_non_public_addr`) and keeps fields that are already set.
  - Caches up to 4,096 addresses.

Everything is synchronous, with no runtime, so lookups can run inline in an ingest pipeline's worker threads while a background job replaces the database files.

## The memory map contract

Files are memory-mapped. **A file must never be modified or truncated while it is open**: on most platforms that is undefined behaviour, and in practice a crash (`SIGBUS`) or wrong answers. Install each new version under a new name, swap it in with `GeoHandle`, and only delete old files. Deleting is safe on Unix, where the mapping keeps the data alive until the last `MmdbFile` drops; on Windows the delete can fail while a file is still mapped, so retry it later.

## Data

DB-IP Lite databases are licensed CC BY 4.0, so attribute them: "IP Geolocation by DB-IP" (<https://db-ip.com>). GeoLite2 needs a MaxMind account and has its own EULA. Neither is shipped with this crate.

The test databases in `tests/data` are MaxMind's synthetic test files from [maxmind/MaxMind-DB](https://github.com/maxmind/MaxMind-DB), licensed MIT or Apache-2.0.

## Development

uSIEM is a git dependency pinned in `Cargo.toml`. To test against a local checkout in `../usiem`:

```sh
cargo test --config "patch.'https://github.com/u-siem/usiem'.usiem.path='../usiem'"
```

Don't commit the `Cargo.lock` that this produces. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT, see [LICENSE](LICENSE).
