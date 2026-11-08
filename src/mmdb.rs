//! Opening, validating and querying MaxMind DB files.

use maxminddb::{MaxMindDbError, Mmap, Reader};
use serde::Deserialize;
use std::borrow::Cow;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use usiem::event::field::Text;
use usiem::processing::enrichment::{GeoIp, GeoIpDataset, NetworkDataset};

/// Which kind of database a file must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Country, city and coordinates (GeoIP2/GeoLite2 City, DB-IP City Lite).
    City,
    /// Autonomous system number and organization (GeoLite2 ASN, DB-IP ASN Lite).
    Asn,
}

impl Kind {
    /// Every kind, in slot order.
    pub const ALL: [Kind; 2] = [Kind::City, Kind::Asn];

    /// The kind's name as [`FromStr`] parses it: `city` or `asn`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::City => "city",
            Kind::Asn => "asn",
        }
    }

    /// Whether a `database_type` metadata string is this kind. Matches
    /// MaxMind (`GeoIP2-City`, `GeoLite2-ASN`) and DB-IP
    /// (`DBIP-City-Lite`, `DBIP-ASN-Lite (compat=GeoLite2-ASN)`) names.
    pub fn accepts(self, database_type: &str) -> bool {
        let lower = database_type.to_ascii_lowercase();
        match self {
            Kind::City => lower.contains("city"),
            Kind::Asn => lower.contains("asn"),
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = GeoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "city" => Ok(Kind::City),
            "asn" => Ok(Kind::Asn),
            other => Err(GeoError::UnknownKind(other.to_string())),
        }
    }
}

/// What a validated file contains, from its metadata section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DatasetMeta {
    /// The kind the file was opened as.
    pub kind: Kind,
    /// The file's own `database_type`, e.g. `GeoLite2-City`.
    pub database_type: String,
    /// When the database was built, in seconds since the Unix epoch.
    pub build_epoch: u64,
    /// Nodes in the search tree.
    pub node_count: u32,
    /// The IP version of the search tree: 4 or 6. An IPv4 database has no
    /// IPv6 data.
    pub ip_version: u16,
}

/// Why a database file was refused, or a kind name not understood.
#[derive(Debug)]
#[non_exhaustive]
pub enum GeoError {
    /// The file could not be read, or is not a MaxMind DB.
    Open(String),
    /// The file is a valid MaxMind DB of a different kind.
    WrongKind {
        /// The kind the caller asked for.
        expected: Kind,
        /// The file's own `database_type`.
        found: String,
    },
    /// The file's search tree or data section is corrupt.
    Corrupt(String),
    /// Not `city` or `asn`.
    UnknownKind(String),
}

impl fmt::Display for GeoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeoError::Open(e) => write!(f, "not a readable MaxMind DB: {e}"),
            GeoError::WrongKind { expected, found } => {
                write!(f, "expected a {expected} database, found {found:?}")
            }
            GeoError::Corrupt(e) => write!(f, "corrupt MaxMind DB: {e}"),
            GeoError::UnknownKind(k) => write!(f, "unknown dataset kind {k:?} (city, asn)"),
        }
    }
}

impl std::error::Error for GeoError {}

/// Addresses looked up while validating: a lookup error on these means the
/// data section is unusable, even when the tree verifies.
const PROBES: [IpAddr; 4] = [
    IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
    IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)),
    IpAddr::V4(Ipv4Addr::new(81, 2, 69, 142)),
];

/// One opened, validated database file, memory-mapped.
///
/// The file must not be modified or truncated while this value (or any
/// clone of an `Arc` holding it) is alive: see the crate docs on
/// [the memory map contract](crate#the-memory-map-contract).
pub struct MmdbFile {
    reader: Reader<Mmap>,
    meta: DatasetMeta,
    path: PathBuf,
}

