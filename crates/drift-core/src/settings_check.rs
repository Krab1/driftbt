//! What a settings save may contain. The UI checks its own boxes, but the web UI and the API take any JSON, so this is the
//! guard that counts: numbers are held to sensible ranges, text to short plain strings, lists to a few entries.

use std::path::Path;

use crate::settings::{Settings, MAX_LIMIT_KIB};

const MAX_ACTIVE: u32 = 1000;
/// Ten years, in minutes.
const MAX_MINUTES: u32 = 5_256_000;
const MAX_RATIO: f32 = 1000.0;
const MAX_CATEGORIES: usize = 100;
const MAX_LISTS: usize = 20;
const MAX_SLOTS: usize = 50;
const MAX_SOURCES: usize = 20;
const MAX_PLUGINS: usize = 100;
const MAX_TOKENS: usize = 100;
const PATH_CHARS: usize = 4096;
const URL_CHARS: usize = 2048;

/// Plain one-line text: no control characters (a newline or NUL in a name or path has no honest use) and a length limit.
fn plain(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(char::is_control)
}

fn name_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && plain(s, max)
}

fn id_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn path_ok(p: &Path) -> bool {
    p.to_str().is_some_and(|s| !s.trim().is_empty() && plain(s, PATH_CHARS))
}

fn http_url(s: &str) -> bool {
    crate::plugins::is_http_url(s) && s.len() <= URL_CHARS && !s.contains(char::is_whitespace) && plain(s, URL_CHARS)
}

