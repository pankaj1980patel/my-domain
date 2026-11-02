//! `mdctl`: my-domain without a UI.
//!
//! Runs one device (its own identity, stored in `--data`), logs in, then
//! reads commands from stdin and prints everything as JSON lines on stdout:
//! every core event (`{"event": "...", "data": {...}}`) and the result of
//! every command (`{"ok": ...}` or `{"error": "..."}`). Scripts and the
//! device lab drive it by writing commands and reading lines.
//!
//! ```text
//! mdctl --name lab-box --data ~/.mdctl \
//!       --server https://... --user U --pass P --key K
//! mdctl --name lab-box --offline U --key K   # LAN only, no server
//! ```
//! `--server/--user/--pass/--key` default to `MD_DEV_SERVER`, `MD_DEV_USER`,
//! `MD_DEV_PASS`, `MD_DEV_KEY` (scripts/lab/test-creds.env).
//!
//! Commands: `identity`, `peers`, `links`, `refresh`, `connect <node>`,
//! `send <node> <text...>`, `add <name> <ip> <quic_port>`, `forget` (remove
//! this device from the server), `quit`.

use std::io::{BufRead, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use mdcore::engine::Engine;
use mdcore::events::{CoreEvent, EventSink};
use mdcore::kv::FileKv;
use mdcore::platform::{IfaceMode, Platform};
use serde_json::{json, Value};

/// A device that lives in a data directory: its id is created once and kept
/// there, with the session and the peer cache.
struct CliPlatform {
    name: String,
    kv: FileKv,
}

impl CliPlatform {
    fn new(name: String, dir: PathBuf) -> Self {
        CliPlatform {
            name,
            kv: FileKv::in_dir(dir),
        }
    }
}

impl Platform for CliPlatform {
    fn device_id(&self) -> String {
        if let Some(id) = self.kv.get("device_id") {
            return id;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let _ = self.kv.set("device_id", &id);
        id
    }
    fn device_name(&self) -> String {
        self.name.clone()
    }
    fn platform_kind(&self) -> &'static str {
        "cli"
    }
    fn iface_mode(&self) -> IfaceMode {
        let v4_addrs = if_addrs::get_if_addrs()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| !i.is_loopback())
            .filter_map(|i| match i.ip() {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            })
            .collect();
        IfaceMode::All { v4_addrs }
    }
    fn kv_get(&self, key: &str) -> Option<String> {
        self.kv.get(key)
    }
    fn kv_set(&self, key: &str, value: &str) {
        if let Err(e) = self.kv.set(key, value) {
            eprintln!("mdctl: failed to save {key}: {e}");
        }
    }
}

/// One JSON object per line on stdout; events and results never interleave
/// mid-line.
#[derive(Clone)]
struct Out(Arc<Mutex<std::io::Stdout>>);

impl Out {
    fn line(&self, v: Value) {
        let mut out = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }
}

struct PrintSink(Out);

impl EventSink for PrintSink {
    fn emit(&self, ev: CoreEvent) {
        self.0
            .line(json!({ "event": ev.name(), "data": ev.ui_payload() }));
    }
}

struct Args {
    name: String,
    data: PathBuf,
    server: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    key: Option<String>,
    offline: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        name: "mdctl".into(),
        data: PathBuf::from(".mdctl"),
        server: env("MD_DEV_SERVER"),
        user: env("MD_DEV_USER"),
        pass: env("MD_DEV_PASS"),
        key: env("MD_DEV_KEY"),
        offline: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--name" => a.name = val()?,
            "--data" => a.data = PathBuf::from(val()?),
            "--server" => a.server = Some(val()?),
            "--user" => a.user = Some(val()?),
            "--pass" => a.pass = Some(val()?),
            "--key" => a.key = Some(val()?),
            "--offline" => a.offline = Some(val()?),
            "-h" | "--help" => {
                return Err("usage: mdctl [--name N] [--data DIR] \
                    [--server URL --user U --pass P | --offline USER] --key KEY"
                    .into())
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn log_in(engine: &Engine<CliPlatform>, a: &Args) -> mdcore::Result<()> {
    let key = a
        .key
        .as_deref()
        .ok_or("--key (or MD_DEV_KEY) is required")?;
    if let Some(user) = &a.offline {
        return engine.dev_login(user, key);
    }
    let (Some(server), Some(user), Some(pass)) = (&a.server, &a.user, &a.pass) else {
        return Err("give --server --user --pass (or MD_DEV_*), or --offline USER".into());
    };
    engine.auth_login(server, user, pass)?;
    engine.set_encryption_key(key)?;
    engine.refresh_from_server()
}

/// Run one command line; returns false on `quit`.
fn run(engine: &Engine<CliPlatform>, out: &Out, line: &str) -> bool {
    let mut words = line.split_whitespace();
    let Some(cmd) = words.next() else {
        return true;
    };
    let arg = |w: Option<&str>| w.map(str::to_string).ok_or("missing argument".to_string());
    let result: Result<Value, String> = (|| match cmd {
        "quit" | "exit" => Ok(Value::Null),
        "identity" => Ok(json!(engine.identity())),
        "peers" => Ok(json!(engine.get_peers())),
        "links" => Ok(json!(engine.link_stats())),
        "refresh" => {
            engine.refresh_from_server()?;
            Ok(json!(engine.get_peers()))
        }
        "connect" => {
            engine.connect(&arg(words.next())?)?;
            Ok(json!("connecting"))
        }
        "send" => {
            let to = arg(words.next())?;
            let text = words.collect::<Vec<_>>().join(" ");
            Ok(json!({ "via": engine.send(&to, &text)? }))
        }
        "forget" => {
            engine.unregister_from_server()?;
            Ok(json!("removed from the server"))
        }
        "add" => {
            let name = arg(words.next())?;
            let ip = arg(words.next())?;
            let port = arg(words.next())?
                .parse::<u16>()
                .map_err(|_| "bad port".to_string())?;
            engine.add_manual_peer(&name, &ip, port)?;
            Ok(json!(format!("manual:{ip}")))
        }
        other => Err(format!("unknown command {other}")),
    })();
    match result {
        Ok(v) => out.line(json!({ "cmd": cmd, "ok": v })),
        Err(e) => out.line(json!({ "cmd": cmd, "error": e })),
    }
    !matches!(cmd, "quit" | "exit")
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let out = Out(Arc::new(Mutex::new(std::io::stdout())));
    let platform = CliPlatform::new(args.name.clone(), args.data.clone());
    let engine = match Engine::start(platform, Arc::new(PrintSink(out.clone()))) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("mdctl: failed to start: {e}");
            std::process::exit(1);
        }
    };
    match log_in(&engine, &args) {
        Ok(()) => out.line(json!({ "ready": engine.identity() })),
        Err(e) => {
            eprintln!("mdctl: login failed: {e}");
            std::process::exit(1);
        }
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !run(&engine, &out, &line) {
            break;
        }
    }
    // Sends are queued; dropping the engine closes its links at once. Give
    // the last messages a moment to leave before exiting.
    std::thread::sleep(std::time::Duration::from_millis(500));
}
