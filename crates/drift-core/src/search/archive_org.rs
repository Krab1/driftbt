//! Internet Archive: a legal, key-less source of public-domain and openly licensed media, software and books.
//! Every item has a torrent at `/download/<id>/<id>_archive.torrent`, seeded by the Archive's own servers.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{ProviderError, SearchProvider, SearchQuery, SearchResult};

pub const DEFAULT_BASE: &str = "https://archive.org";

pub struct ArchiveOrg {
    base: String,
    client: reqwest::Client,
}

impl ArchiveOrg {
    pub fn new() -> Self {
        Self::with_base(DEFAULT_BASE)
    }

    /// For tests and mirrors.
    pub fn with_base(base: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("Drift/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("a default client always builds");
        Self { base: base.trim_end_matches('/').to_string(), client }
    }
}

impl Default for ArchiveOrg {
    fn default() -> Self {
        Self::new()
    }
}

/// Free text safe to put inside a Lucene query: operators and quotes become spaces.
fn lucene_safe(text: &str) -> String {
    let cleaned: String = text.chars().map(|c| if "+-&|!(){}[]^\"~*?:\\/".contains(c) { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Deserialize)]
struct Reply {
    response: Option<Inner>,
}

#[derive(Deserialize)]
struct Inner {
    docs: Vec<Doc>,
}

#[derive(Deserialize)]
struct Doc {
    identifier: String,
    title: Option<serde_json::Value>,
    item_size: Option<serde_json::Value>,
    publicdate: Option<String>,
    mediatype: Option<String>,
}

fn text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => a.first().and_then(text),
        _ => None,
    }
}

fn number(v: &serde_json::Value) -> Option<u64> {
    match v {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[async_trait]
impl SearchProvider for ArchiveOrg {
    fn id(&self) -> &str {
        "archive_org"
    }

    fn name(&self) -> &str {
        "Internet Archive"
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError> {
        let words = lucene_safe(&q.text);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!("({words}) AND mediatype:(movies OR audio OR texts OR software OR data OR etree)");
        let params: Vec<(&str, String)> = vec![
            ("q", query),
            ("fl[]", "identifier".into()),
            ("fl[]", "title".into()),
            ("fl[]", "item_size".into()),
            ("fl[]", "publicdate".into()),
            ("fl[]", "mediatype".into()),
            ("sort[]", "downloads desc".into()),
            ("rows", q.limit.clamp(1, 100).to_string()),
            ("output", "json".into()),
        ];
        let resp = self.client.get(format!("{}/advancedsearch.php", self.base)).query(&params).send().await.map_err(|_| ProviderError::Network)?;
        if !resp.status().is_success() {
            return Err(ProviderError::BadResponse);
        }
        let reply: Reply = resp.json().await.map_err(|_| ProviderError::BadResponse)?;
        let docs = reply.response.ok_or(ProviderError::BadResponse)?.docs;
        Ok(docs
            .into_iter()
            // Identifiers go straight into a URL path, so anything unexpected is dropped.
            .filter(|d| !d.identifier.is_empty() && d.identifier.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))
            .map(|d| SearchResult {
                title: d.title.as_ref().and_then(text).unwrap_or_else(|| d.identifier.clone()),
                sources: Vec::new(),
                info_hash: None,
                magnet: None,
                torrent_url: Some(format!("{}/download/{id}/{id}_archive.torrent", self.base, id = d.identifier)),
                size: d.item_size.as_ref().and_then(number),
                seeders: None,
                leechers: None,
                published: d.publicdate.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|t| t.timestamp()),
                category: d.mediatype,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn user_text_cannot_inject_query_operators() {
        assert_eq!(lucene_safe("big buck bunny"), "big buck bunny");
        assert_eq!(lucene_safe("a AND (b) OR \"c\" -d title:x*"), "a AND b OR c d title x");
        assert_eq!(lucene_safe("  ((( )))  "), "");
    }

    async fn serve_once(reply: String) -> (u16, tokio::sync::oneshot::Receiver<String>) {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string());
            let _ = s.write_all(reply.as_bytes()).await;
        });
        (port, rx)
    }

    fn json(body: &str) -> String {
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
    }

    #[tokio::test]
    async fn turns_documents_into_results_with_torrent_links() {
        let body = r#"{"response":{"numFound":3,"docs":[
          {"identifier":"BigBuckBunny_124","title":"Big Buck Bunny","item_size":"276134947","publicdate":"2008-12-29T17:40:52Z","mediatype":"movies"},
          {"identifier":"weird id/../x","title":"Dropped"},
          {"identifier":"sintel","title":["Sintel (array title)"],"item_size":129241752}
        ]}}"#;
        let (port, seen) = serve_once(json(body)).await;
        let a = ArchiveOrg::with_base(&format!("http://127.0.0.1:{port}"));
        let r = a.search(&SearchQuery { text: "bunny: (the) film".into(), limit: 10 }).await.unwrap();
        assert_eq!(r.len(), 2, "an identifier that isn't plain is dropped, never put in a URL");
        assert_eq!(r[0].title, "Big Buck Bunny");
        assert_eq!(r[0].size, Some(276_134_947));
        assert_eq!(r[0].category.as_deref(), Some("movies"));
        assert_eq!(r[0].published, Some(1_230_572_452));
        assert_eq!(r[0].torrent_url.as_deref(), Some(&*format!("http://127.0.0.1:{port}/download/BigBuckBunny_124/BigBuckBunny_124_archive.torrent")));
        assert_eq!(r[1].title, "Sintel (array title)");
        assert_eq!(r[1].size, Some(129_241_752));
        assert!(r.iter().all(|x| x.seeders.is_none() && x.magnet.is_none()), "no swarm numbers exist for web-seeded items");
        let line = seen.await.unwrap();
        assert!(line.starts_with("GET /advancedsearch.php?") && line.contains("output=json") && line.contains("rows=10"), "{line}");
        assert!(!line.contains("bunny%3A") && !line.contains("%28the%29"), "operators were stripped from the user's words: {line}");
    }

    #[tokio::test]
    async fn empty_queries_make_no_request_and_failures_are_plain() {
        let a = ArchiveOrg::with_base("http://127.0.0.1:9");
        assert_eq!(a.search(&SearchQuery { text: "((()))".into(), limit: 5 }).await.unwrap(), vec![]);
        assert_eq!(a.search(&SearchQuery { text: "real words".into(), limit: 5 }).await.unwrap_err(), ProviderError::Network);
        let (port, _) = serve_once(json("{\"unexpected\":true}")).await;
        let b = ArchiveOrg::with_base(&format!("http://127.0.0.1:{port}"));
        assert_eq!(b.search(&SearchQuery { text: "x".into(), limit: 5 }).await.unwrap_err(), ProviderError::BadResponse);
    }
}
