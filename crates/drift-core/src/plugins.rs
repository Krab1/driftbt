//! Plugins: separate executables found in `<data>/plugins/<id>/` and spoken to over newline-delimited
//! JSON-RPC on stdin/stdout (see `docs/PLUGIN_PROTOCOL.md`). A plugin can crash, hang or lie without taking
//! the app down: calls time out, a crashed plugin is restarted with a growing delay, and after three
//! failures in a row it is switched off until the app restarts. Plugins never touch the engine; they only
//! answer questions (today: search).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub mod registry;

use crate::search::config::Secrets;
use crate::search::{normalize_hash, ProviderError, SearchProvider, SearchQuery, SearchResult};

/// Protocol major version this app speaks.
pub const API: u32 = 1;
const MAX_FAILURES: u32 = 3;
/// One reply line may not be bigger than this.
const MAX_LINE: u64 = 4 * 1024 * 1024;
#[cfg(windows)]
const MEMORY_CAP: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Hosts the plugin says it talks to (`*` = any). Shown to the user; not enforced (see PLUGINS.md).
    pub net: Vec<String>,
    pub fs: Vec<String>,
}

/// One entry of a plugin's settings form. `type` is `text`, `url`, `number`, `bool`, `secret` or `list`.
/// A `list` is a set of rows the user can add and remove; `item` describes the fields of one row (no nested lists).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item: Vec<Field>,
    /// For a `text` field: the only values it may have, drawn as a drop-down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// What each option does, by option: shown under the drop-down while that option is chosen.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub option_help: std::collections::BTreeMap<String, String>,
    /// What the form shows until the user has chosen: a plain field of any kind but `secret`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

const FIELD_KINDS: [&str; 6] = ["text", "url", "number", "bool", "secret", "list"];
const MAX_ROWS: usize = 50;
/// The most rows one plugin call may hand back, whatever limit was asked for.
const MAX_PLUGIN_ROWS: usize = 500;

/// Whether `v` is acceptable for a plain field of this kind.
fn valid_value(kind: &str, v: &Value) -> bool {
    match (kind, v) {
        ("text", Value::String(s)) => s.chars().count() <= 500,
        ("url", Value::String(s)) => s.is_empty() || (is_http_url(s) && s.len() <= 2048 && !s.contains(char::is_whitespace)),
        // A plain number of sensible size: no one needs 1e300 in a plugin form, and a plugin may not cope with it.
        ("number", Value::Number(n)) => n.as_f64().is_some_and(|x| x.is_finite() && x.abs() <= 1e12),
        ("bool", Value::Bool(_)) => true,
        _ => false,
    }
}

/// `http://` or `https://`, in any letter case (a scheme is case-insensitive).
pub(crate) fn is_http_url(s: &str) -> bool {
    let l = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

/// A row is an object with a plain `id` plus only the non-secret fields the list declares.
fn valid_row(f: &Field, row: &Value) -> bool {
    let Some(o) = row.as_object() else { return false };
    o.get("id").and_then(Value::as_str).is_some_and(safe_id) && o.iter().all(|(k, v)| k == "id" || f.item.iter().any(|i| &i.key == k && i.kind != "secret" && valid_value(&i.kind, v)))
}

/// `options` belong to a text field, are few, short and different from each other.
fn options_ok(f: &Field) -> bool {
    (f.options.is_empty() || (f.kind == "text" && f.options.len() <= 20 && f.options.iter().all(|o| !o.is_empty() && o.chars().count() <= 60) && f.options.iter().collect::<std::collections::HashSet<_>>().len() == f.options.len()))
        && f.option_help.iter().all(|(k, v)| f.options.contains(k) && v.chars().count() <= 400)
}

/// The value is allowed for this field of a torrent form (the field's own kind, and one of its options if it has any).
fn valid_option(f: &Field, v: &Value) -> bool {
    valid_value(&f.kind, v) && (f.options.is_empty() || v.as_str().is_some_and(|s| f.options.iter().any(|o| o == s)))
}

/// Keeps only the declared fields with an allowed value: what a plugin or the UI sends is not trusted.
fn sanitize_options(fields: &[Field], v: &Value) -> Map<String, Value> {
    let Some(o) = v.as_object() else { return Map::new() };
    fields.iter().filter_map(|f| o.get(&f.key).filter(|x| valid_option(f, x)).map(|x| (f.key.clone(), x.clone()))).collect()
}

/// The form is well formed: known kinds, unique plain keys, lists only one level deep.
fn form_ok(fields: &[Field], nested: bool) -> bool {
    let mut seen = std::collections::HashSet::new();
    fields.iter().all(|f| {
        let shape = if f.kind == "list" { !nested && !f.item.is_empty() && form_ok(&f.item, true) } else { f.item.is_empty() };
        safe_id(&f.key) && FIELD_KINDS.contains(&f.kind.as_str()) && shape && options_ok(f) && f.default.as_ref().is_none_or(|d| f.kind != "list" && valid_option(f, d)) && seen.insert(f.key.as_str())
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: u32,
    pub kinds: Vec<String>,
    /// What the plugin does, in a sentence or two of plain language, shown in the plugin list.
    #[serde(default)]
    pub description: String,
    /// Program and arguments; the program is a path inside the plugin's folder.
    pub exec: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
    /// The settings form the app draws for this plugin.
    #[serde(default)]
    pub settings: Vec<Field>,
    /// Simulators only: what the user can set for one torrent (text, number or bool fields), asked in the torrent's menu.
    #[serde(default)]
    pub torrent_settings: Vec<Field>,
    /// Simulators only: the plugin answers `get_status`, so the torrent's menu offers its statistics.
    #[serde(default)]
    pub torrent_status: bool,
}

/// What the user typed into plugin settings forms. Plain values live in `.settings.json`, secrets in
/// `.secrets.json` (both beside the plugin folders), so a plugin update never touches them.
struct Config {
    path: PathBuf,
    values: std::sync::Mutex<HashMap<String, Map<String, Value>>>,
    secrets: Secrets,
}

impl Config {
    fn open(dir: &Path) -> Self {
        let path = dir.join(".settings.json");
        let values = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(crate::jsonfile::strip_bom(&b)).ok()).unwrap_or_default();
        Self { path, values: std::sync::Mutex::new(values), secrets: Secrets::open(dir.join(".secrets.json")) }
    }

    /// The saved plain values, limited to what the manifest still declares as a non-secret field. A field that
    /// was removed, or became a secret in a newer version, never reaches the plugin or the UI from here.
    fn values(&self, m: &Manifest) -> Map<String, Value> {
        let mut v = self.values.lock().unwrap().get(&m.id).cloned().unwrap_or_default();
        v.retain(|k, _| m.settings.iter().any(|f| &f.key == k && f.kind != "secret"));
        for f in m.settings.iter().filter(|f| f.kind == "list") {
            if let Some(Value::Array(rows)) = v.get_mut(&f.key) {
                for row in rows.iter_mut().filter_map(Value::as_object_mut) {
                    row.retain(|k, _| k == "id" || f.item.iter().any(|i| &i.key == k && i.kind != "secret"));
                }
            }
        }
        v
    }

    fn secret(&self, id: &str, key: &str) -> String {
        self.secrets.get(&format!("{id}:{key}"))
    }
}

/// Whether `key` names a secret the manifest declares: a top-level secret field, or `<list>.<row id>.<field>` for
/// a secret field of a row that exists in the saved list.
fn declared_secret(m: &Manifest, cfg: &Config, key: &str) -> bool {
    if m.settings.iter().any(|f| f.key == key && f.kind == "secret") {
        return true;
    }
    let parts: Vec<&str> = key.split('.').collect();
    let [list, row, sub] = parts[..] else { return false };
    let Some(f) = m.settings.iter().find(|f| f.key == list && f.kind == "list") else { return false };
    f.item.iter().any(|i| i.key == sub && i.kind == "secret") && cfg.values(m).get(list).and_then(Value::as_array).is_some_and(|rows| rows.iter().any(|r| r["id"] == row))
}

/// A secret's value, and only for keys the manifest declares as secrets.
fn plugin_secret(m: &Manifest, cfg: &Config, key: &str) -> Option<String> {
    declared_secret(m, cfg, key).then(|| cfg.secret(&m.id, key)).filter(|v| !v.is_empty())
}

/// Every secret key the saved form can hold a value for, whether or not one is set.
fn secret_keys(m: &Manifest, cfg: &Config) -> Vec<String> {
    let mut keys: Vec<String> = m.settings.iter().filter(|f| f.kind == "secret").map(|f| f.key.clone()).collect();
    let v = cfg.values(m);
    for f in m.settings.iter().filter(|f| f.kind == "list") {
        for row in v.get(&f.key).and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = row["id"].as_str() else { continue };
            keys.extend(f.item.iter().filter(|i| i.kind == "secret").map(|i| format!("{}.{id}.{}", f.key, i.key)));
        }
    }
    keys
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub call_timeout: Duration,
    pub idle: Duration,
    /// First restart delay; doubles with every failure in a row.
    pub backoff: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { call_timeout: Duration::from_secs(7), idle: Duration::from_secs(300), backoff: Duration::from_secs(2) }
    }
}

