//! HTTP front end for the headless server: the same operations the desktop UI uses, behind a bearer token.
//! Everything of substance lives in drift-core; this file only translates HTTP to those calls.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use drift_core::errors::ApiError;
use drift_core::guard::Adapter;
use drift_core::manager::{GuardStatus, LimitsStatus};
use drift_core::model::{NetworkStatus, TorrentDetails, TorrentId, TorrentPreview, TorrentView};
use drift_core::netctl::FilterStatus;
use drift_core::queue::Move;
use drift_core::rules::Goal;
use drift_core::search::SearchReport;
use drift_core::service::{AddInput, Service};
use drift_core::settings::Settings;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

/// A server-sent event: a name and a JSON payload.
#[derive(Clone, Debug)]
pub struct Event {
    pub name: &'static str,
    pub data: String,
}

impl Event {
    pub fn new(name: &'static str, payload: &impl serde::Serialize) -> Self {
        Self { name, data: serde_json::to_string(payload).unwrap_or_else(|_| "null".into()) }
    }
}

pub struct Shared {
    pub svc: Arc<Service>,
    pub token: String,
    pub events: broadcast::Sender<Event>,
    pub guard: Mutex<GuardStatus>,
    /// Served at `/` when set (the built web UI).
    pub ui_dir: Option<PathBuf>,
}

type S = Arc<Shared>;
type Res<T> = Result<Json<T>, HttpError>;

pub struct HttpError(ApiError, StatusCode);

impl From<ApiError> for HttpError {
    fn from(e: ApiError) -> Self {
        let status = match e.code {
            "not_found" => StatusCode::NOT_FOUND,
            "invalid_input" | "no_playlist" | "no_media" | "search" | "no_sources" | "plugin" | "registry_off" => StatusCode::BAD_REQUEST,
            "blocked" => StatusCode::CONFLICT,
            "timeout" => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self(e, status)
    }
}

impl From<drift_core::engine::EngineError> for HttpError {
    fn from(e: drift_core::engine::EngineError) -> Self {
        ApiError::from(e).into()
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.1, Json(self.0)).into_response()
    }
}

/// Equal without leaking, through timing, how much of a guess matched.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

async fn require_token(State(s): State<S>, req: Request, next: Next) -> Response {
    let given = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    if given.is_some_and(|g| same_token(g.trim(), &s.token)) {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(ApiError::new("unauthorized", "Enter the access token for this server."))).into_response()
    }
}

pub fn router(shared: S) -> Router {
    let api = Router::new()
        .route("/torrents", get(list).post(add))
        .route("/inspect", post(inspect))
        .route("/torrents/{id}", delete(remove))
        .route("/torrents/{id}/pause", post(pause))
        .route("/torrents/{id}/resume", post(resume))
        .route("/torrents/{id}/recheck", post(recheck))
        .route("/torrents/{id}/move", post(move_files))
        .route("/torrents/{id}/category", put(set_category))
        .route("/torrents/{id}/tags", put(set_tags))
        .route("/torrents/{id}/goal", put(set_goal))
        .route("/torrents/{id}/limits", put(set_torrent_limits))
        .route("/torrents/{id}/move", put(move_in_queue))
        .route("/torrents/{id}/files", put(set_files))
        .route("/torrents/{id}/details", get(details))
        .route("/torrents/{id}/playlist", post(playlist))
        .route("/settings", get(get_settings).put(put_settings))
        .route("/limits", get(limits))
        .route("/guard", get(guard))
        .route("/adapters", get(adapters))
        .route("/network", get(network))
        .route("/filter", get(filter))
        .route("/filter/update", post(filter_update))
        .route("/search", post(search))
        .route("/search/test/{id}", post(test_search_source))
        .route("/plugins", get(plugins))
        .route("/plugins/{id}/settings", put(set_plugin_settings))
        .route("/plugins/{id}/secrets/{key}", put(set_plugin_secret))
        .route("/plugins/{id}/log", get(plugin_log))
        .route("/events", get(events))
        .layer(middleware::from_fn_with_state(shared.clone(), require_token))
        // Torrent files arrive as JSON number arrays, so allow generous bodies.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        .route("/health", get(health));

    let mut app = Router::new().nest("/api", api);
    if let Some(dir) = &shared.ui_dir {
        // Single-page app: unknown paths get index.html.
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html"))));
    }
    app.with_state(shared)
}

// --- handlers ---------------------------------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "name": "drift", "version": drift_core::VERSION, "auth": "bearer" }))
}

