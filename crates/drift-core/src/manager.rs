//! Orchestrates the engine with everything it doesn't know about: the download queue, categories and
//! tags, and seeding goals. The app talks to the manager, never to the engine directly.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::engine::{Result, TorrentEngine};
use crate::library::{Library, MetaStore, TorrentMeta};
use crate::model::{AddRequest, TorrentId, TorrentState, TorrentStats, TorrentView};
use crate::queue::{self, Move};
use crate::rules::{self, Goal};
use crate::settings::Settings;

pub type SharedSettings = Arc<Mutex<Settings>>;

/// Metadata is saved at once when it changes shape, and at most this often for the seeding clock.
const SAVE_EVERY_SECS: u64 = 30;
const SEED_GOAL_NOTE: &str = "Seeding goal reached";

struct State {
    lib: Library,
    /// Torrents the last tick left waiting for a slot.
    queued: HashSet<TorrentId>,
    last_tick: Option<Instant>,
    last_save: Instant,
    dirty: bool,
}

pub struct Manager {
    engine: Arc<dyn TorrentEngine>,
    store: Arc<dyn MetaStore>,
    settings: SharedSettings,
    state: Mutex<State>,
}

fn complete(t: &TorrentStats) -> bool {
    t.total_bytes > 0 && t.progress_bytes >= t.total_bytes
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn next_position(lib: &Library) -> u64 {
    lib.values().map(|m| m.position).max().map_or(1, |p| p + 1)
}

impl Manager {
    pub fn new(engine: Arc<dyn TorrentEngine>, store: Arc<dyn MetaStore>, settings: SharedSettings) -> Self {
        let lib = store.load();
        let state = State { lib, queued: HashSet::new(), last_tick: None, last_save: Instant::now(), dirty: false };
        Self { engine, store, settings, state: Mutex::new(state) }
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
        self.engine.resume(id).await?;
        self.with_meta(id, |m| m.user_paused = false);
        // If every slot is busy this immediately pushes the lowest-priority torrent back into the queue.
        self.tick().await.map(|_| ())
    }

    /// Makes sure a torrent is running and, if it is still downloading, first in line: what "Play" needs.
    pub async fn prioritize(&self, id: &TorrentId) -> Result<()> {
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

    pub fn set_tags(&self, id: &TorrentId, tags: Vec<String>) {
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
                if meta.is_some_and(|m| m.goal_reached) && stats.state == TorrentState::Paused && !meta.is_some_and(|m| m.user_paused) {
                    note = Some(SEED_GOAL_NOTE.to_string());
                }
                TorrentView {
                    queue_pos: rank.get(&t.id).copied(),
                    category: meta.and_then(|m| m.category.clone()),
                    tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
                    note,
                    stats,
                }
            })
            .collect()
    }

    /// One scheduling round: track seeding time, stop torrents whose goal is reached, fill free download
    /// slots, and return the list to show. Called by the ticker, and after any action that changes the plan.
    pub async fn tick(&self) -> Result<Vec<TorrentView>> {
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
                    let meta = TorrentMeta { position, added_at: now_secs(), user_paused: t.state == TorrentState::Paused, ..Default::default() };
                    st.lib.insert(t.id.clone(), meta);
                    structural = true;
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
        for id in &plan.resume {
            let _ = self.engine.resume(id).await;
        }

        let st = self.state.lock().unwrap();
        let resumed: HashSet<TorrentId> = plan.resume.into_iter().collect();
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
