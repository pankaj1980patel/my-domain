// my-domain registry server.
//
// Users log in (username/password → JWT), register their devices, and look up
// their OTHER devices' endpoints (IP + TCP/UDP ports) — so clients can connect
// directly without LAN discovery. Backed by MongoDB Atlas. Config in `.env`.

mod admin;
mod auth;
mod devices;
mod error;
mod fcm;
mod limits;
mod models;
mod pair;
mod relay;
mod session;
mod signal;
mod signup;
mod state;
mod store;

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
        .route("/auth/logout", post(auth::logout))
        .route("/auth/logout_all", post(auth::logout_all))
        .route("/auth/password", post(auth::change_password))
        .route("/auth/sessions", get(auth::sessions))
        .route("/auth/sessions/revoke", post(auth::revoke_session))
        .layer(DefaultBodyLimit::max(MAX_AUTH_BODY));
    let admin = Router::new()
        .route("/admin/limits", get(admin::limits))
        .route("/admin/invites", post(admin::invites))
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
        .merge(admin)
        .merge(devices)
        .merge(signals)
        .with_state(state)
}

/// Shortest `JWT_SECRET` that isn't warned about (or, strict, refused).
const MIN_JWT_SECRET: usize = 32;

/// `STRICT_SECRETS=1` (or `true`): refuse to start with weak secrets.
fn strict_secrets() -> bool {
    matches!(
        std::env::var("STRICT_SECRETS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Security review L23: a short secret can be brute-forced offline from any
/// one token. Refused only when strict, so a deploy can't brick the server
/// by default; replace it with `openssl rand -base64 48`.
fn check_jwt_secret(secret: &str, strict: bool) -> anyhow::Result<()> {
    if secret.len() >= MIN_JWT_SECRET {
        return Ok(());
    }
    let msg = format!(
        "JWT_SECRET is only {} bytes; use at least {MIN_JWT_SECRET} (openssl rand -base64 48)",
        secret.len()
    );
    if strict {
        anyhow::bail!("{msg} (STRICT_SECRETS is on)");
    }
    tracing::warn!("{msg}");
    Ok(())
}

/// Read back the lockouts and daily counts saved before a restart. A store
/// that can't be read is logged, not fatal (limits then start empty).
async fn restore_limits(s: &AppState) {
    let r = (
        s.pair.restore().await,
        s.push.restore().await,
        s.signup.restore().await,
    );
    match r {
        (Ok(a), Ok(b), Ok(c)) => {
            tracing::info!("restored limits: {a} lockouts, {b} push and {c} sign-up day counts")
        }
        (a, b, c) => tracing::warn!(
            "restoring limits failed: {:?} {:?} {:?}",
            a.err(),
            b.err(),
            c.err()
        ),
    }
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

    // Limiter records and invite codes expire by themselves (TTL index).
    for coll in [store::LIMITS_COLL, store::INVITES_COLL] {
        let _ = db
            .collection::<mongodb::bson::Document>(coll)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "expires": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::ZERO)
                            .build(),
                    )
                    .build(),
            )
            .await;
    }
    let _ = db
        .collection::<mongodb::bson::Document>(store::LIMITS_COLL)
        .create_index(IndexModel::builder().keys(doc! { "key": 1 }).build())
        .await;

    check_jwt_secret(&jwt_secret, strict_secrets())?;
    let fcm = fcm::FcmClient::from_env();
    if fcm.is_none() {
        tracing::warn!("FCM not configured (set FCM_SA_JSON or GOOGLE_APPLICATION_CREDENTIALS); /signal disabled");
    }
    let store = store::MongoStore::shared(db.clone());
    let sessions = session::Sessions::new(jwt_secret, store.clone());
    let relay_state = relay::RelayState::new(sessions.clone());
    let relay = relay::router(relay_state.clone());
    let pair_state = pair::PairState::new(sessions.clone());
    let pairing = pair::router(pair_state.clone());
    let state = AppState {
        db,
        sessions,
        fcm,
        relay: relay_state,
        push: std::sync::Arc::new(signal::PushLimiter::new(
            signal::PushConfig::from_env(),
            store.clone(),
        )),
        max_devices: limits::env_or("MAX_DEVICES_PER_ACCOUNT", devices::MAX_DEVICES),
        signup: signup::Signup::new(signup::SignupConfig::from_env(), store),
        pair: pair_state,
        admin_token: admin::token_from_env(),
    };
    restore_limits(&state).await;
    tracing::info!(
        "sign-up: {}; /admin: {}",
        state.signup.mode().as_str(),
        if state.admin_token.is_some() {
            "on"
        } else {
            "off"
        }
    );

    let app = api(state)
        // WebSocket relay (own state: sessions only).
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
        start_with(push, signup::SignupConfig::default(), None).await
    }

    async fn start_with(
        push: signal::PushConfig,
        signup: signup::SignupConfig,
        admin_token: Option<&str>,
    ) -> String {
        let client = mongodb::Client::with_uri_str(
            "mongodb://127.0.0.1:9/?serverSelectionTimeoutMS=100&connectTimeoutMS=100",
        )
        .await
        .unwrap();
        let store = store::MemStore::new();
        let sessions = session::Sessions::new(SECRET.into(), store.clone());
        let state = AppState {
            db: client.database("none"),
            sessions: sessions.clone(),
            fcm: None,
            relay: relay::RelayState::new(sessions.clone()),
            push: std::sync::Arc::new(signal::PushLimiter::new(push, store.clone())),
            max_devices: devices::MAX_DEVICES,
            signup: signup::Signup::new(signup, store),
            pair: pair::PairState::new(sessions),
            admin_token: admin_token.map(Into::into),
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

    async fn send(
        base: &str,
        path: &str,
        token: Option<&str>,
        ip: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut r = if body.is_some() {
            reqwest::Client::new().post(format!("{base}{path}"))
        } else {
            reqwest::Client::new().get(format!("{base}{path}"))
        }
        // The server is on loopback, so it takes this as the client.
        .header("x-forwarded-for", ip);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        r.send().await.unwrap()
    }

    fn signup_body() -> Value {
        json!({"username": "newbie", "password": "secret-pw"})
    }

    #[tokio::test]
    async fn sign_ups_are_limited_per_address_and_closed_or_invite_modes_refuse() {
        let base = start_with(
            signal::PushConfig::default(),
            signup::SignupConfig {
                per_client_per_min: 2.0,
                ..Default::default()
            },
            None,
        )
        .await;
        // Past the limits it goes on to the (absent) database: 500.
        for _ in 0..2 {
            let r = send(
                &base,
                "/auth/register",
                None,
                "203.0.113.5",
                Some(signup_body()),
            )
            .await;
            assert_eq!(r.status(), 500);
        }
        let r = send(
            &base,
            "/auth/register",
            None,
            "203.0.113.5",
            Some(signup_body()),
        )
        .await;
        assert_eq!(r.status(), 429);
        assert!(r.headers().contains_key("retry-after"));
        // The same IPv6 /64 is one address; another one isn't limited.
        for (ip, want) in [
            ("2001:db8:7:7::1", 500),
            ("2001:db8:7:7::2", 500),
            ("2001:db8:7:7::3", 429),
            ("2001:db8:7:8::1", 500),
        ] {
            let r = send(&base, "/auth/register", None, ip, Some(signup_body())).await;
            assert_eq!(r.status(), want, "{ip}");
        }

        let closed = start_with(
            signal::PushConfig::default(),
            signup::SignupConfig {
                mode: signup::Mode::Closed,
                ..Default::default()
            },
            None,
        )
        .await;
        let r = send(
            &closed,
            "/auth/register",
            None,
            "203.0.113.6",
            Some(signup_body()),
        )
        .await;
        assert_eq!(r.status(), 403);

        let invite = start_with(
            signal::PushConfig::default(),
            signup::SignupConfig {
                mode: signup::Mode::Invite,
                ..Default::default()
            },
            Some(&"a".repeat(40)),
        )
        .await;
        let r = send(
            &invite,
            "/auth/register",
            None,
            "203.0.113.7",
            Some(signup_body()),
        )
        .await;
        assert_eq!(r.status(), 403, "no invite");
        let mut body = signup_body();
        body["invite"] = json!("wxyz-wxyz-wxyz-wxyz-wxyz");
        let r = send(&invite, "/auth/register", None, "203.0.113.7", Some(body)).await;
        assert_eq!(r.status(), 403, "unknown invite");
        // A real one (from /admin/invites) gets past to the database.
        let admin = "a".repeat(40);
        let r = send(
            &invite,
            "/admin/invites",
            Some(&admin),
            "127.0.0.1",
            Some(json!({})),
        )
        .await;
        assert_eq!(r.status(), 200);
        let code = r.json::<Value>().await.unwrap()["code"]
            .as_str()
            .unwrap()
            .to_uppercase();
        let mut body = signup_body();
        body["invite"] = json!(code);
        let r = send(
            &invite,
            "/auth/register",
            None,
            "203.0.113.8",
            Some(body.clone()),
        )
        .await;
        assert_eq!(r.status(), 500);
        // The sign-up failed, so the invite was given back and still works.
        let r = send(&invite, "/auth/register", None, "203.0.113.9", Some(body)).await;
        assert_eq!(r.status(), 500);
        let r = send(&invite, "/admin/limits", Some(&admin), "127.0.0.1", None).await;
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["signup"]["mode"], "invite");
        assert_eq!(v["signup"]["refused_invite_since_start"], 2);
        assert_eq!(v["invites_with_uses_left"], 1);
    }

    #[tokio::test]
    async fn logout_signs_out_this_session_and_logout_all_every_session() {
        let base = start(signal::PushConfig::default()).await;
        let ip = "203.0.113.20";
        let (a, b) = (
            issue_token(SECRET, "id-ann", "ann").unwrap(),
            issue_token(SECRET, "id-ann", "ann").unwrap(),
        );
        // Signed in: the request goes on to the (absent) database.
        assert_eq!(
            send(&base, "/devices", Some(&a), ip, None).await.status(),
            500
        );
        let r = send(&base, "/auth/logout", Some(&a), ip, Some(json!({}))).await;
        assert_eq!(r.status(), 204);
        assert_eq!(
            send(&base, "/devices", Some(&a), ip, None).await.status(),
            401
        );
        assert_eq!(
            send(&base, "/devices", Some(&b), ip, None).await.status(),
            500
        );

        let r = send(&base, "/auth/logout_all", Some(&b), ip, Some(json!({}))).await;
        assert_eq!(r.status(), 204);
        assert_eq!(
            send(&base, "/devices", Some(&b), ip, None).await.status(),
            401
        );
        let r = send(&base, "/auth/logout", Some(&b), ip, Some(json!({}))).await;
        assert_eq!(r.status(), 401, "already signed out");
        // A password change needs a session too.
        let r = send(
            &base,
            "/auth/password",
            Some(&b),
            ip,
            Some(json!({"password": "x", "new_password": "longer-one"})),
        )
        .await;
        assert_eq!(r.status(), 401);
    }

    #[tokio::test]
    async fn sessions_are_listed_and_one_can_be_signed_out() {
        let base = start(signal::PushConfig::default()).await;
        let ip = "203.0.113.21";
        let (a, b) = (
            issue_token(SECRET, "id-ann", "ann").unwrap(),
            issue_token(SECRET, "id-ann", "ann").unwrap(),
        );
        assert_eq!(
            send(&base, "/auth/sessions", None, ip, None).await.status(),
            401
        );
        // Each session is listed once it has been used.
        let r = send(&base, "/auth/sessions", Some(&b), ip, None).await;
        assert_eq!(r.status(), 200);
        let r = send(&base, "/auth/sessions", Some(&a), ip, None).await;
        assert_eq!(r.status(), 200);
        let list: Vec<Value> = r.json().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["current"], true);
        assert_eq!(list[1]["current"], false);
        for k in ["id", "name", "node_id", "created", "last_seen"] {
            assert!(list[1].get(k).is_some(), "{k}");
        }
        let b_id = list[1]["id"].as_str().unwrap().to_string();

        let revoke = |id: Value| {
            send(
                &base,
                "/auth/sessions/revoke",
                Some(&a),
                ip,
                Some(json!({ "id": id })),
            )
        };
        assert_eq!(revoke(json!("not-a-session")).await.status(), 404);
        assert_eq!(revoke(json!("")).await.status(), 400);
        let r = revoke(json!(b_id)).await;
        assert_eq!(r.status(), 200);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["signed_out_all"], false);
        assert_eq!(
            send(&base, "/auth/sessions", Some(&b), ip, None)
                .await
                .status(),
            401,
            "the signed-out session is refused"
        );
        let r = send(&base, "/auth/sessions", Some(&a), ip, None).await;
        let list: Vec<Value> = r.json().await.unwrap();
        assert_eq!(list.len(), 1, "and gone from the list");
    }

    #[tokio::test]
    async fn admin_routes_are_off_without_a_token_and_show_only_counts() {
        let off = start(signal::PushConfig::default()).await;
        let r = send(&off, "/admin/limits", Some("anything"), "127.0.0.1", None).await;
        assert_eq!(r.status(), 404);
        let r = send(
            &off,
            "/admin/invites",
            Some("x"),
            "127.0.0.1",
            Some(json!({})),
        )
        .await;
        assert_eq!(r.status(), 404);

        let token = "t".repeat(40);
        let on = start_with(
            signal::PushConfig::default(),
            signup::SignupConfig::default(),
            Some(&token),
        )
        .await;
        let r = send(&on, "/admin/limits", Some("wrong"), "127.0.0.1", None).await;
        assert_eq!(r.status(), 401);
        let r = send(&on, "/admin/limits", None, "127.0.0.1", None).await;
        assert_eq!(r.status(), 401);
        // A sign-up from an address shows up as a count, never the address.
        send(
            &on,
            "/auth/register",
            None,
            "198.51.100.77",
            Some(signup_body()),
        )
        .await;
        let r = send(&on, "/admin/limits", Some(&token), "127.0.0.1", None).await;
        assert_eq!(r.status(), 200);
        let text = r.text().await.unwrap();
        assert!(!text.contains("198.51.100"), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["signup"]["mode"], "open");
        assert_eq!(v["pair"]["addresses_locked_out"], 0);
        assert_eq!(v["relay"]["connections"], 0);
    }

    #[test]
    fn a_short_jwt_secret_is_refused_only_when_strict() {
        assert!(check_jwt_secret("short", false).is_ok());
        assert!(check_jwt_secret("short", true).is_err());
        assert!(check_jwt_secret(&"k".repeat(32), true).is_ok());
    }
}