impl Settings {
    /// The settings as they will be stored, or a plain sentence saying what is wrong. Out-of-range numbers are held to
    /// the range (the speed limit to 10 Gbit/s); anything that is not the right kind of value is refused.
    pub fn checked(mut self) -> Result<Settings, String> {
        self.down_limit_kib = self.down_limit_kib.min(MAX_LIMIT_KIB);
        self.up_limit_kib = self.up_limit_kib.min(MAX_LIMIT_KIB);
        self.max_active_downloads = self.max_active_downloads.min(MAX_ACTIVE);
        self.min_seed_minutes = self.min_seed_minutes.min(MAX_MINUTES);

        let g = &mut self.default_goal;
        match g.ratio {
            Some(r) if !r.is_finite() || r < 0.0 => return Err("The seeding ratio must be a number of zero or more.".into()),
            Some(r) => g.ratio = Some(r.min(MAX_RATIO)),
            None => {}
        }
        g.minutes = g.minutes.map(|m| m.min(MAX_MINUTES));

        if self.listen_port == Some(0) {
            return Err("The port must be between 1 and 65535.".into());
        }
        for (what, p) in [("The download folder", &self.download_dir), ("The watch folder", &self.watch_dir), ("The finished-downloads folder", &self.move_on_complete)] {
            if p.as_deref().is_some_and(|p| !path_ok(p)) {
                return Err(format!("{what} isn't a usable path."));
            }
        }
        // An empty proxy box means none; the address itself is checked when it is applied.
        self.proxy = self.proxy.take().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        if self.proxy.as_deref().is_some_and(|p| !plain(p, 512)) {
            return Err("The proxy address isn't valid. Use the form socks5://host:port.".into());
        }

        if self.categories.len() > MAX_CATEGORIES {
            return Err(format!("At most {MAX_CATEGORIES} categories."));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &mut self.categories {
            c.name = c.name.trim().to_string();
            if !name_ok(&c.name, 64) || !seen.insert(c.name.to_lowercase()) {
                return Err("Category names must be different, 1 to 64 characters, on one line.".into());
            }
            if c.save_path.as_deref().is_some_and(|p| !path_ok(p)) {
                return Err(format!("The folder for \"{}\" isn't a usable path.", c.name));
            }
        }

        let f = &self.ip_filter;
        let mut ids = std::collections::HashSet::new();
        if f.lists.len() > MAX_LISTS || !f.lists.iter().all(|l| id_ok(&l.id) && ids.insert(l.id.as_str()) && name_ok(&l.name, 100) && !l.source.trim().is_empty() && plain(&l.source, URL_CHARS)) {
            return Err(format!("The block lists aren't valid: at most {MAX_LISTS}, each with a name and an address or file."));
        }

        let s = &mut self.schedule;
        if s.slots.len() > MAX_SLOTS {
            return Err(format!("At most {MAX_SLOTS} schedule slots."));
        }
        for slot in &mut s.slots {
            if slot.from_min >= 1440 || slot.to_min >= 1440 {
                return Err("A schedule time must be within a day.".into());
            }
            slot.days &= crate::schedule::EVERY_DAY;
            slot.down_kib = slot.down_kib.min(MAX_LIMIT_KIB);
            slot.up_kib = slot.up_kib.min(MAX_LIMIT_KIB);
        }

        if self.vpn_guard.adapter.as_deref().is_some_and(|a| !plain(a, 200)) {
            return Err("The VPN adapter name isn't valid.".into());
        }

        let t = &self.search.torznab;
        let mut ids = std::collections::HashSet::new();
        if t.len() > MAX_SOURCES || !t.iter().all(|c| c.id_is_valid() && c.id.len() <= 64 && ids.insert(c.id.as_str()) && name_ok(&c.name, 100) && http_url(&c.url)) {
            return Err("A search source needs a plain id, a name and an http(s) address.".into());
        }

        if self.plugins.len() > MAX_PLUGINS || !self.plugins.iter().all(|p| id_ok(p)) {
            return Err("The plugin list isn't valid.".into());
        }
        self.plugins.dedup();

        if let Some(p) = &self.theme_pack {
            if !name_ok(&p.name, 100) || !matches!(p.base.as_str(), "light" | "dark") || p.tokens.len() > MAX_TOKENS || !p.tokens.iter().all(|(k, v)| !k.is_empty() && plain(k, 64) && plain(v, 100)) {
                return Err("That colour theme isn't valid.".into());
            }
        }
        Ok(self)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Category;

    fn bad(f: impl FnOnce(&mut Settings)) -> bool {
        let mut s = Settings::default();
        f(&mut s);
        s.checked().is_err()
    }

    #[test]
    fn defaults_pass_and_numbers_are_held_to_range() {
        assert!(Settings::default().checked().is_ok());
        let s = Settings { down_limit_kib: u32::MAX, up_limit_kib: MAX_LIMIT_KIB + 1, max_active_downloads: 99_999, min_seed_minutes: u32::MAX, ..Settings::default() }.checked().unwrap();
        assert_eq!((s.down_limit_kib, s.up_limit_kib, s.max_active_downloads, s.min_seed_minutes), (MAX_LIMIT_KIB, MAX_LIMIT_KIB, 1000, MAX_MINUTES));
        // 10 Gbit/s in bytes per second still fits the engine's 32 bits.
        assert!(u64::from(MAX_LIMIT_KIB) * 1024 <= u64::from(u32::MAX));
    }

    #[test]
    fn values_of_the_wrong_kind_are_refused() {
        assert!(bad(|s| s.listen_port = Some(0)));
        assert!(bad(|s| s.default_goal.ratio = Some(f32::NAN)));
        assert!(bad(|s| s.default_goal.ratio = Some(-1.0)));
        assert!(bad(|s| s.download_dir = Some("C:\\a\nb".into())));
        assert!(bad(|s| s.watch_dir = Some("".into())));
        assert!(bad(|s| s.categories = vec![Category { name: "  ".into(), save_path: None }]));
        assert!(bad(|s| s.categories = vec![Category { name: "A".into(), save_path: None }, Category { name: "a".into(), save_path: None }]));
        assert!(bad(|s| s.plugins = vec!["../x".into()]));
        assert!(bad(|s| s.schedule.slots = vec![crate::schedule::Slot { days: 1, from_min: 1440, to_min: 0, down_kib: 0, up_kib: 0 }]));
        assert!(bad(|s| s.proxy = Some("socks5://h:1\nx".into())));
        assert!(bad(|s| s.vpn_guard.adapter = Some("x".repeat(201))));
    }

    #[test]
    fn schedule_speeds_and_days_are_held_and_an_empty_proxy_means_none() {
        let slot = crate::schedule::Slot { days: 255, from_min: 0, to_min: 60, down_kib: u32::MAX, up_kib: 5 };
        let s = Settings { schedule: crate::schedule::Schedule { enabled: true, slots: vec![slot] }, proxy: Some("  ".into()), ..Settings::default() }.checked().unwrap();
        assert_eq!((s.schedule.slots[0].days, s.schedule.slots[0].down_kib, s.proxy), (127, MAX_LIMIT_KIB, None));
    }
}
