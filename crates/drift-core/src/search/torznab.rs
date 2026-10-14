//! Torznab: the standard search API that Jackett, Prowlarr and most indexer managers speak. The user points
//! Drift at their own indexer, so which sites get searched is their choice and their setup.

use std::time::Duration;

use async_trait::async_trait;
use quick_xml::events::{BytesStart, Event};
use quick_xml::XmlVersion;
use quick_xml::Reader;

use super::{magnet_hash, ProviderError, SearchProvider, SearchQuery, SearchResult};

pub struct Torznab {
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl Torznab {
    /// `base_url` is the indexer's Torznab endpoint, e.g. `http://localhost:9117/api/v2.0/indexers/all/results/torznab/`.
    pub fn new(id: &str, name: &str, base_url: &str, api_key: &str) -> Result<Self, ProviderError> {
        let url = reqwest::Url::parse(base_url.trim()).map_err(|_| ProviderError::Config("The address isn't valid.".into()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ProviderError::Config("Only http and https addresses work.".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("Drift/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ProviderError::Network)?;
        Ok(Self { id: id.into(), name: name.into(), base_url: url.to_string(), api_key: api_key.trim().into(), client })
    }
}

#[async_trait]
impl SearchProvider for Torznab {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError> {
        let mut params: Vec<(&str, String)> = vec![("t", "search".into()), ("q", q.text.clone()), ("limit", q.limit.clamp(1, 100).to_string()), ("extended", "1".into())];
        if !self.api_key.is_empty() {
            params.push(("apikey", self.api_key.clone()));
        }
        // Errors are mapped to fixed messages on purpose: reqwest's own text contains the full address, key included.
        let resp = self.client.get(&self.base_url).query(&params).send().await.map_err(|_| ProviderError::Network)?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ProviderError::Auth);
        }
        if !status.is_success() {
            return Err(ProviderError::BadResponse);
        }
        let body = resp.text().await.map_err(|_| ProviderError::Network)?;
        parse_feed(&body)
    }
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == key).and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok()).map(|v| v.into_owned())
}

/// Reads a Torznab RSS feed. `<error code="1xx"/>` means the key was refused; anything else odd is unreadable.
pub fn parse_feed(xml: &str) -> Result<Vec<SearchResult>, ProviderError> {
    let mut reader = Reader::from_str(xml);
    let mut items: Vec<SearchResult> = Vec::new();
    let mut cur: Option<Item> = None;
    let mut text_of: Option<String> = None;
    let mut buf = String::new();
    let mut saw_feed = false;

    loop {
        match reader.read_event().map_err(|_| ProviderError::BadResponse)? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name().as_ref().to_string();
                match name.as_str() {
                    "rss" | "channel" => saw_feed = true,
                    "error" => {
                        let code: u32 = attr(&e, "code").and_then(|c| c.parse().ok()).unwrap_or(0);
                        return Err(if (100..200).contains(&code) { ProviderError::Auth } else { ProviderError::BadResponse });
                    }
                    "item" => cur = Some(Item::default()),
                    "enclosure" => {
                        if let Some(c) = cur.as_mut() {
                            c.enclosure_url = attr(&e, "url");
                            c.enclosure_len = attr(&e, "length").and_then(|v| v.parse().ok());
                        }
                    }
                    "torznab:attr" | "newznab:attr" => {
                        if let (Some(c), Some(n), Some(v)) = (cur.as_mut(), attr(&e, "name"), attr(&e, "value")) {
                            c.attrs.push((n, v));
                        }
                    }
                    "title" | "link" | "pubDate" | "size" | "category" if cur.is_some() => text_of = Some(name),
                    _ => {}
                }
            }
            Event::Text(t) if text_of.is_some() => buf.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Event::GeneralRef(r) if text_of.is_some() => match r.as_ref() {
                "amp" => buf.push('&'),
                "lt" => buf.push('<'),
                "gt" => buf.push('>'),
                "quot" => buf.push('"'),
                "apos" => buf.push('\''),
                _ => {
                    if let Ok(Some(c)) = r.resolve_char_ref() {
                        buf.push(c);
                    }
                }
            },
            Event::End(e) => {
                if let (Some(c), Some(field)) = (cur.as_mut(), text_of.take()) {
                    let v = std::mem::take(&mut buf).trim().to_string();
                    match field.as_str() {
                        "title" => c.title = v,
                        "link" => c.link = Some(v),
                        "pubDate" => c.pub_date = Some(v),
                        "size" => c.size = v.parse().ok(),
                        "category" if c.category.is_none() => c.category = Some(v),
                        _ => {}
                    }
                }
                buf.clear();
                if e.name().as_ref() == "item" {
                    if let Some(c) = cur.take() {
                        if let Some(r) = c.into_result() {
                            items.push(r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !saw_feed {
        return Err(ProviderError::BadResponse);
    }
    Ok(items)
}

#[derive(Default)]
struct Item {
    title: String,
    link: Option<String>,
    pub_date: Option<String>,
    size: Option<u64>,
    category: Option<String>,
    enclosure_url: Option<String>,
    enclosure_len: Option<u64>,
    attrs: Vec<(String, String)>,
}

impl Item {
    fn get(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn into_result(self) -> Option<SearchResult> {
        if self.title.is_empty() {
            return None;
        }
        let is_magnet = |s: &str| s.to_ascii_lowercase().starts_with("magnet:");
        let magnet = self.get("magneturl").filter(|m| is_magnet(m)).map(str::to_string).or_else(|| self.link.clone().filter(|l| is_magnet(l)));
        let torrent_url = self
            .enclosure_url
            .clone()
            .filter(|u| !is_magnet(u))
            .or_else(|| self.link.clone().filter(|l| !is_magnet(l)))
            .filter(|u| u.starts_with("http://") || u.starts_with("https://"));
        let seeders = self.get("seeders").and_then(|v| v.parse::<u32>().ok());
        // Torznab's "peers" counts everyone, seeders included.
        let leechers = self.get("leechers").and_then(|v| v.parse::<u32>().ok()).or_else(|| {
            let peers = self.get("peers").and_then(|v| v.parse::<u32>().ok())?;
            Some(peers.saturating_sub(seeders.unwrap_or(0)))
        });
        let info_hash = self.get("infohash").map(str::to_string).or_else(|| magnet.as_deref().and_then(magnet_hash));
        Some(SearchResult {
            title: self.title.clone(),
            sources: Vec::new(),
            info_hash,
            magnet,
            torrent_url,
            size: self.size.or(self.enclosure_len).or_else(|| self.get("size").and_then(|v| v.parse().ok())),
            seeders,
            leechers,
            published: self.pub_date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok()).map(|d| d.timestamp()),
            category: self.category.clone().or_else(|| self.get("category").map(str::to_string)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const H: &str = "0123456789abcdef0123456789abcdef01234567";

    fn feed() -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:torznab="http://torznab.com/schemas/2015/feed">
 <channel><title>idx</title>
  <item>
   <title>Ubuntu &amp; Friends 26.04 amd64</title>
   <link>http://idx.local/dl/1?key=abc</link>
   <pubDate>Tue, 13 Oct 2026 10:00:00 +0000</pubDate>
   <size>6482409472</size>
   <category>Linux</category>
   <enclosure url="http://idx.local/dl/1.torrent?key=abc" length="6482409472" type="application/x-bittorrent"/>
   <torznab:attr name="seeders" value="1500"/>
   <torznab:attr name="peers" value="1800"/>
   <torznab:attr name="infohash" value="{H}"/>
  </item>
  <item>
   <title>Magnet only</title>
   <link>magnet:?xt=urn:btih:{H2}&amp;dn=x</link>
   <torznab:attr name="seeders" value="3"/>
   <torznab:attr name="leechers" value="9"/>
  </item>
  <item><title></title><link>http://x/</link></item>
 </channel>
</rss>"#,
            H2 = "fedcba9876543210fedcba9876543210fedcba98"
        )
    }

    #[test]
    fn parses_torznab_attributes_magnets_and_skips_nameless_items() {
        let r = parse_feed(&feed()).unwrap();
        assert_eq!(r.len(), 2);
        let a = &r[0];
        assert_eq!(a.title, "Ubuntu & Friends 26.04 amd64");
        assert_eq!((a.seeders, a.leechers), (Some(1500), Some(300)), "peers include seeders, so leechers = peers - seeders");
        assert_eq!(a.info_hash.as_deref(), Some(H));
        assert_eq!(a.size, Some(6_482_409_472));
        assert_eq!(a.torrent_url.as_deref(), Some("http://idx.local/dl/1.torrent?key=abc"));
        assert_eq!(a.category.as_deref(), Some("Linux"));
        assert_eq!(a.published, Some(1_791_885_600));
        let b = &r[1];
        assert!(b.magnet.as_deref().unwrap().starts_with("magnet:?xt="));
        assert_eq!(b.info_hash.as_deref(), Some("fedcba9876543210fedcba9876543210fedcba98"), "hash taken from the magnet");
        assert_eq!((b.seeders, b.leechers, b.torrent_url.clone()), (Some(3), Some(9), None));
    }

    #[test]
    fn error_elements_and_garbage_are_told_apart() {
        assert_eq!(parse_feed(r#"<error code="100" description="Incorrect user credentials"/>"#).unwrap_err(), ProviderError::Auth);
        assert_eq!(parse_feed(r#"<error code="203" description="Function not available"/>"#).unwrap_err(), ProviderError::BadResponse);
        assert_eq!(parse_feed("<html><body>Not an indexer</body></html>").unwrap_err(), ProviderError::BadResponse);
        assert_eq!(parse_feed("definitely not xml <<<").unwrap_err(), ProviderError::BadResponse);
        assert_eq!(parse_feed("<rss><channel></channel></rss>").unwrap(), vec![]);
    }

    #[test]
    fn construction_rejects_bad_addresses() {
        assert!(matches!(Torznab::new("a", "A", "not a url", "k"), Err(ProviderError::Config(_))));
        assert!(matches!(Torznab::new("a", "A", "ftp://host/x", "k"), Err(ProviderError::Config(_))));
        assert!(Torznab::new("a", "A", "http://localhost:9117/api/", "").is_ok());
    }

    /// Answers the next request with `reply` and reports the request line it saw.
    async fn serve_once(reply: String) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string());
            let _ = s.write_all(reply.as_bytes()).await;
        });
        (port, rx)
    }

    fn http(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn sends_the_query_and_key_and_returns_results() {
        let (port, seen) = serve_once(http("200 OK", &feed())).await;
        let t = Torznab::new("t1", "My indexer", &format!("http://127.0.0.1:{port}/api"), "SECRETKEY").unwrap();
        let r = t.search(&SearchQuery { text: "ubuntu iso".into(), limit: 20 }).await.unwrap();
        assert_eq!(r.len(), 2);
        let line = seen.await.unwrap();
        assert!(line.starts_with("GET /api?"), "{line}");
        for part in ["t=search", "q=ubuntu+iso", "limit=20", "apikey=SECRETKEY"] {
            assert!(line.contains(part), "{part} missing in {line}");
        }
    }

    #[tokio::test]
    async fn refused_keys_unreachable_hosts_and_bad_bodies_map_to_safe_messages() {
        let q = SearchQuery { text: "x".into(), limit: 5 };
        let (p, _) = serve_once(http("401 Unauthorized", "no")).await;
        let e = Torznab::new("t", "T", &format!("http://127.0.0.1:{p}/"), "K").unwrap().search(&q).await.unwrap_err();
        assert_eq!(e, ProviderError::Auth);

        let (p, _) = serve_once(http("200 OK", r#"<error code="101" description="Account suspended"/>"#)).await;
        assert_eq!(Torznab::new("t", "T", &format!("http://127.0.0.1:{p}/"), "K").unwrap().search(&q).await.unwrap_err(), ProviderError::Auth);

        let (p, _) = serve_once(http("500 Internal Server Error", "boom")).await;
        assert_eq!(Torznab::new("t", "T", &format!("http://127.0.0.1:{p}/"), "K").unwrap().search(&q).await.unwrap_err(), ProviderError::BadResponse);

        let dead = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        let e = Torznab::new("t", "T", &format!("http://127.0.0.1:{dead}/"), "TOPSECRET").unwrap().search(&q).await.unwrap_err();
        assert_eq!(e, ProviderError::Network);
        assert!(!e.to_string().contains("TOPSECRET") && !e.to_string().contains("127.0.0.1"), "messages must never carry the address or key");
    }
}
