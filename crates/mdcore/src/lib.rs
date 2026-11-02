//! mdcore — the my-domain shared core.
//!
//! Pure Rust (no tauri/jni/platform deps). Platform specifics live behind the
//! traits in `platform` + `capabilities`; each client (desktop, android, …)
//! implements them and drives the `Engine`.
//!
//! Layering: `Engine` → transports (discovery + UDP/TCP/WS) → `EventSink`.
//! The `service` module is the seam for the future channel-mux + per-feature
//! Service framework (protocol v2).

pub mod capabilities;
pub mod config;
pub mod crypto;
pub mod discovery;
pub mod engine;
pub mod error;
pub mod events;
pub mod features;
pub mod kv;
pub mod mdns;
pub mod model;
pub mod nat;
pub mod platform;
pub mod registry;
pub mod service;
pub mod services;
pub mod signal;
pub mod tls;
pub mod transport;
pub mod wire;

pub use config::{ConnectRungs, CoreConfig, KeepAlive};
pub use engine::Engine;
pub use error::{CoreError, Result};
pub use events::{CoreEvent, EventSink};
pub use platform::{IfaceMode, Platform};
