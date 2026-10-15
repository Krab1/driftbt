//! The plugin registry: a signed list of plugins that can be installed and updated without an app release
//! (`docs/PLUGINS.md`). The index is one JSON file with a minisign signature; each version in it carries the
//! SHA-256 of its zip, so one signature covers every download. Nothing here runs unless the user opted in.
//!
//! Installing never touches a plugin that may be running: the verified, unpacked version waits in
//! `<plugins>/.staging/<id>` and replaces the live folder at the next start (`apply_pending`), when nothing is
//! running. The replaced version is kept in `<plugins>/.previous/<id>`, and rolling back is just staging that copy.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{check, safe_id, Manifest, Permissions, API};

pub const INDEX_URL: &str = "https://raw.githubusercontent.com/Krab1/driftbt/main/registry/index.json";
/// The same minisign key that signs Drift's own updates (base64 of the .pub file, as in tauri.conf.json).
pub const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY1MEE3NzAwODk0QjI0RTAKUldUZ0pFdUpBSGNLOVlZYzdNak8wcjBpRk9XRjB4cXh0dU1lVllodG5pc2ZGQ1VoV1V0Q0dFV0kK";

const MAX_INDEX: usize = 1024 * 1024;
const MAX_ARCHIVE: usize = 32 * 1024 * 1024;
const MAX_UNPACKED: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("Couldn't reach the plugin list. Check your connection and try again.")]
    Network,
    #[error("The plugin list isn't signed correctly, so Drift ignored it.")]
    BadSignature,
    #[error("The plugin list couldn't be read.")]
    BadIndex,
    #[error("That plugin isn't in the list, or none of its versions work with this Drift.")]
    NotFound,
    #[error("The download didn't match its checksum, so it was thrown away.")]
    HashMismatch,
    #[error("The plugin package isn't valid. {0}")]
    BadPackage(String),
    #[error("There's no earlier version to go back to.")]
    NoPrevious,
    #[error("Couldn't write to the plugins folder.")]
    Disk,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub min_app: String,
    pub api: u32,
    #[serde(default)]
    pub changelog: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub version: u32,
    pub plugins: Vec<Entry>,
}

/// What the UI shows for one plugin the registry offers (the newest version that works with this Drift).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub changelog: String,
    /// The version installed now, if any.
    pub installed: Option<String>,
}

/// Dotted numbers compared piece by piece; anything that isn't a number counts as 0.
fn parts(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect()
}

pub fn is_newer(a: &str, b: &str) -> bool {
    newer(a, b)
}

fn newer(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (parts(a), parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x > y
}

fn works_here(v: &Version) -> bool {
    v.api == API && (v.min_app.is_empty() || !newer(&v.min_app, env!("CARGO_PKG_VERSION")))
}

/// The newest version of `id` that this Drift can run.
pub fn pick<'a>(index: &'a Index, id: &str) -> Option<(&'a Entry, &'a Version)> {
    let e = index.plugins.iter().find(|e| e.id == id)?;
    let v = e.versions.iter().filter(|v| works_here(v)).reduce(|best, v| if newer(&v.version, &best.version) { v } else { best })?;
    Some((e, v))
}

pub fn offers(index: &Index, installed: &HashMap<String, String>) -> Vec<Offer> {
    index
        .plugins
        .iter()
        .filter(|e| safe_id(&e.id))
        .filter_map(|e| {
            let (_, v) = pick(index, &e.id)?;
            Some(Offer { id: e.id.clone(), name: e.name.clone(), summary: e.summary.clone(), version: v.version.clone(), changelog: v.changelog.clone(), installed: installed.get(&e.id).cloned() })
        })
        .collect()
}

fn b64(s: &str) -> Result<String, RegistryError> {
    let raw = base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|_| RegistryError::BadSignature)?;
    String::from_utf8(raw).map_err(|_| RegistryError::BadSignature)
}

