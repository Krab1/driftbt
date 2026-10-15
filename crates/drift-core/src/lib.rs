//! Engine-agnostic domain logic for Drift. No UI, no Tauri, no HTTP server.

pub mod background;
pub mod engine;
pub mod fsmove;
pub mod errors;
pub mod events;
pub mod geoip;
pub mod guard;
pub mod ipfilter;
pub mod ipfilter_source;
pub mod jsonfile;
pub mod lock;
pub mod library;
pub mod manager;
pub mod model;
pub mod netctl;
pub mod open_args;
pub mod playlist;
pub mod plugins;
pub mod queue;
pub mod rules;
pub mod schedule;
pub mod search;
pub mod service;
pub mod settings;
pub mod stream;
pub mod ticker;
pub mod watch;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
