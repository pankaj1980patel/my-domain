//! Print from the phone on a computer's printer. The phone's print dialog
//! (an Android print service) lists your computers that print for it, and
//! each one's printers; the document goes to the computer as a PDF, as an
//! ordinary file transfer tagged as a print job, and the computer says how
//! it went.
//!
//!   print_printers_request { from }                 phone → computer
//!   print_printers { from, on, printers: [...] }    computer → phone
//!   file_offer { ..., print: {printer, copies,      phone → computer (the
//!                color, duplex, paper,             usual transfer, see
//!                orientation, title} }             `features::files`)
//!   print_status { from, id, state, reason? }       computer → phone
//!
//! - A computer says `print: 1` in its status and `addr_update` while it
//!   offers printing (`Engine::set_print_offered`: the user's "Let my phone
//!   print here" is on and the computer can print). The phone never sends
//!   a print message, nor a print job, to a device that didn't say it (an
//!   older app would save the PDF as a plain download, and show the
//!   request as chat). What each device said is kept across restarts.
//! - The computer only ever answers a device that asked: the printers to
//!   the one that asked for them, a job's state to the one that sent it.
//! - The PDF is received into the partial folder (`.mydomain-partial/
//!   print`), never the downloads, and handed to the host as `PrintJob`
//!   (not `FileReceived`); the host prints it, deletes it and reports
//!   `print_status` ("waiting" while its user is asked, "printing" once the
//!   printer has it, "done", or "failed" with a reason).
//! - The job's id is its transfer's id. Print jobs are not resumed after
//!   the phone restarts (the print dialog's job is gone by then).
//! - Every field is checked on both ends: a printer's name is what the
//!   computer listed (the host checks it's still there before printing),
//!   copies 1 to `MAX_COPIES`, duplex "none"/"long"/"short", a paper name
//!   of letters, digits and `_.-`, "portrait" or "landscape".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag: this computer prints for your
/// phones; and its version.
pub const FLAG: &str = "print";
const VERSION: u64 = 1;
/// The most copies of one job.
pub const MAX_COPIES: u32 = 99;
/// The most printers one computer lists, and papers one printer lists.
pub const MAX_PRINTERS: usize = 32;
pub const MAX_PAPERS: usize = 100;
/// Where a print job's PDF is received, under the downloads folder.
pub const SUBDIR: &str = ".mydomain-partial/print";
/// The states a job reports.
pub const STATES: &[&str] = &["waiting", "printing", "done", "failed"];
/// Jobs remembered on either side (oldest forgotten first).
const MAX_JOBS: usize = 64;
const CAPS_PREFIX: &str = "print_caps:";

const OLD_PEER: &str = "this computer doesn't print for your phone (or its app is too old)";

/// One printer a computer has.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Printer {
    /// Its name on that computer (CUPS queue name): what a job names.
    pub name: String,
    /// What it's called for people ("HP LaserJet in the hall"); may be "".
    #[serde(default)]
    pub label: String,
    /// The computer's default printer.
    #[serde(default)]
    pub is_default: bool,
    /// It prints in colour.
    #[serde(default)]
    pub color: bool,
    /// It prints on both sides.
    #[serde(default)]
    pub duplex: bool,
    /// Paper sizes, as the printer names them ("A4", "Letter", ...).
    #[serde(default)]
    pub papers: Vec<String>,
    /// The paper it uses unless told otherwise.
    #[serde(default)]
    pub default_paper: Option<String>,
}

/// How to print a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PrintOptions {
    /// The printer's name, as the computer listed it.
    pub printer: String,
    pub copies: u32,
    /// Colour (false: black and white).
    pub color: bool,
    /// "none", "long" (flip on the long edge) or "short".
    pub duplex: String,
    /// The paper's name, as the printer listed it ("" = its default).
    pub paper: String,
    /// "portrait" or "landscape".
    pub orientation: String,
    /// The document's name, for the printer's queue.
    pub title: String,
}

impl Default for PrintOptions {
    fn default() -> Self {
        PrintOptions {
            printer: String::new(),
            copies: 1,
            color: true,
            duplex: "none".into(),
            paper: String::new(),
            orientation: "portrait".into(),
            title: String::new(),
        }
    }
}

/// A printer's name: what a computer may list and a job may name. Passed
/// to the print command as one argument (never through a shell), but
/// still: no control characters, no leading `-` (it would read as an
/// option), nothing that could name a path.
pub fn is_printer_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        && name.trim() == name
}

/// A paper's name ("A4", "Letter", "iso_a4_210x297mm", "Custom.4x6in").
pub fn is_paper_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Text for people (a label, a title): no control characters, trimmed,
/// at most `max` characters.
fn clean_text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

