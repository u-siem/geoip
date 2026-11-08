# Changelog

All notable changes to this crate are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Release tags are the bare version,
with no prefix (`0.1.0`).

## [Unreleased]

## [0.1.0]

First public release.

### Added

- `MmdbFile`: opens and validates a memory-mapped MaxMind DB file of a given
  `Kind` (City or ASN): database type, full-file verification and probe
  lookups. `open_installed` checks only the kind, for files already validated.
- `MmdbGeo`: a City and an optional ASN database queried together,
  implementing uSIEM's `NetworkDataset<GeoIp>` and `GeoIpDataset`.
  IPv4-mapped IPv6 addresses are looked up as IPv4; other IPv6 addresses read
  as "not found" in an IPv4-only database.
- `GeoHandle`: the live databases, replaced atomically with `swap` and
  `update`.
- `GeoEnricher`: the per-worker step that writes the ECS
  `source|destination.geo.*` and `.as.*` fields for public addresses, with a
  bounded per-worker cache.

[Unreleased]: https://github.com/u-siem/geoip/compare/0.1.0...HEAD
[0.1.0]: https://github.com/u-siem/geoip/releases/tag/0.1.0
