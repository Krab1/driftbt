//! What a source hands back, and the fixed reasons it can fail. The app words its own messages from the reason,
//! so nothing a remote server said (and nothing containing an address or key) ever reaches the screen.

use serde::Serialize;

/// One search result, in the shape the plugin protocol expects (docs/PLUGIN_PROTOCOL.md).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Row {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magnet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub torrent_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeders: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leechers: Option<u32>,
    /// Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The badge for this row when one plugin searches several places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Network,
    Auth,
    BadResponse,
    Config,
}

impl Reason {
    /// The word the protocol uses in `error.data.reason`.
    pub fn word(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Auth => "auth",
            Self::BadResponse => "bad_response",
            Self::Config => "config",
        }
    }
}
