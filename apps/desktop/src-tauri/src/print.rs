// Print from the phone on this computer's printer (mdcore
// `features::print`): the phone's print dialog lists this computer and its
// printers, and sends the document here as a PDF.
//
//  - macOS and Linux print through CUPS: the printers from `lpstat -p -d`
//    (names, the default) and `lpoptions -p <printer> -l` (paper sizes,
//    two-sided, colour) plus `lpoptions -p <printer>` (its description);
//    a job with `lp -d <printer> -n <copies> -o sides=… -o media=… …`
//    (each one its own argument, never through a shell), then followed
//    with `lpstat -o` until it leaves the queue. Every command runs with
//    `LC_ALL=C`, so the output parsed is always the English one;
//  - Windows: no printing from the phone. Windows has no built-in way to
//    print a PDF from the command line: `Start-Process -Verb PrintTo`
//    hands the file to whatever app is registered for PDFs (often none,
//    or Edge, which doesn't print that way), so nothing would be reliable.
//    This computer doesn't say `print` there and Settings says why;
//  - "Let my phone print here" (on by default) and "Ask before printing"
//    (off by default) in Settings (`<app config dir>/prefs.json`, keys
//    `print_allow` and `print_ask`, in the settings backup). Asking shows
//    a prompt (`print-ask`); no answer in two minutes is a no;
//  - the phone hears how it goes: "waiting" while asked, "printing" once
//    CUPS has it, "done" when it left the queue, "failed" with why;
//  - the PDF is deleted as soon as CUPS has its copy (or the job is
//    refused or fails); it never stays in the downloads;
//  - the Activity timeline gets a "printed" (or "print_failed") row.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdcore::features::print::{is_paper_name, is_printer_name, PrintOptions, Printer};
use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

const KEY_ALLOW: &str = "print_allow";
const KEY_ASK: &str = "print_ask";
/// How long the user has to answer "Print this?".
const ASK_FOR: Duration = Duration::from_secs(120);
/// How long one CUPS command may take.
const CMD_TIMEOUT: Duration = Duration::from_secs(15);
/// How often, and how long, a job is followed in the queue.
const POLL_EVERY: Duration = Duration::from_secs(3);
const FOLLOW_FOR: Duration = Duration::from_secs(60 * 60);

// --- parsing CUPS' answers (pure, tested) ----------------------------------------

/// The printers in `lpstat -p -d`'s answer, and the default one.
pub fn parse_lpstat(out: &str) -> (Vec<String>, Option<String>) {
    let mut names = Vec::new();
    let mut default = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("printer ") {
            if let Some(name) = rest.split_whitespace().next() {
                if is_printer_name(name) && !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("system default destination:") {
            let name = rest.trim();
            if is_printer_name(name) {
                default = Some(name.to_string());
            }
        }
    }
    (names, default)
}

/// What a printer can do, from `lpoptions -p <printer> -l`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Caps {
    pub color: bool,
    pub duplex: bool,
    pub papers: Vec<String>,
    pub default_paper: Option<String>,
}

/// Lines like `PageSize/Media Size: Letter *A4 Legal`: the option's key
/// (before the `/`), its choices, and the one marked `*` (the default).
fn option_line(line: &str) -> Option<(&str, Vec<&str>, Option<&str>)> {
    let (head, values) = line.split_once(':')?;
    let key = head.split('/').next()?.trim();
    let mut choices = Vec::new();
    let mut default = None;
    for v in values.split_whitespace() {
        let v = match v.strip_prefix('*') {
            Some(d) => {
                default = Some(d);
                d
            }
            None => v,
        };
        choices.push(v);
    }
    Some((key, choices, default))
}

