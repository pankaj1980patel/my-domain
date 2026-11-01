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
}
