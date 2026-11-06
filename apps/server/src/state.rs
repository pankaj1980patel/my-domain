use std::sync::Arc;

use mongodb::Database;

use crate::fcm::FcmClient;

/// Shared application state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub jwt_secret: String,
    /// `None` when no FCM credentials are configured (signaling disabled).
    pub fcm: Option<Arc<FcmClient>>,
    /// Live device WebSockets: signals go there first, FCM is the fallback.
    pub relay: crate::relay::RelayState,
    /// Signal and push limits (see `signal.rs`).
    pub push: Arc<crate::signal::PushLimiter>,
    /// Most devices one account may register.
    pub max_devices: u64,
}