pub fn parse_lpoptions_l(out: &str) -> Caps {
    let mut caps = Caps::default();
    for line in out.lines() {
        let Some((key, choices, default)) = option_line(line) else {
            continue;
        };
        match key {
            "PageSize" | "media" => {
                for c in &choices {
                    // "Custom.WIDTHxHEIGHT" is a form, not a size.
                    if c.starts_with("Custom.") || !is_paper_name(c) {
                        continue;
                    }
                    if !caps.papers.iter().any(|p| p == c) {
                        caps.papers.push(c.to_string());
                    }
                }
                if caps.default_paper.is_none() {
                    caps.default_paper = default
                        .filter(|d| caps.papers.iter().any(|p| p == d))
                        .map(str::to_string);
                }
            }
            "Duplex" | "sides" | "JCLDuplex" | "EFDuplex" | "KMDuplex" => {
                caps.duplex |= choices.iter().any(|c| {
                    let c = c.to_ascii_lowercase();
                    c.contains("tumble") || c.starts_with("two-sided") || c == "duplex"
                });
            }
            "ColorModel" | "print-color-mode" | "ColorMode" | "cupsColorMode" => {
                caps.color |= choices.iter().any(|c| {
                    let c = c.to_ascii_lowercase();
                    !(c.contains("gray")
                        || c.contains("grey")
                        || c.contains("mono")
                        || c.contains("black")
                        || c == "kgray"
                        || c == "bw")
                });
            }
            _ => {}
        }
    }
    caps
}

/// The printer's description, from `lpoptions -p <printer>`'s
/// `printer-info='…'` (quoted, or up to the next space with `\ ` for a
/// space).
pub fn parse_printer_info(out: &str) -> Option<String> {
    let at = out.find("printer-info=")? + "printer-info=".len();
    let rest = &out[at..];
    let mut chars = rest.chars();
    let mut info = String::new();
    match rest.chars().next() {
        Some(q @ ('\'' | '"')) => {
            chars.next();
            for c in chars {
                if c == q {
                    break;
                }
                info.push(c);
            }
        }
        _ => {
            let mut escaped = false;
            for c in chars {
                if escaped {
                    info.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c.is_whitespace() {
                    break;
                } else {
                    info.push(c);
                }
            }
        }
    }
    let info = info.trim().to_string();
    (!info.is_empty()).then_some(info)
}

/// The job's id from `lp`'s answer ("request id is Hall-12 (1 file(s))").
pub fn parse_lp_job(out: &str) -> Option<String> {
    let rest = out.split("request id is ").nth(1)?;
    let id = rest.split_whitespace().next()?;
    (!id.is_empty()).then(|| id.to_string())
}

/// The job is still in `lpstat -o`'s answer (each line starts with its id).
pub fn job_queued(out: &str, job: &str) -> bool {
    out.lines()
        .any(|l| l.split_whitespace().next() == Some(job))
}

/// The arguments to `lp` for printing `file` with `opts` on `printer`
/// (what it can do: a paper it doesn't list is left to its default).
pub fn lp_args(file: &Path, opts: &PrintOptions, printer: &Printer) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = Vec::new();
    let mut push = |s: &str| a.push(s.into());
    push("-d");
    push(&opts.printer);
    push("-n");
    push(
        &opts
            .copies
            .clamp(1, mdcore::features::print::MAX_COPIES)
            .to_string(),
    );
    if !opts.title.is_empty() {
        push("-t");
        push(&opts.title);
    }
    let sides = match opts.duplex.as_str() {
        "long" if printer.duplex => "two-sided-long-edge",
        "short" if printer.duplex => "two-sided-short-edge",
        _ => "one-sided",
    };
    push("-o");
    push(&format!("sides={sides}"));
    if is_paper_name(&opts.paper) && printer.papers.iter().any(|p| p == &opts.paper) {
        push("-o");
        push(&format!("media={}", opts.paper));
    }
    if !opts.color {
        push("-o");
        push("ColorModel=Gray");
        push("-o");
        push("print-color-mode=monochrome");
    }
    if opts.orientation == "landscape" {
        push("-o");
        push("orientation-requested=4");
    }
    // The file after `--`: a name can't read as an option.
    a.push("--".into());
    a.push(file.as_os_str().to_owned());
    a
}

/// The first line of what a failed command said, for the phone.
fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = line.strip_prefix("lp:").unwrap_or(line).trim();
    line.chars().take(200).collect()
}

// --- running CUPS -------------------------------------------------------------------

