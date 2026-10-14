//! Watch folder: `.torrent` files dropped into a folder get added, then renamed so they aren't added twice.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Anything larger is not a torrent file.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// A file this recent may still be being written by a browser or copy; give it time before judging it.
pub const SETTLE: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// Modified long enough ago that a bad parse means a bad file, not a half-written one.
    pub settled: bool,
}

/// `*.torrent` files directly inside `dir` (not subfolders). Marker suffixes (`.added`, `.invalid`) never match.
pub fn scan(dir: &Path, now: SystemTime) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Found> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            let meta = e.metadata().ok()?;
            if !meta.is_file() || !name.ends_with(".torrent") || meta.len() == 0 || meta.len() > MAX_BYTES {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            let age = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).unwrap_or_default();
            Some(Found { path, bytes, settled: age >= SETTLE })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[derive(Clone, Copy)]
pub enum Outcome {
    Added,
    Invalid,
}

/// Renames `x.torrent` to `x.torrent.added` / `x.torrent.invalid` (numbered if that name is taken).
pub fn mark(path: &Path, outcome: Outcome) -> std::io::Result<PathBuf> {
    let suffix = match outcome {
        Outcome::Added => "added",
        Outcome::Invalid => "invalid",
    };
    let base = format!("{}.{suffix}", path.display());
    let mut target = PathBuf::from(&base);
    let mut n = 1;
    while target.exists() {
        target = PathBuf::from(format!("{base}.{n}"));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn finds_only_torrent_files_directly_in_the_folder() {
        let d = dir("find");
        std::fs::write(d.join("a.torrent"), b"x").unwrap();
        std::fs::write(d.join("B.TORRENT"), b"y").unwrap();
        std::fs::write(d.join("a.torrent.added"), b"old").unwrap();
        std::fs::write(d.join("notes.txt"), b"n").unwrap();
        std::fs::write(d.join("empty.torrent"), b"").unwrap();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub").join("deep.torrent"), b"z").unwrap();
        let found: Vec<String> = scan(&d, SystemTime::now()).iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(found, ["B.TORRENT", "a.torrent"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_fresh_file_is_not_settled_until_it_has_aged() {
        let d = dir("settle");
        std::fs::write(d.join("a.torrent"), b"x").unwrap();
        assert!(!scan(&d, SystemTime::now())[0].settled);
        assert!(scan(&d, SystemTime::now() + Duration::from_secs(10))[0].settled);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn marking_renames_and_never_overwrites() {
        let d = dir("mark");
        let p = d.join("a.torrent");
        std::fs::write(&p, b"1").unwrap();
        let first = mark(&p, Outcome::Added).unwrap();
        assert!(first.ends_with("a.torrent.added") && !p.exists());
        std::fs::write(&p, b"2").unwrap();
        let second = mark(&p, Outcome::Added).unwrap();
        assert!(second.ends_with("a.torrent.added.1"), "{second:?}");
        assert_eq!(std::fs::read(&first).unwrap(), b"1");
        assert!(scan(&d, SystemTime::now()).is_empty(), "marked files are never picked up again");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_folder_is_just_empty() {
        assert!(scan(Path::new("Z:/definitely/not/here"), SystemTime::now()).is_empty());
    }
}
