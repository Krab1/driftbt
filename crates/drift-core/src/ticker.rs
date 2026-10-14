//! The single polling loop: reads the engine on a fixed period and hands each snapshot to a sink.
//! Queue and rules run from here later (M4); the UI never polls.

use std::sync::Arc;
use std::time::Duration;

use crate::engine::TorrentEngine;
use crate::model::TorrentStats;

/// Runs forever; spawn it on the app's runtime. Engine errors skip that tick.
pub async fn run(
    engine: Arc<dyn TorrentEngine>,
    period: Duration,
    mut on_snapshot: impl FnMut(Vec<TorrentStats>) + Send,
) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Ok(list) = engine.list().await {
            on_snapshot(list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fake::FakeEngine;
    use crate::model::{AddRequest, AddSource};

    #[tokio::test]
    async fn emits_snapshots_reflecting_engine_state() {
        let engine = Arc::new(FakeEngine::new());
        engine
            .add(AddRequest::new(AddSource::Magnet("0123456789abcdef0123456789abcdef01234567".into())))
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = tokio::spawn(run(engine, Duration::from_millis(5), move |s| {
            let _ = tx.send(s);
        }));
        let first = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        let second = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        handle.abort();
    }
}