async fn list(State(s): State<S>) -> Res<Vec<TorrentView>> {
    Ok(Json(s.svc.manager.snapshot().await?))
}

#[derive(Deserialize)]
struct AddBody {
    input: AddInput,
    category: Option<String>,
}

async fn add(State(s): State<S>, Json(b): Json<AddBody>) -> Res<TorrentId> {
    Ok(Json(s.svc.add(b.input, b.category).await?))
}

#[derive(Deserialize)]
struct InspectBody {
    input: AddInput,
}

async fn inspect(State(s): State<S>, Json(b): Json<InspectBody>) -> Res<TorrentPreview> {
    Ok(Json(s.svc.inspect(b.input).await?))
}

async fn pause(State(s): State<S>, Path(id): Path<String>) -> Res<()> {
    Ok(Json(s.svc.manager.pause(&TorrentId(id)).await?))
}

#[derive(Deserialize)]
struct RelocateBody {
    dest: String,
}

async fn move_files(State(s): State<S>, Path(id): Path<String>, Json(b): Json<RelocateBody>) -> Res<()> {
    Ok(Json(s.svc.manager.relocate(&TorrentId(id), std::path::Path::new(b.dest.trim())).await?))
}

async fn recheck(State(s): State<S>, Path(id): Path<String>) -> Res<()> {
    Ok(Json(s.svc.manager.recheck(&TorrentId(id)).await?))
}

async fn resume(State(s): State<S>, Path(id): Path<String>) -> Res<()> {
    Ok(Json(s.svc.manager.resume(&TorrentId(id)).await?))
}

#[derive(Deserialize)]
struct RemoveQuery {
    #[serde(default)]
    delete_files: bool,
}

async fn remove(State(s): State<S>, Path(id): Path<String>, Query(q): Query<RemoveQuery>) -> Res<()> {
    Ok(Json(s.svc.manager.remove(&TorrentId(id), q.delete_files).await?))
}

#[derive(Deserialize)]
struct CategoryBody {
    category: Option<String>,
}

async fn set_category(State(s): State<S>, Path(id): Path<String>, Json(b): Json<CategoryBody>) -> Json<()> {
    s.svc.manager.set_category(&TorrentId(id), b.category);
    Json(())
}

#[derive(Deserialize)]
struct TagsBody {
    tags: Vec<String>,
}

async fn set_tags(State(s): State<S>, Path(id): Path<String>, Json(b): Json<TagsBody>) -> Json<()> {
    s.svc.manager.set_tags(&TorrentId(id), b.tags);
    Json(())
}

#[derive(Deserialize)]
struct GoalBody {
    goal: Option<Goal>,
}

async fn set_goal(State(s): State<S>, Path(id): Path<String>, Json(b): Json<GoalBody>) -> Json<()> {
    s.svc.manager.set_goal(&TorrentId(id), b.goal);
    Json(())
}

#[derive(Deserialize)]
struct TorrentLimitsBody {
    down_kib: u32,
    up_kib: u32,
}

async fn set_torrent_limits(State(s): State<S>, Path(id): Path<String>, Json(b): Json<TorrentLimitsBody>) -> Res<()> {
    s.svc.manager.set_torrent_limits(&TorrentId(id), b.down_kib, b.up_kib).await?;
    Ok(Json(()))
}

#[derive(Deserialize)]
struct MoveBody {
    to: Move,
}

async fn move_in_queue(State(s): State<S>, Path(id): Path<String>, Json(b): Json<MoveBody>) -> Res<()> {
    Ok(Json(s.svc.manager.move_in_queue(&TorrentId(id), b.to).await?))
}

#[derive(Deserialize)]
struct FilesBody {
    selected: Vec<usize>,
    #[serde(default)]
    delete_unchecked: bool,
}

async fn set_files(State(s): State<S>, Path(id): Path<String>, Json(b): Json<FilesBody>) -> Res<()> {
    Ok(Json(s.svc.manager.set_files(&TorrentId(id), b.selected, b.delete_unchecked).await?))
}

async fn details(State(s): State<S>, Path(id): Path<String>) -> Res<TorrentDetails> {
    Ok(Json(s.svc.engine.details(&TorrentId(id)).await?))
}

async fn playlist(State(s): State<S>, Path(id): Path<String>) -> Res<String> {
    match drift_core::service::write_playlist(s.svc.engine.as_ref(), &TorrentId(id)).await? {
        Some(p) => Ok(Json(p.display().to_string())),
        None => Err(ApiError::new("no_playlist", "A playlist needs at least two video or audio files.").into()),
    }
}