#[derive(Debug, thiserror::Error)]
enum CallError {
    #[error("switched off after repeated failures")]
    Disabled,
    #[error("restarting")]
    Backoff,
    #[error("no answer")]
    Failed,
    /// The plugin answered with an error. `reason` is one of the fixed words it may give in `error.data.reason`.
    #[error("the plugin reported an error")]
    Rpc(Option<Reason>),
}

/// Why a plugin says a call failed. The app words the message itself; plugin text is never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Network,
    Auth,
    BadResponse,
    Config,
}

impl Reason {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "network" => Self::Network,
            "auth" => Self::Auth,
            "bad_response" => Self::BadResponse,
            "config" => Self::Config,
            _ => return None,
        })
    }
}

struct Running {
    _child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    next_id: u64,
    last_used: Instant,
    #[cfg(windows)]
    _job: Option<job::Job>,
}

impl Running {
    /// `secret` answers the plugin's own `get_secret` requests while it works on this call.
    async fn rpc(&mut self, method: &str, params: Value, secret: &(dyn Fn(&str) -> Option<String> + Send + Sync)) -> Result<Value, CallError> {
        self.next_id += 1;
        let id = self.next_id;
        let mut line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.map_err(|_| CallError::Failed)?;
        self.stdin.flush().await.map_err(|_| CallError::Failed)?;
        loop {
            let mut buf = String::new();
            // EOF (the plugin exited), an over-long line or invalid UTF-8 all end the call.
            let n = (&mut self.out).take(MAX_LINE).read_line(&mut buf).await.map_err(|_| CallError::Failed)?;
            if n == 0 || !buf.ends_with('\n') {
                return Err(CallError::Failed);
            }
            let Ok(v) = serde_json::from_str::<Value>(&buf) else { return Err(CallError::Failed) };
            if let Some(m) = v.get("method").and_then(Value::as_str) {
                // The one thing a plugin may ask of the host. Everything else it sends is a notification.
                if let ("get_secret", Some(rid)) = (m, v.get("id")) {
                    let value = v["params"]["key"].as_str().and_then(secret);
                    let mut reply = json!({"jsonrpc": "2.0", "id": rid, "result": {"value": value}}).to_string();
                    reply.push('\n');
                    self.stdin.write_all(reply.as_bytes()).await.map_err(|_| CallError::Failed)?;
                    self.stdin.flush().await.map_err(|_| CallError::Failed)?;
                }
                continue;
            }
            // Replies to older calls carry no matching id.
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return match v.get("result") {
                Some(r) if v.get("error").is_none() => Ok(r.clone()),
                _ => Err(CallError::Rpc(v["error"]["data"]["reason"].as_str().and_then(Reason::parse))),
            };
        }
    }

