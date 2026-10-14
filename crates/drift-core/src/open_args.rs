//! What the OS hands us when it opens a magnet link or a .torrent file with the app.

use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenArg {
    Magnet(String),
    TorrentFile(PathBuf),
}

/// Picks magnet links and `.torrent` paths out of command-line arguments (without the program name).
/// Flags and anything else are ignored; duplicates are dropped, order is kept.
pub fn parse(args: &[String]) -> Vec<OpenArg> {
    let mut seen = HashSet::new();
    args.iter()
        .filter_map(|a| {
            let a = a.trim().trim_matches('"');
            let lower = a.to_ascii_lowercase();
            if lower.starts_with("magnet:") {
                Some(OpenArg::Magnet(a.to_string()))
            } else if !a.starts_with("--") && lower.ends_with(".torrent") {
                Some(OpenArg::TorrentFile(PathBuf::from(a)))
            } else {
                None
            }
        })
        .filter(|a| seen.insert(a.clone_key()))
        .collect()
}

impl OpenArg {
    fn clone_key(&self) -> String {
        match self {
            OpenArg::Magnet(m) => format!("m:{m}"),
            OpenArg::TorrentFile(p) => format!("f:{}", p.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn picks_magnets_and_torrent_files_only() {
        let got = parse(&s(&["--autostart", "MAGNET:?xt=urn:btih:abc", "C:\\dl\\Ubuntu.TORRENT", "notes.txt", "-x"]));
        assert_eq!(
            got,
            vec![OpenArg::Magnet("MAGNET:?xt=urn:btih:abc".into()), OpenArg::TorrentFile("C:\\dl\\Ubuntu.TORRENT".into())]
        );
    }

    #[test]
    fn strips_quotes_and_dedups_in_order() {
        let got = parse(&s(&["\"D:\\a b\\x.torrent\"", "magnet:?xt=urn:btih:1", "D:\\a b\\x.torrent", "magnet:?xt=urn:btih:1"]));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], OpenArg::TorrentFile("D:\\a b\\x.torrent".into()));
    }

    #[test]
    fn empty_and_flag_only_args_give_nothing() {
        assert!(parse(&[]).is_empty());
        assert!(parse(&s(&["--minimized", "--help"])).is_empty());
    }
}
