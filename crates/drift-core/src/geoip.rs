//! Which country an address belongs to, from a small database compiled into the app (DB-IP "IP to Country
//! Lite", CC BY 4.0, see `scripts/build-geoip.mjs`). Everything is offline. The tables are unpacked the first
//! time someone asks, so a user who never opens the peers list pays nothing at start-up.

use std::io::Read;
use std::net::IpAddr;
use std::sync::OnceLock;

static EMBEDDED: &[u8] = include_bytes!("../data/country.bin");

#[derive(Default)]
struct Tables {
    /// (first address, country) sorted; each entry runs until the next one starts. Country 0 = unknown.
    v4: Vec<(u32, u16)>,
    /// Same, keyed by the top 64 bits of the address.
    v6: Vec<(u64, u16)>,
}

fn parse(gz: &[u8]) -> Option<Tables> {
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut raw).ok()?;
    let mut at = 0usize;
    let mut take = |n: usize| {
        let s = raw.get(at..at + n)?;
        at += n;
        Some(s)
    };
    let n4 = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
    let mut v4 = Vec::with_capacity(n4);
    for _ in 0..n4 {
        let e = take(6)?;
        v4.push((u32::from_be_bytes(e[..4].try_into().ok()?), u16::from_be_bytes(e[4..].try_into().ok()?)));
    }
    let n6 = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
    let mut v6 = Vec::with_capacity(n6);
    for _ in 0..n6 {
        let e = take(10)?;
        v6.push((u64::from_be_bytes(e[..8].try_into().ok()?), u16::from_be_bytes(e[8..].try_into().ok()?)));
    }
    Some(Tables { v4, v6 })
}

fn find<K: Ord + Copy>(t: &[(K, u16)], key: K) -> Option<[u8; 2]> {
    let i = t.partition_point(|(s, _)| *s <= key).checked_sub(1)?;
    let c = t[i].1;
    (c != 0).then_some([(c >> 8) as u8, c as u8])
}

pub struct GeoIp {
    source: &'static [u8],
    tables: OnceLock<Tables>,
}

impl GeoIp {
    /// The database compiled into the app.
    pub fn embedded() -> Self {
        Self::from_gz(EMBEDDED)
    }

    fn from_gz(source: &'static [u8]) -> Self {
        Self { source, tables: OnceLock::new() }
    }

    /// Two-letter country code (`"NL"`), or `None` for private, reserved or unknown addresses.
    pub fn country(&self, ip: IpAddr) -> Option<String> {
        let t = self.tables.get_or_init(|| parse(self.source).unwrap_or_default());
        let c = match ip {
            IpAddr::V4(a) => find(&t.v4, u32::from(a)),
            IpAddr::V6(a) => match a.to_ipv4_mapped() {
                Some(v4) => find(&t.v4, u32::from(v4)),
                None => find(&t.v6, (u128::from(a) >> 64) as u64),
            },
        }?;
        std::str::from_utf8(&c).ok().map(str::to_string)
    }

    /// Same, for the `ip:port` / `[v6]:port` text peers are listed with.
    pub fn country_of_addr(&self, addr: &str) -> Option<String> {
        let ip = addr.parse::<std::net::SocketAddr>().map(|s| s.ip()).or_else(|_| addr.parse::<IpAddr>()).ok()?;
        self.country(ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_database_knows_some_famous_addresses() {
        let g = GeoIp::embedded();
        assert_eq!(g.country("8.8.8.8".parse().unwrap()).as_deref(), Some("US"));
        assert_eq!(g.country_of_addr("8.8.8.8:51413").as_deref(), Some("US"));
        assert!(g.country("2001:4860:4860::8888".parse().unwrap()).is_some(), "IPv6 is covered too");
        assert_eq!(g.country("::ffff:8.8.8.8".parse().unwrap()).as_deref(), Some("US"));
        assert_eq!(g.country("192.168.1.5".parse().unwrap()), None, "private ranges are unknown");
        assert_eq!(g.country_of_addr("not an address"), None);
    }

    #[test]
    fn lookup_picks_the_range_that_contains_the_address() {
        let t = [(10u32, 0x4e4c), (20, 0), (30, 0x4445)]; // NL from 10, unknown from 20, DE from 30
        assert_eq!(find(&t, 5), None);
        assert_eq!(find(&t, 10), Some(*b"NL"));
        assert_eq!(find(&t, 19), Some(*b"NL"));
        assert_eq!(find(&t, 20), None);
        assert_eq!(find(&t, u32::MAX), Some(*b"DE"));
    }

    #[test]
    fn a_broken_database_means_no_countries_not_a_crash() {
        let g = GeoIp::from_gz(b"nonsense");
        assert_eq!(g.country("8.8.8.8".parse().unwrap()), None);
    }
}
