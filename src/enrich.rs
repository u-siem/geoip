//! Writing GeoIP and ASN fields into events.

use crate::handle::GeoHandle;
use crate::mmdb::MmdbGeo;
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use usiem::event::fields as f;
use usiem::event::{CoreEvent, FieldValue};
use usiem::processing::enrichment::GeoIp;
use usiem::processing::lookup::is_non_public_addr;

/// Addresses a [`GeoEnricher`] remembers before it clears its cache.
pub const DEFAULT_CACHE: usize = 4096;

/// The address fields enriched, with the fields written for each.
struct Target {
    ip: &'static str,
    country_iso_code: &'static str,
    country_name: &'static str,
    city_name: &'static str,
    lat: &'static str,
    lon: &'static str,
    as_number: &'static str,
    as_org: &'static str,
}

const TARGETS: [Target; 2] = [
    Target {
        ip: f::SOURCE_IP,
        country_iso_code: f::SOURCE_GEO_COUNTRY_ISO_CODE,
        country_name: f::SOURCE_GEO_COUNTRY_NAME,
        city_name: f::SOURCE_GEO_CITY_NAME,
        lat: f::SOURCE_GEO_LOCATION_LAT,
        lon: f::SOURCE_GEO_LOCATION_LON,
        as_number: f::SOURCE_AS_NUMBER,
        as_org: f::SOURCE_AS_ORGANIZATION_NAME,
    },
    Target {
        ip: f::DESTINATION_IP,
        country_iso_code: f::DESTINATION_GEO_COUNTRY_ISO_CODE,
        country_name: f::DESTINATION_GEO_COUNTRY_NAME,
        city_name: f::DESTINATION_GEO_CITY_NAME,
        lat: f::DESTINATION_GEO_LOCATION_LAT,
        lon: f::DESTINATION_GEO_LOCATION_LON,
        as_number: f::DESTINATION_AS_NUMBER,
        as_org: f::DESTINATION_AS_ORGANIZATION_NAME,
    },
];

/// Per-worker enrichment: one per process thread, so it needs no lock.
///
/// Call [`GeoEnricher::refresh`] once per batch to pick up swapped
/// databases, then [`GeoEnricher::enrich`] per event.
///
/// Answers, including "not found", are cached per address. When the cache
/// holds `capacity` addresses it is cleared, and a swap clears it too.
pub struct GeoEnricher {
    handle: Arc<GeoHandle>,
    generation: u64,
    geo: Arc<MmdbGeo>,
    cache: HashMap<IpAddr, Option<GeoIp>>,
    capacity: usize,
}

impl fmt::Debug for GeoEnricher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeoEnricher")
            .field("generation", &self.generation)
            .field("geo", &self.geo)
            .field("cached", &self.cache.len())
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl GeoEnricher {
    /// An enricher over `handle` with a cache of [`DEFAULT_CACHE`] addresses,
    /// starting from the handle's current databases.
    pub fn new(handle: Arc<GeoHandle>) -> Self {
        GeoEnricher::with_capacity(handle, DEFAULT_CACHE)
    }

    /// Like [`GeoEnricher::new`], with a cache of `capacity` addresses (at
    /// least one).
    pub fn with_capacity(handle: Arc<GeoHandle>, capacity: usize) -> Self {
        let (generation, geo) = handle.snapshot();
        GeoEnricher {
            handle,
            generation,
            geo,
            cache: HashMap::new(),
            capacity: capacity.max(1),
        }
    }

    /// Picks up a swapped database; one atomic load when nothing changed.
    pub fn refresh(&mut self) {
        if self.handle.generation() != self.generation {
            (self.generation, self.geo) = self.handle.snapshot();
            self.cache.clear();
        }
    }

    /// Adds `source|destination.geo.*` and `.as.*` for public addresses.
    /// Fields already on the event are left as they are. Returns whether
    /// anything was written.
    pub fn enrich(&mut self, event: &mut CoreEvent) -> bool {
        if self.geo.is_unloaded() {
            return false;
        }
        let mut wrote = false;
        for target in &TARGETS {
            let Some(ip) = event.get_field(target.ip).and_then(ip_of) else {
                continue;
            };
            if is_non_public_addr(ip) {
                continue;
            }
            if let Some(geo) = self.lookup(ip) {
                wrote |= write(event, target, geo);
            }
        }
        wrote
    }

    /// The cached answer for `ip`, looked up on a miss.
    fn lookup(&mut self, ip: IpAddr) -> Option<&GeoIp> {
        if !self.cache.contains_key(&ip) {
            if self.cache.len() >= self.capacity {
                self.cache.clear();
            }
            let found = self.geo.lookup(ip);
            self.cache.insert(ip, found);
        }
        self.cache.get(&ip).and_then(Option::as_ref)
    }
}

fn ip_of(value: &FieldValue) -> Option<IpAddr> {
    match value {
        FieldValue::Ip(ip) => Some(ip.as_ip_addr()),
        FieldValue::Str(s) => s.parse().ok(),
        _ => None,
    }
}

fn write(event: &mut CoreEvent, t: &Target, geo: &GeoIp) -> bool {
    event.reserve_fields(7);
    let mut wrote = false;
    let mut set = |key: &str, value: Option<FieldValue>| {
        if let Some(value) = value
            && event.get_field(key).is_none()
        {
            event.add_field(key.to_string(), value);
            wrote = true;
        }
    };
    set(
        t.country_iso_code,
        geo.country_code.clone().map(FieldValue::Str),
    );
    set(t.country_name, geo.country.clone().map(FieldValue::Str));
    set(t.city_name, geo.city.clone().map(FieldValue::Str));
    set(t.lat, geo.latitude.map(FieldValue::Float));
    set(t.lon, geo.longitude.map(FieldValue::Float));
    set(t.as_number, geo.asn.map(|n| FieldValue::Int(n.into())));
    set(t.as_org, geo.organization.clone().map(FieldValue::Str));
    wrote
}
