//! Insert from phone: on a computer, "Take a photo with <phone>" or "Scan
//! documents with <phone>". The phone shows its camera (once its owner
//! acts on the phone: never by itself), takes the photo or the pages, and
//! sends the result back as an ordinary file transfer under the request's
//! id; the computer then knows it's the answer and puts it in Downloads and
//! on the clipboard.
//!
//!   insert_request { from, id, mode }        computer → phone (reliable):
//!                                            `mode` "photo" or "scan"; `id`
//!                                            is the transfer id the answer
//!                                            will come under
//!   insert_state   { from, id, state }       phone → computer: "open" (the
//!                                            capture screen is up), "sending"
//!                                            (the file follows)
//!   insert_cancel  { from, id, reason? }     either way (reliable):
//!                                            "cancelled", "timeout",
//!                                            "no_permission", "error", ...
//!
//! - Only between devices that said `insert_capture: 1` (in their status,
//!   or in the `addr_update` every link starts with): an older phone would
//!   show the request as chat. This end never sends one to a device that
//!   didn't say it.
//! - Only from your own devices (known peers); the answer is taken as the
//!   answer only from the phone that was asked.
//! - A request ends after `CoreConfig::insert_timeout` (3 minutes) unless
//!   the phone started sending by then: both ends drop it by themselves
//!   (`InsertEnded { reason: "timeout" }`), and the computer tells the phone.
//!   A transfer that started has until `SEND_FOR`; one that fails ends the
//!   request (`reason: "failed"`).
//! - Either end may cancel; a cancel after the file started also stops the
//!   transfer.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device understands the
/// insert messages, and its version.
pub const FLAG: &str = "insert_capture";
const VERSION: u64 = 1;
/// What may be asked for.
pub const MODES: &[&str] = &["photo", "scan"];
/// What the phone may say it's doing.
pub const STATES: &[&str] = &["open", "sending"];
/// How long a request waits for the phone, out of the box.
pub const TIMEOUT: Duration = Duration::from_secs(3 * 60);
/// How long a transfer that started may take before the request is
/// dropped (the transfer itself goes on, as an ordinary file).
pub const SEND_FOR: Duration = Duration::from_secs(30 * 60);

const OLD_PEER: &str = "this device's app is too old to take a photo for you: update it";

/// One request, on either end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The other device: the phone asked, or the computer asking.
    pub node: String,
    /// "photo" or "scan".
    pub mode: String,
    /// When it's dropped.
    pub until: Instant,
    /// The phone said the file is on its way.
    pub sending: bool,
}

/// A request a phone has now (`Engine::insert_requests`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct InsertRequestInfo {
    pub id: String,
    /// The computer asking.
    pub from: String,
    pub mode: String,
}

