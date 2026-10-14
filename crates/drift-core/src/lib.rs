//! Engine-agnostic domain logic for Drift. No UI, no Tauri, no HTTP server.

pub mod engine;
pub mod errors;
pub mod events;
pub mod model;
pub mod open_args;
pub mod settings;
pub mod ticker;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
