// my-domain registry server.
//
// Users log in (username/password → JWT), register their devices, and look up
// their OTHER devices' endpoints (IP + TCP/UDP ports) — so clients can connect
// directly without LAN discovery. Backed by MongoDB Atlas. Config in `.env`.

mod auth;
mod devices;
mod error;
mod fcm;
mod limits;
mod models;
mod pair;
mod relay;
mod signal;
mod state;

use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use mongodb::bson::doc;
use mongodb::options::IndexOptions;
use mongodb::IndexModel;
use serde_json::json;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;

use state::AppState;

/// Liveness + DB connectivity check.
async fn health(State(s): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    let db_ok = s.db.run_command(doc! { "ping": 1 }).await.is_ok();
    let (status, db) = if db_ok {
        (StatusCode::OK, "up")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "down")
    };
    (
        status,
        Json(json!({
            "service": "my-domain-registry",
            "version": env!("CARGO_PKG_VERSION"),
            "status": if db_ok { "ok" } else { "degraded" },
            "db": db,
        })),
    )
}

/// Largest body on the auth routes (a username and a password).
const MAX_AUTH_BODY: usize = 4 * 1024;
/// Largest body on the signal routes (`data` is checked against
/// `signal::MAX_SIGNAL_BYTES` itself).
const MAX_SIGNAL_BODY: usize = 2 * signal::MAX_SIGNAL_BYTES;

