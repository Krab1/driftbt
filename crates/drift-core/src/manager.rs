//! Orchestrates the engine with everything it doesn't know about: the download queue, categories and
//! tags, and seeding goals. The app talks to the manager, never to the engine directly.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::engine::{EngineError, Result, TorrentEngine};
use crate::guard::{self, Adapter, Guard, Verdict};
use crate::library::{Library, MetaStore, TorrentMeta};
use crate::model::{AddRequest, Limits, TorrentId, TorrentState, TorrentStats, TorrentView};
use crate::queue::{self, Move};
use crate::rules::{self, Goal};
use crate::settings::Settings;

pub type SharedSettings = Arc<Mutex<Settings>>;

/// Metadata is saved at once when it changes shape, and at most this often for the seeding clock.
const SAVE_EVERY_SECS: u64 = 30;
const SEED_GOAL_NOTE: &str = "Seeding goal reached";

/// Local time as (weekday 0 = Monday, minutes after midnight); injectable so schedules can be tested.
pub type Clock = Arc<dyn Fn() -> (u8, u16) + Send + Sync>;

fn local_clock() -> (u8, u16) {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    (now.weekday().num_days_from_monday() as u8, (now.hour() * 60 + now.minute()) as u16)
}

/// Which speed limits are in force and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LimitsStatus {
    pub scheduled: bool,
    pub down_bps: Option<u32>,
    pub up_bps: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardStatus {
    pub state: Verdict,
    /// Torrents the guard is holding.
    pub paused: usize,
}

struct State {
    lib: Library,
    applied_limits: Option<Limits>,
    /// Torrents the last tick left waiting for a slot.
    queued: HashSet<TorrentId>,
    last_tick: Option<Instant>,
    last_save: Instant,
    dirty: bool,
    /// Torrents seen incomplete since start-up; only those get a finish time when they complete.
    was_incomplete: HashSet<TorrentId>,
    /// Finished torrents waiting to be moved to the `move_on_complete` folder.
    pending_moves: Vec<TorrentId>,
    /// Short messages shown next to a torrent while it is being moved, or after a move failed.
    move_notes: std::collections::HashMap<TorrentId, String>,
}

pub struct Manager {
    engine: Arc<dyn TorrentEngine>,
    store: Arc<dyn MetaStore>,
    settings: SharedSettings,
    clock: Clock,
    guard: Mutex<Guard>,
    state: Mutex<State>,
}

fn complete(t: &TorrentStats) -> bool {
    t.total_bytes > 0 && t.progress_bytes >= t.total_bytes
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub const MAX_TAGS: usize = 12;
pub const MAX_TAG_LEN: usize = 32;

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TAG_LEN).collect();
        if !t.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&t) || o.to_lowercase() == t.to_lowercase()) {
            out.push(t);
        }
    }
    out.truncate(MAX_TAGS);
    out
}

fn next_position(lib: &Library) -> u64 {
    lib.values().map(|m| m.position).max().map_or(1, |p| p + 1)
}

impl Manager {
    pub fn new(engine: Arc<dyn TorrentEngine>, store: Arc<dyn MetaStore>, settings: SharedSettings) -> Self {
        Self::with_clock(engine, store, settings, Arc::new(local_clock))
    }

    pub fn with_clock(engine: Arc<dyn TorrentEngine>, store: Arc<dyn MetaStore>, settings: SharedSettings, clock: Clock) -> Self {
        let lib = store.load();
        let state = State { lib, applied_limits: None, queued: HashSet::new(), last_tick: None, last_save: Instant::now(), dirty: false, was_incomplete: HashSet::new(), pending_moves: Vec::new(), move_notes: Default::default() };
        Self { engine, store, settings, clock, guard: Mutex::new(Guard::default()), state: Mutex::new(state) }
    }

    fn guard_tripped(&self) -> bool {
        self.guard.lock().unwrap().tripped()
    }

    /// Speed limits for the current local time (a schedule slot wins over the global limits).
    pub fn limits_status(&self) -> LimitsStatus {
        let (weekday, minute) = (self.clock)();
        let (l, scheduled) = self.settings().limits_at(weekday, minute);
        LimitsStatus { scheduled, down_bps: l.down_bps, up_bps: l.up_bps }
    }