    /// A JSON-RPC notification: no id, no reply.
    async fn notify(&mut self, method: &str, params: Value) -> Result<(), CallError> {
        let mut line = json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.map_err(|_| CallError::Failed)?;
        self.stdin.flush().await.map_err(|_| CallError::Failed)
    }
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    failures: u32,
    /// Failed or timed-out `get_selection` / `selection_changed` calls in a row. Counted apart from `failures` so that
    /// UI polling can't switch a plugin off; only a long run of them counts as one real failure.
    soft_failures: u32,
    retry_at: Option<Instant>,
    disabled: bool,
}

const MAX_SOFT_FAILURES: u32 = 5;
/// How long the host waits for a simulator to take one `torrents_updated` line off the pipe.
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Plugin {
    pub manifest: Manifest,
    dir: PathBuf,
    log: PathBuf,
    limits: Limits,
    cfg: Arc<Config>,
    state: tokio::sync::Mutex<State>,
    /// Counts process starts, so the sender of `torrents_updated` knows when a plugin needs the full list again.
    generation: std::sync::atomic::AtomicU32,
}

impl Plugin {
    fn new(manifest: Manifest, dir: PathBuf, log: PathBuf, limits: Limits, cfg: Arc<Config>) -> Arc<Self> {
        Arc::new(Self { manifest, dir, log, limits, cfg, state: Default::default(), generation: Default::default() })
    }

    fn is_simulator(&self) -> bool {
        self.manifest.kinds.iter().any(|k| k == "simulator")
    }

    /// Ends the running process so the next call starts it with the current settings.
    async fn restart(&self) {
        let mut st = self.state.lock().await;
        st.running = None;
        st.failures = 0;
        st.retry_at = None;
        st.disabled = false;
    }

    async fn start(&self) -> Result<Running, CallError> {
        let exe = self.dir.join(&self.manifest.exec[0]);
        let mut cmd = Command::new(exe);
        cmd.args(&self.manifest.exec[1..]).current_dir(&self.dir).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true);
        // A plugin only gets what it needs to start; nothing from the app's environment.
        for k in ["SystemRoot", "TEMP", "TMP"] {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        // stderr goes to a per-plugin log, never to the UI. Truncated at every start so it can't grow forever.
        match self.log.parent().map(std::fs::create_dir_all).transpose().and_then(|_| std::fs::File::create(&self.log)) {
            Ok(f) => cmd.stderr(Stdio::from(f)),
            Err(_) => cmd.stderr(Stdio::null()),
        };
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|_| CallError::Failed)?;
        let stdin = child.stdin.take().ok_or(CallError::Failed)?;
        let out = BufReader::new(child.stdout.take().ok_or(CallError::Failed)?);
        #[cfg(windows)]
        let _job = job::Job::contain(&child, MEMORY_CAP);
        let mut r = Running {
            _child: child,
            stdin,
            out,
            next_id: 0,
            last_used: Instant::now(),
            #[cfg(windows)]
            _job,
        };
        let init = json!({"api": API, "app_version": env!("CARGO_PKG_VERSION"), "settings": self.cfg.values(&self.manifest)});
        tokio::time::timeout(self.limits.call_timeout, r.rpc("initialize", init, &|k| plugin_secret(&self.manifest, &self.cfg, k))).await.map_err(|_| CallError::Failed)??;
        Ok(r)
    }

    fn failed(&self, st: &mut State) {
        st.running = None; // kill_on_drop ends the process
        st.failures += 1;
        if st.failures >= MAX_FAILURES {
            st.disabled = true;
        } else {
            st.retry_at = Some(Instant::now() + self.limits.backoff * 2u32.pow(st.failures - 1));
        }
    }

    async fn call(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, CallError> {
        self.call_within(method, params, self.limits.call_timeout).await
    }

    async fn call_within(self: &Arc<Self>, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        self.call_inner(method, params, timeout, false, true).await
    }

    /// For UI-driven simulator calls: a timeout or a dropped pipe is counted apart from the lifecycle failures above.
    async fn call_soft(self: &Arc<Self>, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        self.call_inner(method, params, timeout, true, true).await
    }

    /// For the reads the UI repeats on its own (the list of what is simulated, a form's values, the statistics): a timeout
    /// or a plugin that doesn't know the method never counts as a failure, so looking can't switch a plugin off.
    async fn call_view(self: &Arc<Self>, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        self.call_inner(method, params, timeout, true, false).await
    }

    /// Starts the process if it isn't running, unless it is switched off or backing off.
    async fn ensure_running(self: &Arc<Self>, st: &mut State) -> Result<(), CallError> {
        if st.disabled {
            return Err(CallError::Disabled);
        }
        if st.retry_at.is_some_and(|t| Instant::now() < t) {
            return Err(CallError::Backoff);
        }
        if st.running.is_none() {
            match tokio::time::timeout(self.limits.call_timeout * 2, self.start()).await {
                Ok(Ok(r)) => {
                    st.running = Some(r);
                    self.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // A simulator owns running jobs, so it stays up until it is switched off or the app ends.
                    if !self.is_simulator() {
                        tokio::spawn(idle_watch(Arc::downgrade(self)));
                    }
                }
                _ => {
                    self.failed(st);
                    return Err(CallError::Failed);
                }
            }
        }
        Ok(())
    }

    async fn call_inner(self: &Arc<Self>, method: &str, params: Value, timeout: Duration, soft: bool, count: bool) -> Result<Value, CallError> {
        let mut st = self.state.lock().await;
        self.ensure_running(&mut st).await?;
        let r = st.running.as_mut().expect("started above");
        match tokio::time::timeout(timeout, r.rpc(method, params, &|k| plugin_secret(&self.manifest, &self.cfg, k))).await {
            Ok(Ok(v)) => {
                r.last_used = Instant::now();
                st.failures = 0;
                st.soft_failures = 0;
                Ok(v)
            }
            // The plugin answered, just not usefully: it is alive, so this isn't a crash.
            Ok(Err(CallError::Rpc(reason))) => {
                r.last_used = Instant::now();
                Err(CallError::Rpc(reason))
            }
            res => {
                if !soft {
                    self.failed(&mut st);
                } else {
                    // A late reply is skipped by id, so a timeout leaves the process usable; a broken pipe doesn't.
                    if res.is_ok() {
                        st.running = None;
                    }
                    // A timeout of a repeated UI read costs nothing; a plugin that dies on one still counts, or it would be respawned forever.
                    st.soft_failures += u32::from(count || res.is_ok());
                    if st.soft_failures >= MAX_SOFT_FAILURES {
                        st.soft_failures = 0;
                        self.failed(&mut st);
                    }
                }
                Err(CallError::Failed)
            }
        }
    }

    /// Sends a notification, starting the process first if needed. A pipe that stays full means the plugin isn't reading:
    /// that counts as a real failure.
    async fn notify(self: &Arc<Self>, method: &str, params: Value) -> Result<(), CallError> {
        let mut st = self.state.lock().await;
        self.ensure_running(&mut st).await?;
        let r = st.running.as_mut().expect("started above");
        match tokio::time::timeout(NOTIFY_TIMEOUT, r.notify(method, params)).await {
            Ok(Ok(())) => Ok(()),
            _ => {
                self.failed(&mut st);
                Err(CallError::Failed)
            }
        }
    }

    /// Asks a running process to end. Does nothing while a call is in progress; the caller tries again later.
    async fn stop(&self) {
        let Ok(mut st) = self.state.try_lock() else { return };
        if let Some(r) = st.running.as_mut() {
            let _ = tokio::time::timeout(Duration::from_millis(500), r.rpc("shutdown", json!({}), &|_| None)).await;
        }
        st.running = None;
    }
}