async fn get_settings(State(s): State<S>) -> Json<Settings> {
    Json(s.svc.settings.lock().unwrap().clone())
}

async fn put_settings(State(s): State<S>, Json(new): Json<Settings>) -> Res<Settings> {
    s.svc.apply_settings(new.clone()).await?;
    Ok(Json(new))
}

async fn limits(State(s): State<S>) -> Json<LimitsStatus> {
    Json(s.svc.manager.limits_status())
}

async fn guard(State(s): State<S>) -> Json<GuardStatus> {
    Json(s.guard.lock().unwrap().clone())
}

async fn adapters() -> Json<Vec<Adapter>> {
    Json(tokio::task::spawn_blocking(drift_core::guard::adapters).await.unwrap_or_default())
}

async fn network(State(s): State<S>) -> Res<NetworkStatus> {
    Ok(Json(s.svc.engine.network_status().await?))
}

async fn filter(State(s): State<S>) -> Json<FilterStatus> {
    let cfg = s.svc.settings.lock().unwrap().clone();
    Json(s.svc.net.filter_status(&cfg))
}

async fn filter_update(State(s): State<S>) -> Res<FilterStatus> {
    Ok(Json(s.svc.update_ip_lists().await?))
}

#[derive(Deserialize)]
struct SearchBody {
    text: String,
}

async fn search(State(s): State<S>, Json(b): Json<SearchBody>) -> Res<SearchReport> {
    Ok(Json(s.svc.search(&b.text).await?))
}

async fn test_search_source(State(s): State<S>, Path(id): Path<String>) -> Res<drift_core::search::SourceStatus> {
    Ok(Json(s.svc.test_search_source(&id).await?))
}

async fn set_plugin_settings(State(s): State<S>, Path(id): Path<String>, Json(values): Json<serde_json::Map<String, serde_json::Value>>) -> Res<()> {
    Ok(Json(s.svc.set_plugin_settings(&id, values).await?))
}

async fn set_plugin_secret(State(s): State<S>, Path((id, key)): Path<(String, String)>, Json(b): Json<KeyBody>) -> Res<()> {
    Ok(Json(s.svc.set_plugin_secret(&id, &key, &b.key).await?))
}

async fn plugin_log(State(s): State<S>, Path(id): Path<String>) -> Res<String> {
    Ok(Json(s.svc.plugin_log(&id)?))
}

async fn plugins(State(s): State<S>) -> Json<Vec<drift_core::plugins::PluginInfo>> {
    Json(s.svc.list_plugins())
}

#[derive(Deserialize)]
struct KeyBody {
    key: String,
}