    /// Pushes the limits for right now to the engine when they differ from what it has.
    pub async fn sync_limits(&self) -> Result<()> {
        let (weekday, minute) = (self.clock)();
        let (want, _) = self.settings().limits_at(weekday, minute);
        if self.state.lock().unwrap().applied_limits == Some(want) {
            return Ok(());
        }
        self.engine.set_limits(want).await?;
        self.state.lock().unwrap().applied_limits = Some(want);
        Ok(())
    }

    /// Torrents doing network work right now.
    async fn running_ids(&self) -> Result<Vec<TorrentId>> {
        Ok(self
            .engine
            .list()
            .await?
            .into_iter()
            .filter(|t| matches!(t.state, TorrentState::Downloading | TorrentState::Seeding | TorrentState::Checking))
            .map(|t| t.id)
            .collect())
    }

    /// One VPN-guard check: pauses everything while the chosen adapter is missing, and (optionally)
    /// resumes exactly what it paused when the adapter returns.
    pub async fn guard_check(&self, adapters: &[Adapter]) -> Result<GuardStatus> {
        let cfg = self.settings().vpn_guard;
        let v = guard::verdict(&cfg, adapters);
        let running = if v == Verdict::Unprotected { self.running_ids().await? } else { Vec::new() };
        let action = self.guard.lock().unwrap().step(v, cfg.auto_resume, &running);
        for id in &action.pause {
            let _ = self.pause(id).await;
        }
        for id in &action.resume {
            let _ = self.resume(id).await;
        }
        Ok(GuardStatus { state: v, paused: self.guard.lock().unwrap().paused_count() })
    }

    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn save(&self, st: &mut State) {
        // A failed save only costs bookkeeping; the torrents themselves are safe in the engine.
        if let Err(e) = self.store.save(&st.lib) {
            eprintln!("library save failed: {e}");
        }
        st.dirty = false;
        st.last_save = Instant::now();
    }

    fn with_meta(&self, id: &TorrentId, f: impl FnOnce(&mut TorrentMeta)) {
        let mut st = self.state.lock().unwrap();
        let pos = next_position(&st.lib);
        f(st.lib.entry(id.clone()).or_insert_with(|| TorrentMeta { position: pos, added_at: now_secs(), ..Default::default() }));
        self.save(&mut st);
    }

    /// Adds a torrent. The save folder comes from the category if it has one, else the request, else
    /// the default download folder. When every download slot is busy the torrent starts paused and waits.
    pub async fn add(&self, mut req: AddRequest, category: Option<String>, tags: Vec<String>) -> Result<TorrentId> {
        let s = self.settings();
        let category_dir = category
            .as_ref()
            .and_then(|c| s.categories.iter().find(|x| &x.name == c))
            .and_then(|c| c.save_path.clone());
        req.output_dir = category_dir.or(req.output_dir).or(s.download_dir.clone());

        if self.guard_tripped() {
            req.paused = true;
        }
        let max = s.max_active_downloads as usize;
        if max > 0 {
            let busy = self
                .engine
                .list()
                .await?
                .iter()
                .filter(|t| !complete(t) && matches!(t.state, TorrentState::Downloading | TorrentState::Checking))
                .count();
            if busy >= max {
                req.paused = true;
            }
        }

        let id = self.engine.add(req).await?;
        let mut st = self.state.lock().unwrap();
        if !st.lib.contains_key(&id) {
            let position = next_position(&st.lib);
            st.lib.insert(id.clone(), TorrentMeta { category, tags, position, added_at: now_secs(), ..Default::default() });
            self.save(&mut st);
        }
        Ok(id)
    }

    pub async fn pause(&self, id: &TorrentId) -> Result<()> {
        self.engine.pause(id).await?;
        self.with_meta(id, |m| m.user_paused = true);
        Ok(())
    }

    pub async fn resume(&self, id: &TorrentId) -> Result<()> {
        // The guard's own resume goes through here too, after it has cleared its tripped flag.
        if self.guard_tripped() {
            return Err(EngineError::Blocked);
        }
        self.engine.resume(id).await?;
        self.with_meta(id, |m| m.user_paused = false);
        // If every slot is busy this immediately pushes the lowest-priority torrent back into the queue.
        self.tick().await.map(|_| ())
    }

