// FCM HTTP v1 client: exchanges a Google service-account for an OAuth access
// token (cached ~55 min) and sends high-priority data messages.
//
// Config (all optional — if absent, `from_env` returns None and signaling is
// disabled):
//   * FCM_SA_JSON               — service-account JSON inline, OR
//   * GOOGLE_APPLICATION_CREDENTIALS — path to the service-account JSON file
//   * FCM_PROJECT_ID            — overrides the project_id from the SA file
//
// The service-account JSON is a SECRET; keep it out of the repo.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Deserialize, Clone)]
struct ServiceAccount {
    project_id: String,
    private_key: String,
    client_email: String,
    token_uri: String,
}

#[derive(Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

#[derive(Debug)]
#[allow(dead_code)] // payloads surfaced via Debug in logs
pub enum FcmError {
    Auth(String),
    Send(String),
    /// The target token is no longer valid; the caller should clear it.
    Unregistered,
}

pub struct FcmClient {
    project_id: String,
    sa: ServiceAccount,
    http: reqwest::Client,
    token: Mutex<Option<(String, Instant)>>,
}

impl FcmClient {
    /// Build from env, or `None` if no credentials are configured.
    pub fn from_env() -> Option<Arc<FcmClient>> {
        let json = std::env::var("FCM_SA_JSON").ok().or_else(|| {
            std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
        })?;
        let sa: ServiceAccount = match serde_json::from_str(&json) {
            Ok(sa) => sa,
            Err(e) => {
                tracing::warn!("FCM disabled: bad service-account JSON: {e}");
                return None;
            }
        };
        let project_id = std::env::var("FCM_PROJECT_ID").unwrap_or_else(|_| sa.project_id.clone());
        tracing::info!("FCM enabled for project {project_id}");
        Some(Arc::new(FcmClient {
            project_id,
            sa,
            http: reqwest::Client::new(),
            token: Mutex::new(None),
        }))
    }

    async fn access_token(&self) -> Result<String, FcmError> {
        if let Some((t, exp)) = self.token.lock().await.as_ref() {
            if Instant::now() < *exp {
                return Ok(t.clone());
            }
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let claims = JwtClaims {
            iss: &self.sa.client_email,
            scope: "https://www.googleapis.com/auth/firebase.messaging",
            aud: &self.sa.token_uri,
            iat: now,
            exp: now + 3600,
        };
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(self.sa.private_key.as_bytes())
            .map_err(|e| FcmError::Auth(format!("bad SA private_key: {e}")))?;
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let assertion =
            jsonwebtoken::encode(&header, &claims, &key).map_err(|e| FcmError::Auth(e.to_string()))?;

        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            expires_in: u64,
        }
        let resp = self
            .http
            .post(&self.sa.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| FcmError::Auth(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(FcmError::Auth(format!("token endpoint {}", resp.status())));
        }
        let tr: TokenResp = resp.json().await.map_err(|e| FcmError::Auth(e.to_string()))?;
        let exp = Instant::now() + Duration::from_secs(tr.expires_in.saturating_sub(300));
        *self.token.lock().await = Some((tr.access_token.clone(), exp));
        Ok(tr.access_token)
    }

    /// Send a high-priority data message. `data` must be a JSON object whose
    /// values are strings (FCM data payloads are string→string).
    pub async fn send_data(&self, token: &str, data: serde_json::Value) -> Result<(), FcmError> {
        let access = self.access_token().await?;
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project_id
        );
        let body = serde_json::json!({
            "message": {
                "token": token,
                "data": data,
                "android": { "priority": "high" },
                "apns": { "headers": { "apns-priority": "10" } }
            }
        });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
            .map_err(|e| FcmError::Send(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let txt = resp.text().await.unwrap_or_default();
        // Only a genuinely dead token should clear the stored token. INVALID_ARGUMENT
        // is a message-format bug on our side — surfacing it (instead of clearing a
        // valid token) is what makes such bugs debuggable.
        if status.as_u16() == 404 || txt.contains("UNREGISTERED") || txt.contains("NOT_FOUND") {
            return Err(FcmError::Unregistered);
        }
        Err(FcmError::Send(format!("FCM {status}: {txt}")))
    }
}