/// Ends the process once it has been idle for `limits.idle`.
async fn idle_watch(p: std::sync::Weak<Plugin>) {
    loop {
        let Some(plugin) = p.upgrade() else { return };
        let idle = plugin.limits.idle;
        drop(plugin);
        tokio::time::sleep((idle / 4).max(Duration::from_millis(20))).await;
        let Some(plugin) = p.upgrade() else { return };
        let mut st = plugin.state.lock().await;
        match st.running.as_mut() {
            None => return,
            Some(r) if r.last_used.elapsed() >= idle => {
                let _ = tokio::time::timeout(Duration::from_millis(500), r.rpc("shutdown", json!({}), &|_| None)).await;
                st.running = None;
                return;
            }
            _ => {}
        }
    }
}

/// What the UI shows for one plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kinds: Vec<String>,
    /// What it does, from its manifest; empty if it doesn't say.
    #[serde(default)]
    pub description: String,
    pub permissions: Permissions,
    pub enabled: bool,
    /// Why it can't be used (wrong protocol version, missing program, switched off after crashes).
    pub problem: Option<String>,
    /// The settings form, the current plain values, and which secret fields already hold a value (never the values).
    pub fields: Vec<Field>,
    pub values: Map<String, Value>,
    pub secrets_set: Vec<String>,
    /// A version installed from the registry that takes over at the next start, and the version a rollback would bring back.
    pub pending: Option<String>,
    pub previous: Option<String>,
}

struct Entry {
    plugin: Option<Arc<Plugin>>,
    info: PluginInfo,
}

/// The plugins found at start-up. Which of them are on is the user's choice and lives in settings.
pub struct Host {
    entries: Vec<Entry>,
    cfg: Arc<Config>,
    dir: PathBuf,
    /// Plugins whose registry update arrived at this start and asks for more access (or is new): off until approved.
    needs_approval: Vec<String>,
    /// What each simulator plugin last said it simulates, by plugin id. A plugin missing here is unknown (never asked, or
    /// the last `get_selection` failed).
    sel: std::sync::Mutex<HashMap<String, Vec<Selection>>>,
}

/// One torrent a simulator plugin reports on: whether it simulates it, and a short status line for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub info_hash: String,
    pub enabled: bool,
    #[serde(default)]
    pub label: Option<String>,
}

/// What the UI shows for one simulator plugin. `ok` is false when it didn't answer `get_selection`; the rows are then empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulatorState {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub selections: Vec<Selection>,
    /// What the plugin lets the user set per torrent (empty: the menu item just switches it on or off).
    #[serde(default)]
    pub fields: Vec<Field>,
    /// Whether the plugin has statistics to show (`get_status`).
    #[serde(default)]
    pub has_status: bool,
}