/// Checks the signature first and only then reads the index, so unsigned data is never interpreted.
pub fn verify_index(pubkey_b64: &str, index: &[u8], sig_b64: &str) -> Result<Index, RegistryError> {
    let key = minisign_verify::PublicKey::decode(&b64(pubkey_b64)?).map_err(|_| RegistryError::BadSignature)?;
    let sig = minisign_verify::Signature::decode(&b64(sig_b64)?).map_err(|_| RegistryError::BadSignature)?;
    key.verify(index, &sig, true).map_err(|_| RegistryError::BadSignature)?;
    let idx: Index = serde_json::from_slice(crate::jsonfile::strip_bom(index)).map_err(|_| RegistryError::BadIndex)?;
    if idx.version != 1 {
        return Err(RegistryError::BadIndex);
    }
    Ok(idx)
}

/// https only, including every redirect; a hard size cap.
async fn get(url: &str, cap: usize) -> Result<Vec<u8>, RegistryError> {
    if !url.starts_with("https://") {
        return Err(RegistryError::Network);
    }
    let policy = reqwest::redirect::Policy::custom(|a| if a.url().scheme() == "https" && a.previous().len() < 3 { a.follow() } else { a.stop() });
    let client = reqwest::Client::builder().redirect(policy).timeout(Duration::from_secs(60)).user_agent(concat!("Drift/", env!("CARGO_PKG_VERSION"))).build().map_err(|_| RegistryError::Network)?;
    let mut resp = client.get(url).send().await.map_err(|_| RegistryError::Network)?;
    if !resp.status().is_success() || resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(RegistryError::Network);
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| RegistryError::Network)? {
        body.extend_from_slice(&chunk);
        if body.len() > cap {
            return Err(RegistryError::Network);
        }
    }
    Ok(body)
}

pub async fn fetch_index(url: &str, pubkey_b64: &str) -> Result<Index, RegistryError> {
    let index = get(url, MAX_INDEX).await?;
    let sig = get(&format!("{url}.sig"), 16 * 1024).await?;
    verify_index(pubkey_b64, &index, &String::from_utf8_lossy(&sig))
}

pub async fn download(v: &Version) -> Result<Vec<u8>, RegistryError> {
    get(&v.url, MAX_ARCHIVE).await
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A file or folder name that means the same thing on every filesystem: no drive letters or streams (`:`),
/// no characters Windows forbids, no trailing dot or space, and not a reserved device name.
fn plain_name(n: &str) -> bool {
    const RESERVED: [&str; 22] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    let stem = n.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    !n.is_empty() && !n.ends_with(['.', ' ']) && !n.contains(['<', '>', ':', '"', '|', '?', '*', '\\', '/']) && !n.chars().any(|c| c.is_control()) && !RESERVED.contains(&stem.as_str())
}

fn unpack(archive: &[u8], to: &Path) -> Result<(), RegistryError> {
    let bad = |m: &str| RegistryError::BadPackage(m.to_string());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|_| bad("It isn't a zip file."))?;
    if zip.len() > MAX_FILES {
        return Err(bad("It has too many files."));
    }
    let mut left = MAX_UNPACKED;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|_| bad("It can't be read."))?;
        // `enclosed_name` is None for absolute paths and `..`, but it lets a later `C:x` component through, and
        // joining that replaces the whole path. So every component must be a plain, Windows-safe name.
        let rel = f.enclosed_name().ok_or_else(|| bad("It contains an unsafe path."))?;
        if !rel.components().all(|c| matches!(c, std::path::Component::Normal(n) if plain_name(&n.to_string_lossy()))) {
            return Err(bad("It contains an unsafe path."));
        }
        let dest = to.join(rel);
        if !dest.starts_with(to) {
            return Err(bad("It contains an unsafe path."));
        }
        if f.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|_| RegistryError::Disk)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|_| RegistryError::Disk)?;
        }
        let mut out = std::fs::File::create(&dest).map_err(|_| RegistryError::Disk)?;
        let n = std::io::copy(&mut (&mut f).take(left + 1), &mut out).map_err(|_| bad("It can't be unpacked."))?;
        if n > left {
            return Err(bad("It is too large when unpacked."));
        }
        left -= n;
        out.flush().map_err(|_| RegistryError::Disk)?;
    }
    Ok(())
}

