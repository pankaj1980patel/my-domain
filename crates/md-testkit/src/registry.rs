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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

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
}

pub struct FakeRegistry {
    url: String,
    state: Arc<Mutex<State>>,
    server: Arc<tiny_http::Server>,
}

impl FakeRegistry {
    /// Start serving on 127.0.0.1:<random port> in a background thread.
    pub fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
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
            url: format!("http://127.0.0.1:{port}"),
            state,
            server,
        }
    }

    /// Like `start`, but /signal goes over `relay`'s WebSocket when the
    /// target device is connected to it (the real server's behaviour).
    pub fn start_with_relay(relay: &crate::FakeRelay) -> Self {
        let reg = Self::start();
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
            let token = format!("tok-{u}");
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
            let token = format!("tok-{u}");
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
                            .is_some_and(|q| q.send(frame.to_string()).is_ok())
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
        _ => {
            drop(st);
            respond(req, 404, json!({"error": "not found"}))
        }
    }
}