impl Printer {
    /// The printer as it may be sent or shown: `None` if its name isn't
    /// one; papers that aren't paper names are dropped.
    pub fn checked(&self) -> Option<Printer> {
        if !is_printer_name(&self.name) {
            return None;
        }
        let mut papers: Vec<String> = Vec::new();
        for p in &self.papers {
            if is_paper_name(p) && !papers.contains(p) && papers.len() < MAX_PAPERS {
                papers.push(p.clone());
            }
        }
        Some(Printer {
            name: self.name.clone(),
            label: clean_text(&self.label, 200),
            is_default: self.is_default,
            color: self.color,
            duplex: self.duplex,
            default_paper: self.default_paper.clone().filter(|p| papers.contains(p)),
            papers,
        })
    }
}

impl PrintOptions {
    /// The options as they may be sent or printed with: `None` if the
    /// printer isn't a printer's name; out-of-range values are brought in
    /// (copies 1..=`MAX_COPIES`, an unknown duplex is one-sided, an
    /// unknown orientation portrait, a bad paper the printer's default).
    pub fn checked(&self) -> Option<PrintOptions> {
        if !is_printer_name(&self.printer) {
            return None;
        }
        let duplex = match self.duplex.as_str() {
            "long" | "short" => self.duplex.clone(),
            _ => "none".into(),
        };
        let orientation = match self.orientation.as_str() {
            "landscape" => "landscape".into(),
            _ => "portrait".into(),
        };
        let paper = if is_paper_name(&self.paper) {
            self.paper.clone()
        } else {
            String::new()
        };
        Some(PrintOptions {
            printer: self.printer.clone(),
            copies: self.copies.clamp(1, MAX_COPIES),
            color: self.color,
            duplex,
            paper,
            orientation,
            title: clean_text(&self.title, 120),
        })
    }

    /// From an offer's `print` field.
    pub fn from_json(v: &Value) -> Option<PrintOptions> {
        if !v.is_object() {
            return None;
        }
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let copies = v
            .get("copies")
            .and_then(Value::as_u64)
            .map(|c| c.min(u64::from(MAX_COPIES)) as u32)
            .unwrap_or(1);
        PrintOptions {
            printer: s("printer"),
            copies,
            color: v.get("color").and_then(Value::as_bool).unwrap_or(true),
            duplex: s("duplex"),
            paper: s("paper"),
            orientation: s("orientation"),
            title: s("title"),
        }
        .checked()
    }
}

/// Both sides' state.
#[derive(Default)]
pub struct Print {
    /// Computer: it prints for your phones now (`set_offered`).
    offered: AtomicBool,
    /// Device -> whether it said `print`.
    caps: Mutex<HashMap<String, bool>>,
    /// Computer: jobs arriving, transfer id -> (phone, options).
    arriving: Mutex<Vec<(String, String, PrintOptions)>>,
    /// Computer: jobs handed to the host, transfer id -> phone (whose
    /// state may be reported).
    received: Mutex<Vec<(String, String)>>,
    /// Phone: jobs sent, transfer id -> computer (whose state is taken).
    sent: Mutex<Vec<(String, String)>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &Print {
    &ctx.extras.print
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Remember `(id, node, ..)` in a short list, newest last.
fn keep<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T) -> bool) {
    list.retain(|x| !same(x));
    list.push(item);
    if list.len() > MAX_JOBS {
        let over = list.len() - MAX_JOBS;
        list.drain(..over);
    }
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it prints.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if offered(ctx) {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `print` value of its status or `addr_update`)
/// whether it prints for your phones. A status that leaves it out says it
/// doesn't (any more).
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() || !ctx.config.print {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    if supports(ctx, node) == on {
        return;
    }
    st(ctx).caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
}

/// `node` said it prints for your phones.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if !ctx.config.print || node.is_empty() {
        return false;
    }
    if let Some(on) = st(ctx).caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    st(ctx).caps.lock().insert(node.to_string(), on);
    on
}

/// Computer: it prints for your phones now (the user's switch, and it can
/// print). A change is told to the linked devices at once.
pub fn set_offered<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    let on = on && ctx.config.print;
    if st(ctx).offered.swap(on, Ordering::Relaxed) != on {
        let ctx = ctx.clone();
        std::thread::spawn(move || super::addr_update::announce(&ctx, None));
    }
}

pub fn offered<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.config.print && st(ctx).offered.load(Ordering::Relaxed)
}

// --- the phone's side ------------------------------------------------------

fn ready<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported(OLD_PEER));
    }
    Ok(())
}

/// Ask computer `node` for its printers (answered with `Printers`). Goes
/// when there's a link (it connects).
pub fn request_printers<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    ready(ctx, node)?;
    let msg = json!({ "type": "print_printers_request", "from": me(ctx) });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
    Ok(())
}

