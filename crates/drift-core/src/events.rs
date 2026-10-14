//! Pure comparisons between consecutive snapshots.

use std::collections::HashMap;

use crate::model::{TorrentId, TorrentState, TorrentStats};

/// Torrents that finished downloading between two snapshots: they were `Downloading` before and
/// are `Seeding` now. Torrents that were merely checked and found complete, or that appear for
/// the first time already seeding, are not completions.
pub fn completions<'a>(prev: &[TorrentStats], curr: &'a [TorrentStats]) -> Vec<&'a TorrentStats> {
    let before: HashMap<&TorrentId, TorrentState> = prev.iter().map(|t| (&t.id, t.state)).collect();
    curr.iter()
        .filter(|t| t.state == TorrentState::Seeding && before.get(&t.id) == Some(&TorrentState::Downloading))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, state: TorrentState) -> TorrentStats {
        TorrentStats {
            id: TorrentId(id.into()),
            name: id.into(),
            state,
            error: None,
            total_bytes: 1,
            progress_bytes: 1,
            uploaded_bytes: 0,
            down_bps: 0,
            up_bps: 0,
            peers_live: 0,
            eta_secs: None,
            tracker: None,
        }
    }

    #[test]
    fn only_downloading_to_seeding_counts() {
        let prev = [t("a", TorrentState::Downloading), t("b", TorrentState::Checking), t("c", TorrentState::Paused), t("d", TorrentState::Seeding)];
        let curr = [t("a", TorrentState::Seeding), t("b", TorrentState::Seeding), t("c", TorrentState::Seeding), t("d", TorrentState::Seeding), t("e", TorrentState::Seeding)];
        let done: Vec<&str> = completions(&prev, &curr).iter().map(|t| t.id.0.as_str()).collect();
        assert_eq!(done, ["a"]);
    }

    #[test]
    fn empty_previous_snapshot_reports_nothing() {
        assert!(completions(&[], &[t("a", TorrentState::Seeding)]).is_empty());
    }
}