/// Snapshots every tick plus `completed`, `guard` and `watch` events. A client that falls behind skips ahead:
/// the next snapshot replaces everything it missed.
async fn events(State(s): State<S>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let rx = s.events.subscribe();
    let first = s.svc.manager.snapshot().await.ok().map(|l| Event::new("snapshot", &l));
    let live = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(e) => return Some((e, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let all = stream::iter(first).chain(live).map(|e| Ok::<_, Infallible>(SseEvent::default().event(e.name).data(e.data)));
    Sse::new(all).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use drift_core::engine::fake::FakeEngine;
    use drift_core::engine::TorrentEngine;
    use drift_core::library::MemoryMetaStore;
    use drift_core::manager::Manager;
    use drift_core::netctl::NetControl;
    use drift_core::search::config::Secrets;
    use drift_core::settings::JsonFileStore;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-0123456789";
    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    fn app(tag: &str) -> (Router, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("drift-srv-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine: Arc<dyn TorrentEngine> = Arc::new(FakeEngine::new());
        let settings: drift_core::manager::SharedSettings = Arc::new(Mutex::new(Settings::default()));
        let manager = Arc::new(Manager::new(engine.clone(), Arc::new(MemoryMetaStore::default()), settings.clone()));
        let svc = Arc::new(Service {
            manager,
            engine: engine.clone(),
            net: Arc::new(NetControl::new(engine, dir.join("ipfilter"))),
            store: Arc::new(JsonFileStore::new(dir.join("settings.json"))),
            settings,
            secrets: Secrets::open(dir.join("secrets.json")),
            plugins: drift_core::plugins::Host::at(&dir),
            data_dir: dir.clone(),
        });
        let (tx, _) = broadcast::channel(8);
        let shared = Arc::new(Shared { svc, token: TOKEN.into(), events: tx, guard: Mutex::new(GuardStatus { state: drift_core::guard::Verdict::Off, paused: 0 }), ui_dir: None });
        (router(shared), dir)
    }

    async fn call(app: &Router, method: &str, uri: &str, token: Option<&str>, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
        let mut b = HttpRequest::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let req = match body {
            Some(v) => b.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[test]
    fn token_comparison_is_exact() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd") && !same_token("abc", "ab") && !same_token("", "abc") && !same_token("abcd", "abc"));
    }

    #[tokio::test]
    async fn only_health_is_open_without_the_token() {
        let (app, dir) = app("auth");
        assert_eq!(call(&app, "GET", "/api/health", None, None).await.0, StatusCode::OK);
        for t in [None, Some("wrong"), Some("")] {
            let (st, body) = call(&app, "GET", "/api/torrents", t, None).await;
            assert_eq!(st, StatusCode::UNAUTHORIZED);
            assert_eq!(body["code"], "unauthorized");
        }
        assert_eq!(call(&app, "GET", "/api/settings", Some("x"), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, "GET", "/api/torrents", Some(TOKEN), None).await.0, StatusCode::OK);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn add_list_pause_resume_remove_over_http() {
        let (app, dir) = app("flow");
        let (st, id) = call(&app, "POST", "/api/torrents", Some(TOKEN), Some(serde_json::json!({ "input": { "kind": "magnet", "value": format!("magnet:?xt=urn:btih:{HASH}") } }))).await;
        assert_eq!(st, StatusCode::OK, "{id}");
        let id = id.as_str().unwrap().to_string();

        let (_, list) = call(&app, "GET", "/api/torrents", Some(TOKEN), None).await;
        assert_eq!(list.as_array().unwrap().len(), 1);

        assert_eq!(call(&app, "POST", &format!("/api/torrents/{id}/pause"), Some(TOKEN), None).await.0, StatusCode::OK);
        // The add finishes in the background; a pause pressed meanwhile is applied when the torrent exists.
        let mut list = serde_json::Value::Null;
        for _ in 0..200 {
            list = call(&app, "GET", "/api/torrents", Some(TOKEN), None).await.1;
            if list[0]["state"] == "paused" && list[0]["total_bytes"].as_u64().unwrap_or(0) > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(list[0]["state"], "paused");
        assert_eq!(call(&app, "POST", &format!("/api/torrents/{id}/resume"), Some(TOKEN), None).await.0, StatusCode::OK);

        let (st, d) = call(&app, "GET", &format!("/api/torrents/{id}/details"), Some(TOKEN), None).await;
        assert_eq!(st, StatusCode::OK, "{d}");

        assert_eq!(call(&app, "DELETE", &format!("/api/torrents/{id}"), Some(TOKEN), None).await.0, StatusCode::OK);
        let (_, list) = call(&app, "GET", "/api/torrents", Some(TOKEN), None).await;
        assert!(list.as_array().unwrap().is_empty());

        let (st, e) = call(&app, "POST", "/api/torrents/nope/pause", Some(TOKEN), None).await;
        assert_eq!((st, e["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
        let (st, e) = call(&app, "POST", "/api/torrents", Some(TOKEN), Some(serde_json::json!({ "input": { "kind": "magnet", "value": "garbage" } }))).await;
        assert_eq!((st, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn settings_round_trip_and_search_without_sources_is_a_plain_error() {
        let (app, dir) = app("settings");
        let (_, mut s) = call(&app, "GET", "/api/settings", Some(TOKEN), None).await;
        s["down_limit_kib"] = 512.into();
        s["search"]["archive_org"] = false.into();
        let (st, saved) = call(&app, "PUT", "/api/settings", Some(TOKEN), Some(s)).await;
        assert_eq!(st, StatusCode::OK, "{saved}");
        let (_, again) = call(&app, "GET", "/api/settings", Some(TOKEN), None).await;
        assert_eq!(again["down_limit_kib"], 512);
        assert!(dir.join("settings.json").exists(), "saved to disk, not just memory");

        let (st, e) = call(&app, "POST", "/api/search", Some(TOKEN), Some(serde_json::json!({ "text": "ubuntu" }))).await;
        assert_eq!((st, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("no_sources")));
        let (st, e) = call(&app, "POST", "/api/search/test/not-saved", Some(TOKEN), None).await;
        assert_eq!((st, e["code"].as_str()), (StatusCode::BAD_REQUEST, Some("search")), "an unknown or switched-off source is a plain error");
        std::fs::remove_dir_all(dir).ok();
    }
}