/// The unpacked folder must be the plugin the index promised, at the promised version, and loadable.
fn validate(dir: &Path, id: &str, version: &str) -> Result<(), RegistryError> {
    let bad = |m: &str| RegistryError::BadPackage(m.to_string());
    let bytes = std::fs::read(dir.join("plugin.json")).map_err(|_| bad("It has no plugin.json."))?;
    let m: Manifest = serde_json::from_slice(crate::jsonfile::strip_bom(&bytes)).map_err(|_| bad("Its plugin.json can't be read."))?;
    if m.id != id || m.version != version {
        return Err(bad("It isn't the plugin and version the list promised."));
    }
    check(&m, dir).map_err(RegistryError::BadPackage)
}

/// Verifies a downloaded zip against the signed index entry and parks it, unpacked, for the next start.
pub fn stage(plugins: &Path, id: &str, v: &Version, archive: &[u8]) -> Result<(), RegistryError> {
    if !safe_id(id) {
        return Err(RegistryError::NotFound);
    }
    if !hex(&Sha256::digest(archive)).eq_ignore_ascii_case(v.sha256.trim()) {
        return Err(RegistryError::HashMismatch);
    }
    // On Windows `EX` and `ex` are the same folder: never let a registry id replace a different plugin's folder.
    if std::fs::read_dir(plugins).is_ok_and(|rd| rd.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(id) && e.file_name().to_string_lossy() != id)) {
        return Err(RegistryError::BadPackage("Another plugin already uses that name.".into()));
    }
    let staging = plugins.join(".staging");
    // A folder of its own per install, so two installs at once can't mix their files.
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp_root = staging.join(".tmp").join(format!("{nonce}-{}", std::process::id()));
    let tmp = tmp_root.join(id);
    std::fs::create_dir_all(&tmp).map_err(|_| RegistryError::Disk)?;
    if let Err(e) = unpack(archive, &tmp).and_then(|_| validate(&tmp, id, &v.version)) {
        let _ = std::fs::remove_dir_all(&tmp_root);
        return Err(e);
    }
    let dest = staging.join(id);
    // If an older staged copy can't be removed completely, keep it as it was instead of leaving a half-deleted folder.
    let aside = staging.join(".tmp").join(format!("old-{nonce}"));
    let had = dest.exists();
    if had && std::fs::rename(&dest, &aside).is_err() {
        let _ = std::fs::remove_dir_all(&tmp_root);
        return Err(RegistryError::Disk);
    }
    let r = std::fs::rename(&tmp, &dest).map_err(|_| RegistryError::Disk);
    if r.is_err() && had {
        let _ = std::fs::rename(&aside, &dest);
    }
    let _ = std::fs::remove_dir_all(&tmp_root);
    let _ = std::fs::remove_dir_all(&aside);
    r
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Stages the version that was replaced last time, to be swapped back in at the next start.
pub fn stage_previous(plugins: &Path, id: &str) -> Result<(), RegistryError> {
    if !safe_id(id) {
        return Err(RegistryError::NotFound);
    }
    let prev = plugins.join(".previous").join(id);
    if !prev.is_dir() {
        return Err(RegistryError::NoPrevious);
    }
    let dest = plugins.join(".staging").join(id);
    let _ = std::fs::remove_dir_all(&dest);
    copy_dir(&prev, &dest).map_err(|_| {
        let _ = std::fs::remove_dir_all(&dest);
        RegistryError::Disk
    })
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    serde_json::from_slice(crate::jsonfile::strip_bom(&std::fs::read(dir.join("plugin.json")).ok()?)).ok()
}

/// Whether `new` may reach something `old` couldn't.
fn wider(old: &Permissions, new: &Permissions) -> bool {
    (!old.net.iter().any(|h| h == "*") && new.net.iter().any(|h| !old.net.contains(h))) || new.fs.iter().any(|p| !old.fs.contains(p))
}

