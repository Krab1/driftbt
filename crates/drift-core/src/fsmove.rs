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
    let copied = copy_file(src, dst)?;
    if copied != std::fs::metadata(src)?.len() {
        let _ = std::fs::remove_file(dst);
        return Err(io::Error::other("the copy came out a different size"));
    }
    std::fs::remove_file(src)
}

/// A plain copy, except that a sparse file stays sparse. The engine only writes the parts of a skipped file that share a
/// download piece with a file you picked, so such a file is mostly empty space; Windows' own copy would write all of
/// that out as zeros and a 600 MB skipped file would take 600 MB.
fn copy_file(src: &Path, dst: &Path) -> io::Result<u64> {
    #[cfg(windows)]
    if let Some(len) = sparse::copy(src, dst)? {
        return Ok(len);
    }
    std::fs::copy(src, dst)
}

#[cfg(windows)]
pub(crate) mod sparse {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SPARSE_FILE;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{FILE_ALLOCATED_RANGE_BUFFER, FSCTL_QUERY_ALLOCATED_RANGES, FSCTL_SET_SPARSE};

    pub fn mark_sparse(f: &File) -> io::Result<()> {
        let mut returned = 0u32;
        // SAFETY: a valid open handle, no input or output buffers.
        let ok = unsafe { DeviceIoControl(f.as_raw_handle(), FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut returned, std::ptr::null_mut()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// The (offset, length) pieces of `f` that hold data, from `start` on, at most 64 at a time.
    fn allocated(f: &File, start: u64, len: u64) -> io::Result<(Vec<(u64, u64)>, bool)> {
        let query = FILE_ALLOCATED_RANGE_BUFFER { FileOffset: start as i64, Length: (len - start) as i64 };
        let mut out = [FILE_ALLOCATED_RANGE_BUFFER { FileOffset: 0, Length: 0 }; 64];
        let mut returned = 0u32;
        // SAFETY: the buffers are the sizes passed; the handle is open.
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle(),
                FSCTL_QUERY_ALLOCATED_RANGES,
                (&raw const query).cast(),
                size_of::<FILE_ALLOCATED_RANGE_BUFFER>() as u32,
                out.as_mut_ptr().cast(),
                size_of_val(&out) as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        let more = ok == 0;
        if more && io::Error::last_os_error().raw_os_error() != Some(ERROR_MORE_DATA as i32) {
            return Err(io::Error::last_os_error());
        }
        let n = returned as usize / size_of::<FILE_ALLOCATED_RANGE_BUFFER>();
        Ok((out[..n].iter().map(|r| (r.FileOffset as u64, r.Length as u64)).collect(), more))
    }

    /// `Ok(None)` when `src` isn't sparse and the caller should copy it normally.
    pub fn copy(src: &Path, dst: &Path) -> io::Result<Option<u64>> {
        let meta = fs::metadata(src)?;
        if meta.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE == 0 {
            return Ok(None);
        }
        let len = meta.len();
        let mut from = File::open(src)?;
        let mut to = OpenOptions::new().write(true).create_new(true).open(dst)?;
        let result = (|| {
            // exFAT, FAT32 and many network shares can't hold sparse files: the copy is then a normal one whose
            // holes read back as zeros, which is still correct.
            let _ = mark_sparse(&to);
            to.set_len(len)?;
            let (mut pos, mut buf) = (0u64, vec![0u8; 1 << 20]);
            while pos < len {
                let (ranges, more) = allocated(&from, pos, len)?;
                if ranges.is_empty() {
                    break;
                }
                for (off, n) in ranges {
                    let (mut at, end) = (off, (off + n).min(len));
                    while at < end {
                        let take = ((end - at) as usize).min(buf.len());
                        from.seek(SeekFrom::Start(at))?;
                        from.read_exact(&mut buf[..take])?;
                        // Zeros inside a data range stay holes in the copy too.
                        if buf[..take].iter().any(|&b| b != 0) {
                            to.seek(SeekFrom::Start(at))?;
                            to.write_all(&buf[..take])?;
                        }
                        at += take as u64;
                    }
                    pos = end;
                }
                if !more {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            drop(to);
            let _ = fs::remove_file(dst);
            return Err(e);
        }
        Ok(Some(len))
    }

    /// Bytes a file really takes on disk (a sparse file takes less than its length).
    pub fn allocated_bytes(p: &Path) -> u64 {
        use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;
        let wide: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(p.as_os_str()).chain(Some(0)).collect();
        let mut hi = 0u32;
        // SAFETY: a NUL-terminated path.
        let lo = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut hi) };
        (u64::from(hi) << 32) | u64::from(lo)
    }
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

    /// A mostly-empty (sparse) file keeps its length and its data when it moves, and does not grow on disk.
    #[cfg(windows)]
    #[test]
    fn a_sparse_file_stays_sparse_when_copied() {
        use std::io::{Seek, SeekFrom, Write};
        let root = scratch("sparse");
        let (src, dst) = (root.join("skipped.bin"), root.join("moved.bin"));
        let len = 200u64 << 20;
        let tail: Vec<u8> = (0..300_000u32).map(|i| (i % 251 + 1) as u8).collect();
        {
            let mut f = std::fs::File::create(&src).unwrap();
            sparse::mark_sparse(&f).unwrap();
            f.set_len(len).unwrap();
            f.seek(SeekFrom::Start(len - tail.len() as u64)).unwrap();
            f.write_all(&tail).unwrap();
        }
        assert!(sparse::allocated_bytes(&src) < 2 << 20, "the test file must start out sparse");
        assert_eq!(copy_file(&src, &dst).unwrap(), len);
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), len);
        assert!(sparse::allocated_bytes(&dst) < 2 << 20, "{} bytes allocated: the copy filled in the holes", sparse::allocated_bytes(&dst));
        let moved = std::fs::read(&dst).unwrap();
        assert_eq!(&moved[moved.len() - tail.len()..], &tail[..]);
        assert!(moved[..moved.len() - tail.len()].iter().all(|&b| b == 0));
        // A normal file still copies as before.
        put(&root.join("plain.txt"), "hello");
        assert_eq!(copy_file(&root.join("plain.txt"), &root.join("plain2.txt")).unwrap(), 5);
        std::fs::remove_dir_all(root).ok();
    }
}
