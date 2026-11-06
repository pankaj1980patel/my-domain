//! A fake registry server: real HTTP on an ephemeral loopback port, with the
//! same routes and JSON shapes as `apps/server`, but users and devices live in
//! memory and there is no MongoDB, JWT or FCM.
//!
//! Tests point an engine at `registry.url()` and exercise the engine's REAL
//! HTTP client code (URL building, auth headers, JSON (de)serialization,
//! error handling). A mock behind a trait would skip all of that.
//!
//! Node analogy: `nock` intercepts requests; this is closer to spinning up an
//! in-memory Express app with supertest.
//!
//! Routes (subset the engine uses):
//!   POST /auth/register | /auth/login  {username,password} -> {token,username}
//!   POST /auth/verify                  {username,password} -> 200 | 401
//!   POST /devices/register  (Bearer)   device record -> upsert
//!   GET  /devices?exclude=<node_id> (Bearer) -> the user's other devices
//!   POST /devices/state     (Bearer)   {node_id, ws_open?, inbound_blocked?, ...}
//!   POST /devices/probe     (Bearer)   -> {tcp_reachable, ws_reachable, udp_reachable}
//!   GET  /health                       -> 200
//!   POST /signal            (Bearer)   recorded (delivery is SignalHub's job)
//!   POST /pair/create (Bearer), POST /pair/claim, GET /pair/status (Bearer),
//!   POST /pair/cancel (Bearer)  adding a device by QR / code (single use;
//!                               no expiry here). `without_pairing` turns
//!                               them off, like a server from before them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};

/// A JWT-shaped session token valid for 7 days, like the real server's
/// (the engine reads `exp` to decide whether a saved session is still good).
fn session_token(username: &str) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + 7 * 24 * 3600;
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let claims = URL_SAFE_NO_PAD
        .encode(json!({"sub": username, "username": username, "exp": exp}).to_string());
    format!("{header}.{claims}.fake-signature")
}

#[derive(Default)]
struct State {
    /// username -> password
    users: HashMap<String, String>,
    /// token -> username
    tokens: HashMap<String, String>,
    /// (username, node_id) -> device JSON as registered/updated
    devices: HashMap<(String, String), Value>,
    /// Every request path seen, in order, e.g. "POST /devices/register".
    requests: Vec<String>,
    /// Bodies of POST /signal
    signals: Vec<Value>,
    /// When set (`start_with_relay`), /signal is delivered over the fake
    /// relay's WebSocket if the target is connected, like the real server.
    relay: Option<crate::relay::RelayConns>,
    /// How many signals went over the WebSocket.
    signals_via_ws: usize,
    /// username -> that account's pairing.
    pairings: HashMap<String, Pairing>,
    /// Answer /pair/* with 404 (an older server).
    no_pairing: bool,
}

#[derive(Clone, Default)]
struct Pairing {
    claim: Option<String>,
    code: Option<String>,
    code_box: Option<String>,
    joined: Option<String>,
}

pub struct FakeRegistry {
    url: String,
    state: Arc<Mutex<State>>,
    server: Arc<tiny_http::Server>,
}

impl FakeRegistry {
    /// Start serving on 127.0.0.1:<random port> in a background thread.
    pub fn start() -> Self {
        Self::start_on("127.0.0.1:0")
    }

