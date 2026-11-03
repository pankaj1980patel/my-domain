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
//! For simulated networks (scripts/lab/nat-sim.sh): `--stun a:3478,b:3478`
//! replaces the public STUN servers, `--relay URL` sets the relay WebSocket,
//! `--signal-channel` keeps it open so signals arrive without FCM, and
//! `--no-lan` turns off mDNS and router port mapping. `--ble` also looks
//! for phones over Bluetooth LE (`md-ble`).
//!
//! `--server/--user/--pass/--key` default to `MD_DEV_SERVER`, `MD_DEV_USER`,
//! `MD_DEV_PASS`, `MD_DEV_KEY` (scripts/lab/test-creds.env).
//!
//! Commands: `identity`, `peers`, `links`, `refresh`, `connect <node>`,
//! `send <node> <text...>`, `status <node>`, `ring <node>`,
//! `open <node> <url>`, `action <node> <name> [json]`,
//! `subscribe <node> <json pkgs>`, `notifact <node> <key> <index> [text]`,
//! `sendfile <node> <path>`, `downloads <dir>`,
//! `share <name> <dir>` (let other devices browse `dir` as `name`),
//! `ls <node> [root] [path] [offset]`, `get <node> <root> <path>`,
//! `text <node> clipboard|notify <text...>`,
//! `add <name> <ip> <quic_port>`, `forget` (remove
//! this device from the server), `quit`.
//!
//! For scripted tests (`--script FILE`, or the same lines on stdin):
//! - `wait <event> [field=value ...] [timeout=SECS]`: block until a matching
//!   event arrives (one that came before the wait counts too), else fail.
//!   `node=` matches the event's device; values may be "quoted".
//! - `echo on|off`: answer every message with `echo: <text>`, so the other
//!   end of a scenario can check a round trip.
//! - `sleep <secs>`.
//! - `@name` works wherever a node id goes (a device's name, any case).
//!
//! With `--script`, lines starting with `#` are comments, `${VAR}` is
//! replaced from the environment, and the first failed command ends the run
//! with exit code 1.

