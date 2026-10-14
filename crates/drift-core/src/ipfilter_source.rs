//! Getting an IP list from the network or a file, safely. Lists are untrusted input, some are served over
//! plain HTTP, so everything is capped and checked before it can reach the engine.

use std::io::{Cursor, Read};
use std::time::Duration;

use crate::ipfilter::{self, Parsed};

#[derive(Debug, Clone, Copy)]
pub struct Caps {
    /// Largest download (compressed size).
    pub download: usize,
    /// Largest text after unzipping.
    pub decoded: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Self { download: 32 << 20, decoded: 128 << 20 }
    }
}

/// Plain-language reasons a list is refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ListError {
    #[error("Couldn't download the list. Check your connection and the address.")]
    Network,
    #[error("The list is too large to be a real IP list, so it was ignored.")]
    TooLarge,
    #[error("That archive doesn't contain an IP list.")]
    NoListInArchive,
    #[error("The file couldn't be read as an IP list.")]
    Unreadable,
    #[error("The list has no usable entries, so it was ignored.")]
    Empty,
    #[error("The list would block more than a quarter of the internet, so it was ignored.")]
    TooBroad,
    #[error("Only http and https addresses are supported.")]
    Scheme,
}

pub struct Fetched {
    pub parsed: Parsed,
    /// Downloaded over plain HTTP: anyone on the path could have changed it.
    pub insecure: bool,
}

fn read_capped(mut r: impl Read, cap: usize) -> Result<Vec<u8>, ListError> {
    let mut out = Vec::new();
    r.by_ref().take(cap as u64 + 1).read_to_end(&mut out).map_err(|_| ListError::Unreadable)?;
    if out.len() > cap {
        return Err(ListError::TooLarge);
    }
    Ok(out)
}

fn is_list_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["ipfilter.dat", ".dat", ".txt", ".p2p", ".list"].iter().any(|s| n == *s || n.ends_with(s))
}

/// Turns downloaded or read bytes into list text: zip (only whitelisted entry names, read in memory,
/// never extracted to disk), gzip, or plain text.
pub fn decode(bytes: &[u8], max_decoded: usize) -> Result<String, ListError> {
    let raw = if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ListError::Unreadable)?;
        let mut all = Vec::new();
        for i in 0..zip.len() {
            let entry = zip.by_index(i).map_err(|_| ListError::Unreadable)?;
            if entry.is_dir() || !entry.name().map(|n| is_list_name(&n)).unwrap_or(false) {
                continue;
            }
            let room = max_decoded.saturating_sub(all.len());
            all.extend(read_capped(entry, room)?);
            all.push(b'\n');
        }
        if all.is_empty() {
            return Err(ListError::NoListInArchive);
        }
        all
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        read_capped(flate2::read::GzDecoder::new(bytes), max_decoded)?
    } else {
        if bytes.len() > max_decoded {
            return Err(ListError::TooLarge);
        }
        bytes.to_vec()
    };
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// Parses and sanity-checks list text.
pub fn validate(text: &str) -> Result<Parsed, ListError> {
    let parsed = ipfilter::parse(text);
    if parsed.ranges.is_empty() {
        return Err(ListError::Empty);
    }
    if parsed.ranges.too_broad() {
        return Err(ListError::TooBroad);
    }
    Ok(parsed)
}

pub fn from_bytes(bytes: &[u8], caps: Caps) -> Result<Parsed, ListError> {
    validate(&decode(bytes, caps.decoded)?)
}

pub fn from_file(path: &std::path::Path, caps: Caps) -> Result<Parsed, ListError> {
    let len = std::fs::metadata(path).map_err(|_| ListError::Unreadable)?.len();
    if len > caps.download as u64 {
        return Err(ListError::TooLarge);
    }
    from_bytes(&std::fs::read(path).map_err(|_| ListError::Unreadable)?, caps)
}