/// Swaps every staged plugin in. Call before the plugins are loaded: nothing is running then, so no file is
/// locked. A staged folder that doesn't check out is dropped, never swapped in. If the swap fails half way the
/// live version is put back and the staged copy waits for the next start. Returns the plugins that are new or ask
/// for more access than before: the caller must switch them off until the user approves them again.
pub fn apply_pending(plugins: &Path) -> Vec<String> {
    let mut needs_approval = Vec::new();
    let staging = plugins.join(".staging");
    let Ok(rd) = std::fs::read_dir(&staging) else { return needs_approval };
    let previous = plugins.join(".previous");
    for e in rd.filter_map(|e| e.ok()) {
        let id = e.file_name().to_string_lossy().into_owned();
        if !safe_id(&id) || !e.path().is_dir() {
            continue;
        }
        let (live, prev) = (plugins.join(&id), previous.join(&id));
        match read_manifest(&e.path()) {
            Some(m) if m.id == id && check(&m, &e.path()).is_ok() => {
                let old = read_manifest(&live);
                if old.is_none_or(|o| wider(&o.permissions, &m.permissions)) {
                    needs_approval.push(id.clone());
                }
            }
            _ => {
                let _ = std::fs::remove_dir_all(e.path());
                continue;
            }
        }
        let had_live = live.exists();
        if had_live {
            if std::fs::create_dir_all(&previous).is_err() {
                continue;
            }
            let _ = std::fs::remove_dir_all(&prev);
            if std::fs::rename(&live, &prev).is_err() {
                continue;
            }
        }
        if std::fs::rename(e.path(), &live).is_err() {
            needs_approval.retain(|x| x != &id);
            if had_live {
                let _ = std::fs::rename(&prev, &live);
            }
        }
    }
    needs_approval
}

/// Installs the plugins that ship inside the app (the first-party search sources) into the plugins folder: a
/// plugin that isn't there is copied in, one that is older than the bundled copy is staged to take over at the
/// next swap, and one that is the same or newer (say, updated from the registry) is left alone. The bundled folders
/// are the app's own files, so unlike downloads they need no signature check, only to be loadable.
pub fn provision(bundled: &Path, plugins: &Path) {
    let Ok(rd) = std::fs::read_dir(bundled) else { return };
    for e in rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()) {
        let id = e.file_name().to_string_lossy().into_owned();
        let Some(m) = read_manifest(&e.path()) else { continue };
        if !safe_id(&id) || m.id != id || check(&m, &e.path()).is_err() {
            continue;
        }
        let live = plugins.join(&id);
        let tmp = plugins.join(".staging").join(".tmp").join(format!("bundled-{id}"));
        let _ = std::fs::remove_dir_all(&tmp);
        match read_manifest(&live) {
            None if !live.exists() => {
                if copy_dir(&e.path(), &tmp).is_err() || std::fs::rename(&tmp, &live).is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
            }
            // Staged only when it beats both the live copy and anything already waiting (a newer registry update stays).
            Some(l) if newer(&m.version, &l.version) && pending_version(plugins, &id).is_none_or(|p| newer(&m.version, &p)) => {
                let staged = plugins.join(".staging").join(&id);
                let _ = std::fs::remove_dir_all(&staged);
                if copy_dir(&e.path(), &tmp).is_err() || std::fs::rename(&tmp, &staged).is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
            }
            _ => {}
        }
    }
}

fn version_in(dir: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(crate::jsonfile::strip_bom(&std::fs::read(dir.join("plugin.json")).ok()?)).ok()?;
    v["version"].as_str().map(str::to_string)
}

/// The version waiting for the next start, if any.
pub fn pending_version(plugins: &Path, id: &str) -> Option<String> {
    version_in(&plugins.join(".staging").join(id))
}