use std::io::{BufRead, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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

/// Every event so far, for `wait`. `next` is the first one a `wait` hasn't
/// consumed yet.
#[derive(Default)]
struct Events {
    seen: Mutex<(Vec<(String, Value)>, usize)>,
    arrived: Condvar,
}

impl Events {
    /// Block until an unconsumed event satisfies `matches`; consume it and
    /// everything before it.
    fn wait(&self, timeout: Duration, matches: impl Fn(&str, &Value) -> bool) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let (list, next) = &mut *g;
            if let Some(i) = (*next..list.len()).find(|&i| matches(&list[i].0, &list[i].1)) {
                *next = i + 1;
                return Some(list[i].1.clone());
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            g = self
                .arrived
                .wait_timeout(g, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

struct PrintSink {
    out: Out,
    events: Arc<Events>,
    /// `echo on`: incoming messages to answer (sent from the main loop, not
    /// from the core's thread).
    echo: Arc<AtomicBool>,
    replies: std::sync::mpsc::Sender<(String, String)>,
}

impl EventSink for PrintSink {
    fn emit(&self, ev: CoreEvent) {
        let (name, data) = (ev.name(), ev.ui_payload());
        self.out.line(json!({ "event": name, "data": data }));
        if let CoreEvent::MessageReceived(m) = &ev {
            if let (true, true, Some(node)) = (self.echo.load(Ordering::Relaxed), m.ok, &m.node_id)
            {
                if !m.text.starts_with("echo: ") {
                    let _ = self
                        .replies
                        .send((node.clone(), format!("echo: {}", m.text)));
                }
            }
        }
        let mut g = self.events.seen.lock().unwrap_or_else(|e| e.into_inner());
        g.0.push((name.to_string(), data));
        drop(g);
        self.events.arrived.notify_all();
    }
}

/// Split a command line into words; "double quotes" group words (for
/// `field="a value"` and texts in `wait`).
fn words(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for (i, c) in line.char_indices() {
        // A JSON argument (`action <node> flashlight {"on":true}`) is the
        // rest of the line, quotes and all.
        if cur.is_empty() && !quoted && matches!(c, '{' | '[') {
            out.push(line[i..].trim_end().to_string());
            return out;
        }
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Replace each `${VAR}` with the environment variable (empty if unset).
fn expand_env(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let var = &rest[start + 2..start + 2 + end];
                out.push_str(&std::env::var(var).unwrap_or_default());
                rest = &rest[start + 2 + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `@name` -> that device's node id (exact name, any case); anything else
/// is already a node id.
fn resolve(engine: &Engine<CliPlatform>, who: &str) -> Result<String, String> {
    let Some(name) = who.strip_prefix('@') else {
        return Ok(who.to_string());
    };
    engine
        .get_peers()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| p.node_id)
        .ok_or(format!("no device named {name}"))
}

/// Does event `data` have `field` equal to `want`? `node` matches the
/// event's device (`node_id`, or the bare id some events carry).
fn field_is(data: &Value, field: &str, want: &str) -> bool {
    let got = match (field, data) {
        ("node", Value::String(s)) => Some(s.clone()),
        ("node", v) => v["node_id"].as_str().map(str::to_string),
        (f, v) => match &v[f] {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        },
    };
    got.as_deref() == Some(want)
}

#[derive(Default)]
struct Args {
    name: String,
    data: PathBuf,
    script: Option<PathBuf>,
    server: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    key: Option<String>,
    offline: Option<String>,
    stun: Option<Vec<String>>,
    relay: Option<String>,
    signal_channel: bool,
    no_lan: bool,
    ble: bool,
}

impl Args {
    fn config(&self) -> mdcore::config::CoreConfig {
        let mut c = mdcore::config::CoreConfig::default();
        if let Some(stun) = &self.stun {
            c.stun_servers = stun.clone();
        }
        c.relay_url = self.relay.clone();
        c.signal_channel = self.signal_channel;
        if self.no_lan {
            c.mdns = false;
            c.port_mapper = None;
        }
        c
    }
}

fn parse_args() -> Result<Args, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut a = Args {
        name: "mdctl".into(),
        data: PathBuf::from(".mdctl"),
        script: None,
        server: env("MD_DEV_SERVER"),
        user: env("MD_DEV_USER"),
        pass: env("MD_DEV_PASS"),
        key: env("MD_DEV_KEY"),
        offline: None,
        ..Args::default()
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
            "--script" => a.script = Some(PathBuf::from(val()?)),
            "--stun" => a.stun = Some(val()?.split(',').map(String::from).collect()),
            "--relay" => a.relay = Some(val()?),
            "--signal-channel" => a.signal_channel = true,
            "--no-lan" => a.no_lan = true,
            "--ble" => a.ble = true,
            "-h" | "--help" => {
                return Err("usage: mdctl [--name N] [--data DIR] [--script FILE] \
                    [--server URL --user U --pass P | --offline USER] --key KEY \
                    [--stun LIST] [--relay URL] [--signal-channel] [--no-lan] [--ble]"
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

struct Ctl<'a> {
    engine: &'a Engine<CliPlatform>,
    out: &'a Out,
    events: &'a Events,
    echo: &'a AtomicBool,
}

/// Run one command line. `Ok(false)` on `quit`; `Err` when the command failed.
fn run(ctl: &Ctl, line: &str) -> Result<bool, ()> {
    let (engine, out) = (ctl.engine, ctl.out);
    let all = words(line);
    let mut words = all.iter().map(String::as_str);
    let Some(cmd) = words.next() else {
        return Ok(true);
    };
    let arg = |w: Option<&str>| w.map(str::to_string).ok_or("missing argument".to_string());
    let result: Result<Value, String> = (|| match cmd {
        "wait" => {
            let event = arg(words.next())?;
            let mut timeout = Duration::from_secs(30);
            let mut want = Vec::new();
            for w in words.by_ref() {
                let (k, v) = w
                    .split_once('=')
                    .ok_or(format!("expected field=value, got {w}"))?;
                match k {
                    "timeout" => {
                        timeout = Duration::from_secs_f64(v.parse().map_err(|_| "bad timeout")?)
                    }
                    "node" => want.push((k.to_string(), resolve(engine, v)?)),
                    _ => want.push((k.to_string(), v.to_string())),
                }
            }
            ctl.events
                .wait(timeout, |name, data| {
                    name == event && want.iter().all(|(k, v)| field_is(data, k, v))
                })
                .ok_or(format!(
                    "timed out after {}s waiting for {event}",
                    timeout.as_secs_f64()
                ))
        }
        "echo" => {
            let on = matches!(words.next(), Some("on"));
            ctl.echo.store(on, Ordering::Relaxed);
            Ok(json!(if on { "echoing" } else { "not echoing" }))
        }
        "sleep" => {
            let secs: f64 = arg(words.next())?.parse().map_err(|_| "bad number")?;
            std::thread::sleep(Duration::from_secs_f64(secs));
            Ok(Value::Null)
        }
        "quit" | "exit" => Ok(Value::Null),
        "identity" => Ok(json!(engine.identity())),
        "peers" => Ok(json!(engine.get_peers())),
        "links" => Ok(json!(engine.link_stats())),
        "refresh" => {
            engine.refresh_from_server()?;
            Ok(json!(engine.get_peers()))
        }
        "connect" => {
            engine.connect(&resolve(engine, &arg(words.next())?)?)?;
            Ok(json!("connecting"))
        }
        "send" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let text = words.collect::<Vec<_>>().join(" ");
            Ok(json!({ "via": engine.send(&to, &text)? }))
        }
        "status" => {
            let to = resolve(engine, &arg(words.next())?)?;
            Ok(json!({ "via": engine.request_status(&to)? }))
        }
        "ring" => {
            let to = resolve(engine, &arg(words.next())?)?;
            Ok(json!({ "via": engine.ring(&to)? }))
        }
        "open" => {
            let to = resolve(engine, &arg(words.next())?)?;
            Ok(json!({ "via": engine.open_url(&to, &arg(words.next())?)? }))
        }
        "action" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let name = arg(words.next())?;
            let args = words.collect::<Vec<_>>().join(" ");
            Ok(json!({ "via": engine.run_action(&to, &name, &args)? }))
        }
        "subscribe" => {
            let to = resolve(engine, &arg(words.next())?)?;
            engine.subscribe_apps(&to, &arg(words.next())?)?;
            Ok(json!("subscribed"))
        }
        "notifact" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let key = arg(words.next())?;
            let index = arg(words.next())?
                .parse::<i64>()
                .map_err(|_| "bad index".to_string())?;
            let text = words.collect::<Vec<_>>().join(" ");
            let text = (!text.is_empty()).then_some(text);
            Ok(json!({ "via": engine.notification_action(&to, &key, index, text.as_deref())? }))
        }
        "sendfile" => {
            let to = resolve(engine, &arg(words.next())?)?;
            Ok(json!({ "id": engine.send_file(&to, &arg(words.next())?)? }))
        }
        "share" => {
            let name = arg(words.next())?;
            let path = arg(words.next())?;
            let mut folders = engine.shared_folders();
            folders.retain(|f| f.name != name);
            folders.push(mdcore::model::SharedFolder { name, path });
            engine.set_shared_folders(&folders)?;
            Ok(json!(folders))
        }
        "ls" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let root = words.next().unwrap_or_default();
            let path = words.next().unwrap_or_default();
            let offset = match words.next() {
                Some(o) => o.parse::<u64>().map_err(|_| "bad offset".to_string())?,
                None => 0,
            };
            Ok(json!({ "req": engine.list_folder(&to, root, path, offset)? }))
        }
        "get" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let root = arg(words.next())?;
            Ok(json!({ "id": engine.download_file(&to, &root, &arg(words.next())?)? }))
        }
        "text" => {
            let to = resolve(engine, &arg(words.next())?)?;
            let mode = arg(words.next())?;
            let text = words.collect::<Vec<_>>().join(" ");
            Ok(json!({ "via": engine.send_text(&to, &text, &mode)? }))
        }
        "downloads" => {
            let dir = arg(words.next())?;
            engine.set_download_dir(&dir);
            Ok(json!(dir))
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
        Ok(v) => {
            out.line(json!({ "cmd": cmd, "ok": v }));
            Ok(!matches!(cmd, "quit" | "exit"))
        }
        Err(e) => {
            out.line(json!({ "cmd": cmd, "error": e }));
            Err(())
        }
    }
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
    let events = Arc::new(Events::default());
    let echo = Arc::new(AtomicBool::new(false));
    let (reply_tx, replies) = std::sync::mpsc::channel();
    let sink = PrintSink {
        out: out.clone(),
        events: events.clone(),
        echo: echo.clone(),
        replies: reply_tx,
    };
    let platform = CliPlatform::new(args.name.clone(), args.data.clone());
    let engine = match Engine::start_with_config(platform, Arc::new(sink), args.config()) {
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
    let engine = Arc::new(engine);
    if args.ble {
        md_ble::central::start(engine.clone());
    }
    // `echo on`: answer from here, off the core's event thread.
    {
        let engine = engine.clone();
        std::thread::spawn(move || {
            for (node, text) in replies {
                // A device that just signed in isn't listed yet; the core
                // fetches the list when it reaches us.
                for _ in 0..50 {
                    if engine.send(&node, &text).is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
    let ctl = Ctl {
        engine: &engine,
        out: &out,
        events: &events,
        echo: &echo,
    };
    let mut failed = false;
    if let Some(path) = &args.script {
        let script = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("mdctl: can't read {}: {e}", path.display());
                std::process::exit(2);
            }
        };
        for line in script.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match run(&ctl, &expand_env(line)) {
                Ok(true) => {}
                Ok(false) => break,
                Err(()) => {
                    failed = true;
                    break;
                }
            }
        }
    } else {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if run(&ctl, &line) == Ok(false) {
                break;
            }
        }
    }
    // Sends are queued; dropping the engine closes its links at once. Give
    // the last messages a moment to leave before exiting.
    std::thread::sleep(Duration::from_millis(500));
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_argument_is_kept_verbatim() {
        assert_eq!(
            words(r#"action @phone flashlight {"on": true}"#),
            ["action", "@phone", "flashlight", r#"{"on": true}"#]
        );
    }

    #[test]
    fn words_keep_quoted_values_together() {
        assert_eq!(
            words(r#"wait message-received text="echo: hi there" timeout=5"#),
            [
                "wait",
                "message-received",
                "text=echo: hi there",
                "timeout=5"
            ]
        );
    }

    #[test]
    fn env_vars_are_expanded() {
        std::env::set_var("MDCTL_TEST_PEER", "phone");
        assert_eq!(expand_env("send @${MDCTL_TEST_PEER} hi"), "send @phone hi");
        assert_eq!(expand_env("x ${MDCTL_TEST_UNSET_VAR}y"), "x y");
        assert_eq!(expand_env("no vars"), "no vars");
    }
}
