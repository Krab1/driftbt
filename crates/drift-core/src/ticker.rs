//! The single polling loop: runs one manager round on a period and hands each snapshot to a sink.
//! The UI never polls.

use std::sync::Arc;
use std::time::Duration;

use crate::manager::Manager;
use crate::model::TorrentView;

/// Runs forever; spawn it on the app's runtime. `period` is asked before every wait so the app can
/// slow the loop down while the window is hidden. Engine errors skip that tick.
pub async fn run(
    manager: Arc<Manager>,
    period: impl Fn() -> Duration + Send,
    mut on_snapshot: impl FnMut(Vec<TorrentView>) + Send,
) {
    loop {
        if let Ok(list) = manager.tick().await {
            on_snapshot(list);
        }
        if manager.has_pending_moves() {
            // Moving can take a while on another drive, so it never holds up the snapshots.
            let m = manager.clone();
            tokio::spawn(async move { m.run_pending_moves().await });
        }
        tokio::time::sleep(period()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::FakeEngine;
    use crate::engine::TorrentEngine;
    use crate::library::MemoryMetaStore;
    use crate::model::{AddRequest, AddSource};
    use crate::settings::Settings;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    async fn manager_with_one() -> Arc<Manager> {
        let engine = Arc::new(FakeEngine::new());
        engine.add(AddRequest::new(AddSource::Magnet("0123456789abcdef0123456789abcdef01234567".into()))).await.unwrap();
        Arc::new(Manager::new(engine, Arc::new(MemoryMetaStore::default()), Arc::new(Mutex::new(Settings::default()))))
    }

    #[tokio::test]
    async fn emits_snapshots_reflecting_engine_state() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run(manager_with_one().await, || Duration::from_millis(5), move |s| {
            let _ = tx.send(s);
        }));
        let first = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        let second = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!((first.len(), second.len()), (1, 1));
        handle.abort();
    }

    #[tokio::test]
    async fn period_is_asked_before_every_wait() {
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = asked.clone();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run(
            manager_with_one().await,
            move || {
                counter.fetch_add(1, Ordering::Relaxed);
                Duration::from_millis(5)
            },
            move |s| {
                let _ = tx.send(s);
            },
        ));
        for _ in 0..3 {
            rx.recv().await.unwrap();
        }
        assert!(asked.load(Ordering::Relaxed) >= 3);
        handle.abort();
    }

    #[tokio::test]
    async fn a_long_period_stops_the_loop_from_emitting() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run(manager_with_one().await, || Duration::from_secs(3600), move |s| {
            let _ = tx.send(s);
        }));
        rx.recv().await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.is_err());
        handle.abort();
    }
}
