//! IP filter lists: parsing (eMule `ipfilter.dat`, PeerGuardian text, plain ranges and CIDR), merging,
//! sanity checks, and a normalised text form the engine can load. Pure, no I/O.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A list that blocks more than this share of IPv4 space is almost certainly broken or hostile.
pub const MAX_V4_COVERAGE: f64 = 0.25;

/// Sorted, merged, inclusive ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ranges {
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}

fn merge<T: Ord + Copy + Increment>(mut v: Vec<(T, T)>) -> Vec<(T, T)> {
    v.sort();
    let mut out: Vec<(T, T)> = Vec::with_capacity(v.len());
    for (s, e) in v {
        match out.last_mut() {
            // Overlapping or directly adjacent: one range.
            Some(last) if s <= last.1.next_or_max() => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

trait Increment {
    fn next_or_max(self) -> Self;
}
impl Increment for u32 {
    fn next_or_max(self) -> Self {
        self.saturating_add(1)
    }
}
impl Increment for u128 {
    fn next_or_max(self) -> Self {
        self.saturating_add(1)
    }
}

impl Ranges {
    pub fn new(v4: Vec<(u32, u32)>, v6: Vec<(u128, u128)>) -> Self {
        Self { v4: merge(v4), v6: merge(v6) }
    }

    /// Number of merged ranges.
    pub fn len(&self) -> usize {
        self.v4.len() + self.v6.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        fn hit<T: Ord + Copy>(v: &[(T, T)], x: T) -> bool {
            match v.binary_search_by(|(s, _)| s.cmp(&x)) {
                Ok(_) => true,
                Err(0) => false,
                Err(i) => v[i - 1].1 >= x,
            }
        }
        match ip {
            IpAddr::V4(a) => hit(&self.v4, u32::from(a)),
            IpAddr::V6(a) => hit(&self.v6, u128::from(a)),
        }
    }

    /// Fraction (0..=1) of the IPv4 address space these ranges cover.
    pub fn v4_coverage(&self) -> f64 {
        let covered: u64 = self.v4.iter().map(|(s, e)| u64::from(*e) - u64::from(*s) + 1).sum();
        covered as f64 / 4_294_967_296.0
    }

    pub fn too_broad(&self) -> bool {
        self.v4_coverage() > MAX_V4_COVERAGE
    }

    pub fn union(&self, other: &Ranges) -> Ranges {
        Ranges::new([self.v4.clone(), other.v4.clone()].concat(), [self.v6.clone(), other.v6.clone()].concat())
    }

    /// One `x:start-end` line per range: the format the engine loads. The label never contains a colon.
    pub fn to_list_text(&self) -> String {
        let mut out = String::with_capacity(self.len() * 34);
        for (s, e) in &self.v4 {
            out.push_str(&format!("x:{}-{}\n", Ipv4Addr::from(*s), Ipv4Addr::from(*e)));
        }
        for (s, e) in &self.v6 {
            out.push_str(&format!("x:{}-{}\n", Ipv6Addr::from(*s), Ipv6Addr::from(*e)));
        }
        out
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub ranges: Ranges,
    /// Lines that produced a blocking range.
    pub blocked_lines: usize,
    /// Lines that looked like data but could not be understood.
    pub skipped_lines: usize,
}

enum Line {
    Block(IpAddr, IpAddr),
    /// Valid but intentionally not blocking (eMule access level >= 128).
    Allowed,
    Skip,
    Ignore,
}

/// Like `str::parse`, but accepts zero-padded IPv4 octets (`001.002.003.000`), which eMule lists use.
fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    if s.contains(':') {
        return s.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    let mut octets = [0u8; 4];
    let mut parts = s.split('.');
    for o in &mut octets {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *o = p.parse().ok()?;
    }
    parts.next().is_none().then(|| IpAddr::V4(Ipv4Addr::from(octets)))
}

fn parse_range(s: &str) -> Option<(IpAddr, IpAddr)> {
    let (a, b) = s.split_once('-')?;
    let (a, b) = (parse_ip(a)?, parse_ip(b)?);
    match (a, b) {
        (IpAddr::V4(x), IpAddr::V4(y)) if x <= y => Some((a, b)),
        (IpAddr::V6(x), IpAddr::V6(y)) if x <= y => Some((a, b)),
        _ => None,
    }
}

fn parse_cidr(s: &str) -> Option<(IpAddr, IpAddr)> {
    let (ip, bits) = s.split_once('/')?;
    let ip = parse_ip(ip)?;
    let bits: u32 = bits.trim().parse().ok()?;
    match ip {
        IpAddr::V4(a) if bits <= 32 => {
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            let start = u32::from(a) & mask;
            Some((Ipv4Addr::from(start).into(), Ipv4Addr::from(start | !mask).into()))
        }
        IpAddr::V6(a) if bits <= 128 => {
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            let start = u128::from(a) & mask;
            Some((Ipv6Addr::from(start).into(), Ipv6Addr::from(start | !mask).into()))
        }
        _ => None,
    }
}

fn parse_line(raw: &str) -> Line {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with("//") || line.starts_with(';') {
        return Line::Ignore;
    }
    // eMule: "1.2.3.4 - 1.2.3.255 , 000 , Description"
    if line.contains(',') {
        let mut parts = line.splitn(3, ',');
        let range = parts.next().unwrap_or("");
        let level = parts.next().map(str::trim);
        if let Some((a, b)) = parse_range(range) {
            return match level.map(str::parse::<u32>) {
                Some(Ok(l)) if l >= 128 => Line::Allowed,
                _ => Line::Block(a, b),
            };
        }
    }
    // PeerGuardian: "Some description:1.2.3.4-1.2.3.255" (the description may contain colons).
    for (i, _) in line.match_indices(':') {
        if let Some((a, b)) = parse_range(&line[i + 1..]) {
            return Line::Block(a, b);
        }
    }
    if let Some((a, b)) = parse_range(line).or_else(|| parse_cidr(line)) {
        return Line::Block(a, b);
    }
    if let Some(ip) = parse_ip(line) {
        return Line::Block(ip, ip);
    }
    Line::Skip
}

/// Parses a whole list. Unknown lines are counted, never fatal.
pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut v4, mut v6) = (Vec::new(), Vec::new());
    let (mut blocked_lines, mut skipped_lines) = (0, 0);
    for raw in text.lines() {
        match parse_line(raw) {
            Line::Block(IpAddr::V4(a), IpAddr::V4(b)) => {
                v4.push((u32::from(a), u32::from(b)));
                blocked_lines += 1;
            }
            Line::Block(IpAddr::V6(a), IpAddr::V6(b)) => {
                v6.push((u128::from(a), u128::from(b)));
                blocked_lines += 1;
            }
            Line::Block(..) | Line::Skip => skipped_lines += 1,
            Line::Allowed | Line::Ignore => {}
        }
    }
    Parsed { ranges: Ranges::new(v4, v6), blocked_lines, skipped_lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn emule_dat_with_access_levels() {
        let p = parse(
            "\u{feff}# comment\n\
             001.002.003.000 - 001.002.003.255 , 000 , Bad Corp\n\
             010.000.000.000 - 010.255.255.255 , 255 , Allowed range\n\
             8.8.8.0 - 8.8.8.255 , 100 , Another\n",
        );
        assert_eq!(p.blocked_lines, 2);
        assert!(p.ranges.contains(ip("1.2.3.77")));
        assert!(p.ranges.contains(ip("8.8.8.8")));
        assert!(!p.ranges.contains(ip("10.1.1.1")), "level >= 128 means not filtered");
        assert_eq!(p.skipped_lines, 0);
    }

    #[test]
    fn peerguardian_text_including_colons_in_the_description_and_ipv6() {
        let p = parse("Evil Inc:1.1.1.0-1.1.1.255\nNote: weird: name:2.2.2.2-2.2.2.9\nv6 net:2001:db8::-2001:db8::ffff\n");
        assert_eq!(p.blocked_lines, 3);
        assert!(p.ranges.contains(ip("1.1.1.1")) && p.ranges.contains(ip("2.2.2.5")) && p.ranges.contains(ip("2001:db8::42")));
        assert!(!p.ranges.contains(ip("2.2.2.10")) && !p.ranges.contains(ip("2001:db9::1")));
    }

    #[test]
    fn plain_ranges_cidr_and_single_addresses() {
        let p = parse("3.0.0.0-3.0.0.9\n4.4.0.0/16\n5.5.5.5\n6.0.0.0/0x\n");
        assert_eq!(p.blocked_lines, 3);
        assert_eq!(p.skipped_lines, 1, "a malformed CIDR is counted, not fatal");
        assert!(p.ranges.contains(ip("4.4.255.255")) && !p.ranges.contains(ip("4.5.0.0")));
        assert!(p.ranges.contains(ip("5.5.5.5")) && !p.ranges.contains(ip("5.5.5.6")));
    }

    #[test]
    fn garbage_reversed_and_mixed_family_lines_are_skipped() {
        let p = parse("nonsense\n9.9.9.9-1.1.1.1\n1.1.1.1-::1\n300.1.1.1-300.1.1.2\n\n");
        assert_eq!(p.blocked_lines, 0);
        assert_eq!(p.skipped_lines, 4);
        assert!(p.ranges.is_empty());
    }

    #[test]
    fn overlapping_and_adjacent_ranges_merge_and_boundaries_are_inclusive() {
        let p = parse("1.0.0.0-1.0.0.10\n1.0.0.5-1.0.0.20\n1.0.0.21-1.0.0.30\n9.9.9.9\n");
        assert_eq!(p.ranges.len(), 2, "three ranges collapse to one, plus the single address");
        for edge in ["1.0.0.0", "1.0.0.30", "9.9.9.9"] {
            assert!(p.ranges.contains(ip(edge)), "{edge}");
        }
        for out in ["0.255.255.255", "1.0.0.31", "9.9.9.8", "9.9.9.10"] {
            assert!(!p.ranges.contains(ip(out)), "{out}");
        }
    }

    #[test]
    fn coverage_and_the_too_broad_guard() {
        let all_but_a_bit = parse("0.0.0.0-191.255.255.255\n");
        assert!((all_but_a_bit.ranges.v4_coverage() - 0.75).abs() < 1e-9);
        assert!(all_but_a_bit.ranges.too_broad());
        assert!(!parse("10.0.0.0/8\n").ranges.too_broad());
        assert!(parse("0.0.0.0/0\n").ranges.too_broad());
    }

    #[test]
    fn normalised_text_round_trips_through_the_parser_and_union_merges() {
        let a = parse("1.0.0.0-1.0.0.255\n2001:db8::/32\n").ranges;
        let b = parse("1.0.1.0-1.0.1.255\n").ranges;
        let both = a.union(&b);
        assert_eq!(both.len(), 2, "adjacent v4 ranges merge across lists; the v6 range stays");
        let text = both.to_list_text();
        assert!(text.lines().all(|l| l.starts_with("x:")));
        assert_eq!(parse(&text).ranges, both);
    }

    #[test]
    fn a_large_list_loads_fast_and_small() {
        let mut s = String::new();
        for i in 0..200_000u32 {
            let a = (i * 17) % 4_000_000_000;
            s.push_str(&format!("{}-{}\n", Ipv4Addr::from(a), Ipv4Addr::from(a + 5)));
        }
        let t = std::time::Instant::now();
        let p = parse(&s);
        assert!(p.ranges.len() > 100_000);
        assert!(t.elapsed().as_millis() < 1500, "took {:?}", t.elapsed());
        assert!(p.ranges.contains(ip("0.0.0.3")));
    }
}
