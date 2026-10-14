//! Engine-agnostic domain logic for Drift. No UI, no Tauri, no HTTP server.

pub mod engine;
pub mod model;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
