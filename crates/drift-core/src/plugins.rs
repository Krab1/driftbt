//! Plugins: separate executables found in `<data>/plugins/<id>/` and spoken to over newline-delimited
//! JSON-RPC on stdin/stdout (see `docs/PLUGIN_PROTOCOL.md`). A plugin can crash, hang or lie without taking
//! the app down: calls time out, a crashed plugin is restarted with a growing delay, and after three
//! failures in a row it is switched off until the app restarts. Plugins never touch the engine; they only
//! answer questions (today: search).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api: u32,
    pub kinds: Vec<String>,
    /// Program and arguments; the program is a path inside the plugin's folder.
    pub exec: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
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
        Self { call_timeout: Duration::from_secs(5), idle: Duration::from_secs(300), backoff: Duration::from_secs(2) }
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
    #[error("the plugin reported an error")]
    Rpc,
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
    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
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
            // Notifications (e.g. `log`) and replies to older calls carry no matching id.
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            return match v.get("result") {
                Some(r) if v.get("error").is_none() => Ok(r.clone()),
                _ => Err(CallError::Rpc),
            };
        }
    }
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    failures: u32,
    retry_at: Option<Instant>,
    disabled: bool,
}

pub struct Plugin {
    pub manifest: Manifest,
    dir: PathBuf,
    log: PathBuf,
    limits: Limits,
    state: tokio::sync::Mutex<State>,
}

impl Plugin {
    fn new(manifest: Manifest, dir: PathBuf, log: PathBuf, limits: Limits) -> Arc<Self> {
        Arc::new(Self { manifest, dir, log, limits, state: Default::default() })
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
        let init = json!({"api": API, "app_version": env!("CARGO_PKG_VERSION"), "settings": {}});
        tokio::time::timeout(self.limits.call_timeout, r.rpc("initialize", init)).await.map_err(|_| CallError::Failed)??;
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
        let mut st = self.state.lock().await;
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
                    tokio::spawn(idle_watch(Arc::downgrade(self)));
                }
                _ => {
                    self.failed(&mut st);
                    return Err(CallError::Failed);
                }
            }
        }
        let r = st.running.as_mut().expect("started above");
        match tokio::time::timeout(self.limits.call_timeout, r.rpc(method, params)).await {
            Ok(Ok(v)) => {
                r.last_used = Instant::now();
                st.failures = 0;
                Ok(v)
            }
            // The plugin answered, just not usefully: it is alive, so this isn't a crash.
            Ok(Err(CallError::Rpc)) => {
                r.last_used = Instant::now();
                Err(CallError::Rpc)
            }
            _ => {
                self.failed(&mut st);
                Err(CallError::Failed)
            }
        }
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
                let _ = tokio::time::timeout(Duration::from_millis(500), r.rpc("shutdown", json!({}))).await;
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
    pub permissions: Permissions,
    pub enabled: bool,
    /// Why it can't be used (wrong protocol version, missing program, switched off after crashes).
    pub problem: Option<String>,
}

struct Entry {
    plugin: Option<Arc<Plugin>>,
    info: PluginInfo,
}

/// The plugins found at start-up. Which of them are on is the user's choice and lives in settings.
pub struct Host {
    entries: Vec<Entry>,
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
    if m.exec.is_empty() || !inside(&m.exec[0]) || !dir.join(&m.exec[0]).is_file() {
        return Err("Its program is missing.".into());
    }
    Ok(())
}

impl Host {
    pub fn empty() -> Self {
        Self { entries: Vec::new() }
    }

    /// The plugins under `<data>/plugins`, with default limits and logs in `<data>/plugin-logs`.
    pub fn at(data_dir: &Path) -> Arc<Self> {
        Arc::new(Self::load(&data_dir.join("plugins"), &data_dir.join("plugin-logs"), Limits::default()))
    }

    /// Reads every `<dir>/<id>/plugin.json`. Broken plugins are listed with the reason instead of being dropped.
    pub fn load(dir: &Path, logs: &Path, limits: Limits) -> Self {
        let mut entries = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else { return Self { entries } };
        let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let Ok(bytes) = std::fs::read(d.join("plugin.json")) else { continue };
            let folder = d.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let m: Manifest = match serde_json::from_slice(crate::jsonfile::strip_bom(&bytes)) {
                Ok(m) => m,
                Err(_) => {
                    let info = PluginInfo { id: folder.clone(), name: folder, version: String::new(), kinds: vec![], permissions: Permissions::default(), enabled: false, problem: Some("Its plugin.json can't be read.".into()) };
                    entries.push(Entry { plugin: None, info });
                    continue;
                }
            };
            let problem = check(&m, &d).err();
            let info = PluginInfo { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), kinds: m.kinds.clone(), permissions: m.permissions.clone(), enabled: false, problem: problem.clone() };
            let plugin = problem.is_none().then(|| Plugin::new(m.clone(), d.clone(), logs.join(format!("{}.log", m.id)), limits));
            entries.push(Entry { plugin, info });
        }
        Self { entries }
    }

    pub fn list(&self, enabled: &[String]) -> Vec<PluginInfo> {
        self.entries
            .iter()
            .map(|e| {
                let mut i = e.info.clone();
                i.enabled = e.plugin.is_some() && enabled.contains(&i.id);
                if let Some(p) = &e.plugin {
                    // `try_lock`: a plugin in the middle of a call is by definition not switched off.
                    if p.state.try_lock().is_ok_and(|s| s.disabled) {
                        i.problem = Some("It stopped responding and was switched off. Restart Drift to try it again.".into());
                    }
                }
                i
            })
            .collect()
    }

    /// Whether the plugin's process is alive right now (it starts on first use and ends when idle).
    pub async fn is_running(&self, id: &str) -> bool {
        match self.entries.iter().find_map(|e| e.plugin.as_ref().filter(|p| p.manifest.id == id)) {
            Some(p) => p.state.lock().await.running.is_some(),
            None => false,
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
                sources: vec![source.to_string()],
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
        .take(limit)
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
            CallError::Rpc => ProviderError::BadResponse,
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
        let raw = |t: &str, h: Option<&str>, m: Option<&str>, u: Option<&str>| RawResult { title: t.into(), info_hash: h.map(Into::into), magnet: m.map(Into::into), torrent_url: u.map(Into::into), size: None, seeders: None, leechers: None, published: None, category: None };
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