fn client() -> Result<reqwest::Client, ListError> {
    // At most three redirects, only to http(s), and never from https down to http.
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let ok_scheme = matches!(attempt.url().scheme(), "http" | "https");
        let downgrade = attempt.previous().last().is_some_and(|p| p.scheme() == "https") && attempt.url().scheme() == "http";
        if attempt.previous().len() >= 3 || !ok_scheme || downgrade {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("Drift/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| ListError::Network)
}

/// Downloads `url` (http or https) with a hard size cap, then decodes and validates it.
pub async fn fetch(url: &str, caps: Caps) -> Result<Fetched, ListError> {
    let parsed_url = reqwest::Url::parse(url).map_err(|_| ListError::Scheme)?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(ListError::Scheme);
    }
    let insecure = parsed_url.scheme() == "http";
    let mut resp = client()?.get(parsed_url).send().await.map_err(|_| ListError::Network)?;
    if !resp.status().is_success() {
        return Err(ListError::Network);
    }
    if resp.content_length().is_some_and(|n| n > caps.download as u64) {
        return Err(ListError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| ListError::Network)? {
        body.extend_from_slice(&chunk);
        if body.len() > caps.download {
            return Err(ListError::TooLarge);
        }
    }
    Ok(Fetched { parsed: from_bytes(&body, caps)?, insecure })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const EMULE: &str = "001.002.003.000 - 001.002.003.255 , 000 , Bad\n010.000.000.000 - 010.255.255.255 , 255 , fine\n";

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn decodes_zip_gzip_and_plain_text() {
        let want = |b: &[u8]| from_bytes(b, Caps::default()).unwrap().ranges.len();
        assert_eq!(want(&zip_of(&[("ipfilter.dat", EMULE.as_bytes())])), 1);
        assert_eq!(want(&gz(EMULE.as_bytes())), 1);
        assert_eq!(want(EMULE.as_bytes()), 1);
    }

    #[test]
    fn only_whitelisted_zip_entries_are_read_and_nothing_touches_disk_paths() {
        let z = zip_of(&[("../../evil.exe", b"MZ"), ("readme.html", b"<b>hi</b>"), ("sub/lists/ipfilter.dat", EMULE.as_bytes())]);
        assert_eq!(from_bytes(&z, Caps::default()).unwrap().ranges.len(), 1);
        let none = zip_of(&[("photo.jpg", b"x"), ("notes.md", b"y")]);
        assert_eq!(from_bytes(&none, Caps::default()).unwrap_err(), ListError::NoListInArchive);
    }

    #[test]
    fn zip_and_gzip_bombs_are_stopped_by_the_decoded_cap() {
        let big = vec![b'0'; 4 << 20];
        let tiny = Caps { download: 1 << 20, decoded: 1 << 20 };
        assert_eq!(from_bytes(&zip_of(&[("ipfilter.dat", &big)]), tiny).unwrap_err(), ListError::TooLarge);
        assert_eq!(from_bytes(&gz(&big), tiny).unwrap_err(), ListError::TooLarge);
        assert_eq!(from_bytes(&big, tiny).unwrap_err(), ListError::TooLarge);
    }

    #[test]
    fn empty_or_nonsense_and_overbroad_lists_are_refused() {
        assert_eq!(validate("").unwrap_err(), ListError::Empty);
        assert_eq!(validate("hello\nworld\n").unwrap_err(), ListError::Empty);
        assert_eq!(validate("0.0.0.0-200.255.255.255\n").unwrap_err(), ListError::TooBroad);
        assert!(validate("1.2.3.0/24\n").is_ok());
    }

    #[test]
    fn files_are_size_capped_too() {
        let dir = std::env::temp_dir().join(format!("drift-ipsrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("a.txt");
        std::fs::write(&ok, EMULE).unwrap();
        assert!(from_file(&ok, Caps::default()).is_ok());
        assert_eq!(from_file(&ok, Caps { download: 10, decoded: 10 }).unwrap_err(), ListError::TooLarge);
        assert_eq!(from_file(&dir.join("missing.txt"), Caps::default()).unwrap_err(), ListError::Unreadable);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One-shot HTTP server that answers every request with `reply`.
    async fn serve(reply: Vec<u8>) -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = s.read(&mut buf).await;
                    let _ = s.write_all(&reply).await;
                });
            }
        });
        port
    }

    fn http(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut r = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        r
    }

    #[tokio::test]
    async fn downloads_validates_and_flags_plain_http() {
        let port = serve(http("200 OK", "", &zip_of(&[("ipfilter.dat", EMULE.as_bytes())]))).await;
        let f = fetch(&format!("http://127.0.0.1:{port}/ipfilter.zip"), Caps::default()).await.unwrap();
        assert_eq!(f.parsed.ranges.len(), 1);
        assert!(f.insecure, "plain http must be flagged");
    }

    #[tokio::test]
    async fn refuses_oversized_bad_status_odd_schemes_and_hostile_redirects() {
        let big = serve(http("200 OK", "", &vec![b'a'; 4096])).await;
        let caps = Caps { download: 1024, decoded: 1024 };
        assert_eq!(fetch(&format!("http://127.0.0.1:{big}/x"), caps).await.err(), Some(ListError::TooLarge));

        let missing = serve(http("404 Not Found", "", b"no")).await;
        assert_eq!(fetch(&format!("http://127.0.0.1:{missing}/x"), Caps::default()).await.err(), Some(ListError::Network));

        assert_eq!(fetch("ftp://example.org/list.zip", Caps::default()).await.err(), Some(ListError::Scheme));
        assert_eq!(fetch("file:///C:/Windows/win.ini", Caps::default()).await.err(), Some(ListError::Scheme));

        let to_file = serve(http("302 Found", "Location: file:///C:/Windows/win.ini\r\n", b"")).await;
        assert!(fetch(&format!("http://127.0.0.1:{to_file}/x"), Caps::default()).await.is_err(), "a redirect to a file: URL must not be followed");
    }
}