/// Run `cmd args` (no shell) with `LC_ALL=C`: (it succeeded, stdout,
/// stderr). Killed after `CMD_TIMEOUT`.
fn run(cmd: &str, args: &[std::ffi::OsString]) -> Result<(bool, String, String), String> {
    let mut child = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run {cmd}: {e}"))?;
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = child.stdout.take().map(|o| read(Box::new(o)));
    let err = child.stderr.take().map(|e| read(Box::new(e)));
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() < CMD_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(30))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let err = err.and_then(|h| h.join().ok()).unwrap_or_default();
    match status {
        Some(s) => Ok((s.success(), out, err)),
        None => Err(format!("{cmd} took too long")),
    }
}

fn args(list: &[&str]) -> Vec<std::ffi::OsString> {
    list.iter().map(Into::into).collect()
}

/// This computer can print from the phone: CUPS' `lp` is there (never on
/// Windows, see the top).
pub fn supported() -> bool {
    #[cfg(windows)]
    {
        false
    }
    #[cfg(not(windows))]
    {
        let on_path = std::env::var_os("PATH").is_some_and(|p| {
            std::env::split_paths(&p).any(|d| d.join("lp").is_file() && d.join("lpstat").is_file())
        });
        on_path || Path::new("/usr/bin/lp").is_file()
    }
}

/// This computer's printers (none where printing isn't supported, or CUPS
/// doesn't answer).
pub fn list_printers() -> Vec<Printer> {
    if !supported() {
        return Vec::new();
    }
    let Ok((_, out, _)) = run("lpstat", &args(&["-p", "-d"])) else {
        return Vec::new();
    };
    let (names, default) = parse_lpstat(&out);
    names
        .into_iter()
        .take(mdcore::features::print::MAX_PRINTERS)
        .map(|name| {
            let caps = run("lpoptions", &args(&["-p", &name, "-l"]))
                .map(|(_, o, _)| parse_lpoptions_l(&o))
                .unwrap_or_default();
            let label = run("lpoptions", &args(&["-p", &name]))
                .ok()
                .and_then(|(_, o, _)| parse_printer_info(&o))
                .unwrap_or_default();
            Printer {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
                label,
                color: caps.color,
                duplex: caps.duplex,
                papers: caps.papers,
                default_paper: caps.default_paper,
            }
        })
        .collect()
}

// --- the settings ---------------------------------------------------------------------

/// "Let my phone print here" (on unless turned off) and "Ask before
/// printing" (off unless turned on).
pub struct PrintPrefs {
    kv: Option<FileKv>,
    allow: AtomicBool,
    ask: AtomicBool,
    /// Jobs waiting for an answer: id -> where it goes.
    asks: Mutex<HashMap<String, mpsc::Sender<bool>>>,
}

impl PrintPrefs {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
        PrintPrefs {
            allow: AtomicBool::new(get(KEY_ALLOW).as_deref() != Some("off")),
            ask: AtomicBool::new(get(KEY_ASK).as_deref() == Some("on")),
            kv,
            asks: Mutex::new(HashMap::new()),
        }
    }

    fn save(&self, key: &str, on: bool) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, if on { "on" } else { "off" }) {
                eprintln!("[print] couldn't save the setting: {e}");
            }
        }
    }
}

fn allow(app: &AppHandle) -> bool {
    app.try_state::<PrintPrefs>()
        .is_none_or(|p| p.allow.load(Ordering::Relaxed))
}

fn ask(app: &AppHandle) -> bool {
    app.try_state::<PrintPrefs>()
        .is_some_and(|p| p.ask.load(Ordering::Relaxed))
}

/// Tell the core whether this computer prints for your phones (at start,
/// and when the switch changes).
pub fn init(app: &AppHandle) {
    if let Some(engine) = app.try_state::<crate::Eng>() {
        engine.set_print_offered(allow(app) && supported());
    }
}

// --- the core's events ------------------------------------------------------------------

/// A phone asks for the printers: list them (off the core's thread).
pub fn on_printers_requested(app: &AppHandle, from: &str) {
    let (app, from) = (app.clone(), from.to_string());
    std::thread::spawn(move || {
        let list = if allow(&app) {
            list_printers()
        } else {
            Vec::new()
        };
        if let Some(engine) = app.try_state::<crate::Eng>() {
            if let Err(e) = engine.send_printers(&from, &list) {
                eprintln!("[print] couldn't send the printers: {e}");
            }
        }
    });
}

