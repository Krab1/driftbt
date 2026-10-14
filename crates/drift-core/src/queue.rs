//! Download queue: which incomplete torrents run and which wait for a slot. Pure; the manager applies the plan.

use std::collections::HashSet;

use crate::model::{TorrentId, TorrentState};

pub struct Item {
    pub id: TorrentId,
    /// Queue position, lower runs first.
    pub position: u64,
    /// The user paused it on purpose; the queue never touches it.
    pub user_paused: bool,
    /// What the engine reports right now.
    pub engine_state: TorrentState,
    pub complete: bool,
    /// Stopped by a seeding goal; the queue leaves it alone.
    pub goal_stopped: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub resume: Vec<TorrentId>,
    pub pause: Vec<TorrentId>,
    /// Torrents that are waiting for a free slot (engine-paused by us).
    pub queued: HashSet<TorrentId>,
}

/// `max_active == 0` means unlimited. Only incomplete, unpaused, healthy torrents compete for slots;
/// seeding torrents never take one.
pub fn plan(items: &[Item], max_active: usize) -> Plan {
    let mut candidates: Vec<&Item> = items
        .iter()
        .filter(|i| !i.complete && !i.user_paused && !i.goal_stopped && i.engine_state != TorrentState::Error)
        .collect();
    candidates.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));

    let mut plan = Plan::default();
    for (n, item) in candidates.into_iter().enumerate() {
        let runs = max_active == 0 || n < max_active;
        let running = matches!(item.engine_state, TorrentState::Downloading | TorrentState::Checking);
        if runs {
            if item.engine_state == TorrentState::Paused {
                plan.resume.push(item.id.clone());
            }
        } else {
            plan.queued.insert(item.id.clone());
            if running {
                plan.pause.push(item.id.clone());
            }
        }
    }
    plan
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Move {
    Top,
    Up,
    Down,
    Bottom,
}

/// New order after moving `id` within `order` (ids sorted by current position). Unknown ids change nothing.
pub fn moved(order: &[TorrentId], id: &TorrentId, mv: Move) -> Vec<TorrentId> {
    let mut v = order.to_vec();
    let Some(i) = v.iter().position(|x| x == id) else { return v };
    let to = match mv {
        Move::Top => 0,
        Move::Up => i.saturating_sub(1),
        Move::Down => (i + 1).min(v.len() - 1),
        Move::Bottom => v.len() - 1,
    };
    let item = v.remove(i);
    v.insert(to, item);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use TorrentState::*;

    fn id(s: &str) -> TorrentId {
        TorrentId(s.into())
    }

    fn item(name: &str, position: u64, state: TorrentState) -> Item {
        Item { id: id(name), position, user_paused: false, engine_state: state, complete: false, goal_stopped: false }
    }

    #[test]
    fn first_n_run_the_rest_wait() {
        let items = [item("c", 3, Downloading), item("a", 1, Downloading), item("b", 2, Downloading), item("d", 4, Downloading)];
        let p = plan(&items, 2);
        assert!(p.resume.is_empty());
        assert_eq!(p.pause, vec![id("c"), id("d")]);
        assert_eq!(p.queued, [id("c"), id("d")].into_iter().collect());
    }

    #[test]
    fn a_freed_slot_resumes_the_next_in_line_and_only_it() {
        // a finished (seeding), b runs, c and d wait (paused by us)
        let mut a = item("a", 1, Seeding);
        a.complete = true;
        let items = [a, item("b", 2, Downloading), item("c", 3, Paused), item("d", 4, Paused)];
        let p = plan(&items, 2);
        assert_eq!(p.resume, vec![id("c")]);
        assert!(p.pause.is_empty());
        assert_eq!(p.queued, [id("d")].into_iter().collect());
    }

    #[test]
    fn user_paused_goal_stopped_errors_and_seeds_never_take_a_slot() {
        let mut paused = item("p", 1, Paused);
        paused.user_paused = true;
        let mut stopped = item("g", 2, Paused);
        stopped.goal_stopped = true;
        let errored = item("e", 3, Error);
        let mut seeding = item("s", 4, Seeding);
        seeding.complete = true;
        let items = [paused, stopped, errored, seeding, item("x", 5, Paused)];
        let p = plan(&items, 1);
        assert_eq!(p.resume, vec![id("x")], "the single slot goes to the only real candidate");
        assert!(p.pause.is_empty() && p.queued.is_empty());
    }

    #[test]
    fn zero_means_unlimited_and_checking_counts_as_running() {
        let items = [item("a", 1, Checking), item("b", 2, Paused), item("c", 3, Downloading)];
        let p = plan(&items, 0);
        assert_eq!(p.resume, vec![id("b")]);
        assert!(p.queued.is_empty());
        let p = plan(&items, 1);
        assert_eq!(p.pause, vec![id("c")], "checking a keeps its slot, c is pushed out");
    }

    #[test]
    fn ties_break_by_id_so_the_plan_is_stable() {
        let items = [item("b", 1, Downloading), item("a", 1, Downloading)];
        assert_eq!(plan(&items, 1).pause, vec![id("b")]);
    }

    #[test]
    fn reordering() {
        let o = [id("a"), id("b"), id("c"), id("d")];
        let names = |v: Vec<TorrentId>| v.into_iter().map(|x| x.0).collect::<Vec<_>>().join("");
        assert_eq!(names(moved(&o, &id("c"), Move::Top)), "cabd");
        assert_eq!(names(moved(&o, &id("c"), Move::Up)), "acbd");
        assert_eq!(names(moved(&o, &id("a"), Move::Up)), "abcd", "up from the top stays put");
        assert_eq!(names(moved(&o, &id("b"), Move::Down)), "acbd");
        assert_eq!(names(moved(&o, &id("d"), Move::Down)), "abcd");
        assert_eq!(names(moved(&o, &id("a"), Move::Bottom)), "bcda");
        assert_eq!(names(moved(&o, &id("zzz"), Move::Top)), "abcd");
    }
}
