//! Fixtures are MaxMind's test databases (tests/data, MIT or Apache-2.0,
//! from github.com/maxmind/MaxMind-DB).

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use usiem::event::{CoreEvent, FieldValue, Ip, Origin, RawData, Timestamp};
use usiem::processing::enrichment::{GeoIpDataset, NetworkDataset};
use usiem_geoip::{GeoEnricher, GeoError, GeoHandle, Kind, MmdbFile, MmdbGeo};

const CITY: &str = "tests/data/GeoIP2-City-Test.mmdb";
const ASN: &str = "tests/data/GeoLite2-ASN-Test.mmdb";
const COUNTRY: &str = "tests/data/GeoIP2-Country-Test.mmdb";

fn both() -> MmdbGeo {
    let city = Arc::new(MmdbFile::open(CITY, Kind::City).expect("city test db"));
    let asn = Arc::new(MmdbFile::open(ASN, Kind::Asn).expect("asn test db"));
    MmdbGeo::empty().with(city).with(asn)
}

fn ip(s: &str) -> IpAddr {
    s.parse().expect("test address")
}

fn event(fields: &[(&str, FieldValue)]) -> CoreEvent {
    let now = Timestamp::now();
    let mut e = CoreEvent::new(
        now,
        now,
        Origin::from_hostname("t"),
        RawData::Text("x".into()),
    );
    for (k, v) in fields {
        e.add_field(k.to_string(), v.clone());
    }
    e
}