    /// Serve on `bind` (`ip:port`; port 0 picks one).
    pub fn start_on(bind: &str) -> Self {
        let server = Arc::new(tiny_http::Server::http(bind).unwrap());
        let local = server.server_addr().to_ip().unwrap();
        let state: Arc<Mutex<State>> = Arc::default();
        {
            let server = server.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                for req in server.incoming_requests() {
                    handle(&state, req);
                }
            });
        }
        FakeRegistry {
            url: format!("http://{local}"),
            state,
            server,
        }
    }

    /// Like `start`, but /signal goes over `relay`'s WebSocket when the
    /// target device is connected to it (the real server's behaviour).
    pub fn start_with_relay(relay: &crate::FakeRelay) -> Self {
        Self::start_on_with_relay("127.0.0.1:0", relay)
    }

    /// `start_on` with signals going over `relay` (see `start_with_relay`).
    pub fn start_on_with_relay(bind: &str, relay: &crate::FakeRelay) -> Self {
        let reg = Self::start_on(bind);
        reg.state.lock().unwrap().relay = Some(relay.conns.clone());
        reg
    }

    /// Signals delivered over the WebSocket (not left for a push).
    pub fn signals_via_ws(&self) -> usize {
        self.state.lock().unwrap().signals_via_ws
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Pre-create an account (what `/auth/register` would do).
    pub fn add_user(&self, username: &str, password: &str) {
        self.state
            .lock()
            .unwrap()
            .users
            .insert(username.to_string(), password.to_string());
    }

    /// The stored record for one device, if registered.
    pub fn device(&self, username: &str, node_id: &str) -> Option<Value> {
        self.state
            .lock()
            .unwrap()
            .devices
            .get(&(username.to_string(), node_id.to_string()))
            .cloned()
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn signals(&self) -> Vec<Value> {
        self.state.lock().unwrap().signals.clone()
    }

    /// Behave like a server from before `/pair/*` (404).
    pub fn without_pairing(&self) {
        self.state.lock().unwrap().no_pairing = true;
    }

    /// The code box stored for `username`'s pairing (what the server holds).
    pub fn pairing_box(&self, username: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .pairings
            .get(username)
            .and_then(|p| p.code_box.clone())
    }

    /// The claim `username`'s pairing was created with.
    pub fn pairing_claim(&self, username: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .pairings
            .get(username)
            .and_then(|p| p.claim.clone())
    }
}

impl Drop for FakeRegistry {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

fn respond(req: tiny_http::Request, code: u16, body: Value) {
    let header =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let resp = tiny_http::Response::from_string(body.to_string())
        .with_status_code(code)
        .with_header(header);
    let _ = req.respond(resp);
}

fn handle(state: &Arc<Mutex<State>>, mut req: tiny_http::Request) {
    let method = req.method().to_string();
    let (path, query) = match req.url().split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (req.url().to_string(), String::new()),
    };
    let mut raw = String::new();
    let _ = req.as_reader().read_to_string(&mut raw);
    let body: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let bearer = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer ").map(String::from));

    let mut st = state.lock().unwrap();
    st.requests.push(format!("{method} {path}"));
    let token = bearer.clone().unwrap_or_default();
    let user = bearer.and_then(|t| st.tokens.get(&t).cloned());
    let s = |k: &str| body[k].as_str().unwrap_or_default().to_string();

    match (method.as_str(), path.as_str()) {
        ("GET", "/health") => {
            drop(st);
            respond(req, 200, json!({"status": "ok"}))
        }
        ("POST", "/auth/register") => {
            let (u, p) = (s("username").to_lowercase(), s("password"));
            if st.users.contains_key(&u) {
                drop(st);
                return respond(req, 409, json!({"error": "username already taken"}));
            }
            st.users.insert(u.clone(), p);
            // One token per user (the fake relay groups devices by token).
            let token = st
                .tokens
                .iter()
                .find(|(_, user)| **user == u)
                .map(|(t, _)| t.clone())
                .unwrap_or_else(|| session_token(&u));
            st.tokens.insert(token.clone(), u.clone());
            drop(st);
            respond(req, 200, json!({"token": token, "username": u}))
        }
        ("POST", "/auth/login") | ("POST", "/auth/verify") => {
            let (u, p) = (s("username").to_lowercase(), s("password"));
            if st.users.get(&u) != Some(&p) {
                drop(st);
                return respond(req, 401, json!({"error": "invalid username or password"}));
            }
            // One token per user (the fake relay groups devices by token).
            let token = st
                .tokens
                .iter()
                .find(|(_, user)| **user == u)
                .map(|(t, _)| t.clone())
                .unwrap_or_else(|| session_token(&u));
            st.tokens.insert(token.clone(), u.clone());
            drop(st);
            respond(req, 200, json!({"token": token, "username": u}))
        }
        (_, p) if p.starts_with("/devices") || p.starts_with("/signal") => {
            let Some(user) = user else {
                drop(st);
                return respond(req, 401, json!({"error": "missing or invalid token"}));
            };
            match (method.as_str(), p) {
                ("POST", "/devices/register") => {
                    let node = s("node_id");
                    let entry = st.devices.entry((user, node)).or_insert_with(|| json!({}));
                    // `$set`-style upsert: merge fields, keep the rest.
                    if let (Some(dst), Some(src)) = (entry.as_object_mut(), body.as_object()) {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                    drop(st);
                    respond(req, 200, json!({}))
                }
                ("GET", "/devices") => {
                    let exclude = query
                        .split('&')
                        .find_map(|kv| kv.strip_prefix("exclude="))
                        .unwrap_or_default()
                        .to_string();
                    let list: Vec<Value> = st
                        .devices
                        .iter()
                        .filter(|((u, n), _)| *u == user && *n != exclude)
                        .map(|(_, d)| {
                            let mut d = d.clone();
                            // Like the real server: never hand out FCM tokens.
                            if let Some(o) = d.as_object_mut() {
                                o.remove("fcm_token");
                                o.entry("ws_open").or_insert(json!(false));
                                o.entry("inbound_blocked").or_insert(json!(false));
                            }
                            d
                        })
                        .collect();
                    drop(st);
                    respond(req, 200, Value::Array(list))
                }
                ("POST", "/devices/state") => {
                    let node = s("node_id");
                    if let Some(Value::Object(dst)) = st.devices.get_mut(&(user, node)) {
                        for (k, v) in body.as_object().into_iter().flatten() {
                            if !v.is_null() {
                                dst.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    drop(st);
                    respond(req, 200, json!({}))
                }
                ("POST", "/devices/probe") => {
                    drop(st);
                    respond(
                        req,
                        200,
                        json!({"tcp_reachable": true, "ws_reachable": true, "udp_reachable": false}),
                    )
                }
                ("POST", "/signal") => {
                    let frame = json!({"signal": {
                        "type": s("type"), "sender": s("from"), "payload": body["data"].to_string()
                    }});
                    let sent = st.relay.as_ref().is_some_and(|conns| {
                        conns
                            .lock()
                            .unwrap()
                            .get(&(token.clone(), s("to")))
                            .is_some_and(|(_, q)| {
                                q.send(tungstenite::Message::Text(frame.to_string()))
                                    .is_ok()
                            })
                    });
                    if sent {
                        st.signals_via_ws += 1;
                    }
                    st.signals.push(body);
                    drop(st);
                    respond(req, 200, json!({}))
                }
                _ => {
                    drop(st);
                    respond(req, 404, json!({"error": "not found"}))
                }
            }
        }
        (_, p) if p.starts_with("/pair/") => {
            if st.no_pairing {
                drop(st);
                return respond(req, 404, Value::Null);
            }
            if p == "/pair/claim" {
                let claim = body["claim"].as_str().map(String::from);
                let code = body["code"].as_str().map(String::from);
                let found = st.pairings.iter().find_map(|(u, pr)| {
                    let by_claim = claim.is_some() && pr.claim == claim;
                    let by_code = code.is_some() && pr.code == code;
                    (by_claim || by_code).then(|| (u.clone(), by_code))
                });
                let Some((u, by_code)) = found else {
                    drop(st);
                    return respond(
                        req,
                        410,
                        json!({"error": "this join code has expired or was already used"}),
                    );
                };
                let token = st
                    .tokens
                    .iter()
                    .find(|(_, user)| **user == u)
                    .map(|(t, _)| t.clone())
                    .unwrap_or_else(|| session_token(&u));
                st.tokens.insert(token.clone(), u.clone());
                let pr = st.pairings.get_mut(&u).unwrap();
                pr.claim = None;
                pr.code = None;
                pr.joined = Some(s("name"));
                let code_box = pr.code_box.clone().filter(|_| by_code);
                drop(st);
                return respond(
                    req,
                    200,
                    json!({"token": token, "username": u, "code_box": code_box}),
                );
            }
            let Some(user) = user else {
                drop(st);
                return respond(req, 401, json!({"error": "unauthorized"}));
            };
            match (method.as_str(), p) {
                ("POST", "/pair/create") => {
                    let code = s("code");
                    if st
                        .pairings
                        .iter()
                        .any(|(u, pr)| *u != user && pr.code.as_deref() == Some(code.as_str()))
                    {
                        drop(st);
                        return respond(req, 409, json!({"error": "code in use"}));
                    }
                    st.pairings.insert(
                        user,
                        Pairing {
                            claim: Some(s("claim")),
                            code: Some(code.clone()),
                            code_box: body["code_box"].as_str().map(String::from),
                            joined: None,
                        },
                    );
                    drop(st);
                    respond(req, 200, json!({"code": code, "expires_in": 300}))
                }
                ("GET", "/pair/status") => {
                    let out = match st.pairings.get(&user).cloned() {
                        None => json!({"state": "none"}),
                        Some(Pairing {
                            joined: Some(name), ..
                        }) => {
                            st.pairings.remove(&user);
                            json!({"state": "joined", "name": name})
                        }
                        Some(_) => json!({"state": "waiting"}),
                    };
                    drop(st);
                    respond(req, 200, out)
                }
                ("POST", "/pair/cancel") => {
                    st.pairings.remove(&user);
                    drop(st);
                    respond(req, 200, json!({}))
                }
                _ => {
                    drop(st);
                    respond(req, 404, json!({"error": "not found"}))
                }
            }
        }
        _ => {
            drop(st);
            respond(req, 404, json!({"error": "not found"}))
        }
    }
}
