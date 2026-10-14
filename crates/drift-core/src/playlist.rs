//! `.m3u8` playlists for multi-file torrents (episodes, albums). Pure.

use std::cmp::Ordering;

const MEDIA: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "wmv", "ts", "mpg", "mpeg", "flv", "mp3", "flac", "ogg", "opus", "wav", "m4a", "aac", "wma",
];

pub fn is_media(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| MEDIA.iter().any(|m| ext.eq_ignore_ascii_case(m)))
}

/// Compares text with digit runs as numbers, ignoring case: `E2` sorts before `E10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(&ch) = it.peek() {
                        if !ch.is_ascii_digit() {
                            break;
                        }
                        s.push(ch);
                        it.next();
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let ord = n.len().cmp(&m.len()).then_with(|| n.cmp(&m));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(c), Some(d)) => {
                let ord = c.to_lowercase().cmp(d.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Playlist text for the media files among `files` (paths relative to the playlist's folder), or
/// `None` when there is nothing to play or only one file (a playlist of one is noise).
pub fn m3u8(files: &[String]) -> Option<String> {
    let mut media: Vec<String> = files.iter().filter(|f| is_media(f)).map(|f| f.replace('\\', "/")).collect();
    if media.len() < 2 {
        return None;
    }
    media.sort_by(|a, b| natural_cmp(a, b));
    let mut out = String::from("#EXTM3U\n");
    for f in media {
        out.push_str(&f);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn natural_order_puts_e2_before_e10() {
        let list = m3u8(&s(&["Show.S01E10.mkv", "Show.S01E2.mkv", "Show.S01E1.mkv"])).unwrap();
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines, ["#EXTM3U", "Show.S01E1.mkv", "Show.S01E2.mkv", "Show.S01E10.mkv"]);
    }

    #[test]
    fn keeps_media_only_case_insensitive_and_normalises_slashes() {
        let list = m3u8(&s(&["Album\\02 B.FLAC", "Album/cover.jpg", "Album/01 A.mp3", "readme.txt"])).unwrap();
        assert_eq!(list, "#EXTM3U\nAlbum/01 A.mp3\nAlbum/02 B.FLAC\n");
    }

    #[test]
    fn nothing_or_a_single_file_makes_no_playlist() {
        assert_eq!(m3u8(&s(&["a.txt", "b.jpg"])), None);
        assert_eq!(m3u8(&s(&["only.mkv", "notes.txt"])), None);
        assert_eq!(m3u8(&[]), None);
    }

    #[test]
    fn leading_zeros_and_mixed_case_compare_sensibly() {
        assert_eq!(natural_cmp("ep002", "ep10"), Ordering::Less);
        assert_eq!(natural_cmp("EP2", "ep2"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "a1"), Ordering::Less);
    }
}