#[derive(Default)]
pub struct InsertCapture {
    /// Computer: the requests it made, by id.
    asked: Mutex<HashMap<String, Pending>>,
    /// Phone: the requests made of it, by id.
    asks: Mutex<HashMap<String, Pending>>,
    /// Device -> whether it said `insert_capture`.
    caps: Mutex<HashMap<String, bool>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &InsertCapture {
    &ctx.extras.insert
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// The requests of `map` whose time is up at `now`, taken out of it.
pub fn expire(map: &mut HashMap<String, Pending>, now: Instant) -> Vec<(String, Pending)> {
    let gone: Vec<String> = map
        .iter()
        .filter(|(_, p)| p.until <= now)
        .map(|(id, _)| id.clone())
        .collect();
    let mut out: Vec<(String, Pending)> = gone
        .into_iter()
        .filter_map(|id| map.remove(&id).map(|p| (id, p)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A reason as a cancel may carry it: a short lowercase word.
pub fn is_reason(r: &str) -> bool {
    !r.is_empty() && r.len() <= 24 && r.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it understands the
/// insert messages.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.insert_capture {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `insert_capture` value of its status or
/// `addr_update`) whether it understands them.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    st(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` said it understands the insert messages.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.insert_capture && st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value) {
    crate::engine::send_reliably(ctx, node, &msg.to_string());
}

// --- the computer's side ------------------------------------------------------

/// Ask phone `node` for a photo or a scan (`mode`). Returns the request's
/// id, which the answer's transfer will carry. A request already waiting
/// on that phone is cancelled first (one at a time per phone).
pub fn request<P: Platform>(ctx: &NetCtx<P>, node: &str, mode: &str) -> crate::Result<String> {
    if !MODES.contains(&mode) {
        return Err(CoreError::InvalidInput(format!(
            "not something a phone can insert: {mode}"
        )));
    }
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    let known = st(ctx).caps.lock().get(node).copied();
    match known {
        Some(true) if ctx.config.insert_capture => {}
        // Not heard from it since this app started: connect, and it says.
        None if ctx.config.insert_capture && !ctx.ws_conns.lock().contains_key(node) => {
            crate::engine::start_ladder(ctx, node);
            return Err(CoreError::NoLink("not connected yet: connecting".into()));
        }
        _ => return Err(CoreError::Unsupported(OLD_PEER)),
    }
    let waiting: Vec<String> = st(ctx)
        .asked
        .lock()
        .iter()
        .filter(|(_, p)| p.node == node && !p.sending)
        .map(|(id, _)| id.clone())
        .collect();
    for id in waiting {
        cancel(ctx, &id, "cancelled");
    }
    let id = crate::transport::outbox::new_id();
    st(ctx).asked.lock().insert(
        id.clone(),
        Pending {
            node: node.to_string(),
            mode: mode.to_string(),
            until: Instant::now() + ctx.config.insert_timeout,
            sending: false,
        },
    );
    send(
        ctx,
        node,
        &json!({ "type": "insert_request", "from": me(ctx), "id": id, "mode": mode }),
    );
    Ok(id)
}

/// The requests this computer is waiting on: (id, phone, mode, sending).
pub fn asked<P: Platform>(ctx: &NetCtx<P>) -> Vec<(String, String, String, bool)> {
    let mut list: Vec<_> = st(ctx)
        .asked
        .lock()
        .iter()
        .map(|(id, p)| (id.clone(), p.node.clone(), p.mode.clone(), p.sending))
        .collect();
    list.sort();
    list
}

/// A file `from` sent under transfer id `id` arrived: if it's the answer
/// to a request this computer made of that phone, the request is done and
/// its mode is returned (the caller reports `InsertReceived` instead of an
/// ordinary file).
pub(crate) fn take_answer<P: Platform>(ctx: &NetCtx<P>, id: &str, from: &str) -> Option<String> {
    if !ctx.config.insert_capture {
        return None;
    }
    let mut asked = st(ctx).asked.lock();
    if asked.get(id).is_some_and(|p| p.node == from) {
        return asked.remove(id).map(|p| p.mode);
    }
    None
}

/// A transfer `id` failed: if it was a request's answer, the request ends.
pub(crate) fn transfer_failed<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    if !ctx.config.insert_capture {
        return;
    }
    let gone = st(ctx).asked.lock().remove(id);
    if let Some(p) = gone {
        ctx.sink.emit(CoreEvent::InsertEnded {
            node_id: p.node,
            id: id.to_string(),
            reason: "failed".into(),
        });
    }
}

// --- the phone's side ---------------------------------------------------------

/// The requests made of this phone now, oldest id first.
pub fn requests<P: Platform>(ctx: &NetCtx<P>) -> Vec<InsertRequestInfo> {
    let now = Instant::now();
    let mut list: Vec<InsertRequestInfo> = st(ctx)
        .asks
        .lock()
        .iter()
        .filter(|(_, p)| p.until > now)
        .map(|(id, p)| InsertRequestInfo {
            id: id.clone(),
            from: p.node.clone(),
            mode: p.mode.clone(),
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    list
}

fn asking<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<Pending> {
    st(ctx)
        .asks
        .lock()
        .get(id)
        .filter(|p| p.until > Instant::now())
        .cloned()
        .ok_or_else(|| CoreError::InvalidInput("that request has ended".into()))
}

/// Tell the computer the capture screen for request `id` is up.
pub fn opened<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<()> {
    let p = asking(ctx, id)?;
    send(
        ctx,
        &p.node,
        &json!({ "type": "insert_state", "from": me(ctx), "id": id, "state": "open" }),
    );
    Ok(())
}

/// Send the photo or the scan (`path`) to the computer that asked, as the
/// answer to request `id`. The request is done on this end then.
pub fn send_file<P: Platform>(ctx: &NetCtx<P>, id: &str, path: &Path) -> crate::Result<()> {
    let p = asking(ctx, id)?;
    if !path.is_file() {
        return Err(CoreError::InvalidInput("not a file".into()));
    }
    // Said first, in the same queue as the offer: the computer stops
    // counting down before the file arrives.
    send(
        ctx,
        &p.node,
        &json!({ "type": "insert_state", "from": me(ctx), "id": id, "state": "sending" }),
    );
    super::files::start_with_id(ctx, &p.node, path, id)?;
    st(ctx).asks.lock().remove(id);
    Ok(())
}

// --- both ---------------------------------------------------------------------

/// Cancel request `id` on this end (asked or asking), and tell the other
/// end why (`reason`, "cancelled" if it isn't a short word). A transfer
/// that started stops too. Nothing to do for a request that already ended.
pub fn cancel<P: Platform>(ctx: &NetCtx<P>, id: &str, reason: &str) {
    let reason = if is_reason(reason) {
        reason
    } else {
        "cancelled"
    };
    let asked = st(ctx).asked.lock().remove(id);
    let asks = st(ctx).asks.lock().remove(id);
    let Some(p) = asked.clone().or(asks) else {
        return;
    };
    if asked.as_ref().is_some_and(|p| p.sending) {
        super::files::cancel(ctx, id);
    }
    send(
        ctx,
        &p.node,
        &json!({ "type": "insert_cancel", "from": me(ctx), "id": id, "reason": reason }),
    );
}

/// Every couple of seconds: drop the requests whose time is up.
pub(crate) fn on_timer<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.config.insert_capture {
        return;
    }
    let now = Instant::now();
    let asked = expire(&mut st(ctx).asked.lock(), now);
    let asks = expire(&mut st(ctx).asks.lock(), now);
    for (id, p) in asked {
        if !p.sending {
            send(
                ctx,
                &p.node,
                &json!({ "type": "insert_cancel", "from": me(ctx), "id": id, "reason": "timeout" }),
            );
        }
        ctx.sink.emit(CoreEvent::InsertEnded {
            node_id: p.node,
            id,
            reason: "timeout".into(),
        });
    }
    for (id, p) in asks {
        ctx.sink.emit(CoreEvent::InsertEnded {
            node_id: p.node,
            id,
            reason: "timeout".into(),
        });
    }
}

// --- receiving ------------------------------------------------------------------

/// `insert_request`, `insert_state` and `insert_cancel`, from your devices.
pub struct InsertHandler;

impl<P: Platform> ControlHandler<P> for InsertHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["insert_request", "insert_state", "insert_cancel"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        let id = msg.str("id");
        if !super::files::is_transfer_id(&id) {
            return;
        }
        match msg.kind() {
            "insert_request" => {
                let mode = msg.str("mode");
                if !MODES.contains(&mode.as_str()) {
                    return;
                }
                {
                    let mut asks = st(ctx).asks.lock();
                    // A repeat (the reliable queue resends) is not news.
                    if asks.contains_key(&id) {
                        return;
                    }
                    asks.insert(
                        id.clone(),
                        Pending {
                            node: from.clone(),
                            mode: mode.clone(),
                            until: Instant::now() + ctx.config.insert_timeout,
                            sending: false,
                        },
                    );
                }
                ctx.sink.emit(CoreEvent::InsertRequest { from, id, mode });
            }
            "insert_state" => {
                let state = msg.str("state");
                if !STATES.contains(&state.as_str()) {
                    return;
                }
                {
                    let mut asked = st(ctx).asked.lock();
                    let Some(p) = asked.get_mut(&id).filter(|p| p.node == from) else {
                        return;
                    };
                    if state == "sending" {
                        p.sending = true;
                        p.until = Instant::now() + SEND_FOR;
                    }
                }
                ctx.sink.emit(CoreEvent::InsertState { from, id, state });
            }
            "insert_cancel" => {
                let reason = msg
                    .opt("reason")
                    .filter(|r| is_reason(r))
                    .unwrap_or_else(|| "cancelled".into());
                let asked = {
                    let mut m = st(ctx).asked.lock();
                    match m.get(&id) {
                        Some(p) if p.node == from => m.remove(&id),
                        _ => None,
                    }
                };
                let asks = {
                    let mut m = st(ctx).asks.lock();
                    match m.get(&id) {
                        Some(p) if p.node == from => m.remove(&id),
                        _ => None,
                    }
                };
                if asked.is_none() && asks.is_none() {
                    return;
                }
                if asked.as_ref().is_some_and(|p| p.sending) {
                    super::files::cancel(ctx, &id);
                }
                ctx.sink.emit(CoreEvent::InsertEnded {
                    node_id: from,
                    id,
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

    fn pending(node: &str, until: Instant, sending: bool) -> Pending {
        Pending {
            node: node.into(),
            mode: "scan".into(),
            until,
            sending,
        }
    }

    #[test]
    fn requests_expire_only_when_their_time_is_up() {
        let now = Instant::now();
        let mut m = HashMap::new();
        m.insert("b".to_string(), pending("phone", now, false));
        m.insert(
            "a".to_string(),
            pending("phone", now - Duration::from_secs(1), false),
        );
        m.insert("c".to_string(), pending("phone", now + TIMEOUT, false));
        let gone = expire(&mut m, now);
        assert_eq!(
            gone.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(m.len(), 1);
        assert!(m.contains_key("c"));
        // Nothing left to expire.
        assert!(expire(&mut m, now).is_empty());
        // ...until three minutes have gone by.
        assert_eq!(expire(&mut m, now + TIMEOUT).len(), 1);
        assert!(m.is_empty());
    }

    #[test]
    fn a_transfer_gets_longer_than_a_request() {
        const { assert!(SEND_FOR.as_secs() > TIMEOUT.as_secs()) };
        assert_eq!(TIMEOUT, Duration::from_secs(180));
    }

    #[test]
    fn reasons_are_short_words() {
        assert!(is_reason("cancelled"));
        assert!(is_reason("no_permission"));
        assert!(!is_reason(""));
        assert!(!is_reason("Cancelled"));
        assert!(!is_reason("<b>x</b>"));
        assert!(!is_reason(&"a".repeat(25)));
    }

    #[test]
    fn modes_and_states_are_the_known_words() {
        assert_eq!(MODES, ["photo", "scan"]);
        assert_eq!(STATES, ["open", "sending"]);
    }

    #[test]
    fn request_info_has_its_shape() {
        let info = InsertRequestInfo {
            id: "abcd".into(),
            from: "pc".into(),
            mode: "photo".into(),
        };
        assert_eq!(
            serde_json::to_value(info).unwrap(),
            json!({"id": "abcd", "from": "pc", "mode": "photo"})
        );
    }
}
