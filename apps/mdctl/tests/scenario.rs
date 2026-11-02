//! `mdctl` end to end: two real processes on loopback, one echoing, one
//! running a scenario script against it.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

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

/// Start an echoing device; returns it and its QUIC port.
fn echo_box() -> (Child, u16) {
    let mut child = mdctl("echo-box")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = loop {
        let line = lines.next().expect("echo-box exited").unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        if let Some(p) = v["ready"]["quic_port"].as_u64() {
            break p as u16;
        }
    };
    // Keep draining its output so it never blocks on a full pipe.
    std::thread::spawn(move || lines.for_each(drop));
    writeln!(child.stdin.as_mut().unwrap(), "echo on").unwrap();
    (child, port)
}

fn run_script(name: &str, script: &str) -> std::process::Output {
    let file = temp(name).join("scenario.mdctl");
    std::fs::write(&file, script).unwrap();
    mdctl(name).arg("--script").arg(&file).output().unwrap()
}

#[test]
fn a_scenario_gets_its_message_echoed_back() {
    let (mut echo, port) = echo_box();
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
    let _ = echo.kill();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "scenario failed:\n{stdout}");
    assert!(stdout.contains("echo: second"), "{stdout}");
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