/// A simulator's live numbers for one torrent, as tables of text and a short log. The plugin keeps it in memory; Drift
/// only passes it on when the user opens the statistics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimStatus {
    pub tables: Vec<SimTable>,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SimTable {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

fn cut(v: &Value, max: usize) -> String {
    // Control characters and the invisible direction marks that could make a cell read differently from what it holds.
    v.as_str().map(|s| s.chars().filter(|c| !c.is_control() && !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')).take(max).collect()).unwrap_or_default()
}

/// Plugin output is untrusted: a handful of short tables and log lines.
fn sanitize_status(v: &Value) -> Option<SimStatus> {
    let s = v.get("status").unwrap_or(v);
    let tables = s["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .take(6)
        .map(|t| SimTable {
            title: cut(&t["title"], 60),
            columns: t["columns"].as_array().into_iter().flatten().take(12).map(|c| cut(c, 40)).collect(),
            rows: t["rows"].as_array().into_iter().flatten().take(50).map(|r| r.as_array().into_iter().flatten().take(12).map(|c| cut(c, 120)).collect()).collect(),
        })
        .collect();
    let log = s["log"].as_array().into_iter().flatten().take(60).map(|l| cut(l, 200)).collect();
    s.is_object().then_some(SimStatus { tables, log })
}

const SELECTION_TIMEOUT: Duration = Duration::from_secs(2);
const SELECTION_CHANGED_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SELECTIONS: usize = 5000;

/// Plugin output is untrusted: keep well-formed rows with a known hash form and a short label.
fn sanitize_selections(v: &Value) -> Option<Vec<Selection>> {
    let rows = v.get("selections")?.as_array()?;
    Some(
        rows.iter()
            .filter_map(|r| {
                let info_hash = normalize_hash(r["info_hash"].as_str()?)?;
                let label = r["label"].as_str().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect::<String>()).filter(|l| !l.is_empty());
                Some(Selection { info_hash, enabled: r["enabled"].as_bool()?, label })
            })
            .take(MAX_SELECTIONS)
            .collect(),
    )
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A relative path that stays inside the plugin's folder.
fn inside(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty() && path.is_relative() && !path.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

fn check(m: &Manifest, dir: &Path) -> Result<(), String> {
    let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !safe_id(&m.id) || m.id != folder {
        return Err("Its id doesn't match its folder name.".into());
    }
    if m.api != API {
        return Err(format!("It was made for plugin protocol {}, and this Drift speaks {API}.", m.api));
    }
    if !form_ok(&m.settings, false) {
        return Err("Its settings form isn't valid.".into());
    }
    if m.settings.iter().chain(m.settings.iter().flat_map(|f| f.item.iter())).any(|f| !f.options.is_empty()) {
        return Err("Its settings form isn't valid (drop-down options are only for per-torrent settings).".into());
    }
    if !form_ok(&m.torrent_settings, false) || m.torrent_settings.iter().any(|f| !matches!(f.kind.as_str(), "text" | "number" | "bool")) {
        return Err("Its per-torrent settings form isn't valid.".into());
    }
    if m.exec.is_empty() || !inside(&m.exec[0]) || !dir.join(&m.exec[0]).is_file() {
        return Err("Its program is missing.".into());
    }
    Ok(())
}

impl Host {
    pub fn empty() -> Self {
        Self { entries: Vec::new(), cfg: Arc::new(Config { path: PathBuf::new(), values: Default::default(), secrets: Secrets::open(PathBuf::new()) }), dir: PathBuf::new(), needs_approval: Vec::new(), sel: Default::default() }
    }

    /// The plugins under `<data>/plugins`, with default limits and logs in `<data>/plugin-logs`.
    pub fn at(data_dir: &Path) -> Arc<Self> {
        Self::at_with_bundled(data_dir, None)
    }

    /// Like `at`, and first installs or upgrades the plugins that ship inside the app (see `registry::provision`).
    pub fn at_with_bundled(data_dir: &Path, bundled: Option<&Path>) -> Arc<Self> {
        if let Some(b) = bundled {
            registry::provision(b, &data_dir.join("plugins"));
        }
        // Updates and rollbacks chosen last time take over now, before anything is running.
        let needs_approval = registry::apply_pending(&data_dir.join("plugins"));
        let mut host = Self::load(&data_dir.join("plugins"), &data_dir.join("plugin-logs"), Limits::default());
        host.needs_approval = needs_approval;
        Arc::new(host)
    }

    /// Reads every `<dir>/<id>/plugin.json`. Broken plugins are listed with the reason instead of being dropped.
    pub fn load(dir: &Path, logs: &Path, limits: Limits) -> Self {
        let mut entries = Vec::new();
        let cfg = Arc::new(Config::open(dir));
        let Ok(rd) = std::fs::read_dir(dir) else { return Self { entries, cfg, dir: dir.to_path_buf(), needs_approval: Vec::new(), sel: Default::default() } };
        let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let Ok(bytes) = std::fs::read(d.join("plugin.json")) else { continue };
            let folder = d.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let m: Manifest = match serde_json::from_slice(crate::jsonfile::strip_bom(&bytes)) {
                Ok(m) => m,
                Err(_) => {
                    let info = PluginInfo { id: folder.clone(), name: folder, version: String::new(), kinds: vec![], description: String::new(), permissions: Permissions::default(), enabled: false, problem: Some("Its plugin.json can't be read.".into()), fields: vec![], values: Map::new(), secrets_set: vec![], pending: None, previous: None };
                    entries.push(Entry { plugin: None, info });
                    continue;
                }
            };
            let problem = check(&m, &d).err();
            let info = PluginInfo { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), kinds: m.kinds.clone(), description: m.description.clone(), permissions: m.permissions.clone(), enabled: false, problem: problem.clone(), fields: m.settings.clone(), values: Map::new(), secrets_set: vec![], pending: None, previous: None };
            let plugin = problem.is_none().then(|| Plugin::new(m.clone(), d.clone(), logs.join(format!("{}.log", m.id)), limits, cfg.clone()));
            entries.push(Entry { plugin, info });
        }
        Self { entries, cfg, dir: dir.to_path_buf(), needs_approval: Vec::new(), sel: Default::default() }
    }

    /// Plugins that came in through the registry at this start and must be approved again before they run.
    pub fn needs_approval(&self) -> &[String] {
        &self.needs_approval
    }

    pub fn list(&self, enabled: &[String]) -> Vec<PluginInfo> {
        self.entries
            .iter()
            .map(|e| {
                let mut i = e.info.clone();
                i.enabled = e.plugin.is_some() && enabled.contains(&i.id);
                if let Some(p) = &e.plugin {
                    i.values = self.cfg.values(&p.manifest);
                }
                i.pending = registry::pending_version(&self.dir, &i.id);
                i.previous = registry::previous_version(&self.dir, &i.id);
                if let Some(p) = &e.plugin {
                    i.secrets_set = secret_keys(&p.manifest, &self.cfg).into_iter().filter(|k| !self.cfg.secret(&i.id, k).is_empty()).collect();
                }
                if let Some(p) = &e.plugin {
                    // `try_lock`: a plugin in the middle of a call is by definition not switched off.
                    if p.state.try_lock().is_ok_and(|s| s.disabled) {
                        i.problem = Some("It stopped responding and was switched off. Save its settings or restart Drift to try it again.".into());
                    }
                }
                i
            })
            .collect()
    }

    fn find(&self, id: &str) -> Result<&Arc<Plugin>, String> {
        self.entries.iter().find_map(|e| e.plugin.as_ref().filter(|p| p.manifest.id == id)).ok_or_else(|| "That plugin isn't available.".to_string())
    }

    /// Saves the plain (non-secret) values of a plugin's form and restarts it so it sees them.
    pub async fn set_values(&self, id: &str, values: Map<String, Value>) -> Result<(), String> {
        let p = self.find(id)?;
        for (k, v) in &values {
            let f = p.manifest.settings.iter().find(|f| &f.key == k && f.kind != "secret").ok_or("That setting doesn't exist.")?;
            let ok = match (f.kind.as_str(), v) {
                ("list", Value::Array(rows)) => {
                    let ids: std::collections::HashSet<_> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
                    rows.len() <= MAX_ROWS && ids.len() == rows.len() && rows.iter().all(|r| valid_row(f, r))
                }
                (kind, v) => valid_value(kind, v),
            };
            if !ok {
                return Err(format!("\"{}\" doesn't have a valid value.", f.label));
            }
        }
        // A row whose address was edited loses its saved secrets: a key stored for one server must not be sent to another.
        let old = self.cfg.values(&p.manifest);
        for f in p.manifest.settings.iter().filter(|f| f.kind == "list") {
            let before = old.get(&f.key).and_then(Value::as_array);
            for row in values.get(&f.key).and_then(Value::as_array).into_iter().flatten() {
                let Some(rid) = row["id"].as_str() else { continue };
                let prev = before.and_then(|b| b.iter().find(|r| r["id"] == rid));
                let moved = prev.is_some_and(|pr| f.item.iter().filter(|i| i.kind == "url").any(|i| pr[&i.key] != row[&i.key]));
                if moved {
                    for i in f.item.iter().filter(|i| i.kind == "secret") {
                        let _ = self.cfg.secrets.set(&format!("{id}:{}.{rid}.{}", f.key, i.key), "");
                    }
                }
            }
        }
        {
            let mut all = self.cfg.values.lock().unwrap();
            all.insert(id.to_string(), values);
            let bytes = serde_json::to_vec_pretty(&*all).map_err(|e| e.to_string())?;
            crate::jsonfile::write_atomic(&self.cfg.path, &bytes).map_err(|_| "Couldn't save. Check that the app data folder is writable.".to_string())?;
        }
        // Rows the user removed take their secrets with them.
        let live = secret_keys(&p.manifest, &self.cfg);
        let prefix = format!("{id}:");
        let keep: Vec<String> = self.cfg.secrets.keys().into_iter().filter(|k| !k.starts_with(&prefix) || !k.contains('.') || live.contains(&k[prefix.len()..].to_string())).collect();
        let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
        let _ = self.cfg.secrets.retain(&keep);
        p.restart().await;
        Ok(())
    }

    /// Saves one secret field. An empty value removes it.
    pub async fn set_secret(&self, id: &str, key: &str, value: &str) -> Result<(), String> {
        let p = self.find(id)?;
        if !declared_secret(&p.manifest, &self.cfg, key) {
            return Err("That isn't a secret setting. Save the list first if this is a new row.".into());
        }
        // The old value may be in the log; the log is only rewritten when the plugin next starts.
        let value = value.trim();
        if value.chars().count() > 4096 || value.chars().any(char::is_control) {
            return Err("That key is too long or has line breaks in it.".into());
        }
        let _ = std::fs::remove_file(&p.log);
        self.cfg.secrets.set(&format!("{id}:{key}"), value).map_err(|_| "Couldn't save. Check that the app data folder is writable.".to_string())?;
        p.restart().await;
        Ok(())
    }

    /// The end of what the plugin wrote to its error output, with its secrets blanked out. Empty if it never ran.
    pub fn log(&self, id: &str) -> Result<String, String> {
        const TAIL: u64 = 32 * 1024;
        let p = self.find(id)?;
        let mut text = match std::fs::File::open(&p.log) {
            Ok(mut f) => {
                use std::io::{Read, Seek, SeekFrom};
                let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL)));
                let mut b = Vec::new();
                let _ = f.take(TAIL).read_to_end(&mut b);
                let mut t = String::from_utf8_lossy(&b).into_owned();
                if len > TAIL {
                    // Start at a line boundary so a secret can't be cut in two and slip past the blanking below.
                    t = t.split_once('\n').map(|x| x.1.to_string()).unwrap_or_default();
                }
                t
            }
            Err(_) => String::new(),
        };
        for v in self.cfg.secrets.values_with_prefix(&format!("{id}:")) {
            text = text.replace(&v, "••••");
        }
        Ok(text)
    }

    /// Whether the plugin's process is alive right now (it starts on first use and ends when idle).
    pub async fn is_running(&self, id: &str) -> bool {
        match self.entries.iter().find_map(|e| e.plugin.as_ref().filter(|p| p.manifest.id == id)) {
            Some(p) => p.state.lock().await.running.is_some(),
            None => false,
        }
    }

    /// Whether any enabled plugin wants to hear about finished torrents.
    pub fn has_postprocessors(&self, enabled: &[String]) -> bool {
        self.postprocessors(enabled).next().is_some()
    }

    fn postprocessors<'a>(&'a self, enabled: &'a [String]) -> impl Iterator<Item = &'a Arc<Plugin>> {
        self.entries.iter().filter_map(|e| e.plugin.as_ref()).filter(move |p| enabled.contains(&p.manifest.id) && p.manifest.kinds.iter().any(|k| k == "postprocess"))
    }

    /// Tells every enabled post-processing plugin that a torrent finished, one after another, and returns what each
    /// asked for (or `Err` for a plugin that failed). Plugin output is untrusted: see `sanitize_actions`.
    pub async fn completed(&self, enabled: &[String], event: &impl Serialize) -> Vec<(String, Result<Vec<Action>, ()>)> {
        let event = serde_json::to_value(event).unwrap_or(Value::Null);
        let mut out = Vec::new();
        for p in self.postprocessors(enabled) {
            let r = p.call_within("completed", event.clone(), p.limits.call_timeout * 40).await.map(|v| sanitize_actions(&v)).map_err(|_| ());
            out.push((p.manifest.name.clone(), r));
        }
        out
    }

    fn simulators<'a>(&'a self, enabled: &'a [String]) -> impl Iterator<Item = &'a Arc<Plugin>> {
        self.entries.iter().filter_map(|e| e.plugin.as_ref()).filter(move |p| enabled.contains(&p.manifest.id) && p.is_simulator())
    }

    /// Ids of the enabled plugins of kind `simulator`.
    pub fn simulator_ids(&self, enabled: &[String]) -> Vec<String> {
        self.simulators(enabled).map(|p| p.manifest.id.clone()).collect()
    }

    /// How many times the plugin's process has been started. A change means it lost whatever it had been told.
    pub fn generation(&self, id: &str) -> u32 {
        self.find(id).map_or(0, |p| p.generation.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Sends `torrents_updated` (a notification: no reply is awaited) to one simulator, starting it if needed.
    /// Returns whether the plugin took it.
    pub async fn torrents_updated(&self, id: &str, event: &impl Serialize) -> bool {
        let (Ok(p), Ok(v)) = (self.find(id), serde_json::to_value(event)) else { return false };
        p.notify("torrents_updated", v).await.is_ok()
    }

    /// Asks one simulator what it simulates and remembers the answer. Failures are logged and leave the plugin unknown.
    async fn refresh_selection(&self, p: &Arc<Plugin>) -> SimulatorState {
        let id = p.manifest.id.clone();
        let rows = match p.call_view("get_selection", json!({}), SELECTION_TIMEOUT).await {
            Ok(v) => sanitize_selections(&v),
            Err(_) => None,
        };
        let mut sel = self.sel.lock().unwrap();
        match rows {
            Some(rows) => {
                sel.insert(id.clone(), rows.clone());
                SimulatorState { id, name: p.manifest.name.clone(), ok: true, selections: rows, fields: p.manifest.torrent_settings.clone(), has_status: p.manifest.torrent_status }
            }
            None => {
                eprintln!("simulator plugin {id}: get_selection failed");
                sel.remove(&id);
                SimulatorState { id, name: p.manifest.name.clone(), ok: false, selections: vec![], fields: vec![], has_status: false }
            }
        }
    }

    /// Refreshes and returns the selection of every enabled simulator. Never fails: a plugin that doesn't answer is `ok: false`.
    pub async fn simulator_states(&self, enabled: &[String]) -> Vec<SimulatorState> {
        let mut out = Vec::new();
        for p in self.simulators(enabled) {
            out.push(self.refresh_selection(p).await);
        }
        out
    }

    /// Tells one simulator the user switched a torrent on or off, then refreshes what it reports.
    /// `torrent` is the `selection_changed.torrent` object; the plugin decides what to do with it.
    /// `options` are the per-torrent settings the user chose (see `Manifest::torrent_settings`); only declared fields
    /// with allowed values are passed on, and only to a plugin that declares any.
    pub async fn set_simulation(&self, enabled_ids: &[String], id: &str, info_hash: &str, enabled: bool, torrent: &impl Serialize, options: Option<&Value>) -> Result<SimulatorState, String> {
        let p = self.simulators(enabled_ids).find(|p| p.manifest.id == id).ok_or("That plugin isn't turned on.")?;
        let mut params = json!({"info_hash": info_hash, "enabled": enabled, "torrent": torrent});
        if let Some(o) = options.filter(|_| enabled && !p.manifest.torrent_settings.is_empty()) {
            params["options"] = Value::Object(sanitize_options(&p.manifest.torrent_settings, o));
        }
        p.call_soft("selection_changed", params, SELECTION_CHANGED_TIMEOUT).await.map_err(|_| format!("{} didn't respond.", p.manifest.name))?;
        Ok(self.refresh_selection(p).await)
    }

    /// The values the dialog for one torrent starts from: the torrent's own if it is simulated, else the plugin's defaults.
    pub async fn simulator_options(&self, enabled_ids: &[String], id: &str, info_hash: &str) -> Result<Map<String, Value>, String> {
        let p = self.simulators(enabled_ids).find(|p| p.manifest.id == id).filter(|p| !p.manifest.torrent_settings.is_empty()).ok_or("That plugin isn't turned on, or has no settings per torrent.")?;
        let v = p.call_view("get_options", json!({"info_hash": info_hash}), SELECTION_TIMEOUT).await.map_err(|_| format!("{} didn't respond.", p.manifest.name))?;
        Ok(sanitize_options(&p.manifest.torrent_settings, &v["options"]))
    }

    /// The plugin's live numbers for one torrent. Nothing is kept here: it is asked each time the statistics are open.
    pub async fn simulator_status(&self, enabled_ids: &[String], id: &str, info_hash: &str) -> Result<SimStatus, String> {
        let p = self.simulators(enabled_ids).find(|p| p.manifest.id == id).filter(|p| p.manifest.torrent_status).ok_or("That plugin isn't turned on, or has no statistics.")?;
        let v = p.call_view("get_status", json!({"info_hash": info_hash}), SELECTION_TIMEOUT).await.map_err(|_| format!("{} didn't respond.", p.manifest.name))?;
        sanitize_status(&v).ok_or_else(|| format!("{} sent no statistics.", p.manifest.name))
    }

    /// Ends the processes of simulators that are no longer enabled (or all of them with an empty list, at app exit).
    pub async fn stop_unwanted_simulators(&self, enabled: &[String]) {
        for p in self.entries.iter().filter_map(|e| e.plugin.as_ref()).filter(|p| p.is_simulator() && !enabled.contains(&p.manifest.id)) {
            p.stop().await;
            self.sel.lock().unwrap().remove(&p.manifest.id);
        }
    }

    /// Search sources for the enabled plugins that offer search.
    pub fn search_providers(&self, enabled: &[String]) -> Vec<Arc<dyn SearchProvider>> {
        self.entries
            .iter()
            .filter_map(|e| e.plugin.as_ref())
            .filter(|p| enabled.contains(&p.manifest.id) && p.manifest.kinds.iter().any(|k| k == "search"))
            .map(|p| Arc::new(ExternalSearchProvider { plugin: p.clone() }) as Arc<dyn SearchProvider>)
            .collect()
    }
}

/// What a post-processing plugin may ask for. It never touches files itself through the app: `Move` goes through the
/// engine (so seeding carries on from the new place), and renaming files inside a torrent isn't offered because
/// that would break the data the torrent is seeding.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Move the torrent's files into this folder (an absolute path).
    Move(PathBuf),
    /// Show the user a short message.
    Notify(String),
}

const MAX_ACTIONS: usize = 20;

/// Keeps the actions Drift understands, clamped: an unknown type, a relative or oversized path or an empty message is dropped.
fn sanitize_actions(v: &Value) -> Vec<Action> {
    v.get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| match a["type"].as_str()? {
            "move" => {
                let dest = a["dest"].as_str()?.trim();
                (dest.len() <= 1024 && Path::new(dest).is_absolute()).then(|| Action::Move(PathBuf::from(dest)))
            }
            "notify" => {
                let m: String = a["message"].as_str()?.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
                (!m.is_empty()).then_some(Action::Notify(m))
            }
            _ => None,
        })
        .take(MAX_ACTIONS)
        .collect()
}