    /// Moves a torrent's files to `dest` (see `TorrentEngine::relocate`). Refused while the VPN guard holds traffic,
    /// because the torrent is re-added at the new place and would reconnect.
    pub async fn relocate(&self, id: &TorrentId, dest: &std::path::Path) -> Result<()> {
        if self.guard_tripped() {
            return Err(EngineError::Blocked);
        }
        std::fs::create_dir_all(dest).map_err(|e| EngineError::Backend(e.to_string()))?;
        self.state.lock().unwrap().move_notes.insert(id.clone(), "Moving files…".into());
        let r = self.engine.relocate(id, dest).await;
        {
            let mut st = self.state.lock().unwrap();
            match &r {
                Ok(()) => drop(st.move_notes.remove(id)),
                Err(e) => {
                    eprintln!("move failed: {e}");
                    st.move_notes.insert(id.clone(), "Couldn't move the files. They are still in their old folder.".into());
                }
            }
        }
        r?;
        self.tick().await.map(|_| ())
    }

    pub fn has_pending_moves(&self) -> bool {
        !self.state.lock().unwrap().pending_moves.is_empty()
    }

    /// Moves the torrents that just finished to the `move_on_complete` folder, one after another.
    pub async fn run_pending_moves(&self) {
        let ids = std::mem::take(&mut self.state.lock().unwrap().pending_moves);
        for id in ids {
            let Some(dest) = self.settings().move_on_complete else { return };
            let _ = self.relocate(&id, &dest).await; // a failure is already shown on the torrent
        }
    }

    /// Verifies the downloaded data again (see `TorrentEngine::recheck`). Refused while the VPN guard holds
    /// traffic, because a recheck re-adds the torrent and would otherwise reconnect it.
    pub async fn recheck(&self, id: &TorrentId) -> Result<()> {
        if self.guard_tripped() {
            return Err(EngineError::Blocked);
        }
        self.engine.recheck(id).await?;
        self.tick().await.map(|_| ())
    }

    /// Makes sure a torrent is running and, if it is still downloading, first in line: what "Play" needs.
    pub async fn prioritize(&self, id: &TorrentId) -> Result<()> {
        if self.guard_tripped() {
            return Err(EngineError::Blocked);
        }
        let list = self.engine.list().await?;
        let t = list.iter().find(|t| &t.id == id).ok_or(crate::engine::EngineError::NotFound)?;
        let incomplete = !complete(t);
        // Resuming a torrent that is already running is an error in the engine, so only wake paused ones.
        if t.state == TorrentState::Paused {
            self.engine.resume(id).await?;
        }
        self.with_meta(id, |m| m.user_paused = false);
        if incomplete {
            self.move_in_queue(id, Move::Top).await
        } else {
            self.tick().await.map(|_| ())
        }
    }

    pub async fn remove(&self, id: &TorrentId, delete_files: bool) -> Result<()> {
        self.engine.remove(id, delete_files).await?;
        let mut st = self.state.lock().unwrap();
        st.lib.remove(id);
        st.queued.remove(id);
        self.save(&mut st);
        Ok(())
    }

    pub fn set_category(&self, id: &TorrentId, category: Option<String>) {
        self.with_meta(id, |m| m.category = category);
    }

    /// Tags are trimmed, de-duplicated ignoring case, and limited in number and length.
    pub fn set_tags(&self, id: &TorrentId, tags: Vec<String>) {
        let tags = clean_tags(tags);
        self.with_meta(id, |m| m.tags = tags);
    }

    /// Replaces this torrent's seeding goal (`None` = use the default) and re-arms it.
    pub fn set_goal(&self, id: &TorrentId, goal: Option<Goal>) {
        self.with_meta(id, |m| {
            m.goal = goal;
            m.goal_reached = false;
        });
    }

    /// Moves a downloading/queued torrent within the queue and applies the new order straight away.
    pub async fn move_in_queue(&self, id: &TorrentId, mv: Move) -> Result<()> {
        let list = self.engine.list().await?;
        {
            let mut st = self.state.lock().unwrap();
            let mut order: Vec<&TorrentStats> = list.iter().filter(|t| !complete(t)).collect();
            order.sort_by_key(|t| (st.lib.get(&t.id).map_or(u64::MAX, |m| m.position), t.id.clone()));
            let ids: Vec<TorrentId> = order.iter().map(|t| t.id.clone()).collect();
            for (n, tid) in queue::moved(&ids, id, mv).into_iter().enumerate() {
                if let Some(m) = st.lib.get_mut(&tid) {
                    m.position = n as u64 + 1;
                }
            }
            self.save(&mut st);
        }
        self.tick().await.map(|_| ())
    }

