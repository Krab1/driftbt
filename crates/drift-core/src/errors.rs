//! Turns raw engine/OS error text into one plain sentence the UI can show (UI_SPEC copy rules:
//! say what happened and what to do; never surface raw error strings).

/// `None` input stays `None`. Unknown errors get a generic sentence; the raw text is the caller's to log.
pub fn plain_error(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let l = raw.to_lowercase();
    // Match whole codes: "(os error 3)" must not match "(os error 32)".
    let code = |n: u32| l.contains(&format!("(os error {n})"));
    let msg = if code(112) || l.contains("no space left") || l.contains("not enough space") || l.contains("disk full") {
        "The download drive is full. Free up space, then resume."
    } else if code(5) || l.contains("access is denied") || l.contains("permission denied") {
        "Drift can't write to the download folder. Choose another folder or check its permissions."
    } else if code(32) || l.contains("being used by another process") {
        "Another program is using the file. Close it, then resume."
    } else if code(3) || code(2) || l.contains("cannot find the path") || l.contains("no such file") {
        "The download folder can't be found. Reconnect the drive or choose another folder."
    } else if code(206) || l.contains("path too long") || l.contains("filename too long") {
        "A file path in this torrent is too long for Windows. Choose a shorter download folder."
    } else {
        "Something went wrong with this torrent. Try resuming it."
    };
    Some(msg.to_string())
}

/// What every front end (desktop IPC, HTTP) shows when a call fails: already plain language.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &'static str, message: &str) -> Self {
        Self { code, message: message.to_string() }
    }
}

impl From<crate::engine::EngineError> for ApiError {
    fn from(e: crate::engine::EngineError) -> Self {
        use crate::engine::EngineError as E;
        match e {
            E::NotFound => Self::new("not_found", "That torrent no longer exists."),
            E::InvalidInput(_) => Self::new("invalid_input", "That isn't a valid magnet link or torrent file."),
            E::Blocked => Self::new("blocked", "The VPN guard is holding torrents because your VPN isn't connected."),
            E::Timeout => Self::new("timeout", "Couldn't fetch the torrent info. Check the link, or try again in a moment."),
            E::Backend(raw) => {
                eprintln!("engine error: {raw}");
                Self::new("engine", "The torrent engine hit a problem. Try again.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::plain_error;

    #[test]
    fn maps_known_windows_and_unix_errors() {
        let disk = Some("write failed: There is not enough space on the disk. (os error 112)");
        assert!(plain_error(disk).unwrap().contains("drive is full"));
        assert!(plain_error(Some("Access is denied. (os error 5)")).unwrap().contains("can't write"));
        assert!(plain_error(Some("No such file or directory (os error 2)")).unwrap().contains("can't be found"));
        assert!(plain_error(Some("The process cannot access the file because it is being used by another process. (os error 32)")).unwrap().contains("Another program"));
    }

    #[test]
    fn similar_error_codes_do_not_collide() {
        // 3 vs 32, 5 vs 53: each must land on its own message.
        let locked = plain_error(Some("x (os error 32)")).unwrap();
        let missing = plain_error(Some("x (os error 3)")).unwrap();
        assert_ne!(locked, missing);
        assert!(!plain_error(Some("x (os error 53)")).unwrap().contains("can't write"));
    }

    #[test]
    fn unknown_text_is_hidden_and_none_stays_none() {
        let m = plain_error(Some("stack overflow in frobnicator at 0xdeadbeef")).unwrap();
        assert!(!m.contains("frobnicator"));
        assert_eq!(plain_error(None), None);
    }
}
