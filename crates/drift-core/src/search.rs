//! Search across user-chosen sources. Sources are providers behind one small contract; adding one never
//! touches the others. The aggregator asks them all at once, tolerates any of them failing or being slow,
//! merges duplicates and ranks the result.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod config;
pub mod migrate;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    /// Names of every source that returned it.
    pub sources: Vec<String>,
    /// Lowercase 40-character hex.
    pub info_hash: Option<String>,
    pub magnet: Option<String>,
    /// http(s) address of the .torrent file.
    pub torrent_url: Option<String>,
    pub size: Option<u64>,
    pub seeders: Option<u32>,
    pub leechers: Option<u32>,
    /// Unix seconds.
    pub published: Option<i64>,
    pub category: Option<String>,
}

/// Plain-language reasons a source failed. Never contains an address or key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("Couldn't reach this source. Check the address and your connection.")]
    Network,
    #[error("This source refused the API key. Check it in Settings.")]
    Auth,
    #[error("This source answered with something Drift couldn't read.")]
    BadResponse,
    #[error("This source isn't fully set up yet. {0}")]
    Config(String),
    #[error("This plugin didn't answer. It may have crashed or been switched off.")]
    Plugin,
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Stable key, e.g. `archive_org` or `torznab-1`.
    fn id(&self) -> &str;
    /// Shown in the UI and on result badges.
    fn name(&self) -> &str;
    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub id: String,
    pub name: String,
    pub outcome: Outcome,
    pub count: usize,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchReport {
    pub results: Vec<SearchResult>,
    pub sources: Vec<SourceStatus>,
}

/// `xt=urn:btih:` hash of a magnet link as lowercase hex (accepts the 32-character base32 form too).
pub fn magnet_hash(magnet: &str) -> Option<String> {
    let q = magnet.split_once('?')?.1;
    let h = q.split('&').find_map(|kv| kv.strip_prefix("xt=urn:btih:").or_else(|| kv.strip_prefix("xt=URN:BTIH:")))?;
    normalize_hash(h)
}

pub fn normalize_hash(h: &str) -> Option<String> {
    let h = h.trim();
    if h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(h.to_ascii_lowercase());
    }
    if h.len() == 32 {
        // base32 (RFC 4648, no padding) -> 20 bytes -> hex
        let mut bits = 0u32;
        let mut have = 0;
        let mut out = String::with_capacity(40);
        for c in h.bytes() {
            let v = match c.to_ascii_uppercase() {
                b'A'..=b'Z' => c.to_ascii_uppercase() - b'A',
                b'2'..=b'7' => c - b'2' + 26,
                _ => return None,
            };
            bits = (bits << 5) | u32::from(v);
            have += 5;
            if have >= 8 {
                have -= 8;
                out.push_str(&format!("{:02x}", (bits >> have) & 0xff));
            }
        }
        return (out.len() == 40).then_some(out);
    }
    None
}

