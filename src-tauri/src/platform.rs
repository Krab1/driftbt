//! OS-facing helpers kept out of `core`: reading opened files, revealing paths in Explorer.

use std::path::Path;

use drift_core::open_args::{self, OpenArg};
use serde::Serialize;

/// A .torrent file larger than this is almost certainly not a torrent; skip it rather than read it all.
const MAX_TORRENT_FILE: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Clone, Default)]
pub struct DraftFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What the Add dialog should be prefilled with after the OS opened magnets or files with the app.
#[derive(Serialize, Clone, Default)]
pub struct OpenPayload {
    pub magnets: Vec<String>,
    pub files: Vec<DraftFile>,
}

impl OpenPayload {
    pub fn is_empty(&self) -> bool {
        self.magnets.is_empty() && self.files.is_empty()
    }

    pub fn merge(&mut self, other: OpenPayload) {
        self.magnets.extend(other.magnets);
        self.files.extend(other.files);
    }
}

pub fn open_payload(args: &[String]) -> OpenPayload {
    let mut out = OpenPayload::default();
    for a in open_args::parse(args) {
        match a {
            OpenArg::Magnet(m) => out.magnets.push(m),
            OpenArg::TorrentFile(p) => match read_small(&p) {
                Some(bytes) => out.files.push(DraftFile {
                    name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    bytes,
                }),
                None => eprintln!("skipping unreadable or oversized torrent file: {}", p.display()),
            },
        }
    }
    out
}

fn read_small(p: &Path) -> Option<Vec<u8>> {
    if std::fs::metadata(p).ok()?.len() > MAX_TORRENT_FILE {
        return None;
    }
    std::fs::read(p).ok()
}

/// Start-with-Windows entry. Always per-user (HKCU): the plugin we replaced wrote to the machine-wide
/// key whenever the app ran elevated.
pub mod autostart {
    use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};

    pub const FLAG: &str = "--autostart";

    fn entry() -> Result<AutoLaunch, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        AutoLaunchBuilder::new()
            .set_app_name("Drift")
            .set_app_path(&exe.display().to_string())
            .set_args(&[FLAG])
            .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
            .build()
            .map_err(|e| e.to_string())
    }

    pub fn is_enabled() -> bool {
        entry().and_then(|e| e.is_enabled().map_err(|e| e.to_string())).unwrap_or(false)
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let e = entry()?;
        (if enabled { e.enable() } else { e.disable() }).map_err(|e| e.to_string())
    }
}

/// Network adapters as Windows sees them. Used by the VPN guard.
pub fn adapters() -> Vec<drift_core::guard::Adapter> {
    #[cfg(windows)]
    {
        windows_adapters()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn windows_adapters() -> Vec<drift_core::guard::Adapter> {
    use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC};

    fn text(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: Windows hands back NUL-terminated wide strings that live inside the buffer we own.
        let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
    }

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u8> = Vec::new();
    for _ in 0..4 {
        // u64 elements keep the buffer 8-byte aligned, as the structures require.
        buf = vec![0u8; size as usize];
        // SAFETY: `buf` is at least `size` bytes and outlives the call; Windows writes a linked list into it.
        let rc = unsafe { GetAdaptersAddresses(u32::from(AF_UNSPEC), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        if rc == 0 {
            break;
        }
        if rc != ERROR_BUFFER_OVERFLOW {
            return Vec::new();
        }
    }

    let mut out = Vec::new();
    let mut cur = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    while !cur.is_null() {
        // SAFETY: `cur` walks the list Windows wrote into `buf`; each node is valid until the buffer is dropped.
        let a = unsafe { &*cur };
        let mut has_ipv4 = false;
        let mut u = a.FirstUnicastAddress;
        while !u.is_null() {
            let ua = unsafe { &*u };
            let sa = ua.Address.lpSockaddr;
            if !sa.is_null() && unsafe { (*sa).sa_family } == AF_INET {
                has_ipv4 = true;
            }
            u = ua.Next;
        }
        out.push(drift_core::guard::Adapter {
            name: text(a.FriendlyName),
            description: text(a.Description),
            up: a.OperStatus == IfOperStatusUp,
            has_ipv4,
        });
        cur = a.Next;
    }
    out
}

/// Opens a file with whatever the user has set as its default app.
pub fn open_default(path: &Path) {
    // Tests set this to read the generated file instead of launching the user's media player.
    #[cfg(debug_assertions)]
    if std::env::var_os("DRIFT_NO_OPEN").is_some() {
        return;
    }
    #[cfg(windows)]
    {
        // rundll32 hands the path to ShellExecute without going through cmd.exe quoting rules.
        let _ = std::process::Command::new("rundll32.exe").arg("url.dll,FileProtocolHandler").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// Selects `path` in Explorer; falls back to opening its folder when the file doesn't exist yet.
pub fn reveal(path: &Path, fallback_dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer.exe");
        if path.exists() {
            cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        } else {
            cmd.raw_arg(format!("\"{}\"", fallback_dir.display()));
        }
        let _ = cmd.spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = (path, fallback_dir);
    }
}