/// Send the PDF at `path` to computer `node` to print with `opts`.
/// Returns the job's id (its transfer's id): the transfer's own events
/// say how sending goes, `PrintStatus` how printing goes.
pub fn print_file<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    path: &Path,
    opts: &PrintOptions,
) -> crate::Result<String> {
    ready(ctx, node)?;
    let opts = opts
        .checked()
        .ok_or_else(|| CoreError::InvalidInput("not a printer's name".into()))?;
    let id = crate::transport::outbox::new_id();
    let extra = json!({ "print": opts });
    keep(
        &mut st(ctx).sent.lock(),
        (id.clone(), node.to_string()),
        |x| x.0 == id,
    );
    match super::files::start_with_extra(ctx, node, path, &id, Some(extra)) {
        Ok(id) => Ok(id),
        Err(e) => {
            st(ctx).sent.lock().retain(|x| x.0 != id);
            Err(e)
        }
    }
}

// --- the computer's side ---------------------------------------------------

/// A file offer with a `print` field arrived from `from`: where its PDF
/// goes (under `downloads`), or why it's refused.
pub fn offer_dest<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    id: &str,
    print: &Value,
    downloads: &Path,
) -> Result<PathBuf, &'static str> {
    if !offered(ctx) {
        return Err("printing is off on this computer");
    }
    let Some(opts) = PrintOptions::from_json(print) else {
        return Err("not a printer on this computer");
    };
    keep(
        &mut st(ctx).arriving.lock(),
        (id.to_string(), from.to_string(), opts),
        |x| x.0 == id,
    );
    Ok(downloads.join(SUBDIR))
}

/// Transfer `id` was received at `path`: if it's a print job, hand it to
/// the host (`PrintJob`) and say so (the caller then doesn't report a
/// received file).
pub fn received<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    from: &str,
    path: &Path,
    name: &str,
    size: u64,
) -> bool {
    let job = {
        let mut arriving = st(ctx).arriving.lock();
        let at = arriving.iter().position(|x| x.0 == id && x.1 == from);
        at.map(|i| arriving.remove(i))
    };
    let Some((_, _, options)) = job else {
        return false;
    };
    keep(
        &mut st(ctx).received.lock(),
        (id.to_string(), from.to_string()),
        |x| x.0 == id,
    );
    ctx.sink.emit(CoreEvent::PrintJob {
        id: id.to_string(),
        from: from.to_string(),
        name: name.to_string(),
        path: path.to_string_lossy().into_owned(),
        size,
        options,
    });
    true
}

/// Answer `node`'s request with this computer's printers (the host lists
/// them when it sees `PrintersRequested`).
pub fn send_printers<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    printers: &[Printer],
) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    let on = offered(ctx);
    let list: Vec<Printer> = if on {
        printers
            .iter()
            .filter_map(Printer::checked)
            .take(MAX_PRINTERS)
            .collect()
    } else {
        Vec::new()
    };
    let msg = json!({ "type": "print_printers", "from": me(ctx), "on": on, "printers": list });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
    Ok(())
}

/// Tell the phone that sent job `id` how it goes: "waiting" (this
/// computer's user is asked), "printing", "done" or "failed" (`reason`).
pub fn report<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    state: &str,
    reason: Option<&str>,
) -> crate::Result<()> {
    if !STATES.contains(&state) {
        return Err(CoreError::InvalidInput(format!("not a job state: {state}")));
    }
    let node = st(ctx)
        .received
        .lock()
        .iter()
        .find(|x| x.0 == id)
        .map(|x| x.1.clone())
        .ok_or_else(|| CoreError::InvalidInput("no such print job".into()))?;
    let mut msg = json!({ "type": "print_status", "from": me(ctx), "id": id, "state": state });
    if let Some(why) = reason {
        msg["reason"] = clean_text(why, 300).into();
    }
    crate::engine::send_reliably(ctx, &node, &msg.to_string());
    if matches!(state, "done" | "failed") {
        st(ctx).received.lock().retain(|x| x.0 != id);
    }
    Ok(())
}

pub struct PrintHandler;