fn merge_key(r: &SearchResult) -> String {
    match &r.info_hash {
        Some(h) => format!("h:{h}"),
        // No hash to go on: the same title and size from two sources is very likely the same release.
        None => format!("t:{}:{}", r.title.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "), r.size.unwrap_or(0)),
    }
}

fn merge_into(a: &mut SearchResult, b: SearchResult) {
    for s in b.sources {
        if !a.sources.contains(&s) {
            a.sources.push(s);
        }
    }
    a.magnet = a.magnet.take().or(b.magnet);
    a.torrent_url = a.torrent_url.take().or(b.torrent_url);
    a.info_hash = a.info_hash.take().or(b.info_hash);
    a.size = a.size.or(b.size);
    a.seeders = a.seeders.max(b.seeders);
    a.leechers = a.leechers.max(b.leechers);
    a.published = a.published.max(b.published);
    a.category = a.category.take().or(b.category);
}

/// Merges duplicates (by info-hash, else by title and size) and sorts: most seeders first, results that
/// report no seeder count after those that do, then by title.
pub fn merge_and_rank(all: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<SearchResult> = Vec::new();
    for mut r in all {
        r.info_hash = r.info_hash.take().and_then(|h| normalize_hash(&h)).or_else(|| r.magnet.as_deref().and_then(magnet_hash));
        let key = merge_key(&r);
        match by_key.get(&key) {
            Some(&i) => merge_into(&mut out[i], r),
            None => {
                by_key.insert(key, out.len());
                out.push(r);
            }
        }
    }
    out.sort_by(|a, b| {
        b.seeders.is_some().cmp(&a.seeders.is_some()).then(b.seeders.cmp(&a.seeders)).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

/// Asks every provider at once. A slow or failing provider never fails the search.
pub async fn search_all(providers: &[Arc<dyn SearchProvider>], q: &SearchQuery, timeout: Duration) -> SearchReport {
    let runs = providers.iter().map(|p| {
        let p = p.clone();
        let q = q.clone();
        async move {
            let r = tokio::time::timeout(timeout, p.search(&q)).await;
            (p.id().to_string(), p.name().to_string(), r)
        }
    });
    let mut report = SearchReport::default();
    let mut all = Vec::new();
    for (id, name, r) in futures_util::future::join_all(runs).await {
        let (outcome, count, message) = match r {
            Ok(Ok(mut found)) => {
                // A source that already labelled its rows (a plugin searching several places) keeps its labels.
                for f in found.iter_mut().filter(|f| f.sources.is_empty()) {
                    f.sources.push(name.clone());
                }
                let n = found.len();
                all.extend(found);
                (Outcome::Ok, n, None)
            }
            Ok(Err(e)) => (Outcome::Failed, 0, Some(e.to_string())),
            Err(_) => (Outcome::TimedOut, 0, Some("This source took too long to answer.".to_string())),
        };
        report.sources.push(SourceStatus { id, name, outcome, count, message });
    }
    report.results = merge_and_rank(all);
    report.results.truncate(q.limit.max(1));
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        id: &'static str,
        delay: Duration,
        result: Result<Vec<SearchResult>, ProviderError>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SearchProvider for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            self.id
        }
        async fn search(&self, _: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            self.result.clone()
        }
    }

    fn fake(id: &'static str, delay_ms: u64, result: Result<Vec<SearchResult>, ProviderError>) -> Arc<dyn SearchProvider> {
        Arc::new(Fake { id, delay: Duration::from_millis(delay_ms), result, calls: Arc::new(AtomicUsize::new(0)) })
    }

    fn r(title: &str, hash: Option<&str>, seeders: Option<u32>) -> SearchResult {
        SearchResult { title: title.into(), info_hash: hash.map(str::to_string), seeders, size: Some(100), ..Default::default() }
    }

    const H1: &str = "0123456789abcdef0123456789abcdef01234567";
    const H2: &str = "fedcba9876543210fedcba9876543210fedcba98";

    fn q() -> SearchQuery {
        SearchQuery { text: "x".into(), limit: 50 }
    }

    #[tokio::test]
    async fn asks_every_source_at_once_not_one_after_another() {
        let ps = [fake("a", 150, Ok(vec![r("A", Some(H1), Some(1))])), fake("b", 150, Ok(vec![r("B", Some(H2), Some(2))])), fake("c", 150, Ok(vec![]))];
        let t = std::time::Instant::now();
        let rep = search_all(&ps, &q(), Duration::from_secs(5)).await;
        assert!(t.elapsed() < Duration::from_millis(400), "took {:?}: sources ran sequentially", t.elapsed());
        assert_eq!(rep.sources.len(), 3);
        assert_eq!(rep.results.len(), 2);
    }

    #[tokio::test]
    async fn a_failing_or_slow_source_never_fails_the_search_and_says_why() {
        let ps = [
            fake("good", 10, Ok(vec![r("Found", Some(H1), Some(5))])),
            fake("bad", 10, Err(ProviderError::Auth)),
            fake("slow", 2000, Ok(vec![r("Late", Some(H2), Some(9))])),
        ];
        let rep = search_all(&ps, &q(), Duration::from_millis(200)).await;
        let status = |id: &str| rep.sources.iter().find(|s| s.id == id).unwrap();
        assert_eq!(status("good").outcome, Outcome::Ok);
        assert_eq!(status("bad").outcome, Outcome::Failed);
        assert!(status("bad").message.as_deref().unwrap().contains("API key"));
        assert_eq!(status("slow").outcome, Outcome::TimedOut);
        assert_eq!(rep.results.len(), 1);
        assert_eq!(rep.results[0].sources, ["good"]);
    }

    #[tokio::test]
    async fn the_same_torrent_from_two_sources_is_one_result_with_both_badges() {
        let mut from_b = r("Same release", Some(&H1.to_uppercase()), Some(40));
        from_b.magnet = Some(format!("magnet:?xt=urn:btih:{H1}"));
        let ps = [fake("a", 1, Ok(vec![r("Same release", Some(H1), Some(12))])), fake("b", 1, Ok(vec![from_b]))];
        let rep = search_all(&ps, &q(), Duration::from_secs(1)).await;
        assert_eq!(rep.results.len(), 1);
        let x = &rep.results[0];
        assert_eq!(x.sources, ["a", "b"]);
        assert_eq!(x.seeders, Some(40), "the better seeder count wins");
        assert!(x.magnet.is_some(), "a magnet from either source is kept");
        assert_eq!(x.info_hash.as_deref(), Some(H1), "hashes are normalised to lowercase");
    }

    #[test]
    fn results_without_a_hash_merge_on_title_and_size_only() {
        let a = SearchResult { title: "Big  Film".into(), size: Some(5), ..Default::default() };
        let b = SearchResult { title: "big film".into(), size: Some(5), seeders: Some(3), ..Default::default() };
        let c = SearchResult { title: "big film".into(), size: Some(6), ..Default::default() };
        assert_eq!(merge_and_rank(vec![a, b, c]).len(), 2);
    }

    #[test]
    fn ranking_puts_most_seeded_first_and_unknown_counts_after_known_ones() {
        let ranked = merge_and_rank(vec![
            r("zeta", Some(H1), None),
            r("alpha", Some(H2), Some(1)),
            r("beta", Some("00000000000000000000000000000000000000aa"), Some(50)),
            r("Aardvark", Some("00000000000000000000000000000000000000bb"), None),
        ]);
        let titles: Vec<_> = ranked.iter().map(|x| x.title.as_str()).collect();
        assert_eq!(titles, ["beta", "alpha", "Aardvark", "zeta"]);
    }

    #[tokio::test]
    async fn the_limit_trims_the_merged_list() {
        let many: Vec<_> = (0..30).map(|i| r(&format!("t{i}"), Some(&format!("{i:040x}")), Some(i))).collect();
        let rep = search_all(&[fake("a", 1, Ok(many))], &SearchQuery { text: "x".into(), limit: 10 }, Duration::from_secs(1)).await;
        assert_eq!(rep.results.len(), 10);
        assert_eq!(rep.results[0].seeders, Some(29));
        assert_eq!(rep.sources[0].count, 30, "the source still reports how many it returned");
    }

    #[test]
    fn magnet_hashes_hex_and_base32() {
        assert_eq!(magnet_hash(&format!("magnet:?xt=urn:btih:{}&dn=x", H1.to_uppercase())).as_deref(), Some(H1));
        // base32 of bytes 0x01..0x14
        assert_eq!(magnet_hash("magnet:?dn=a&xt=urn:btih:AEBAGBAFAYDQQCIKBMGA2DQPCAIREEYU").as_deref(), Some("0102030405060708090a0b0c0d0e0f1011121314"));
        assert_eq!(magnet_hash("magnet:?xt=urn:btih:nope"), None);
        assert_eq!(magnet_hash("not a magnet"), None);
    }
}
