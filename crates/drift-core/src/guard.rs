//! VPN guard: pause everything when the chosen network adapter isn't up, resume when it returns.
//!
//! This is an application-level kill switch. The engine cannot bind to an adapter on Windows, so there is a
//! gap of up to one check interval between the adapter dropping and torrents being paused.

use serde::{Deserialize, Serialize};

use crate::model::TorrentId;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VpnGuard {
    pub enabled: bool,
    /// Matched case-insensitively against the adapter's name and description.
    pub adapter: Option<String>,
    /// Resume the torrents the guard paused once the adapter is back.
    pub auto_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub up: bool,
    pub has_ipv4: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Off,
    Protected,
    Unprotected,
}

/// Is the chosen adapter present, up and holding an IPv4 address? No adapter chosen counts as unprotected:
/// a guard that guards nothing must not look safe.
pub fn verdict(cfg: &VpnGuard, adapters: &[Adapter]) -> Verdict {
    if !cfg.enabled {
        return Verdict::Off;
    }
    let Some(want) = cfg.adapter.as_deref().map(str::trim).filter(|w| !w.is_empty()) else {
        return Verdict::Unprotected;
    };
    let want = want.to_lowercase();
    let ok = adapters.iter().any(|a| a.up && a.has_ipv4 && (a.name.to_lowercase().contains(&want) || a.description.to_lowercase().contains(&want)));
    if ok {
        Verdict::Protected
    } else {
        Verdict::Unprotected
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub pause: Vec<TorrentId>,
    pub resume: Vec<TorrentId>,
}

#[derive(Default)]
pub struct Guard {
    tripped: bool,
    /// Torrents the guard paused (so it only resumes those, never ones the user paused).
    paused_by_guard: Vec<TorrentId>,
}

impl Guard {
    pub fn tripped(&self) -> bool {
        self.tripped
    }

    pub fn paused_count(&self) -> usize {
        self.paused_by_guard.len()
    }

    /// One check. `running` are the torrents currently active in the engine.
    pub fn step(&mut self, v: Verdict, auto_resume: bool, running: &[TorrentId]) -> Action {
        match v {
            Verdict::Unprotected => {
                self.tripped = true;
                // Also catches anything started while tripped (a manual resume, a watch-folder add).
                for id in running {
                    if !self.paused_by_guard.contains(id) {
                        self.paused_by_guard.push(id.clone());
                    }
                }
                Action { pause: running.to_vec(), resume: Vec::new() }
            }
            Verdict::Protected | Verdict::Off => {
                if !self.tripped {
                    return Action::default();
                }
                self.tripped = false;
                let ids = std::mem::take(&mut self.paused_by_guard);
                Action { pause: Vec::new(), resume: if auto_resume { ids } else { Vec::new() } }
            }
        }
    }
}

/// Network adapters as Windows sees them. Used by the VPN guard.
pub fn adapters() -> Vec<Adapter> {
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
fn windows_adapters() -> Vec<Adapter> {
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
        out.push(Adapter {
            name: text(a.FriendlyName),
            description: text(a.Description),
            up: a.OperStatus == IfOperStatusUp,
            has_ipv4,
        });
        cur = a.Next;
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> TorrentId {
        TorrentId(s.into())
    }

    fn adapter(name: &str, desc: &str, up: bool, v4: bool) -> Adapter {
        Adapter { name: name.into(), description: desc.into(), up, has_ipv4: v4 }
    }

    fn cfg(name: &str) -> VpnGuard {
        VpnGuard { enabled: true, adapter: Some(name.into()), auto_resume: true }
    }

    #[test]
    fn verdicts() {
        let nics = [adapter("Ethernet", "Realtek PCIe", true, true), adapter("ProtonVPN", "WireGuard Tunnel", true, true)];
        assert_eq!(verdict(&VpnGuard::default(), &nics), Verdict::Off);
        assert_eq!(verdict(&cfg("proton"), &nics), Verdict::Protected, "case-insensitive name match");
        assert_eq!(verdict(&cfg("wireguard"), &nics), Verdict::Protected, "description match");
        assert_eq!(verdict(&cfg("OpenVPN"), &nics), Verdict::Unprotected, "adapter missing");
        assert_eq!(verdict(&VpnGuard { enabled: true, adapter: None, auto_resume: true }, &nics), Verdict::Unprotected, "nothing chosen is not safe");
        assert_eq!(verdict(&VpnGuard { enabled: true, adapter: Some("  ".into()), auto_resume: true }, &nics), Verdict::Unprotected);
    }

    #[test]
    fn an_adapter_that_is_down_or_has_no_address_does_not_count() {
        assert_eq!(verdict(&cfg("vpn"), &[adapter("MyVPN", "", false, true)]), Verdict::Unprotected);
        assert_eq!(verdict(&cfg("vpn"), &[adapter("MyVPN", "", true, false)]), Verdict::Unprotected);
    }

    #[test]
    fn pauses_what_runs_then_resumes_only_those() {
        let mut g = Guard::default();
        assert_eq!(g.step(Verdict::Protected, true, &[id("a"), id("b")]), Action::default(), "protected: hands off");

        let a = g.step(Verdict::Unprotected, true, &[id("a"), id("b")]);
        assert_eq!(a.pause, vec![id("a"), id("b")]);
        assert!(g.tripped() && g.paused_count() == 2);

        // a torrent started while tripped is paused too, and remembered
        let a = g.step(Verdict::Unprotected, true, &[id("c")]);
        assert_eq!(a.pause, vec![id("c")]);
        assert_eq!(g.paused_count(), 3);

        let a = g.step(Verdict::Protected, true, &[]);
        assert_eq!(a.resume, vec![id("a"), id("b"), id("c")]);
        assert!(!g.tripped() && g.paused_count() == 0);
        assert_eq!(g.step(Verdict::Protected, true, &[]), Action::default(), "and then nothing more");
    }

    #[test]
    fn without_auto_resume_the_user_decides() {
        let mut g = Guard::default();
        g.step(Verdict::Unprotected, false, &[id("a")]);
        let a = g.step(Verdict::Protected, false, &[]);
        assert!(a.resume.is_empty());
        assert!(!g.tripped());
    }

    #[test]
    fn turning_the_guard_off_while_tripped_releases_it() {
        let mut g = Guard::default();
        g.step(Verdict::Unprotected, true, &[id("a")]);
        let a = g.step(Verdict::Off, true, &[]);
        assert_eq!(a.resume, vec![id("a")]);
        assert!(!g.tripped());
    }
}
