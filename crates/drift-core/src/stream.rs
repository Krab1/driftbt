//! A tiny loopback HTTP server that lets a media player read a file while it is still downloading.
//!
//! Safety: it binds to 127.0.0.1 only, every URL carries a random per-run token, and the `Host` header
//! must be the loopback address (so a web page can't reach it through DNS rebinding).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::engine::{EngineError, TorrentEngine};
use crate::model::TorrentId;

const CHUNK: usize = 64 * 1024;
const MAX_HEAD: usize = 16 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// A torrent that was just resumed isn't live for a moment; wait this long for it before giving up.
const OPEN_ATTEMPTS: u32 = 40;
const OPEN_RETRY: Duration = Duration::from_millis(500);

pub struct StreamServer {
    port: u16,
    token: String,
}

impl StreamServer {
    /// Binds a random loopback port and serves until the runtime shuts down.
    pub async fn start(engine: Arc<dyn TorrentEngine>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let expected = token.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else { continue };
                let (engine, token) = (engine.clone(), expected.clone());
                tokio::spawn(async move {
                    let _ = serve(sock, engine, &token, port).await;
                });
            }
        });
        Ok(Self { port, token })
    }

    pub fn url(&self, id: &TorrentId, file: usize, name: &str) -> String {
        format!("http://127.0.0.1:{}/s/{}/{}/{}/{}", self.port, self.token, id.0, file, percent_encode(name))
    }
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // RandomState is seeded from the OS; two of them give 128 bits an outside process can't guess.
    (0..2).map(|_| format!("{:016x}", RandomState::new().build_hasher().finish())).collect()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` against a file of `len` bytes → inclusive `(start, end)`.
/// `None` means the range is unsatisfiable.
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    // Multiple ranges aren't supported; take the first, which players accept.
    let first = spec.split(',').next()?.trim();
    let (a, b) = first.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (len.saturating_sub(n.min(len)), len - 1)
        }
        (s, "") => (s.parse().ok()?, len - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(len - 1)),
    };
    (start <= end && start < len).then_some((start, end))
}

pub fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("mov") => "video/quicktime",
        Some("mpg" | "mpeg") => "video/mpeg",
        Some("ts") => "video/mp2t",
        Some("wmv") => "video/x-ms-wmv",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("ogg" | "opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("m4a" | "aac") => "audio/mp4",
        _ => "application/octet-stream",
    }
}

async fn respond(sock: &mut TcpStream, status: &str, extra: &str, body: &str) -> std::io::Result<()> {
    let msg = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
    sock.write_all(msg.as_bytes()).await
}

async fn read_head(sock: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = sock.read(&mut chunk).await?;
        if n == 0 || buf.len() + n > MAX_HEAD {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn serve(mut sock: TcpStream, engine: Arc<dyn TorrentEngine>, token: &str, port: u16) -> std::io::Result<()> {
    let head = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut sock)).await.map_err(|_| std::io::ErrorKind::TimedOut)??;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let headers: HashMap<String, &str> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim())).collect();

    let host_ok = headers.get("host").is_some_and(|h| *h == format!("127.0.0.1:{port}") || *h == format!("localhost:{port}"));
    if !host_ok {
        return respond(&mut sock, "403 Forbidden", "", "forbidden").await;
    }
    if method != "GET" && method != "HEAD" {
        return respond(&mut sock, "405 Method Not Allowed", "Allow: GET, HEAD\r\n", "").await;
    }

    // /s/<token>/<info-hash>/<file index>/<name>
    let parts: Vec<&str> = path.trim_start_matches('/').splitn(5, '/').collect();
    let (id, index, name) = match parts.as_slice() {
        ["s", t, id, index, name] if *t == token => match index.parse::<usize>() {
            Ok(i) if id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()) => (TorrentId(id.to_ascii_lowercase()), i, *name),
            _ => return respond(&mut sock, "404 Not Found", "", "not found").await,
        },
        _ => return respond(&mut sock, "404 Not Found", "", "not found").await,
    };

    let mut opened = None;
    for attempt in 0..OPEN_ATTEMPTS {
        match engine.open_file(&id, index).await {
            Ok(f) => {
                opened = Some(f);
                break;
            }
            Err(EngineError::Backend(_)) if attempt + 1 < OPEN_ATTEMPTS => tokio::time::sleep(OPEN_RETRY).await,
            Err(EngineError::NotFound | EngineError::InvalidInput(_)) => return respond(&mut sock, "404 Not Found", "", "not found").await,
            Err(_) => return respond(&mut sock, "503 Service Unavailable", "Retry-After: 5\r\n", "the torrent is not ready").await,
        }
    }
    let Some(mut file) = opened else {
        return respond(&mut sock, "503 Service Unavailable", "Retry-After: 5\r\n", "the torrent is not ready").await;
    };

    let len = file.len;
    let (status, start, end) = match headers.get("range") {
        None => ("200 OK", 0, len.saturating_sub(1)),
        Some(r) => match parse_range(r, len) {
            Some((s, e)) => ("206 Partial Content", s, e),
            None => return respond(&mut sock, "416 Range Not Satisfiable", &format!("Content-Range: bytes */{len}\r\n"), "").await,
        },
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n",
        content_type(name)
    );
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes()).await?;
    if method == "HEAD" || count == 0 {
        return Ok(());
    }

    file.reader.seek(std::io::SeekFrom::Start(start)).await?;
    let mut left = count;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let want = (left as usize).min(CHUNK);
        let n = file.reader.read(&mut buf[..want]).await?;
        if n == 0 {
            break;
        }
        // A closed connection (the player skipped ahead) is normal; just stop.
        if sock.write_all(&buf[..n]).await.is_err() {
            break;
        }
        left -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::FakeEngine;
    use crate::model::{AddRequest, AddSource};

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)), "end is clamped");
        assert_eq!(parse_range("bytes=0-0,10-20", 1000), Some((0, 0)), "only the first range is honoured");
        assert_eq!(parse_range("bytes=1000-", 1000), None, "starts past the end");
        assert_eq!(parse_range("bytes=50-10", 1000), None);
        assert_eq!(parse_range("bytes=abc", 1000), None);
        assert_eq!(parse_range("items=0-5", 1000), None);
        assert_eq!(parse_range("bytes=0-5", 0), None);
    }

    #[test]
    fn encoding_and_types() {
        assert_eq!(percent_encode("My Movie (2020).mkv"), "My%20Movie%20%282020%29.mkv");
        assert_eq!(content_type("A.MKV"), "video/x-matroska");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    async fn request(port: u16, raw: &str) -> Vec<u8> {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(raw.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        out
    }

    fn split(resp: &[u8]) -> (String, Vec<u8>) {
        let at = resp.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&resp[..at]).into_owned(), resp[at + 4..].to_vec())
    }

    #[tokio::test]
    async fn serves_whole_files_ranges_and_refuses_strangers() {
        let engine = Arc::new(FakeEngine::new());
        let id = engine.add(AddRequest::new(AddSource::Magnet("0123456789abcdef0123456789abcdef01234567".into()))).await.unwrap();
        let server = StreamServer::start(engine.clone()).await.unwrap();
        let url = server.url(&id, 0, "clip one.mp4");
        let path = url.split_once(&format!("127.0.0.1:{}", server.port)).unwrap().1.to_string();
        let host = format!("127.0.0.1:{}", server.port);
        let expected: Vec<u8> = (0..1000u64).map(|i| i.wrapping_mul(7) as u8).collect();

        // whole file
        let (h, body) = split(&request(server.port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 200 OK"), "{h}");
        assert!(h.contains("Content-Type: video/mp4") && h.contains("Accept-Ranges: bytes"));
        assert_eq!(body, expected);

        // a middle range, as a player seeking would ask
        let (h, body) = split(&request(server.port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nRange: bytes=100-199\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 100-199/1000"), "{h}");
        assert_eq!(body, expected[100..200]);

        // HEAD has headers and no body
        let (h, body) = split(&request(server.port, &format!("HEAD {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await);
        assert!(h.contains("Content-Length: 1000") && body.is_empty());

        // impossible range
        let (h, _) = split(&request(server.port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nRange: bytes=5000-\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 416") && h.contains("bytes */1000"));

        // wrong Host (DNS rebinding), wrong token, wrong method, unknown torrent
        let (h, _) = split(&request(server.port, &format!("GET {path} HTTP/1.1\r\nHost: evil.example\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 403"), "{h}");
        let bad = path.replacen(&server.token, "0000", 1);
        let (h, _) = split(&request(server.port, &format!("GET {bad} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 404"), "{h}");
        let (h, _) = split(&request(server.port, &format!("POST {path} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 405"), "{h}");
        let other = path.replacen(&id.0, "ffffffffffffffffffffffffffffffffffffffff", 1);
        let (h, _) = split(&request(server.port, &format!("GET {other} HTTP/1.1\r\nHost: {host}\r\n\r\n")).await);
        assert!(h.starts_with("HTTP/1.1 404"), "{h}");
    }
}