impl fmt::Debug for MmdbFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MmdbFile")
            .field("meta", &self.meta)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl MmdbFile {
    /// Memory-maps and validates `path` as a `kind` database: the metadata
    /// type must match, the whole file must pass `verify`, and probe
    /// lookups must decode.
    ///
    /// The file must never be modified while it is open. Callers install
    /// databases under new names and only ever delete old ones.
    ///
    /// # Errors
    ///
    /// - [`GeoError::Open`] when the file cannot be read or mapped, or is
    ///   not a MaxMind DB (no metadata section, unsupported format).
    /// - [`GeoError::WrongKind`] when its `database_type` is not `kind`.
    /// - [`GeoError::Corrupt`] when the search tree or data section fails
    ///   verification, or a probe lookup cannot be decoded as `kind`.
    pub fn open(path: impl AsRef<Path>, kind: Kind) -> Result<Self, GeoError> {
        let file = MmdbFile::open_installed(path, kind)?;
        file.reader
            .verify()
            .map_err(|e| GeoError::Corrupt(e.to_string()))?;
        for ip in PROBES {
            if file.meta.ip_version == 4 && ip.is_ipv6() {
                continue;
            }
            file.probe(ip)
                .map_err(|e| GeoError::Corrupt(format!("lookup of {ip}: {e}")))?;
        }
        Ok(file)
    }

    /// Like [`MmdbFile::open`] but only checks the kind: for files that
    /// passed `open` when they were installed. Skips the full-file
    /// `verify`, which takes seconds on a large City database.
    ///
    /// A corrupt file opened this way is not unsound: lookups into it fail,
    /// and [`MmdbGeo`] counts them in [`MmdbGeo::lookup_errors`].
    ///
    /// # Errors
    ///
    /// [`GeoError::Open`] or [`GeoError::WrongKind`], as for
    /// [`MmdbFile::open`].
    pub fn open_installed(path: impl AsRef<Path>, kind: Kind) -> Result<Self, GeoError> {
        let path = path.as_ref();
        // SAFETY: `open_mmap` is unsafe only because the mapped bytes must
        // not change while the map lives. That is this type's documented
        // caller contract (the crate docs' memory map contract): files are
        // written once under a fresh name, then only read or deleted, never
        // modified or truncated in place. Every other invariant (bounds,
        // pointers, UTF-8) is checked by `maxminddb` on each access.
        #[allow(unsafe_code, reason = "the one memory map, see SAFETY above")]
        let reader =
            unsafe { Reader::open_mmap(path) }.map_err(|e| GeoError::Open(e.to_string()))?;
        let md = reader.metadata();
        if !kind.accepts(&md.database_type) {
            return Err(GeoError::WrongKind {
                expected: kind,
                found: md.database_type.clone(),
            });
        }
        let meta = DatasetMeta {
            kind,
            database_type: md.database_type.clone(),
            build_epoch: md.build_epoch,
            node_count: md.node_count,
            ip_version: md.ip_version,
        };
        Ok(MmdbFile {
            reader,
            meta,
            path: path.to_path_buf(),
        })
    }

    /// What the file contains, from its metadata.
    pub fn meta(&self) -> &DatasetMeta {
        &self.meta
    }

    /// The path the file was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn probe(&self, ip: IpAddr) -> Result<(), MaxMindDbError> {
        let found = self.reader.lookup(ip)?;
        match self.meta.kind {
            Kind::City => found.decode::<CityRecord<'_>>().map(drop),
            Kind::Asn => found.decode::<AsnRecord<'_>>().map(drop),
        }
    }

    fn city(&self, ip: IpAddr, out: &mut GeoIp) -> Result<bool, MaxMindDbError> {
        let Some(rec) = self.reader.lookup(ip)?.decode::<CityRecord<'_>>()? else {
            return Ok(false);
        };
        let country = rec.country.or(rec.registered_country);
        if let Some(country) = country {
            out.country_code = country.iso_code.map(Text::from);
            out.country = country.names.and_then(|n| n.en).map(Text::from);
        }
        out.city = rec
            .city
            .and_then(|c| c.names)
            .and_then(|n| n.en)
            .map(Text::from);
        if let Some(loc) = rec.location {
            out.latitude = loc.latitude;
            out.longitude = loc.longitude;
        }
        Ok(true)
    }

    fn asn(&self, ip: IpAddr, out: &mut GeoIp) -> Result<bool, MaxMindDbError> {
        let Some(rec) = self.reader.lookup(ip)?.decode::<AsnRecord<'_>>()? else {
            return Ok(false);
        };
        out.asn = rec.autonomous_system_number;
        out.organization = rec.autonomous_system_organization.map(Text::from);
        Ok(true)
    }
}

// Narrow records: only the fields enrichment writes are decoded, and
// strings borrow from the mapped file.

