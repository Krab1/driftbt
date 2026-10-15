//! Moving a torrent's files to another folder: same-drive moves are renames, cross-drive moves copy then delete,
//! and a failure part-way puts everything back so the torrent is never left split across two places.

use std::io;
use std::path::{Path, PathBuf};

/// Moves each `rels` entry from `from/<rel>` to `to/<rel>`. Refuses to overwrite anything.
/// On error nothing has moved (files already moved are moved back).
pub fn move_files(from: &Path, to: &Path, rels: &[PathBuf]) -> io::Result<()> {
    for r in rels {
        if to.join(r).exists() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists in the destination folder", r.display())));
        }
    }
    let mut done: Vec<&PathBuf> = Vec::new();
    for r in rels {
        // A file the torrent hasn't created yet (unselected, not downloaded) simply isn't there to move.
        if !from.join(r).exists() {
            continue;
        }
        if let Err(e) = move_one(&from.join(r), &to.join(r)) {
            for d in done.iter().rev() {
                let _ = move_one(&to.join(d), &from.join(d));
            }
            return Err(e);
        }
        done.push(r);
    }
    for r in rels {
        // Tidy up folders the move emptied, never the folder the torrent was saved in.
        let mut dir = from.join(r);
        while dir.pop() && dir != from && dir.starts_with(from) {
            if std::fs::remove_dir(&dir).is_err() {
                break;
            }
        }
    }
    Ok(())
}

fn move_one(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // Different drive (or a rename the OS refused): copy, then remove the original only if the copy is whole.
    let copied = std::fs::copy(src, dst)?;
    if copied != std::fs::metadata(src)?.len() {
        let _ = std::fs::remove_file(dst);
        return Err(io::Error::other("the copy came out a different size"));
    }
    std::fs::remove_file(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("drift-mv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn files_move_with_their_folders_and_the_empty_source_folders_go() {
        let root = scratch("ok");
        let (from, to) = (root.join("from"), root.join("to"));
        put(&from.join("album/cd1/a.bin"), "a");
        put(&from.join("album/b.bin"), "b");
        let rels = [PathBuf::from("album/cd1/a.bin"), PathBuf::from("album/b.bin"), PathBuf::from("album/never-downloaded.bin")];
        move_files(&from, &to, &rels).unwrap();
        assert_eq!(std::fs::read_to_string(to.join("album/cd1/a.bin")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(to.join("album/b.bin")).unwrap(), "b");
        assert!(!from.join("album").exists(), "emptied folders are removed");
        assert!(from.exists(), "the save folder itself stays");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn nothing_is_overwritten_and_nothing_moves_when_a_file_is_in_the_way() {
        let root = scratch("clash");
        let (from, to) = (root.join("from"), root.join("to"));
        put(&from.join("a.bin"), "new");
        put(&from.join("b.bin"), "new");
        put(&to.join("b.bin"), "mine");
        let e = move_files(&from, &to, &[PathBuf::from("a.bin"), PathBuf::from("b.bin")]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(from.join("a.bin").exists() && !to.join("a.bin").exists());
        assert_eq!(std::fs::read_to_string(to.join("b.bin")).unwrap(), "mine");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_failure_part_way_moves_the_first_files_back() {
        let root = scratch("undo");
        let (from, to) = (root.join("from"), root.join("to"));
        put(&from.join("a.bin"), "a");
        put(&from.join("sub/b.bin"), "b");
        // A file named "sub" in the destination makes the second move fail after the first has succeeded.
        put(&to.join("sub"), "i am a file, not a folder");
        let rels = [PathBuf::from("a.bin"), PathBuf::from("sub/b.bin")];
        assert!(move_files(&from, &to, &rels).is_err());
        assert_eq!(std::fs::read_to_string(from.join("a.bin")).unwrap(), "a", "the first file is back");
        assert!(!to.join("a.bin").exists());
        std::fs::remove_dir_all(root).ok();
    }
}