impl<P: Platform> ControlHandler<P> for PrintHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["print_printers_request", "print_printers", "print_status"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Only your own devices.
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        match msg.kind() {
            "print_printers_request" => {
                if offered(ctx) {
                    ctx.sink.emit(CoreEvent::PrintersRequested { from });
                } else {
                    // Off: say so at once (the phone drops its printers).
                    let _ = send_printers(ctx, &from, &[]);
                }
            }
            "print_printers" => {
                let on = msg.flag("on");
                let printers: Vec<Printer> = if on {
                    serde_json::from_value::<Vec<Printer>>(msg.json("printers"))
                        .unwrap_or_default()
                        .iter()
                        .filter_map(Printer::checked)
                        .take(MAX_PRINTERS)
                        .collect()
                } else {
                    Vec::new()
                };
                ctx.sink.emit(CoreEvent::Printers { from, on, printers });
            }
            "print_status" => {
                let id = msg.str("id");
                let state = msg.str("state");
                if !STATES.contains(&state.as_str()) {
                    return;
                }
                // Only a job this phone sent to that computer.
                let ours = st(ctx).sent.lock().iter().any(|x| x.0 == id && x.1 == from);
                if !ours {
                    return;
                }
                if matches!(state.as_str(), "done" | "failed") {
                    st(ctx).sent.lock().retain(|x| x.0 != id);
                }
                let reason = msg
                    .opt("reason")
                    .map(|r| clean_text(&r, 300))
                    .filter(|r| !r.is_empty());
                ctx.sink.emit(CoreEvent::PrintStatus {
                    from,
                    id,
                    state,
                    reason,
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_names_are_checked() {
        for ok in [
            "HP_LaserJet",
            "Brother HL-L2350DW",
            "Canon_MG3600_series",
            "Büro",
        ] {
            assert!(is_printer_name(ok), "{ok}");
        }
        for bad in ["", "-d", "a/b", "a\\b", "x\ny", " lead", &"p".repeat(128)] {
            assert!(!is_printer_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn paper_names_are_tokens() {
        for ok in [
            "A4",
            "Letter",
            "iso_a4_210x297mm",
            "Custom.4x6in",
            "na_letter_8.5x11in",
        ] {
            assert!(is_paper_name(ok), "{ok}");
        }
        for bad in ["", "-o", "A4 x", "A4;rm", "a=b", &"a".repeat(65)] {
            assert!(!is_paper_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn options_are_brought_in_range() {
        let o = PrintOptions::from_json(&json!({
            "printer": "Office", "copies": 5000, "color": false, "duplex": "sideways",
            "paper": "A4; rm -rf", "orientation": "upside", "title": "a\u{7}b"
        }))
        .unwrap();
        assert_eq!(o.copies, MAX_COPIES);
        assert!(!o.color);
        assert_eq!(o.duplex, "none");
        assert_eq!(o.paper, "");
        assert_eq!(o.orientation, "portrait");
        assert_eq!(o.title, "ab");
        let o = PrintOptions::from_json(&json!({
            "printer": "Office", "copies": 0, "duplex": "short", "paper": "Letter",
            "orientation": "landscape"
        }))
        .unwrap();
        assert_eq!(o.copies, 1);
        assert!(o.color, "colour unless told otherwise");
        assert_eq!(o.duplex, "short");
        assert_eq!(o.paper, "Letter");
        assert_eq!(o.orientation, "landscape");
        assert!(PrintOptions::from_json(&json!({"printer": "-x"})).is_none());
        assert!(PrintOptions::from_json(&json!("Office")).is_none());
    }

    #[test]
    fn options_round_trip_on_the_wire() {
        let o = PrintOptions {
            printer: "Hall".into(),
            copies: 2,
            color: false,
            duplex: "long".into(),
            paper: "A4".into(),
            orientation: "portrait".into(),
            title: "Boarding pass".into(),
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(PrintOptions::from_json(&v), Some(o));
    }

    #[test]
    fn a_printer_keeps_only_good_papers() {
        let p = Printer {
            name: "Hall".into(),
            label: "Hall\nprinter".into(),
            is_default: true,
            color: true,
            duplex: false,
            papers: vec![
                "A4".into(),
                "A4".into(),
                "bad paper".into(),
                "Letter".into(),
            ],
            default_paper: Some("bad paper".into()),
        }
        .checked()
        .unwrap();
        assert_eq!(p.papers, vec!["A4", "Letter"]);
        assert_eq!(p.default_paper, None);
        assert_eq!(p.label, "Hallprinter");
        assert!(Printer {
            name: "-x".into(),
            ..Default::default()
        }
        .checked()
        .is_none());
    }

    #[test]
    fn job_lists_stay_short() {
        let mut list = Vec::new();
        for i in 0..(MAX_JOBS + 10) {
            keep(&mut list, (i.to_string(), "n".to_string()), |x| {
                x.0 == i.to_string()
            });
        }
        assert_eq!(list.len(), MAX_JOBS);
        assert_eq!(list[0].0, "10");
        keep(&mut list, ("20".to_string(), "m".to_string()), |x| {
            x.0 == "20"
        });
        assert_eq!(list.len(), MAX_JOBS);
        assert_eq!(list.last().unwrap().1, "m");
    }
}
