//! `mdctl` end to end: two real processes on loopback, one echoing, one
//! running a scenario script against it.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MDCTL: &str = env!("CARGO_BIN_EXE_mdctl");

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mdctl-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Offline device (same user + key as every other test device), so no
/// server is needed.
fn mdctl(name: &str) -> Command {
    let mut c = Command::new(MDCTL);
    c.args([
        "--name",
        name,
        "--offline",
        "scenario-user",
        "--key",
        "scenario key",
    ])
    .arg("--data")
    .arg(temp(name))
    .env_remove("MD_DEV_SERVER");
    c
}

/// A device the test drives over stdin, with its output lines (as JSON)
/// arriving on a channel.
struct Device {
    child: Child,
    port: u16,
    lines: Receiver<serde_json::Value>,
}

impl Device {
    /// Start device `name` and wait until it's ready.
    fn start(name: &str) -> Device {
        let mut child = mdctl(name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let port = loop {
            let line = lines.next().expect("device exited").unwrap();
            let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            if let Some(p) = v["ready"]["quic_port"].as_u64() {
                break p as u16;
            }
        };
        // Keep draining its output so it never blocks on a full pipe.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in lines.map_while(Result::ok) {
                let _ = tx.send(serde_json::from_str(&line).unwrap_or_default());
            }
        });
        Device {
            child,
            port,
            lines: rx,
        }
    }

    fn cmd(&mut self, line: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{line}").unwrap();
    }

    /// The data of the first `event` it prints within `timeout`.
    fn next_event(&self, event: &str, timeout: Duration) -> Option<serde_json::Value> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let v = self.lines.recv_timeout(left).ok()?;
            if v["event"] == event {
                return Some(v["data"].clone());
            }
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Start an echoing device.
fn echo_box() -> Device {
    let mut d = Device::start("echo-box");
    d.cmd("echo on");
    d
}

fn run_script(name: &str, script: &str) -> std::process::Output {
    let file = temp(name).join("scenario.mdctl");
    std::fs::write(&file, script).unwrap();
    mdctl(name).arg("--script").arg(&file).output().unwrap()
}

#[test]
fn a_scenario_gets_its_message_echoed_back() {
    let echo = echo_box();
    let port = echo.port;
    let out = run_script(
        "scenario-sender",
        &format!(
            "# Reach the echo box by address, then check the round trip.\n\
             add echo-box 127.0.0.1 {port}\n\
             send manual:127.0.0.1 hello there\n\
             wait message-received text=\"echo: hello there\" timeout=20\n\
             send @echo-box second\n\
             wait message-received text=\"echo: second\" timeout=20\n"
        ),
    );
    drop(echo);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "scenario failed:\n{stdout}");
    assert!(stdout.contains("echo: second"), "{stdout}");
}

#[test]
fn a_file_sent_by_script_arrives_whole() {
    let mut inbox = Device::start("file-box");
    let downloads = temp("file-box-downloads");
    inbox.cmd(&format!("downloads {}", downloads.display()));
    let src = temp("file-sender-src").join("note.txt");
    let content = "my-domain file transfer\n".repeat(2000);
    std::fs::write(&src, &content).unwrap();
    let out = run_script(
        "file-sender",
        &format!(
            "add file-box 127.0.0.1 {port}\n\
             send manual:127.0.0.1 hello\n\
             wait message-delivered timeout=20\n\
             sendfile @file-box {src}\n\
             wait file-sent name=note.txt timeout=30\n",
            port = inbox.port,
            src = src.display()
        ),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "scenario failed:\n{stdout}");
    let got = inbox
        .next_event("file-received", Duration::from_secs(10))
        .expect("no file-received on the receiving side");
    assert_eq!(got["name"], "note.txt", "{got}");
    assert_eq!(got["size"].as_u64(), Some(content.len() as u64), "{got}");
    let path = got["path"].as_str().unwrap();
    assert!(
        std::path::Path::new(path).starts_with(&downloads),
        "saved outside the downloads dir: {path}"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), content);
}

#[test]
fn an_action_reaches_the_other_device_with_its_args() {
    let inbox = Device::start("action-box");
    let out = run_script(
        "action-sender",
        &format!(
            "add action-box 127.0.0.1 {port}\n\
             send manual:127.0.0.1 hello\n\
             wait message-delivered timeout=20\n\
             action @action-box volume {{\"step\":1}}\n",
            port = inbox.port
        ),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "scenario failed:\n{stdout}");
    let got = inbox
        .next_event("action-event", Duration::from_secs(10))
        .expect("no action-event on the receiving side");
    assert_eq!(got["name"], "volume", "{got}");
    assert_eq!(got["args"]["step"], 1, "{got}");
}

#[test]
fn a_failed_wait_ends_the_script_with_exit_code_1() {
    let out = run_script(
        "scenario-timeout",
        "wait message-received text=\"never sent\" timeout=0.5\nidentity\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("timed out"), "{stdout}");
    assert!(
        !stdout.contains("\"cmd\":\"identity\""),
        "stopped at the failure"
    );
}
