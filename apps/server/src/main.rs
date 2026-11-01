// my-domain registry server.
//
// Users log in (username/password → JWT), register their devices, and look up
// their OTHER devices' endpoints (IP + TCP/UDP ports) — so clients can connect
// directly without LAN discovery. Backed by MongoDB Atlas. Config in `.env`.

mod auth;
mod devices;
mod error;
mod fcm;
mod models;
mod signal;
mod state;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use mongodb::bson::doc;
use mongodb::options::IndexOptions;
use mongodb::{IndexModel};
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

    let fcm = fcm::FcmClient::from_env();
    if fcm.is_none() {
        tracing::warn!("FCM not configured (set FCM_SA_JSON or GOOGLE_APPLICATION_CREDENTIALS); /signal disabled");
    }
    let state = AppState { db, jwt_secret, fcm };

    let app = Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/verify", post(auth::verify))
        .route("/devices/register", post(devices::register))
        .route("/devices", get(devices::list))
        .route("/devices/unregister", post(devices::unregister))
        .route("/devices/state", post(devices::state))
        .route("/devices/probe", post(devices::probe))
        .route("/signal", post(signal::relay))
        .route("/signal/selfping", post(signal::selfping))
        .layer(CorsLayer::permissive())
        .with_state(state);

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