/// "Print <file> from <phone>?" (`print-ask`).
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct PrintAsk {
    pub id: String,
    pub node_id: String,
    pub name: String,
    pub printer: String,
    pub copies: u32,
}

/// A print job's PDF arrived: print it (off the core's thread).
pub fn on_job(app: &AppHandle, id: &str, from: &str, name: &str, path: &str, opts: &PrintOptions) {
    let (app, id, from, name, path, opts) = (
        app.clone(),
        id.to_string(),
        from.to_string(),
        name.to_string(),
        path.to_string(),
        opts.clone(),
    );
    std::thread::spawn(move || {
        let status = |state: &str, why: Option<&str>| {
            if let Some(engine) = app.try_state::<crate::Eng>() {
                let _ = engine.print_status(&id, state, why);
            }
        };
        let file = Path::new(&path);
        let shown = if opts.title.is_empty() {
            &name
        } else {
            &opts.title
        };
        let fail = |why: &str| {
            let _ = std::fs::remove_file(file);
            status("failed", Some(why));
            crate::activity::record(&app, "print_failed", Some(&from), shown, why);
        };
        if !allow(&app) || !supported() {
            return fail("printing is off on this computer");
        }
        let Some(printer) = list_printers().into_iter().find(|p| p.name == opts.printer) else {
            return fail("that printer isn't on this computer any more");
        };
        if ask(&app) {
            status("waiting", None);
            if !asked(&app, &id, &from, shown, &opts) {
                return fail("not printed: declined on the computer");
            }
        }
        let job = match run("lp", &lp_args(file, &opts, &printer)) {
            Ok((true, out, _)) => parse_lp_job(&out),
            Ok((false, _, err)) => {
                let why = first_line(&err);
                return fail(if why.is_empty() {
                    "the printer refused it"
                } else {
                    &why
                });
            }
            Err(e) => return fail(&e),
        };
        // CUPS has its own copy now.
        let _ = std::fs::remove_file(file);
        status("printing", None);
        crate::activity::record(&app, "printed", Some(&from), shown, &printer.name);
        if let Some(job) = job {
            follow(&job);
        }
        status("done", None);
    });
}

/// Wait while the job is in the queue (up to `FOLLOW_FOR`; if CUPS stops
/// answering, it's taken as printed).
fn follow(job: &str) {
    let start = Instant::now();
    while start.elapsed() < FOLLOW_FOR {
        std::thread::sleep(POLL_EVERY);
        match run("lpstat", &args(&["-o"])) {
            Ok((_, out, _)) if job_queued(&out, job) => continue,
            _ => return,
        }
    }
}

/// Ask this computer's user; true if they said yes in time.
fn asked(app: &AppHandle, id: &str, from: &str, name: &str, opts: &PrintOptions) -> bool {
    let Some(prefs) = app.try_state::<PrintPrefs>() else {
        return false;
    };
    let (tx, rx) = mpsc::channel();
    prefs
        .asks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), tx);
    crate::tray::show_window(app);
    let _ = app.emit(
        "print-ask",
        PrintAsk {
            id: id.to_string(),
            node_id: from.to_string(),
            name: name.to_string(),
            printer: opts.printer.clone(),
            copies: opts.copies,
        },
    );
    let yes = rx.recv_timeout(ASK_FOR).unwrap_or(false);
    prefs
        .asks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    // Gone from the window either way.
    let _ = app.emit("print-ask-done", id.to_string());
    yes
}

// --- Tauri commands ---------------------------------------------------------------------

/// What Settings shows.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct PrintSetting {
    /// "Let my phone print here".
    pub allow: bool,
    /// "Ask before printing".
    pub ask: bool,
    /// This computer can print from the phone (CUPS; never Windows).
    pub supported: bool,
    /// Its printers (listed only while allowed).
    pub printers: Vec<Printer>,
}

fn setting(app: &AppHandle) -> PrintSetting {
    let allow = allow(app);
    PrintSetting {
        allow,
        ask: ask(app),
        supported: supported(),
        printers: if allow { list_printers() } else { Vec::new() },
    }
}