    /// The list the UI shows: engine stats plus categories, tags, queue positions and notes. No side effects.
    pub async fn snapshot(&self) -> Result<Vec<TorrentView>> {
        let list = self.engine.list().await?;
        let st = self.state.lock().unwrap();
        Ok(Self::views(list, &st, &HashSet::new(), &HashSet::new()))
    }

    fn views(list: Vec<TorrentStats>, st: &State, resumed: &HashSet<TorrentId>, goal_paused: &HashSet<TorrentId>) -> Vec<TorrentView> {
        let mut incomplete: Vec<(u64, &TorrentId)> =
            list.iter().filter(|t| !complete(t)).map(|t| (st.lib.get(&t.id).map_or(u64::MAX, |m| m.position), &t.id)).collect();
        incomplete.sort();
        let rank: std::collections::HashMap<&TorrentId, u32> = incomplete.iter().enumerate().map(|(n, (_, id))| (*id, n as u32 + 1)).collect();

        list.iter()
            .map(|t| {
                let meta = st.lib.get(&t.id);
                let mut stats = t.clone();
                let mut note = None;
                if st.queued.contains(&t.id) && matches!(t.state, TorrentState::Paused | TorrentState::Downloading | TorrentState::Checking) {
                    stats.state = TorrentState::Queued;
                    stats.down_bps = 0;
                    stats.eta_secs = None;
                } else if resumed.contains(&t.id) && t.state == TorrentState::Paused {
                    stats.state = TorrentState::Downloading;
                }
                if goal_paused.contains(&t.id) {
                    stats.state = TorrentState::Paused;
                }
                let moving = st.move_notes.get(&t.id).cloned();
                if meta.is_some_and(|m| m.goal_reached) && stats.state == TorrentState::Paused && !meta.is_some_and(|m| m.user_paused) {
                    note = Some(SEED_GOAL_NOTE.to_string());
                }
                TorrentView {
                    queue_pos: rank.get(&t.id).copied(),
                    category: meta.and_then(|m| m.category.clone()),
                    tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
                    note: moving.or(note),
                    added_at: meta.map(|m| m.added_at).filter(|&a| a > 0),
                    finished_at: meta.and_then(|m| m.finished_at),
                    stats,
                }
            })
            .collect()
    }