/// Wraps a plugin as a search source, so the aggregator can't tell it from a built-in one.
pub struct ExternalSearchProvider {
    plugin: Arc<Plugin>,
}

#[derive(Deserialize)]
struct RawResult {
    title: String,
    #[serde(default)]
    info_hash: Option<String>,
    #[serde(default)]
    magnet: Option<String>,
    #[serde(default)]
    torrent_url: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    seeders: Option<u32>,
    #[serde(default)]
    leechers: Option<u32>,
    #[serde(default)]
    published: Option<i64>,
    #[serde(default)]
    category: Option<String>,
    /// A label for this row's badge when one plugin searches several places (for example several indexers).
    #[serde(default)]
    source: Option<String>,
}

/// Plugin output is untrusted: keep only usable rows and clamp what the UI will show.
fn sanitize(raw: Vec<RawResult>, source: &str, limit: usize) -> Vec<SearchResult> {
    raw.into_iter()
        .filter_map(|r| {
            let magnet = r.magnet.filter(|m| m.starts_with("magnet:?") && m.len() <= 4096);
            let torrent_url = r.torrent_url.filter(|u| (u.starts_with("http://") || u.starts_with("https://")) && u.len() <= 2048);
            let info_hash = r.info_hash.as_deref().and_then(normalize_hash).or_else(|| magnet.as_deref().and_then(crate::search::magnet_hash));
            let title: String = r.title.trim().chars().take(300).collect();
            if title.is_empty() || (magnet.is_none() && torrent_url.is_none() && info_hash.is_none()) {
                return None;
            }
            Some(SearchResult {
                title,
                sources: vec![r.source.map(|s| s.trim().chars().take(60).collect::<String>()).filter(|s| !s.is_empty()).unwrap_or_else(|| source.to_string())],
                info_hash,
                magnet,
                torrent_url,
                size: r.size,
                seeders: r.seeders,
                leechers: r.leechers,
                published: r.published,
                category: r.category.map(|c| c.chars().take(60).collect()),
            })
        })
        // Not cut to the search limit here: a plugin that searches several places sends each place's best rows,
        // and the aggregator ranks everything together before trimming to the limit.
        .take(limit.max(MAX_PLUGIN_ROWS))
        .collect()
}

