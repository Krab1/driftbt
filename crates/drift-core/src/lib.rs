//! Engine-agnostic domain logic for Drift. No UI, no Tauri, no HTTP server.

pub mod engine;
pub mod errors;
pub mod events;
pub mod guard;
pub mod ipfilter;
pub mod ipfilter_source;
pub mod jsonfile;
pub mod library;
pub mod manager;
pub mod model;
pub mod netctl;
pub mod open_args;
pub mod playlist;
pub mod queue;
pub mod rules;
pub mod schedule;
pub mod search;
pub mod settings;
pub mod stream;
pub mod ticker;
pub mod watch;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