    /// One scheduling round: track seeding time, stop torrents whose goal is reached, fill free download
    /// slots, and return the list to show. Called by the ticker, and after any action that changes the plan.
    pub async fn tick(&self) -> Result<Vec<TorrentView>> {
        // A failed limit update must not stop the scheduler; it is retried next tick.
        let _ = self.sync_limits().await;
        let list = self.engine.list().await?;
        let s = self.settings();
        let min_seed = u64::from(s.min_seed_minutes) * 60;

        let (plan, goal_stopped) = {
            let mut st = self.state.lock().unwrap();
            let now = Instant::now();
            let dt = st.last_tick.map_or(0, |t| now.duration_since(t).as_secs().min(10));
            st.last_tick = Some(now);

            let mut structural = false;
            let mut goal_stopped = HashSet::new();
            let mut items = Vec::with_capacity(list.len());
            for t in &list {
                if !st.lib.contains_key(&t.id) {
                    // A torrent we've never seen that is already paused was paused by its owner; keep it that way.
                    let position = next_position(&st.lib);
                    let finished_at = complete(t).then(now_secs);
                    let meta = TorrentMeta { position, added_at: now_secs(), user_paused: t.state == TorrentState::Paused, finished_at, ..Default::default() };
                    st.lib.insert(t.id.clone(), meta);
                    structural = true;
                }
                if !complete(t) {
                    st.was_incomplete.insert(t.id.clone());
                } else if st.was_incomplete.remove(&t.id) {
                    let m = st.lib.get_mut(&t.id).expect("inserted above");
                    if m.finished_at.is_none() {
                        m.finished_at = Some(now_secs());
                        structural = true;
                        if s.move_on_complete.is_some() {
                            st.pending_moves.push(t.id.clone());
                        }
                    }
                }
                let m = st.lib.get_mut(&t.id).expect("inserted above");
                if t.state == TorrentState::Seeding {
                    m.seed_secs += dt;
                    st.dirty = true;
                    let m = st.lib.get_mut(&t.id).expect("present");
                    let goal = rules::effective(m.goal, s.default_goal);
                    if !m.goal_reached && !goal.is_empty() && rules::reached(goal, t.uploaded_bytes, t.total_bytes, m.seed_secs, min_seed) {
                        m.goal_reached = true;
                        goal_stopped.insert(t.id.clone());
                        structural = true;
                    }
                }
                let m = &st.lib[&t.id];
                items.push(queue::Item {
                    id: t.id.clone(),
                    position: m.position,
                    user_paused: m.user_paused,
                    engine_state: t.state,
                    complete: complete(t),
                    goal_stopped: false,
                });
            }
            let plan = queue::plan(&items, s.max_active_downloads as usize);
            st.queued = plan.queued.clone();
            if structural || (st.dirty && st.last_save.elapsed().as_secs() >= SAVE_EVERY_SECS) {
                self.save(&mut st);
            }
            (plan, goal_stopped)
        };

        for id in goal_stopped.iter().chain(plan.pause.iter()) {
            let _ = self.engine.pause(id).await;
        }
        let held = self.guard_tripped();
        if !held {
            for id in &plan.resume {
                let _ = self.engine.resume(id).await;
            }
        }

        let st = self.state.lock().unwrap();
        let resumed: HashSet<TorrentId> = if held { HashSet::new() } else { plan.resume.into_iter().collect() };
        Ok(Self::views(list, &st, &resumed, &goal_stopped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::FakeEngine;
    use crate::library::MemoryMetaStore;
    use crate::model::AddSource;
    use crate::settings::Category;

    fn mag(n: u32) -> AddRequest {
        AddRequest::new(AddSource::Magnet(format!("{n:040x}")))
    }

    fn id(n: u32) -> TorrentId {
        TorrentId(format!("{n:040x}"))
    }

    fn setup(max: u32) -> (Arc<FakeEngine>, Manager, SharedSettings) {
        let engine = Arc::new(FakeEngine::new());
        let settings = Arc::new(Mutex::new(Settings { max_active_downloads: max, download_dir: Some("D:/dl".into()), ..Settings::default() }));
        let m = Manager::new(engine.clone(), Arc::new(MemoryMetaStore::default()), settings.clone());
        (engine, m, settings)
    }

    fn states(v: &[TorrentView]) -> Vec<(u32, TorrentState)> {
        let mut x: Vec<_> = v.iter().map(|t| (u32::from_str_radix(&t.stats.id.0, 16).unwrap(), t.stats.state)).collect();
        x.sort_by_key(|(n, _)| *n);
        x
    }

    #[tokio::test]
    async fn only_max_active_download_and_a_finished_one_frees_its_slot() {
        use TorrentState::*;
        let (engine, m, _) = setup(2);
        for n in 1..=4 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Downloading), (2, Downloading), (3, Queued), (4, Queued)]);
        let pos: Vec<_> = v.iter().map(|t| t.queue_pos.unwrap()).collect();
        assert_eq!(pos, [1, 2, 3, 4]);

        engine.advance(&id(1), 10_000); // torrent 1 finishes and starts seeding
        m.tick().await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Seeding), (2, Downloading), (3, Downloading), (4, Queued)]);
        assert_eq!(v.iter().find(|t| t.stats.id == id(1)).unwrap().queue_pos, None, "finished torrents leave the queue");
    }

    #[tokio::test]
    async fn finish_time_is_stamped_once_when_a_torrent_is_seen_completing() {
        let (engine, m, _) = setup(0);
        m.add(mag(1), None, vec![]).await.unwrap();
        let v = m.tick().await.unwrap();
        assert!(v[0].added_at.is_some() && v[0].finished_at.is_none());
        engine.advance(&id(1), 10_000);
        let first = m.tick().await.unwrap()[0].finished_at;
        assert!(first.is_some());
        assert_eq!(m.tick().await.unwrap()[0].finished_at, first, "stays put on later ticks");
    }

    #[test]
    fn tags_are_cleaned() {
        let raw = vec![" iso ".into(), "ISO".into(), "".into(), "two  words".into(), "x".repeat(100)];
        let c = clean_tags(raw);
        assert_eq!(c[..2], ["iso".to_string(), "two words".to_string()]);
        assert_eq!(c[2].len(), MAX_TAG_LEN);
        assert_eq!(clean_tags((0..50).map(|i| format!("t{i}")).collect()).len(), MAX_TAGS);
    }

    #[tokio::test]
    async fn recheck_goes_to_the_engine_and_keeps_the_torrent_in_the_library() {
        let (engine, m, _) = setup(0);
        m.add(mag(1), None, vec![]).await.unwrap();
        m.set_category(&id(1), Some("Linux".into()));
        m.recheck(&id(1)).await.unwrap();
        assert_eq!(engine.rechecked(), [id(1)]);
        assert_eq!(m.snapshot().await.unwrap()[0].category.as_deref(), Some("Linux"));
        assert!(matches!(m.recheck(&id(9)).await, Err(EngineError::NotFound)));
    }

    #[tokio::test]
    async fn a_torrent_that_finishes_is_moved_to_the_completed_folder_once() {
        let dest = std::env::temp_dir().join(format!("drift-done-{}", std::process::id()));
        let (engine, m, settings) = setup(0);
        settings.lock().unwrap().move_on_complete = Some(dest.clone());
        m.add(mag(1), None, vec![]).await.unwrap();
        m.add(mag(2), None, vec![]).await.unwrap();
        m.tick().await.unwrap();
        assert!(!m.has_pending_moves(), "nothing to move while downloading");
        engine.advance(&id(1), 10_000);
        m.tick().await.unwrap();
        assert!(m.has_pending_moves());
        m.run_pending_moves().await;
        assert_eq!(engine.output_dir(&id(1)), Some(dest.clone()));
        assert_eq!(engine.output_dir(&id(2)), Some("D:/dl".into()), "unfinished torrents stay put");
        assert!(!m.has_pending_moves());
        m.tick().await.unwrap();
        assert!(!m.has_pending_moves(), "a finished torrent is moved once, not at every tick");
        std::fs::remove_dir_all(dest).ok();
    }

    #[tokio::test]
    async fn a_user_paused_torrent_is_never_resumed_by_the_queue() {
        use TorrentState::*;
        let (_, m, _) = setup(2);
        for n in 1..=3 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        m.pause(&id(2)).await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Downloading), (2, Paused), (3, Downloading)], "the paused torrent's slot goes to the next in line");
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Downloading), (2, Paused), (3, Downloading)]);
    }

    #[tokio::test]
    async fn a_torrent_first_seen_paused_stays_paused() {
        use TorrentState::*;
        let (engine, m, _) = setup(2);
        let mut req = mag(9);
        req.paused = true;
        engine.add(req).await.unwrap(); // added behind the manager's back, e.g. by an older version
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(9, Paused)]);
        assert_eq!(m.tick().await.unwrap()[0].stats.state, Paused);
    }

    #[tokio::test]
    async fn prioritize_wakes_a_waiting_torrent_and_is_harmless_on_a_running_one() {
        use TorrentState::*;
        let (_, m, _) = setup(1);
        for n in 1..=3 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        m.tick().await.unwrap();
        // already running: must not fail
        m.prioritize(&id(1)).await.unwrap();
        // a queued torrent jumps the queue and takes the slot
        m.prioritize(&id(3)).await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Queued), (2, Queued), (3, Downloading)]);
        assert!(matches!(m.prioritize(&id(99)).await, Err(crate::engine::EngineError::NotFound)));
    }

    #[tokio::test]
    async fn moving_a_torrent_to_the_top_swaps_who_runs() {
        use TorrentState::*;
        let (_, m, _) = setup(1);
        for n in 1..=3 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        m.tick().await.unwrap();
        m.move_in_queue(&id(3), Move::Top).await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v), [(1, Queued), (2, Queued), (3, Downloading)]);
        assert_eq!(v.iter().find(|t| t.stats.id == id(3)).unwrap().queue_pos, Some(1));
    }

    #[tokio::test]
    async fn unlimited_slots_queue_nothing() {
        let (_, m, _) = setup(0);
        for n in 1..=6 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        assert!(m.tick().await.unwrap().iter().all(|t| t.stats.state == TorrentState::Downloading));
    }

    #[tokio::test]
    async fn the_ratio_goal_stops_seeding_once_and_respects_a_later_resume() {
        use TorrentState::*;
        let (engine, m, settings) = setup(3);
        settings.lock().unwrap().default_goal = Goal { ratio: Some(1.0), minutes: None };
        m.add(mag(1), None, vec![]).await.unwrap();
        engine.advance(&id(1), 10_000);
        m.tick().await.unwrap();
        assert_eq!(m.tick().await.unwrap()[0].stats.state, Seeding, "ratio 0 so far: keep seeding");

        engine.set_uploaded(&id(1), 1000); // fake torrents are 1000 bytes: ratio 1.0
        let v = m.tick().await.unwrap();
        assert_eq!(v[0].stats.state, Paused);
        assert_eq!(v[0].note.as_deref(), Some("Seeding goal reached"));

        m.resume(&id(1)).await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(v[0].stats.state, Seeding, "the goal fires once; the user's resume is respected");
        assert_eq!(v[0].note, None);

        m.set_goal(&id(1), Some(Goal { ratio: Some(1.0), minutes: None }));
        assert_eq!(m.tick().await.unwrap()[0].stats.state, Paused, "a new goal re-arms it");
    }

    #[tokio::test]
    async fn the_minimum_seed_time_holds_back_the_goal() {
        let (engine, m, settings) = setup(3);
        {
            let mut s = settings.lock().unwrap();
            s.default_goal = Goal { ratio: Some(0.5), minutes: None };
            s.min_seed_minutes = 60;
        }
        m.add(mag(1), None, vec![]).await.unwrap();
        engine.advance(&id(1), 10_000);
        engine.set_uploaded(&id(1), 5000);
        m.tick().await.unwrap();
        assert_eq!(m.tick().await.unwrap()[0].stats.state, TorrentState::Seeding, "ratio 5.0 but under the 60 minute minimum");
    }

    #[tokio::test]
    async fn categories_tags_and_removal() {
        let (engine, m, settings) = setup(3);
        settings.lock().unwrap().categories = vec![Category { name: "Linux".into(), save_path: Some("E:/linux".into()) }];
        let a = m.add(mag(1), Some("Linux".into()), vec!["iso".into()]).await.unwrap();
        m.add(mag(2), None, vec![]).await.unwrap();
        let v = m.snapshot().await.unwrap();
        let one = v.iter().find(|t| t.stats.id == a).unwrap();
        assert_eq!((one.category.as_deref(), one.tags.clone()), (Some("Linux"), vec!["iso".to_string()]));
        assert_eq!(engine.output_dir(&a), Some("E:/linux".into()), "the category folder reaches the engine");
        assert_eq!(engine.output_dir(&id(2)), Some("D:/dl".into()), "no category: the default download folder");

        m.set_category(&a, None);
        m.set_tags(&a, vec!["a".into(), "b".into()]);
        let v = m.snapshot().await.unwrap();
        let one = v.iter().find(|t| t.stats.id == a).unwrap();
        assert_eq!((one.category.clone(), one.tags.len()), (None, 2));

        m.remove(&a, false).await.unwrap();
        assert_eq!(m.snapshot().await.unwrap().len(), 1);
    }


    fn clock_at(weekday: u8, minute: u16) -> (Clock, Arc<std::sync::atomic::AtomicU32>) {
        let cell = Arc::new(std::sync::atomic::AtomicU32::new(u32::from(weekday) * 10_000 + u32::from(minute)));
        let c = cell.clone();
        (Arc::new(move || { let v = c.load(std::sync::atomic::Ordering::Relaxed); ((v / 10_000) as u8, (v % 10_000) as u16) }), cell)
    }

    #[tokio::test]
    async fn a_schedule_slot_replaces_the_global_limits_and_hands_them_back_afterwards() {
        use crate::schedule::{Schedule, Slot, MON};
        let engine = Arc::new(FakeEngine::new());
        let settings = Arc::new(Mutex::new(Settings {
            limits_enabled: true,
            down_limit_kib: 5000,
            up_limit_kib: 0,
            schedule: Schedule { enabled: true, slots: vec![Slot { days: MON, from_min: 9 * 60, to_min: 17 * 60, down_kib: 100, up_kib: 50 }] },
            ..Settings::default()
        }));
        let (clock, now) = clock_at(0, 8 * 60);
        let m = Manager::with_clock(engine.clone(), Arc::new(MemoryMetaStore::default()), settings, clock);

        m.tick().await.unwrap();
        assert_eq!(engine.limits(), Limits { down_bps: Some(5000 * 1024), up_bps: None }, "before the slot: global limits");
        assert!(!m.limits_status().scheduled);

        now.store(10 * 60, std::sync::atomic::Ordering::Relaxed); // Monday 10:00
        m.tick().await.unwrap();
        assert_eq!(engine.limits(), Limits { down_bps: Some(100 * 1024), up_bps: Some(50 * 1024) });
        assert!(m.limits_status().scheduled);

        now.store(17 * 60, std::sync::atomic::Ordering::Relaxed); // slot ended
        m.tick().await.unwrap();
        assert_eq!(engine.limits(), Limits { down_bps: Some(5000 * 1024), up_bps: None });
    }

    fn vpn(name: &str, up: bool) -> Vec<Adapter> {
        vec![Adapter { name: name.into(), description: String::new(), up, has_ipv4: true }]
    }

    fn guarded(adapter: &str) -> (Arc<FakeEngine>, Manager) {
        let (engine, m, settings) = setup(5);
        settings.lock().unwrap().vpn_guard = guard::VpnGuard { enabled: true, adapter: Some(adapter.into()), auto_resume: true };
        (engine, m)
    }

    #[tokio::test]
    async fn the_vpn_guard_pauses_everything_holds_the_queue_and_resumes_only_its_own() {
        use TorrentState::*;
        let (engine, m) = guarded("proton");
        for n in 1..=3 {
            m.add(mag(n), None, vec![]).await.unwrap();
        }
        m.pause(&id(3)).await.unwrap(); // the user paused this one themselves

        let up = m.guard_check(&vpn("ProtonVPN", true)).await.unwrap();
        assert_eq!((up.state, up.paused), (Verdict::Protected, 0));
        assert_eq!(states(&m.tick().await.unwrap()), [(1, Downloading), (2, Downloading), (3, Paused)]);

        let down = m.guard_check(&vpn("ProtonVPN", false)).await.unwrap();
        assert_eq!((down.state, down.paused), (Verdict::Unprotected, 2));
        assert_eq!(states(&m.tick().await.unwrap()), [(1, Paused), (2, Paused), (3, Paused)]);

        // while held: a new torrent starts paused, resume is refused, and the queue doesn't leak anything out
        m.add(mag(4), None, vec![]).await.unwrap();
        assert!(matches!(m.resume(&id(1)).await, Err(EngineError::Blocked)));
        assert!(matches!(m.prioritize(&id(1)).await, Err(EngineError::Blocked)));
        let v = m.tick().await.unwrap();
        assert!(v.iter().all(|t| t.stats.state != Downloading), "{:?}", states(&v));
        assert!(engine.list().await.unwrap().iter().all(|t| t.state != Downloading));

        // the adapter returns: the guard's own torrents resume, the user's paused one stays paused
        m.guard_check(&vpn("ProtonVPN", true)).await.unwrap();
        let v = m.tick().await.unwrap();
        assert_eq!(states(&v)[..2], [(1, Downloading), (2, Downloading)]);
        assert_eq!(states(&v)[2], (3, Paused));
    }

    #[tokio::test]
    async fn an_unchosen_adapter_counts_as_unprotected_and_off_is_hands_off() {
        let (_, m) = guarded("");
        m.add(mag(1), None, vec![]).await.unwrap();
        assert_eq!(m.guard_check(&[]).await.unwrap().state, Verdict::Unprotected);
        assert_eq!(m.tick().await.unwrap()[0].stats.state, TorrentState::Paused);

        let (_, m, _) = setup(5);
        m.add(mag(1), None, vec![]).await.unwrap();
        assert_eq!(m.guard_check(&[]).await.unwrap().state, Verdict::Off);
        assert_eq!(m.tick().await.unwrap()[0].stats.state, TorrentState::Downloading);
    }

    #[tokio::test]
    async fn bookkeeping_survives_a_restart() {
        let engine = Arc::new(FakeEngine::new());
        let store: Arc<dyn MetaStore> = Arc::new(MemoryMetaStore::default());
        let settings = Arc::new(Mutex::new(Settings::default()));
        let first = Manager::new(engine.clone(), store.clone(), settings.clone());
        let a = first.add(mag(1), Some("Docs".into()), vec!["x".into()]).await.unwrap();
        first.pause(&a).await.unwrap();
        drop(first);

        let second = Manager::new(engine, store, settings);
        let v = second.tick().await.unwrap();
        assert_eq!(v[0].category.as_deref(), Some("Docs"));
        assert_eq!(v[0].stats.state, TorrentState::Paused, "still user-paused after the restart");
    }
}