#[derive(Deserialize)]
struct CityRecord<'a> {
    #[serde(borrow, default)]
    country: Option<CountryRecord<'a>>,
    #[serde(borrow, default)]
    registered_country: Option<CountryRecord<'a>>,
    #[serde(borrow, default)]
    city: Option<NamedRecord<'a>>,
    #[serde(default)]
    location: Option<LocationRecord>,
}

#[derive(Deserialize)]
struct CountryRecord<'a> {
    #[serde(borrow, default)]
    iso_code: Option<&'a str>,
    #[serde(borrow, default)]
    names: Option<Names<'a>>,
}

#[derive(Deserialize)]
struct NamedRecord<'a> {
    #[serde(borrow, default)]
    names: Option<Names<'a>>,
}

#[derive(Deserialize)]
struct Names<'a> {
    #[serde(borrow, default)]
    en: Option<&'a str>,
}

#[derive(Deserialize)]
struct LocationRecord {
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

#[derive(Deserialize)]
struct AsnRecord<'a> {
    #[serde(default)]
    autonomous_system_number: Option<u32>,
    #[serde(borrow, default)]
    autonomous_system_organization: Option<&'a str>,
}

/// A City database and an optional ASN database, queried together.
/// Cheap to clone: files are shared, so replacing one kind keeps the
/// other mapped once.
#[derive(Debug, Clone, Default)]
pub struct MmdbGeo {
    city: Option<Arc<MmdbFile>>,
    asn: Option<Arc<MmdbFile>>,
    errors: Arc<AtomicU64>,
}

impl MmdbGeo {
    /// No databases: every lookup returns `None`.
    pub fn empty() -> Self {
        MmdbGeo::default()
    }

    /// A copy with `file` installed in its kind's slot.
    pub fn with(&self, file: Arc<MmdbFile>) -> Self {
        let mut next = self.clone();
        match file.meta.kind {
            Kind::City => next.city = Some(file),
            Kind::Asn => next.asn = Some(file),
        }
        next
    }

    /// A copy with `kind`'s database removed.
    pub fn without(&self, kind: Kind) -> Self {
        let mut next = self.clone();
        match kind {
            Kind::City => next.city = None,
            Kind::Asn => next.asn = None,
        }
        next
    }

    /// The database installed in `kind`'s slot, if any.
    pub fn file(&self, kind: Kind) -> Option<&Arc<MmdbFile>> {
        match kind {
            Kind::City => self.city.as_ref(),
            Kind::Asn => self.asn.as_ref(),
        }
    }

    /// Whether no database is loaded.
    pub fn is_unloaded(&self) -> bool {
        self.city.is_none() && self.asn.is_none()
    }

    /// Lookups that failed on a decode error since this value was built.
    pub fn lookup_errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Everything known about `ip`, or `None` when no loaded database has it.
    /// Decode errors count in [`MmdbGeo::lookup_errors`] and read as "not found".
    pub fn lookup(&self, ip: IpAddr) -> Option<GeoIp> {
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            IpAddr::V4(_) => ip,
        };
        let mut out = GeoIp::default();
        let mut found = false;
        if let Some(city) = &self.city {
            found |= self.count_err(city.city(ip, &mut out));
        }
        if let Some(asn) = &self.asn {
            found |= self.count_err(asn.asn(ip, &mut out));
        }
        found.then_some(out)
    }

    fn count_err(&self, r: Result<bool, MaxMindDbError>) -> bool {
        r.unwrap_or_else(|_| {
            self.errors.fetch_add(1, Ordering::Relaxed);
            false
        })
    }
}

impl NetworkDataset<GeoIp> for MmdbGeo {
    fn lookup_by_ip(&self, ip: &IpAddr) -> Option<Cow<'_, GeoIp>> {
        self.lookup(*ip).map(Cow::Owned)
    }

    fn is_empty(&self) -> bool {
        self.is_unloaded()
    }

    /// Search-tree nodes, the closest cheap measure of size a MaxMind DB has.
    fn network_count(&self) -> usize {
        [&self.city, &self.asn]
            .into_iter()
            .flatten()
            .map(|f| usize::try_from(f.meta.node_count).unwrap_or(usize::MAX))
            .fold(0, usize::saturating_add)
    }
}

impl GeoIpDataset for MmdbGeo {}