fn text(e: &CoreEvent, key: &str) -> Option<String> {
    match e.get_field(key)? {
        FieldValue::Str(s) => Some(s.to_string()),
        other => panic!("{key} is {other:?}"),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("usiem-geoip-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn city_and_asn_merge() {
    let geo = both();
    let se = geo.lookup(ip("89.160.20.128")).unwrap();
    assert_eq!(se.country_code.as_deref(), Some("SE"));
    assert_eq!(se.city.as_deref(), Some("Linköping"));
    assert_eq!(se.asn, Some(29518));
    assert_eq!(se.organization.as_deref(), Some("Bredband2 AB"));

    let asn_only = geo.lookup(ip("1.128.0.1")).unwrap();
    assert_eq!(asn_only.asn, Some(1221));
    assert_eq!(asn_only.country_code, None);

    assert!(geo.lookup(ip("8.8.8.8")).is_none());
    assert_eq!(geo.lookup_errors(), 0);
}

#[test]
fn ipv4_mapped_ipv6_reads_as_ipv4() {
    let geo = both();
    let mapped = geo.lookup(ip("::ffff:81.2.69.142")).unwrap();
    assert_eq!(mapped.city.as_deref(), Some("London"));
    assert_eq!(
        geo.lookup(ip("2001:218::1"))
            .unwrap()
            .country_code
            .as_deref(),
        Some("JP")
    );
}

#[test]
fn implements_usiem_dataset_traits() {
    let geo = both();
    assert!(!NetworkDataset::is_empty(&geo));
    assert!(geo.network_count() > 0);
    assert!(geo.lookup_geo(&ip("81.2.69.142")).is_some());
    assert!(NetworkDataset::is_empty(&MmdbGeo::empty()));
}

#[test]
fn validation_rejects_wrong_kind_and_garbage() {
    assert!(matches!(
        MmdbFile::open(COUNTRY, Kind::City),
        Err(GeoError::WrongKind { .. })
    ));
    assert!(matches!(
        MmdbFile::open(CITY, Kind::Asn),
        Err(GeoError::WrongKind { .. })
    ));

    let dir = scratch("garbage");
    let junk = dir.join("junk.mmdb");
    std::fs::write(&junk, b"<html>not found</html>").unwrap();
    assert!(matches!(
        MmdbFile::open(&junk, Kind::City),
        Err(GeoError::Open(_))
    ));

    // A truncated download: the metadata marker sits at the end, so cut
    // from the middle and keep the tail.
    let bytes = std::fs::read(CITY).unwrap();
    let mut cut = bytes[..bytes.len() / 3].to_vec();
    cut.extend_from_slice(&bytes[bytes.len() - 2048..]);
    let truncated = dir.join("cut.mmdb");
    std::fs::write(&truncated, cut).unwrap();
    assert!(MmdbFile::open(&truncated, Kind::City).is_err());

    assert!("city".parse::<Kind>().is_ok());
    assert!(matches!(
        "country".parse::<Kind>(),
        Err(GeoError::UnknownKind(_))
    ));
}

#[test]
fn enricher_writes_ecs_fields() {
    let handle = Arc::new(GeoHandle::new(both()));
    let mut enricher = GeoEnricher::new(handle);
    let mut e = event(&[
        ("source.ip", FieldValue::Str("89.160.20.128".into())),
        (
            "destination.ip",
            FieldValue::Ip(Ip::from(ip("81.2.69.142"))),
        ),
    ]);
    assert!(enricher.enrich(&mut e));
    assert_eq!(
        text(&e, "source.geo.country_iso_code").as_deref(),
        Some("SE")
    );
    assert_eq!(
        text(&e, "source.geo.country_name").as_deref(),
        Some("Sweden")
    );
    assert_eq!(
        e.get_field("source.as.number"),
        Some(&FieldValue::Int(29518))
    );
    assert_eq!(
        text(&e, "source.as.organization.name").as_deref(),
        Some("Bredband2 AB")
    );
    assert_eq!(
        text(&e, "destination.geo.city_name").as_deref(),
        Some("London")
    );
    assert_eq!(
        e.get_field("destination.geo.location.lat"),
        Some(&FieldValue::Float(51.5142))
    );
    assert!(e.get_field("destination.as.number").is_none());
}

#[test]
fn enricher_skips_private_unknown_and_existing() {
    let mut enricher = GeoEnricher::new(Arc::new(GeoHandle::new(both())));
    for private in [
        "10.1.2.3",
        "192.168.1.1",
        "127.0.0.1",
        "fe80::1",
        "100.64.0.1",
        "not-an-ip",
    ] {
        let mut e = event(&[("source.ip", FieldValue::Str(private.into()))]);
        assert!(!enricher.enrich(&mut e), "{private}");
        assert_eq!(e.fields().len(), 1);
    }
    let mut e = event(&[
        ("source.ip", FieldValue::Str("81.2.69.142".into())),
        ("source.geo.country_iso_code", FieldValue::Str("XX".into())),
    ]);
    assert!(enricher.enrich(&mut e));
    assert_eq!(
        text(&e, "source.geo.country_iso_code").as_deref(),
        Some("XX")
    );
    assert_eq!(text(&e, "source.geo.city_name").as_deref(), Some("London"));
}

#[test]
fn empty_handle_enriches_nothing() {
    let mut enricher = GeoEnricher::new(Arc::new(GeoHandle::default()));
    let mut e = event(&[("source.ip", FieldValue::Str("81.2.69.142".into()))]);
    assert!(!enricher.enrich(&mut e));
}

#[test]
fn refresh_picks_up_a_swap_and_drops_the_cache() {
    let handle = Arc::new(GeoHandle::default());
    let mut enricher = GeoEnricher::with_capacity(handle.clone(), 2);
    let probe = || event(&[("source.ip", FieldValue::Str("81.2.69.142".into()))]);

    let mut e = probe();
    assert!(!enricher.enrich(&mut e));

    let city = Arc::new(MmdbFile::open(CITY, Kind::City).unwrap());
    let generation = handle.update(|g| g.with(city.clone()));
    assert_eq!(generation, 1);
    // Not seen until the next batch boundary.
    assert!(!enricher.enrich(&mut probe()));
    enricher.refresh();
    let mut e = probe();
    assert!(enricher.enrich(&mut e));
    assert_eq!(text(&e, "source.geo.city_name").as_deref(), Some("London"));

    // Installing ASN keeps the city database.
    let asn = Arc::new(MmdbFile::open(ASN, Kind::Asn).expect("asn test db"));
    handle.update(|g| g.with(asn));
    enricher.refresh();
    let mut e = event(&[("source.ip", FieldValue::Str("89.160.20.128".into()))]);
    enricher.enrich(&mut e);
    assert_eq!(
        text(&e, "source.geo.city_name").as_deref(),
        Some("Linköping")
    );
    assert_eq!(
        e.get_field("source.as.number"),
        Some(&FieldValue::Int(29518))
    );

    // A cache of two keeps working past its capacity.
    for addr in [
        "2.125.160.216",
        "81.2.69.142",
        "89.160.20.128",
        "2001:218::1",
    ] {
        let mut e = event(&[("source.ip", FieldValue::Str(addr.into()))]);
        assert!(enricher.enrich(&mut e), "{addr}");
    }
}

#[test]
fn swapping_under_concurrent_readers() {
    let city = Arc::new(MmdbFile::open(CITY, Kind::City).unwrap());
    let asn = Arc::new(MmdbFile::open(ASN, Kind::Asn).expect("asn test db"));
    let handle = Arc::new(GeoHandle::new(MmdbGeo::empty().with(city.clone())));
    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicU64::new(0));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let handle = handle.clone();
            let stop = stop.clone();
            let reads = reads.clone();
            std::thread::spawn(move || {
                let mut enricher = GeoEnricher::new(handle);
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    enricher.refresh();
                    let mut e = event(&[("source.ip", FieldValue::Str("89.160.20.128".into()))]);
                    assert!(enricher.enrich(&mut e));
                    assert_eq!(
                        text(&e, "source.geo.country_iso_code").as_deref(),
                        Some("SE")
                    );
                    n += 1;
                    reads.fetch_add(1, Ordering::Relaxed);
                }
                n
            })
        })
        .collect();
    // Swap until every reader has seen many generations.
    let mut i = 0u64;
    while i < 200 || reads.load(Ordering::Relaxed) < 50_000 {
        i += 1;
        let geo = if i.is_multiple_of(2) {
            MmdbGeo::empty().with(city.clone()).with(asn.clone())
        } else {
            MmdbGeo::empty().with(city.clone())
        };
        handle.swap(geo);
        std::thread::yield_now();
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        assert!(r.join().unwrap() > 0);
    }
    assert_eq!(handle.generation(), i);
}
