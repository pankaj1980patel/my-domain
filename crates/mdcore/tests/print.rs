//! Print from the phone on a computer's printer (features::print): only a
//! computer that says `print` is asked for printers or sent a job, the
//! printers come back checked, the job's PDF arrives as a print job (never
//! a download), its state goes back to the phone that sent it, and an app
//! from before never gets a print message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::features::print::{PrintOptions, Printer};
use mdcore::{CoreConfig, CoreEvent};

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-print-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_supported(a: &TestNode, b: &TestNode, on: bool) {
    let deadline = Instant::now() + WAIT;
    while a.engine.print_supported(&b.node_id()) != on {
        assert!(Instant::now() < deadline, "never heard print = {on}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn no_print_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("print")
    )));
}

fn options(printer: &str) -> PrintOptions {
    PrintOptions {
        printer: printer.into(),
        copies: 2,
        color: false,
        duplex: "long".into(),
        paper: "A4".into(),
        orientation: "portrait".into(),
        title: "Boarding pass".into(),
    }
}

#[test]
fn a_phone_prints_on_a_computer_that_offers_it() {
    let phone = TestNode::start("print-phone");
    let pc = TestNode::start("print-pc");
    TestNode::pair(&phone, &pc);
    let dl = temp_dir("pc-dl");
    pc.engine.set_download_dir(dl.to_str().unwrap());
    link(&phone, &pc, "hi");

    // Not offered yet: nothing goes.
    assert!(!phone.engine.print_supported(&pc.node_id()));
    assert!(phone.engine.request_printers(&pc.node_id()).is_err());
    let src = temp_dir("phone-src");
    let pdf = src.join("pass.pdf");
    std::fs::write(&pdf, b"%PDF-1.4 a boarding pass").unwrap();
    assert!(phone
        .engine
        .print_file(&pc.node_id(), pdf.to_str().unwrap(), &options("Hall"))
        .is_err());

    // The computer turns it on: the phone hears at once.
    pc.engine.set_print_offered(true);
    assert!(pc.engine.print_offered());
    wait_supported(&phone, &pc, true);
    assert!(
        !pc.engine.print_supported(&phone.node_id()),
        "a phone doesn't print"
    );

    // The printers, checked on the way.
    phone.engine.request_printers(&pc.node_id()).unwrap();
    let asker = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::PrintersRequested { from } => Some(from.clone()),
            _ => None,
        })
        .expect("the computer was asked");
    assert_eq!(asker, phone.node_id());
    let hall = Printer {
        name: "Hall".into(),
        label: "Hall printer".into(),
        is_default: true,
        color: true,
        duplex: true,
        papers: vec!["A4".into(), "Letter".into()],
        default_paper: Some("A4".into()),
    };
    let bad = Printer {
        name: "-o evil".into(),
        ..Default::default()
    };
    pc.engine
        .send_printers(&asker, &[hall.clone(), bad])
        .unwrap();
    let (on, printers) = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Printers { on, printers, .. } => Some((*on, printers.clone())),
            _ => None,
        })
        .expect("the printers came");
    assert!(on);
    assert_eq!(printers, vec![hall]);

    // The job: a print job on the computer, not a download.
    let id = phone
        .engine
        .print_file(&pc.node_id(), pdf.to_str().unwrap(), &options("Hall"))
        .unwrap();
    let (path, opts, from) = pc
        .sink
        .wait_until(Duration::from_secs(30), |e| match e {
            CoreEvent::PrintJob {
                id: got,
                path,
                options,
                from,
                ..
            } if *got == id => Some((path.clone(), options.clone(), from.clone())),
            _ => None,
        })
        .expect("the job arrived");
    assert_eq!(from, phone.node_id());
    assert_eq!(opts, options("Hall"));
    let path = PathBuf::from(path);
    assert!(path.starts_with(dl.join(".mydomain-partial").join("print")));
    assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4 a boarding pass");
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::FileReceived { .. })));
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileSent { id: got, .. } if *got == id => Some(()),
            _ => None,
        })
        .expect("the phone hears it was sent");

    // Its state goes back to the phone.
    assert!(pc.engine.print_status(&id, "sideways", None).is_err());
    assert!(pc.engine.print_status("not-a-job", "done", None).is_err());
    pc.engine.print_status(&id, "printing", None).unwrap();
    pc.engine
        .print_status(&id, "failed", Some("out of paper"))
        .unwrap();
    let states: Vec<(String, Option<String>)> = {
        let deadline = Instant::now() + WAIT;
        loop {
            let s: Vec<_> = phone
                .sink
                .all()
                .iter()
                .filter_map(|e| match e {
                    CoreEvent::PrintStatus {
                        id: got,
                        state,
                        reason,
                        ..
                    } if *got == id => Some((state.clone(), reason.clone())),
                    _ => None,
                })
                .collect();
            if s.len() >= 2 || Instant::now() > deadline {
                break s;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert_eq!(
        states,
        vec![
            ("printing".to_string(), None),
            ("failed".to_string(), Some("out of paper".to_string()))
        ]
    );
    // Ended: no more states for it.
    assert!(pc.engine.print_status(&id, "done", None).is_err());

    // Turned off: the phone hears, and nothing more goes.
    pc.engine.set_print_offered(false);
    wait_supported(&phone, &pc, false);
    assert!(phone.engine.request_printers(&pc.node_id()).is_err());
    no_print_as_chat(&pc);
    no_print_as_chat(&phone);
}

#[test]
fn an_app_from_before_never_gets_a_print_message() {
    let phone = TestNode::start("print-new");
    let old = TestNode::start_full(
        MockPlatform::new("print-old"),
        CoreConfig {
            print: false,
            ..test_config()
        },
    );
    TestNode::pair(&phone, &old);
    // Even told to, an app without printing never offers it.
    old.engine.set_print_offered(true);
    assert!(!old.engine.print_offered());
    link(&phone, &old, "hello");
    phone.engine.request_status(&old.node_id()).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("print").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without print");
    assert!(!phone.engine.print_supported(&old.node_id()));
    assert!(phone.engine.request_printers(&old.node_id()).is_err());
    std::thread::sleep(Duration::from_millis(300));
    no_print_as_chat(&old);
}