#[tauri::command]
#[specta::specta]
pub async fn print_setting(app: AppHandle) -> PrintSetting {
    let fallback = PrintSetting {
        allow: allow(&app),
        ask: ask(&app),
        supported: false,
        printers: Vec::new(),
    };
    tauri::async_runtime::spawn_blocking(move || setting(&app))
        .await
        .unwrap_or(fallback)
}

/// Turn "Let my phone print here" on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_print_allow(on: bool, app: AppHandle) -> PrintSetting {
    if let Some(p) = app.try_state::<PrintPrefs>() {
        p.allow.store(on, Ordering::Relaxed);
        p.save(KEY_ALLOW, on);
    }
    init(&app);
    print_setting(app).await
}

/// Turn "Ask before printing" on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_print_ask(on: bool, app: AppHandle) -> PrintSetting {
    if let Some(p) = app.try_state::<PrintPrefs>() {
        p.ask.store(on, Ordering::Relaxed);
        p.save(KEY_ASK, on);
    }
    print_setting(app).await
}

/// The answer to "Print this?" for job `id`.
#[tauri::command]
#[specta::specta]
pub fn print_answer(id: String, yes: bool, app: AppHandle) {
    if let Some(p) = app.try_state::<PrintPrefs>() {
        if let Some(tx) = p.asks.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
            let _ = tx.send(yes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const LPSTAT_MAC: &str =
        "printer HP_LaserJet_Pro is idle.  enabled since Tue Oct  1 10:00:00 2024
printer Brother_HL_L2350DW_series is disabled since Mon Sep 30 09:00:00 2024 -
\tPaused
printer Canon_MG3600 now printing Canon_MG3600-41.  enabled since Tue Oct  1 11:00:00 2024
system default destination: Brother_HL_L2350DW_series
";

    #[test]
    fn lpstat_lists_printers_and_the_default() {
        let (names, default) = parse_lpstat(LPSTAT_MAC);
        assert_eq!(
            names,
            vec![
                "HP_LaserJet_Pro",
                "Brother_HL_L2350DW_series",
                "Canon_MG3600"
            ]
        );
        assert_eq!(default.as_deref(), Some("Brother_HL_L2350DW_series"));
        let (names, default) =
            parse_lpstat("printer Office is idle.\nno system default destination\n");
        assert_eq!(names, vec!["Office"]);
        assert_eq!(default, None);
        assert_eq!(
            parse_lpstat("lpstat: No destinations added.\n"),
            (vec![], None)
        );
        // A name that would read as an option is never listed.
        assert_eq!(
            parse_lpstat("printer -d is idle.\n").0,
            Vec::<String>::new()
        );
    }

    const LPOPTIONS_L: &str =
        "PageSize/Media Size: Letter Legal *A4 A5 Executive Env10 Custom.WIDTHxHEIGHT
InputSlot/Media Source: *Auto Tray1 Manual
Duplex/2-Sided Printing: *None DuplexNoTumble DuplexTumble
ColorModel/Color Mode: Gray *RGB
cupsPrintQuality/Quality: Draft *Normal High
";

    #[test]
    fn lpoptions_says_what_a_printer_can_do() {
        let caps = parse_lpoptions_l(LPOPTIONS_L);
        assert_eq!(
            caps.papers,
            vec!["Letter", "Legal", "A4", "A5", "Executive", "Env10"]
        );
        assert_eq!(caps.default_paper.as_deref(), Some("A4"));
        assert!(caps.duplex);
        assert!(caps.color);
        // A black-and-white, one-sided printer.
        let mono = parse_lpoptions_l(
            "PageSize/Page Size: *Letter A4\nColorModel/Output Mode: *Gray\nDuplex/Two-sided: *None\n",
        );
        assert!(!mono.color);
        assert!(!mono.duplex);
        assert_eq!(mono.default_paper.as_deref(), Some("Letter"));
        // Driverless (IPP Everywhere) names.
        let ipp = parse_lpoptions_l(
            "media/Media: *iso_a4_210x297mm na_letter_8.5x11in\nsides/2-Sided: *one-sided two-sided-long-edge two-sided-short-edge\nprint-color-mode/Color: monochrome *color\n",
        );
        assert_eq!(ipp.papers, vec!["iso_a4_210x297mm", "na_letter_8.5x11in"]);
        assert!(ipp.duplex && ipp.color);
        assert_eq!(parse_lpoptions_l(""), Caps::default());
    }

    #[test]
    fn the_description_comes_from_printer_info() {
        let out = "copies=1 device-uri=ipp://hp.local/ipp/print finishings=3 printer-info='HP LaserJet Pro (hall)' printer-is-shared=false printer-make-and-model='HP LaserJet Pro M404'";
        assert_eq!(
            parse_printer_info(out).as_deref(),
            Some("HP LaserJet Pro (hall)")
        );
        let out = "printer-info=Office\\ printer printer-location=desk";
        assert_eq!(parse_printer_info(out).as_deref(), Some("Office printer"));
        assert_eq!(parse_printer_info("copies=1"), None);
        assert_eq!(parse_printer_info("printer-info='' copies=1"), None);
    }

    #[test]
    fn lp_answers_with_a_job_and_the_queue_lists_it() {
        assert_eq!(
            parse_lp_job("request id is HP_LaserJet_Pro-123 (1 file(s))\n").as_deref(),
            Some("HP_LaserJet_Pro-123")
        );
        assert_eq!(parse_lp_job("lp: Error\n"), None);
        let queue = "HP_LaserJet_Pro-123 pankaj 20480 Tue Oct  1 10:00:00 2024\nHP_LaserJet_Pro-1234 pankaj 1 Tue\n";
        assert!(job_queued(queue, "HP_LaserJet_Pro-123"));
        assert!(!job_queued(queue, "HP_LaserJet_Pro-12"));
        assert!(!job_queued("", "HP_LaserJet_Pro-123"));
    }

    fn hall() -> Printer {
        Printer {
            name: "Hall".into(),
            duplex: true,
            color: true,
            papers: vec!["A4".into(), "Letter".into()],
            ..Default::default()
        }
    }

    fn strs(a: &[std::ffi::OsString]) -> Vec<String> {
        a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn lp_gets_one_argument_per_option() {
        let opts = PrintOptions {
            printer: "Hall".into(),
            copies: 3,
            color: false,
            duplex: "long".into(),
            paper: "A4".into(),
            orientation: "landscape".into(),
            title: "Boarding pass; rm -rf ~".into(),
        };
        let file = PathBuf::from("/tmp/dl/.mydomain-partial/print/-pass.pdf");
        assert_eq!(
            strs(&lp_args(&file, &opts, &hall())),
            vec![
                "-d",
                "Hall",
                "-n",
                "3",
                "-t",
                "Boarding pass; rm -rf ~",
                "-o",
                "sides=two-sided-long-edge",
                "-o",
                "media=A4",
                "-o",
                "ColorModel=Gray",
                "-o",
                "print-color-mode=monochrome",
                "-o",
                "orientation-requested=4",
                "--",
                "/tmp/dl/.mydomain-partial/print/-pass.pdf",
            ]
        );
    }

    #[test]
    fn lp_leaves_out_what_the_printer_cant_do() {
        let opts = PrintOptions {
            printer: "Hall".into(),
            copies: 500,
            duplex: "short".into(),
            paper: "Tabloid".into(),
            ..Default::default()
        };
        let one_sided = Printer {
            duplex: false,
            ..hall()
        };
        let a = strs(&lp_args(Path::new("a.pdf"), &opts, &one_sided));
        assert_eq!(
            a,
            vec![
                "-d",
                "Hall",
                "-n",
                "99",
                "-o",
                "sides=one-sided",
                "--",
                "a.pdf"
            ]
        );
        let a = strs(&lp_args(Path::new("a.pdf"), &opts, &hall()));
        assert!(a.contains(&"sides=two-sided-short-edge".to_string()));
        assert!(
            !a.iter().any(|s| s.starts_with("media=")),
            "not one of its papers"
        );
        assert!(
            !a.iter().any(|s| s.contains("Gray")),
            "colour unless told otherwise"
        );
    }

    #[test]
    fn a_failure_says_its_first_line() {
        assert_eq!(
            first_line("\nlp: The printer or class does not exist.\nmore"),
            "The printer or class does not exist."
        );
        assert_eq!(first_line(""), "");
    }
}
