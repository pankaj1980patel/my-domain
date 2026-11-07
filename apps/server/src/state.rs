use std::sync::Arc;

use mongodb::Database;

use crate::fcm::FcmClient;

/// Shared application state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    /// Issuing and checking tokens, and signing sessions out.
    pub sessions: Arc<crate::session::Sessions>,
    /// `None` when no FCM credentials are configured (signaling disabled).
    pub fcm: Option<Arc<FcmClient>>,
    /// Live device WebSockets: signals go there first, FCM is the fallback.
    pub relay: crate::relay::RelayState,
    /// Signal and push limits (see `signal.rs`).
    pub push: Arc<crate::signal::PushLimiter>,
    /// Most devices one account may register.
    pub max_devices: u64,
    /// Sign-up mode and limits (see `signup.rs`).
    pub signup: Arc<crate::signup::Signup>,
    /// The pairing routes' state (for `/admin/limits`).
    pub pair: crate::pair::PairState,
    /// `ADMIN_TOKEN`; the `/admin/*` routes don't exist without it.
    pub admin_token: Option<Arc<str>>,
}
