//! Engine-agnostic domain logic for Drift. No UI, no Tauri, no HTTP server.

pub mod engine;
pub mod errors;
pub mod events;
pub mod jsonfile;
pub mod library;
pub mod manager;
pub mod model;
pub mod open_args;
pub mod playlist;
pub mod queue;
pub mod rules;
pub mod settings;
pub mod ticker;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
