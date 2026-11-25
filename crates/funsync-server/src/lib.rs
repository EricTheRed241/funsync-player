//! FunSync in Rust: one server that owns the library and serves the web UI
//! (port 5300), built beside the Electron app to reach feature parity
//! (`docs/rust-app/parity.md`). Phase 1: data folder, import from the old
//! app (read-only), library scan, media streaming, thumbnails, scripts.

pub mod access;
pub mod assoc;
pub mod associate;
pub mod autostart;
pub mod cli;
pub mod data;
pub mod delete;
pub mod devices;
pub mod edits;
pub mod eroscripts;
pub mod groupings;
pub mod heresphere;
pub mod import;
pub mod log;
pub mod media;
pub mod posters;
pub mod prefs;
pub mod queue;
pub mod remote;
pub mod scan;
pub mod server;
pub mod settings;
pub mod sites;
pub mod stereo;
pub mod vr;