/// The version that can be rolled back to, if any.
pub fn previous_version(plugins: &Path, id: &str) -> Option<String> {
    version_in(&plugins.join(".previous").join(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn manifest(id: &str, version: &str) -> String {
        format!(r#"{{"id":"{id}","name":"P","version":"{version}","api":1,"kinds":["search"],"exec":["p.exe"]}}"#)
    }

    fn pkg(id: &str, version: &str) -> Vec<u8> {
        zip_of(&[("plugin.json", manifest(id, version).as_bytes()), ("p.exe", b"MZ")])
    }

    fn ver(v: &str, zip: &[u8]) -> Version {
        Version { version: v.into(), url: "https://example.org/p.zip".into(), sha256: hex(&Sha256::digest(zip)), min_app: String::new(), api: 1, changelog: String::new() }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("drift-reg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // A throwaway minisign key made only for these tests, and a signature over INDEX made with it.
    const TEST_PUB: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEREOENDQTMwQkFEMTQ5N0EKUldSNlNkRzZNTXFNM2Q1WFhrQ0FlUlpPSDAvYU1HSTNxWi9wSjRuOXFYc1l4eDgrdjg1alpHSnAK";
    const INDEX: &[u8] = br#"{"version":1,"plugins":[]}"#;
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSNlNkRzZNTXFNM2VQWnBzdUNYUjFsMmp5d3ZMN0ZFcDc0SUJxQWk4Q1FTalFxb1VhZXkrb2dWTEtDV0ZOelJSOFhib2lDalZvRUxjU1E5VXdYQUJ4MytWLy96VE1WVlFjPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMDMzMzM3CWZpbGU6aWR4Lmpzb24KRnd2dFNBbmNITXpzRGQ3YTlhSEtQZUFQb1h3OGNMRFNPVERXMlZyVWV4U3hMWEtmenFxZ1llQTVnQmlEcW9FUXZldVJvazZ4VG1qL1ZiUmxXelRlQXc9PQo=";

    #[test]
    fn only_a_correctly_signed_index_is_read() {
        let idx = verify_index(TEST_PUB, INDEX, TEST_SIG).unwrap();
        assert!(idx.plugins.is_empty());
        assert_eq!(verify_index(TEST_PUB, br#"{"version":1,"plugins":[{"id":"evil","name":"x","versions":[]}]}"#, TEST_SIG).unwrap_err(), RegistryError::BadSignature, "a changed index is refused");
        assert_eq!(verify_index(PUBKEY, INDEX, TEST_SIG).unwrap_err(), RegistryError::BadSignature, "a signature from another key is refused");
        assert_eq!(verify_index(TEST_PUB, INDEX, "not base64!").unwrap_err(), RegistryError::BadSignature);
    }

    #[test]
    fn the_index_published_in_the_repo_verifies_with_the_baked_in_key() {
        // Editing registry/index.json without signing it again would break every install; this catches that.
        let index = include_bytes!("../../../../registry/index.json");
        let sig = include_str!("../../../../registry/index.json.sig");
        verify_index(PUBKEY, index, sig).expect("registry/index.json must be signed with the Drift key (see docs/PLUGINS.md)");
    }

    #[test]
    fn the_newest_version_that_works_here_is_offered() {
        let v = |version: &str, api: u32, min_app: &str| Version { version: version.into(), url: String::new(), sha256: String::new(), min_app: min_app.into(), api, changelog: String::new() };
        let idx = Index {
            version: 1,
            plugins: vec![
                Entry { id: "a".into(), name: "A".into(), summary: String::new(), versions: vec![v("1.2.0", 1, ""), v("1.10.0", 1, ""), v("2.0.0", 2, ""), v("3.0.0", 1, "99.0.0")] },
                Entry { id: "b".into(), name: "B".into(), summary: String::new(), versions: vec![v("1.0.0", 2, "")] },
            ],
        };
        let o = offers(&idx, &HashMap::from([("a".to_string(), "1.2.0".to_string())]));
        assert_eq!(o.len(), 1, "b has nothing this Drift can run");
        assert_eq!((o[0].version.as_str(), o[0].installed.as_deref()), ("1.10.0", Some("1.2.0")), "1.10 is newer than 1.9, and a wrong api or too-new min_app is skipped");
    }

    #[test]
    fn a_tampered_or_wrong_package_is_rejected_and_leaves_nothing_behind() {
        let root = scratch("reject");
        let good = pkg("ex", "1.0.0");
        let mut v = ver("1.0.0", &good);
        let mut tampered = good.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(stage(&root, "ex", &v, &tampered).unwrap_err(), RegistryError::HashMismatch);

        // Promised one plugin or version, shipped another.
        for (id, version) in [("other", "1.0.0"), ("ex", "9.9.9")] {
            let z = pkg(id, version);
            v = ver("1.0.0", &z);
            assert!(matches!(stage(&root, "ex", &v, &z), Err(RegistryError::BadPackage(_))), "{id} {version}");
        }
        // A path that climbs out of the folder, and an archive with no program.
        let z = zip_of(&[("../evil.txt", b"x"), ("plugin.json", manifest("ex", "1.0.0").as_bytes())]);
        assert!(matches!(stage(&root, "ex", &ver("1.0.0", &z), &z), Err(RegistryError::BadPackage(_))));
        let z = zip_of(&[("plugin.json", manifest("ex", "1.0.0").as_bytes())]);
        assert!(matches!(stage(&root, "ex", &ver("1.0.0", &z), &z), Err(RegistryError::BadPackage(_))), "its program is missing");
        assert!(!root.join("evil.txt").exists() && !root.join(".staging").join("evil.txt").exists());
        assert!(pending_version(&root, "ex").is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn install_update_and_rollback_swap_at_start_and_keep_the_old_version() {
        let root = scratch("swap");
        let z1 = pkg("ex", "1.0.0");
        stage(&root, "ex", &ver("1.0.0", &z1), &z1).unwrap();
        assert_eq!(pending_version(&root, "ex").as_deref(), Some("1.0.0"));
        assert!(!root.join("ex").exists(), "nothing changes until the next start");
        apply_pending(&root);
        assert_eq!(version_in(&root.join("ex")).as_deref(), Some("1.0.0"));
        assert!(pending_version(&root, "ex").is_none() && previous_version(&root, "ex").is_none());

        let z2 = pkg("ex", "1.1.0");
        stage(&root, "ex", &ver("1.1.0", &z2), &z2).unwrap();
        assert_eq!(version_in(&root.join("ex")).as_deref(), Some("1.0.0"), "the live version keeps working while the update waits");
        apply_pending(&root);
        assert_eq!((version_in(&root.join("ex")).as_deref(), previous_version(&root, "ex").as_deref()), (Some("1.1.0"), Some("1.0.0")));

        stage_previous(&root, "ex").unwrap();
        apply_pending(&root);
        assert_eq!((version_in(&root.join("ex")).as_deref(), previous_version(&root, "ex").as_deref()), (Some("1.0.0"), Some("1.1.0")), "rolling back keeps the newer one for rolling forward");
        assert_eq!(stage_previous(&root, "nothing").unwrap_err(), RegistryError::NoPrevious);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn names_that_escape_or_confuse_windows_are_refused() {
        for bad in ["foo/C:bar", "foo/D:x", "a.txt:stream", "CON", "nul.txt", "dir/trailing.", "dir/trailing ", "q?.exe", "a<b"] {
            let z = zip_of(&[("plugin.json", manifest("ex", "1.0.0").as_bytes()), ("p.exe", b"MZ"), (bad, b"x")]);
            let root = scratch("names");
            let r = stage(&root, "ex", &ver("1.0.0", &z), &z);
            assert!(matches!(r, Err(RegistryError::BadPackage(_))), "{bad} should be refused, got {r:?}");
            assert!(pending_version(&root, "ex").is_none());
            std::fs::remove_dir_all(root).ok();
        }
        assert!(plain_name("p.exe") && plain_name("lib") && plain_name("my plugin-1.2_x"));
    }

    #[test]
    fn a_broken_staged_folder_is_dropped_and_widened_access_needs_approval() {
        let root = scratch("approve");
        let put = |dir: &Path, version: &str, net: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("p.exe"), "MZ").unwrap();
            std::fs::write(dir.join("plugin.json"), format!(r#"{{"id":"ex","name":"P","version":"{version}","api":1,"kinds":["search"],"exec":["p.exe"],"permissions":{{"net":[{net}]}}}}"#)).unwrap();
        };
        put(&root.join("ex"), "1.0.0", r#""a.example""#);

        // Staged without a plugin.json (say a half-deleted folder): it must not replace the working plugin.
        std::fs::create_dir_all(root.join(".staging").join("ex")).unwrap();
        assert!(apply_pending(&root).is_empty());
        assert_eq!(version_in(&root.join("ex")).as_deref(), Some("1.0.0"));
        assert!(!root.join(".staging").join("ex").exists());

        // Same access: takes over quietly. More access: takes over but must be approved again.
        put(&root.join(".staging").join("ex"), "1.1.0", r#""a.example""#);
        assert!(apply_pending(&root).is_empty());
        put(&root.join(".staging").join("ex"), "1.2.0", r#""a.example","b.example""#);
        assert_eq!(apply_pending(&root), ["ex"]);
        assert_eq!(version_in(&root.join("ex")).as_deref(), Some("1.2.0"));
        // A plugin that wasn't there before is new, so it needs approval too.
        put(&root.join(".staging").join("fresh"), "1.0.0", "");
        std::fs::write(root.join(".staging").join("fresh").join("plugin.json"), manifest("fresh", "1.0.0")).unwrap();
        assert_eq!(apply_pending(&root), ["fresh"]);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn bundled_plugins_are_installed_once_and_upgraded_only_when_the_app_has_a_newer_one() {
        let (app, data) = (scratch("bundled-app"), scratch("bundled-data"));
        let put = |dir: &Path, version: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("p.exe"), version).unwrap();
            std::fs::write(dir.join("plugin.json"), manifest("ex", version)).unwrap();
        };
        put(&app.join("ex"), "1.0.0");
        std::fs::create_dir_all(app.join("not-a-plugin")).unwrap();
        std::fs::write(app.join("readme.txt"), "x").unwrap();

        provision(&app, &data);
        assert_eq!(version_in(&data.join("ex")).as_deref(), Some("1.0.0"), "a missing plugin is installed");
        assert!(!data.join("not-a-plugin").exists());

        // The user's copy is newer (from the registry): the app's older one never replaces it.
        put(&data.join("ex"), "1.5.0");
        provision(&app, &data);
        assert_eq!(version_in(&data.join("ex")).as_deref(), Some("1.5.0"));

        // A newer app brings a newer plugin: it is staged, then swapped in, keeping the old one for rollback.
        put(&app.join("ex"), "2.0.0");
        provision(&app, &data);
        assert_eq!(pending_version(&data, "ex").as_deref(), Some("2.0.0"));
        assert_eq!(version_in(&data.join("ex")).as_deref(), Some("1.5.0"), "nothing changes until the swap");
        assert!(apply_pending(&data).is_empty(), "same access as before, so no new approval");
        assert_eq!((version_in(&data.join("ex")).as_deref(), previous_version(&data, "ex").as_deref()), (Some("2.0.0"), Some("1.5.0")));
        provision(&app, &data);
        assert!(pending_version(&data, "ex").is_none(), "already current: nothing staged again");

        // A registry update waiting to take over is newer than what the app carries: it is not replaced.
        put(&data.join(".staging").join("ex"), "3.0.0");
        provision(&app, &data);
        assert_eq!(pending_version(&data, "ex").as_deref(), Some("3.0.0"));
        std::fs::remove_dir_all(app).ok();
        std::fs::remove_dir_all(data).ok();
    }

    #[test]
    fn a_swap_that_cannot_finish_puts_the_live_version_back() {
        let root = scratch("fail");
        std::fs::create_dir_all(root.join("ex")).unwrap();
        std::fs::write(root.join("ex").join("plugin.json"), manifest("ex", "1.0.0")).unwrap();
        // The staged "folder" is a file, so moving it into place fails after the live folder was set aside.
        std::fs::create_dir_all(root.join(".staging").join("ex")).unwrap();
        std::fs::write(root.join(".staging").join("ex").join("plugin.json"), manifest("ex", "2.0.0")).unwrap();
        std::fs::write(root.join(".staging").join("ex").join("p.exe"), "MZ").unwrap();
        // Make the destination impossible: a file where `.previous` must be a folder.
        std::fs::write(root.join(".previous"), "x").unwrap();
        apply_pending(&root);
        assert_eq!(version_in(&root.join("ex")).as_deref(), Some("1.0.0"), "the working version is untouched");
        assert_eq!(pending_version(&root, "ex").as_deref(), Some("2.0.0"), "the update waits for another try");
        std::fs::remove_dir_all(root).ok();
    }
}