#[async_trait]
impl SearchProvider for ExternalSearchProvider {
    fn id(&self) -> &str {
        &self.plugin.manifest.id
    }

    fn name(&self) -> &str {
        &self.plugin.manifest.name
    }

    async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchResult>, ProviderError> {
        let v = self.plugin.call("search", json!({"query": q.text, "limit": q.limit})).await.map_err(|e| match e {
            CallError::Rpc(Some(Reason::Network)) => ProviderError::Network,
            CallError::Rpc(Some(Reason::Auth)) => ProviderError::Auth,
            CallError::Rpc(Some(Reason::Config)) => ProviderError::Config("Check this plugin's settings.".into()),
            CallError::Rpc(_) => ProviderError::BadResponse,
            _ => ProviderError::Plugin,
        })?;
        let raw: Vec<RawResult> = serde_json::from_value(v.get("results").cloned().unwrap_or(Value::Null)).map_err(|_| ProviderError::BadResponse)?;
        Ok(sanitize(raw, self.name(), q.limit))
    }
}

#[cfg(windows)]
mod job {
    //! A Job Object caps the plugin's memory and ends it if the app dies.
    use tokio::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::*;

    pub struct Job(HANDLE);
    // The handle is only closed on drop.
    unsafe impl Send for Job {}

    impl Job {
        pub fn contain(child: &Child, memory: usize) -> Option<Job> {
            let process = child.raw_handle()?;
            // SAFETY: plain Win32 calls on handles we own; the limits struct is zeroed then filled in.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                info.ProcessMemoryLimit = memory;
                let ok = SetInformationJobObject(h, JobObjectExtendedLimitInformation, std::ptr::addr_of!(info).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) != 0
                    && AssignProcessToJobObject(h, process as HANDLE) != 0;
                if !ok {
                    CloseHandle(h);
                    return None;
                }
                Some(Job(h))
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_must_stay_inside_the_plugin_folder() {
        assert!(inside("p.exe") && inside("bin/p.exe") && inside("./p.exe"));
        assert!(!inside("../p.exe") && !inside("") && !inside("C:\\Windows\\x.exe") && !inside("/bin/sh"));
    }

    #[test]
    fn plugin_rows_are_cleaned() {
        let good = "0123456789abcdef0123456789abcdef01234567";
        let raw = |t: &str, h: Option<&str>, m: Option<&str>, u: Option<&str>| RawResult { title: t.into(), info_hash: h.map(Into::into), magnet: m.map(Into::into), torrent_url: u.map(Into::into), size: None, seeders: None, leechers: None, published: None, category: None, source: None };
        let rows = vec![
            raw("ok", Some(&good.to_uppercase()), None, None),
            raw("", Some(good), None, None),
            raw("nothing to open", None, None, None),
            raw("bad link", None, Some("javascript:alert(1)"), Some("file:///c:/x")),
            raw("by url", None, None, Some("https://example.org/a.torrent")),
        ];
        let out = sanitize(rows, "P", 10);
        assert_eq!(out.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["ok", "by url"]);
        assert_eq!(out[0].info_hash.as_deref(), Some(good));
        assert_eq!(out[0].sources, ["P"]);
    }
}
