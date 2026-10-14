#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Composition root: engine, store, search registry and platform services are wired here (M1+).
fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running Drift");
}