/// The HTTP API on `state`, each group of routes with its own body cap
/// (over it: 413 before the handler runs).
fn api(state: AppState) -> Router {
    let auth = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/verify", post(auth::verify))
        .layer(DefaultBodyLimit::max(MAX_AUTH_BODY));
    let devices = Router::new()
        .route("/devices/register", post(devices::register))
        .route("/devices", get(devices::list))
        .route("/devices/unregister", post(devices::unregister))
        .route("/devices/state", post(devices::state))
        .route("/devices/probe", post(devices::probe))
        .layer(DefaultBodyLimit::max(devices::MAX_BODY));
    let signals = Router::new()
        .route("/signal", post(signal::relay))
        .route("/signal/selfping", post(signal::selfping))
        .layer(DefaultBodyLimit::max(MAX_SIGNAL_BODY));
    Router::new()
        .route("/health", get(health))
        .merge(auth)
        .merge(devices)
        .merge(signals)
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mongo_uri = std::env::var("MONGO_URI").expect("MONGO_URI must be set (see .env)");
    let mongo_db = std::env::var("MONGO_DB").unwrap_or_else(|_| "device_registry".into());
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set (see .env)");
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let client = mongodb::Client::with_uri_str(&mongo_uri).await?;
    let db = client.database(&mongo_db);

    // Best-effort unique indexes.
    let _ = db
        .collection::<models::User>(models::USERS_COLL)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "username": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await;
    let _ = db
        .collection::<models::Device>(models::DEVICES_COLL)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "user_id": 1, "node_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await;

    if jwt_secret.len() < 32 {
        // Security review L23: a short secret can be brute-forced offline
        // from any one token. Not refused, so a deploy can't brick the
        // server; replace it with `openssl rand -base64 48`.
        tracing::warn!(
            "JWT_SECRET is only {} bytes; use at least 32 (openssl rand -base64 48)",
            jwt_secret.len()
        );
    }
    let fcm = fcm::FcmClient::from_env();
    if fcm.is_none() {
        tracing::warn!("FCM not configured (set FCM_SA_JSON or GOOGLE_APPLICATION_CREDENTIALS); /signal disabled");
    }
    let relay_state = relay::RelayState::new(jwt_secret.clone());
    let relay = relay::router(relay_state.clone());
    let pairing = pair::router(pair::PairState::new(jwt_secret.clone()));
    let state = AppState {
        db,
        jwt_secret,
        fcm,
        relay: relay_state,
        push: std::sync::Arc::new(signal::PushLimiter::new(signal::PushConfig::from_env())),
        max_devices: limits::env_or("MAX_DEVICES_PER_ACCOUNT", devices::MAX_DEVICES),
    };

    let app = api(state)
        // WebSocket relay (own state: needs only the JWT secret).
        .merge(relay)
        // Add a device by QR code / 6-digit code (own state, in memory).
        .merge(pairing)
        .layer(CorsLayer::permissive());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("my-domain registry listening on http://{addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod api_tests {
    //! The API's caps over HTTP. The database is one that isn't there
    //! (the driver connects lazily), so these check what is refused before
    //! any query: a request that gets past the caps fails with 500.
    use super::*;
    use crate::auth::issue_token;
    use serde_json::Value;

    const SECRET: &str = "api-test-secret";

    async fn start(push: signal::PushConfig) -> String {
        let client = mongodb::Client::with_uri_str(
            "mongodb://127.0.0.1:9/?serverSelectionTimeoutMS=100&connectTimeoutMS=100",
        )
        .await
        .unwrap();
        let state = AppState {
            db: client.database("none"),
            jwt_secret: SECRET.into(),
            fcm: None,
            relay: relay::RelayState::new(SECRET.into()),
            push: std::sync::Arc::new(signal::PushLimiter::new(push)),
            max_devices: devices::MAX_DEVICES,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                api(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        format!("http://{addr}")
    }

    async fn post(base: &str, path: &str, body: &Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{base}{path}"))
            .bearer_auth(issue_token(SECRET, "id-ann", "ann").unwrap())
            .json(body)
            .send()
            .await
            .unwrap()
    }

    fn device() -> Value {
        json!({"node_id": "n1", "name": "Pixel", "tcp_port": 1, "udp_port": 2})
    }

    #[tokio::test]
    async fn oversized_registrations_are_refused_before_the_database() {
        let base = start(signal::PushConfig::default()).await;
        // A body over the cap: 413.
        let mut big = device();
        big["padding"] = json!("x".repeat(devices::MAX_BODY));
        assert_eq!(post(&base, "/devices/register", &big).await.status(), 413);
        // A field over its cap: 400, naming it.
        for (field, v) in [
            ("name", json!("n".repeat(devices::MAX_NAME + 1))),
            ("node_id", json!("i".repeat(devices::MAX_NODE_ID + 1))),
            ("fcm_token", json!("t".repeat(devices::MAX_FCM_TOKEN + 1))),
            ("platform", json!("p".repeat(64))),
            ("tcp_port", json!(1_000_000)),
        ] {
            let mut d = device();
            d[field] = v;
            let r = post(&base, "/devices/register", &d).await;
            assert_eq!(r.status(), 400, "{field}");
            let body: Value = r.json().await.unwrap();
            assert!(body["error"].as_str().unwrap().starts_with(field), "{body}");
        }
        let r = post(
            &base,
            "/devices/state",
            &json!({"node_id": "n1", "reflexive_ip": "9".repeat(100)}),
        )
        .await;
        assert_eq!(r.status(), 400);
        let r = post(
            &base,
            "/devices/probe",
            &json!({"node_id": "n1", "check": vec!["tcp"; 50]}),
        )
        .await;
        assert_eq!(r.status(), 400);
        // Within the caps it goes on to the (absent) database.
        assert_eq!(
            post(&base, "/devices/register", &device()).await.status(),
            500
        );
    }

    #[tokio::test]
    async fn oversized_signals_are_refused_and_a_flood_gets_429_with_retry_after() {
        let base = start(signal::PushConfig {
            signals_per_min: 30.0,
            ..Default::default()
        })
        .await;
        let sig = |data: Value| json!({"from": "a", "to": "b", "type": "signal", "data": data});
        let r = post(
            &base,
            "/signal",
            &sig(json!("x".repeat(signal::MAX_SIGNAL_BYTES + 10))),
        )
        .await;
        assert_eq!(r.status(), 413);
        let r = post(
            &base,
            "/signal",
            &sig(json!("x".repeat(3 * signal::MAX_SIGNAL_BYTES))),
        )
        .await;
        assert_eq!(r.status(), 413, "over the body cap");
        let mut long_kind = sig(json!({}));
        long_kind["type"] = json!("k".repeat(100));
        assert_eq!(post(&base, "/signal", &long_kind).await.status(), 400);
        // Bursts of 6 (30 a minute / 5); two were charged above (the one
        // over the body cap never reached the handler).
        for _ in 0..4 {
            assert_eq!(post(&base, "/signal", &sig(json!({}))).await.status(), 500);
        }
        let r = post(&base, "/signal", &sig(json!({}))).await;
        assert_eq!(r.status(), 429);
        let secs: u64 = r.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=2).contains(&secs), "{secs}");
        // Self-pings share the account's signal budget.
        let r = post(&base, "/signal/selfping", &json!({"node_id": "a"})).await;
        assert_eq!(r.status(), 429);
    }
}
