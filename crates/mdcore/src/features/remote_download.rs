//! "Download on <device>": share a download link on one device and
//! another of yours downloads the file itself, straight from the web.
//!
//!   dl_request { from, id, url, name? }                    asker → downloader
//!   dl_status  { from, id, state, name, host, size, done, error? }
//!                                                           downloader → asker
//!   dl_cancel  { from, id }                                 asker → downloader
//!
//! The downloading device (`here`) checks the link (`link_fetch`: http(s)
//! only, never to this device or the local network unless its setting
//! allows LAN links), asks the server about it (HEAD: name and size), asks
//! the user the first time for each device (`RemoteDownloadSettings`:
//! "ask", "always" or "never"; a size over the limit is refused first),
//! then downloads it: one at a time, the others queued, a dropped
//! connection resumed with `Range`. The file goes to the downloads folder
//! under a safe name (`Content-Disposition`, else the link) and is never
//! opened. Its states go back to the asker (`dl_status`, reliably; progress
//! about once a second, best effort), so both list it as a transfer.
//! Either side can cancel it.
//!
//! Both sides also make the usual transfer events with the download's id
//! (`FileProgress`, then `FileReceived` / `FileSent` or `FileFailed`), so
//! it shows (and is cancelled, `Engine::cancel_file`) like any transfer,
//! and a `RemoteDownload` event with the whole item whenever its state
//! changes (the link, its host, and the states a transfer doesn't have:
//! asking, queued).
//!
//! Only devices that said `remote_download: 1` (status and `addr_update`)
//! are sent a `dl_request`: an older app would show it as chat.
//!
//! Downloads not yet ended are kept in the platform's store (both sides,
//! at each change of state), so they survive a restart: the downloading
//! device checks, asks or queues them again, and one that was downloading
//! carries on from its partial file with `Range` (it says where it is
//! again); the asker keeps waiting for news (`STALE` from the restart).
//! A failed or cancelled one asked from here can be asked again (`retry`).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{RemoteDownloadItem, RemoteDownloadSettings};
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::link_fetch::{self, FetchError, FetchOpts};
use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying a device downloads links.
pub const FLAG: &str = "remote_download";
const VERSION: u64 = 1;
const CAPS_PREFIX: &str = "remote_download_caps:";
const SETTINGS_KEY: &str = "remote_download_settings";
const JOBS_KEY: &str = "remote_download_jobs";

/// How long the downloading device waits for its user to answer.
pub const ASK_FOR: Duration = Duration::from_secs(10 * 60);
/// The asker gives up on a download it hears nothing about for this long.
const STALE: Duration = Duration::from_secs(15 * 60);
/// Progress events here at most this often...
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// ...and to the asker.
const STATUS_EVERY: Duration = Duration::from_secs(1);
/// Tries after the first when the connection drops.
const RETRIES: u32 = 4;
const RETRY_WAIT: Duration = Duration::from_secs(2);
/// Finished downloads kept in the list.
const KEEP_FINISHED: usize = 50;
/// Most downloads one device may have waiting or running here at once:
/// each holds a thread (asking can take `ASK_FOR`) and a prompt.
pub const MAX_OPEN_PER_DEVICE: usize = 8;
const MB: u64 = 1024 * 1024;

const POLICIES: [&str; 3] = ["ask", "always", "never"];

struct Job {
    item: RemoteDownloadItem,
    cancel: Arc<AtomicBool>,
    /// The user's answer, while asking.
    answer: Option<bool>,
    /// The asker: when the downloader last said something.
    heard: Instant,
}

/// Downloads both ways, and who can do them.
#[derive(Default)]
pub struct RemoteDownloads {
    jobs: Mutex<HashMap<String, Job>>,
    answered: Condvar,
    /// Ids waiting to download here, in order.
    queue: Mutex<VecDeque<String>>,
    running: Mutex<bool>,
    caps: Mutex<HashMap<String, u64>>,
    /// The last run's downloads were taken back.
    restored: AtomicBool,
    /// What was last written to the store.
    saved: Mutex<String>,
}

fn now_ms() -> u64 {
    crate::transport::media::now_ms()
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

fn state<P: Platform>(ctx: &NetCtx<P>) -> &RemoteDownloads {
    &ctx.extras.remote_download
}

// --- who downloads links --------------------------------------------------------

pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.remote_download {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `remote_download` value of its status or
/// `addr_update`) whether it downloads links. Kept across restarts.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let level = flag.as_u64().unwrap_or(0);
    if node.is_empty() || caps(ctx, node) == level {
        return;
    }
    state(ctx).caps.lock().insert(node.to_string(), level);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), &level.to_string());
}

fn caps<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    if let Some(level) = state(ctx).caps.lock().get(node) {
        return *level;
    }
    let level = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    state(ctx).caps.lock().insert(node.to_string(), level);
    level
}

/// `node` said it downloads links (and this end knows the messages).
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.remote_download && !node.starts_with("manual:") && caps(ctx, node) >= 1
}

// --- settings ----------------------------------------------------------------

/// This device's answer to downloads asked from the others.
pub fn settings<P: Platform>(ctx: &NetCtx<P>) -> RemoteDownloadSettings {
    ctx.platform
        .kv_get(SETTINGS_KEY)
        .and_then(|s| serde_json::from_str::<RemoteDownloadSettings>(&s).ok())
        .filter(|s| POLICIES.contains(&s.policy.as_str()))
        .unwrap_or_default()
}

pub fn set_settings<P: Platform>(
    ctx: &NetCtx<P>,
    mut s: RemoteDownloadSettings,
) -> crate::Result<()> {
    if !POLICIES.contains(&s.policy.as_str()) {
        return Err(CoreError::InvalidInput(format!(
            "unknown choice {:?} (ask, always or never)",
            s.policy
        )));
    }
    s.allowed.retain(|n| !n.trim().is_empty() && n.len() <= 128);
    s.allowed.sort();
    s.allowed.dedup();
    let json = serde_json::to_string(&s).map_err(|e| CoreError::Other(e.to_string()))?;
    ctx.platform.kv_set(SETTINGS_KEY, &json);
    Ok(())
}

// --- the list and its events -------------------------------------------------------

/// Every download either way this run, the newest first.
pub fn list<P: Platform>(ctx: &NetCtx<P>) -> Vec<RemoteDownloadItem> {
    let mut v: Vec<RemoteDownloadItem> = state(ctx)
        .jobs
        .lock()
        .values()
        .map(|j| j.item.clone())
        .collect();
    v.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(a.id.cmp(&b.id)));
    v
}

/// Whether `id` is a download (either way) this device knows.
pub fn knows<P: Platform>(ctx: &NetCtx<P>, id: &str) -> bool {
    state(ctx).jobs.lock().contains_key(id)
}

fn item<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<RemoteDownloadItem> {
    state(ctx).jobs.lock().get(id).map(|j| j.item.clone())
}

/// Change `id` (unless it ended); the item after, if it changed.
fn update<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    f: impl FnOnce(&mut RemoteDownloadItem),
) -> Option<RemoteDownloadItem> {
    let mut jobs = state(ctx).jobs.lock();
    let job = jobs.get_mut(id)?;
    if job.item.finished() {
        return None;
    }
    let before = job.item.clone();
    f(&mut job.item);
    if job.item == before {
        return None;
    }
    job.item.updated_at = now_ms();
    Some(job.item.clone())
}

/// The transfer event matching `item` (so it shows with the transfers).
fn transfer_event(item: &RemoteDownloadItem) -> CoreEvent {
    let (id, node_id, name) = (item.id.clone(), item.node_id.clone(), item.name.clone());
    match (item.state.as_str(), item.here) {
        ("done", true) => CoreEvent::FileReceived {
            id,
            from: node_id,
            name,
            path: item.path.clone().unwrap_or_default(),
            size: item.size,
            clipboard: false,
            sync: false,
        },
        ("done", false) => CoreEvent::FileSent { id, node_id, name },
        (s, here) if item.finished() => CoreEvent::FileFailed {
            id,
            node_id,
            name,
            incoming: here,
            reason: if s == "cancelled" {
                "cancelled".into()
            } else {
                item.error.clone().unwrap_or_else(|| s.to_string())
            },
        },
        (_, here) => CoreEvent::FileProgress {
            id,
            node_id,
            name,
            done: item.done,
            total: item.size.max(item.done),
            incoming: here,
            resumed: false,
            batch: None,
        },
    }
}

/// Its state changed: say so here (the item, then as a transfer), and
/// keep the ones not ended for after a restart.
fn emit<P: Platform>(ctx: &NetCtx<P>, item: &RemoteDownloadItem) {
    save_jobs(ctx);
    ctx.sink
        .emit(CoreEvent::RemoteDownload { item: item.clone() });
    ctx.sink.emit(transfer_event(item));
}

fn save_jobs<P: Platform>(ctx: &NetCtx<P>) {
    if !state(ctx).restored.load(Ordering::Relaxed) {
        // Not over what the last run left before it's taken back.
        return;
    }
    let mut items: Vec<RemoteDownloadItem> = state(ctx)
        .jobs
        .lock()
        .values()
        .filter(|j| !j.item.finished())
        .map(|j| j.item.clone())
        .collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    let json = serde_json::to_string(&items).unwrap_or_default();
    let mut last = state(ctx).saved.lock();
    if *last != json {
        ctx.platform.kv_set(JOBS_KEY, &json);
        *last = json;
    }
}

/// On the core's timer: once the downloads folder is known, take back the
/// downloads the last run hadn't finished.
pub fn on_timer<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.config.remote_download
        || state(ctx).restored.load(Ordering::Relaxed)
        || ctx.files.download_dir.lock().is_none()
    {
        return;
    }
    if state(ctx).restored.swap(true, Ordering::Relaxed) {
        return;
    }
    let saved: Vec<RemoteDownloadItem> = ctx
        .platform
        .kv_get(JOBS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut queued = false;
    for mut item in saved {
        if item.finished() || !super::files::is_transfer_id(&item.id) || knows(ctx, &item.id) {
            continue;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let again = item.here
            && matches!(item.state.as_str(), "checking" | "asking")
            && url::Url::parse(&item.url).is_ok();
        if item.here && !again {
            // It was queued or downloading: in the queue again, carrying
            // on from its partial file.
            item.state = "queued".into();
        }
        item.updated_at = now_ms();
        state(ctx).jobs.lock().insert(
            item.id.clone(),
            Job {
                item: item.clone(),
                cancel: cancel.clone(),
                answer: None,
                heard: Instant::now(),
            },
        );
        emit(ctx, &item);
        if !item.here {
            watch(ctx, &item.id);
            continue;
        }
        report(ctx, &item, true);
        if again {
            if let Ok(url) = url::Url::parse(&item.url) {
                let (ctx, id) = (ctx.clone(), item.id.clone());
                std::thread::spawn(move || prepare(&ctx, &id, &url, &cancel));
            }
        } else {
            state(ctx).queue.lock().push_back(item.id.clone());
            queued = true;
        }
    }
    save_jobs(ctx);
    if queued {
        pump(ctx);
    }
}

/// Ask again for a download asked from here that failed or was cancelled
/// (same device, same link). `Ok(None)` if `id` isn't a download.
pub fn retry<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<Option<String>> {
    let Some(it) = item(ctx, id) else {
        return Ok(None);
    };
    if it.here {
        return Err(CoreError::InvalidInput(
            "ask for it again on the other device".into(),
        ));
    }
    if !it.finished() || it.state == "done" {
        return Err(CoreError::InvalidInput(
            "that download hasn't failed".into(),
        ));
    }
    request(ctx, &it.node_id, &it.url).map(|n| Some(n.id))
}

/// Forget the oldest finished ones past `KEEP_FINISHED`.
fn trim<P: Platform>(ctx: &NetCtx<P>) {
    let mut jobs = state(ctx).jobs.lock();
    let mut done: Vec<(u64, String)> = jobs
        .values()
        .filter(|j| j.item.finished())
        .map(|j| (j.item.updated_at, j.item.id.clone()))
        .collect();
    if done.len() <= KEEP_FINISHED {
        return;
    }
    done.sort();
    for (_, id) in done.iter().take(done.len() - KEEP_FINISHED) {
        jobs.remove(id);
    }
}

fn status_msg<P: Platform>(ctx: &NetCtx<P>, item: &RemoteDownloadItem) -> String {
    let mut m = json!({
        "type": "dl_status",
        "from": me(ctx),
        "id": item.id,
        "state": item.state,
        "name": item.name,
        "host": item.host,
        "size": item.size,
        "done": item.done,
    });
    if let Some(e) = &item.error {
        m["error"] = e.clone().into();
    }
    m.to_string()
}

/// Tell the asker where its download is (state changes reliably).
fn report<P: Platform>(ctx: &NetCtx<P>, item: &RemoteDownloadItem, reliably: bool) {
    if !item.here {
        return;
    }
    let msg = status_msg(ctx, item);
    if reliably {
        crate::engine::send_reliably(ctx, &item.node_id, &msg);
    } else {
        send_control_to(ctx, &item.node_id, &msg);
    }
}

/// End `id` (once): `state` "done", "failed", "cancelled" or "refused".
fn finish<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    end: &str,
    error: Option<String>,
    path: Option<PathBuf>,
) {
    let Some(item) = update(ctx, id, |it| {
        it.state = end.to_string();
        it.error = error;
        if let Some(p) = path {
            it.path = Some(p.to_string_lossy().into_owned());
            it.done = it.size;
        }
    }) else {
        return;
    };
    if let Some(job) = state(ctx).jobs.lock().get(id) {
        job.cancel.store(true, Ordering::Relaxed);
    }
    state(ctx).answered.notify_all();
    emit(ctx, &item);
    report(ctx, &item, true);
    trim(ctx);
}

fn short(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The name a link's file would have before the server says.
fn guess_name(url: &str) -> String {
    link_fetch::pick_name(url, None, None)
}

// --- asking another device --------------------------------------------------------

/// Ask `node` to download the first web link in `text`. Only to a device
/// that downloads links; whether this link may go is its call.
pub fn request<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    text: &str,
) -> crate::Result<RemoteDownloadItem> {
    let url = super::read_later::find_url(text).ok_or_else(|| {
        CoreError::InvalidInput("there's no web link (http or https) to download".into())
    })?;
    let url = link_fetch::check_url(&url, true)
        .map_err(|p| CoreError::InvalidInput(p.reason().into()))?
        .to_string();
    if !supports(ctx, node) {
        return Err(CoreError::InvalidInput(
            "that device can't download links yet (update my-domain on it)".into(),
        ));
    }
    let id = format!("dl{}", crate::transport::outbox::new_id());
    let now = now_ms();
    let item = RemoteDownloadItem {
        id: id.clone(),
        node_id: node.to_string(),
        here: false,
        host: link_fetch::host_of(&url),
        name: guess_name(&url),
        url: url.clone(),
        state: "requested".into(),
        started_at: now,
        updated_at: now,
        ..RemoteDownloadItem::default()
    };
    state(ctx).jobs.lock().insert(
        id.clone(),
        Job {
            item: item.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            answer: None,
            heard: Instant::now(),
        },
    );
    emit(ctx, &item);
    let msg = json!({
        "type": "dl_request",
        "from": me(ctx),
        "id": id,
        "url": url,
        "name": item.name,
    });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
    watch(ctx, &id);
    Ok(item)
}

/// The asker: heard nothing about `id` for a long while: it's gone.
fn watch<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let (ctx2, id2) = (ctx.clone(), id.to_string());
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let quiet = match state(&ctx2).jobs.lock().get(&id2) {
            Some(j) if !j.item.finished() => j.heard.elapsed() >= STALE,
            _ => return,
        };
        if quiet {
            finish(
                &ctx2,
                &id2,
                "failed",
                Some("the other device didn't answer".into()),
                None,
            );
            return;
        }
    });
}

/// The downloading device said where it is.
fn on_status<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let new_state = msg.str("state");
    {
        let mut jobs = state(ctx).jobs.lock();
        match jobs.get_mut(&id) {
            Some(j) if !j.item.here && j.item.node_id == from => j.heard = Instant::now(),
            _ => return,
        }
    }
    let known = [
        "checking",
        "asking",
        "queued",
        "downloading",
        "done",
        "failed",
        "cancelled",
        "refused",
    ];
    if !known.contains(&new_state.as_str()) {
        return;
    }
    let name = link_fetch::safe_name(&msg.str("name"));
    let size = msg.json("size").as_u64().unwrap_or(0);
    let done = msg.json("done").as_u64().unwrap_or(0);
    let error = msg.opt("error").map(|e| short(&e, 300));
    let before = item(ctx, &id).map(|i| i.state);
    let ends = matches!(
        new_state.as_str(),
        "done" | "failed" | "cancelled" | "refused"
    );
    let Some(item) = update(ctx, &id, |it| {
        if !msg.str("name").trim().is_empty() {
            it.name = name;
        }
        it.size = size;
        it.done = done;
        if !ends {
            it.state = new_state.clone();
        }
    }) else {
        if ends {
            finish(ctx, &id, &new_state, error, None);
        }
        return;
    };
    if ends {
        finish(ctx, &id, &new_state, error, None);
    } else if before.as_deref() != Some(new_state.as_str()) {
        emit(ctx, &item);
    } else {
        ctx.sink.emit(transfer_event(&item));
    }
}

// --- downloading here -------------------------------------------------------------

fn refuse<P: Platform>(ctx: &NetCtx<P>, from: &str, id: &str, url: &str, why: &str) {
    let item = RemoteDownloadItem {
        id: id.to_string(),
        node_id: from.to_string(),
        here: true,
        host: link_fetch::host_of(url),
        name: guess_name(url),
        state: "refused".into(),
        error: Some(why.to_string()),
        ..RemoteDownloadItem::default()
    };
    report(ctx, &item, true);
}

/// Another device asks this one to download a link.
fn on_request<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    if !super::files::is_transfer_id(&id) || !ctx.peers.lock().contains_key(from) {
        return;
    }
    // Asked again (a resent message): the answer again, if it ended.
    if let Some(it) = item(ctx, &id) {
        if it.finished() && it.here && it.node_id == from {
            report(ctx, &it, true);
        }
        return;
    }
    let url = short(msg.str("url").trim(), link_fetch::MAX_URL);
    let s = settings(ctx);
    if s.policy == "never" {
        return refuse(
            ctx,
            from,
            &id,
            &url,
            "downloads from your devices are turned off on that device",
        );
    }
    let parsed = match link_fetch::check_url(&url, s.allow_lan) {
        Ok(u) => u,
        Err(p) => return refuse(ctx, from, &id, &url, p.reason()),
    };
    if ctx.files.download_dir.lock().is_none() {
        return refuse(ctx, from, &id, &url, "that device has no downloads folder");
    }
    let open = state(ctx)
        .jobs
        .lock()
        .values()
        .filter(|j| j.item.here && j.item.node_id == from && !j.item.finished())
        .count();
    if open >= MAX_OPEN_PER_DEVICE {
        return refuse(
            ctx,
            from,
            &id,
            &url,
            "too many downloads are waiting on that device",
        );
    }
    let now = now_ms();
    let item = RemoteDownloadItem {
        id: id.clone(),
        node_id: from.to_string(),
        here: true,
        host: link_fetch::host_of(parsed.as_str()),
        name: guess_name(parsed.as_str()),
        url: parsed.to_string(),
        state: "checking".into(),
        started_at: now,
        updated_at: now,
        ..RemoteDownloadItem::default()
    };
    let cancel = Arc::new(AtomicBool::new(false));
    state(ctx).jobs.lock().insert(
        id.clone(),
        Job {
            item: item.clone(),
            cancel: cancel.clone(),
            answer: None,
            heard: Instant::now(),
        },
    );
    emit(ctx, &item);
    report(ctx, &item, false);
    let ctx = ctx.clone();
    std::thread::spawn(move || prepare(&ctx, &id, &parsed, &cancel));
}

/// Ask the server about it, then the user if need be, then queue it.
fn prepare<P: Platform>(ctx: &NetCtx<P>, id: &str, url: &url::Url, cancel: &AtomicBool) {
    let s = settings(ctx);
    let opts = FetchOpts {
        allow_lan: s.allow_lan,
        max_bytes: s.max_mb.saturating_mul(MB),
        retries: 0,
        retry_wait: RETRY_WAIT,
        cancel,
    };
    match link_fetch::probe(url, &opts) {
        Ok(p) => {
            update(ctx, id, |it| {
                it.name = p.name;
                it.size = p.size.unwrap_or(0);
            });
        }
        Err(FetchError::Cancelled) => return,
        Err(FetchError::Link(p)) => {
            return finish(ctx, id, "refused", Some(p.reason().into()), None)
        }
        // Many servers don't answer HEAD: the download says the rest.
        Err(_) => {}
    }
    let Some(it) = item(ctx, id) else {
        return;
    };
    if opts.max_bytes > 0 && it.size > opts.max_bytes {
        return finish(
            ctx,
            id,
            "refused",
            Some(FetchError::TooBig(it.size).reason()),
            None,
        );
    }
    if s.policy == "ask" && !s.allowed.contains(&it.node_id) {
        let Some(asking) = update(ctx, id, |it| it.state = "asking".into()) else {
            return;
        };
        emit(ctx, &asking);
        report(ctx, &asking, true);
        let deadline = Instant::now() + ASK_FOR;
        let answer = {
            let st = state(ctx);
            let mut jobs = st.jobs.lock();
            loop {
                match jobs.get(id) {
                    Some(j) if j.item.finished() => return,
                    Some(j) if j.answer.is_some() => break j.answer,
                    Some(_) => {}
                    None => return,
                }
                if Instant::now() >= deadline {
                    break None;
                }
                st.answered.wait_until(&mut jobs, deadline);
            }
        };
        match answer {
            Some(true) => {}
            Some(false) => {
                return finish(
                    ctx,
                    id,
                    "refused",
                    Some("it wasn't allowed on that device".into()),
                    None,
                )
            }
            None => {
                return finish(
                    ctx,
                    id,
                    "failed",
                    Some("nobody answered on that device".into()),
                    None,
                )
            }
        }
    }
    let Some(queued) = update(ctx, id, |it| it.state = "queued".into()) else {
        return;
    };
    state(ctx).queue.lock().push_back(id.to_string());
    emit(ctx, &queued);
    report(ctx, &queued, true);
    pump(ctx);
}

/// Start the next queued download unless one is running.
fn pump<P: Platform>(ctx: &NetCtx<P>) {
    let next = {
        let mut running = state(ctx).running.lock();
        if *running {
            return;
        }
        let next = state(ctx).queue.lock().pop_front();
        if next.is_some() {
            *running = true;
        }
        next
    };
    let Some(id) = next else {
        return;
    };
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        run(&ctx, &id);
        *state(&ctx).running.lock() = false;
        pump(&ctx);
    });
}

/// Download `id` now.
fn run<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let (url, cancel) = match state(ctx).jobs.lock().get(id) {
        Some(j) if !j.item.finished() => (j.item.url.clone(), j.cancel.clone()),
        _ => return,
    };
    let Ok(url) = url::Url::parse(&url) else {
        return finish(ctx, id, "failed", Some("the link isn't valid".into()), None);
    };
    let Some(dir) = ctx.files.download_dir.lock().clone() else {
        return finish(
            ctx,
            id,
            "failed",
            Some("there's no downloads folder".into()),
            None,
        );
    };
    let partial = dir.join(super::files::PARTIAL_DIR);
    if let Err(e) = std::fs::create_dir_all(&partial) {
        return finish(
            ctx,
            id,
            "failed",
            Some(format!("couldn't save the file ({e})")),
            None,
        );
    }
    let part = partial.join(format!("{id}.part"));
    let Some(started) = update(ctx, id, |it| it.state = "downloading".into()) else {
        return;
    };
    emit(ctx, &started);
    report(ctx, &started, true);
    let s = settings(ctx);
    let opts = FetchOpts {
        allow_lan: s.allow_lan,
        max_bytes: s.max_mb.saturating_mul(MB),
        retries: RETRIES,
        retry_wait: RETRY_WAIT,
        cancel: &cancel,
    };
    let (mut shown, mut sent) = (Instant::now(), Instant::now());
    let result = link_fetch::download(&url, &part, &opts, &mut |done, total, name| {
        let Some(it) = update(ctx, id, |it| {
            it.done = done;
            if let Some(t) = total {
                it.size = t;
            }
            if !name.is_empty() {
                it.name = name.to_string();
            }
        }) else {
            return;
        };
        if shown.elapsed() >= PROGRESS_EVERY {
            shown = Instant::now();
            ctx.sink.emit(transfer_event(&it));
        }
        if sent.elapsed() >= STATUS_EVERY {
            sent = Instant::now();
            report(ctx, &it, false);
        }
    });
    match result {
        Ok(got) => {
            update(ctx, id, |it| {
                it.name = got.name.clone();
                it.size = got.size;
            });
            match super::files::move_into(&part, &dir, &got.name) {
                Some(path) => finish(ctx, id, "done", None, Some(path)),
                None => {
                    let _ = std::fs::remove_file(&part);
                    finish(
                        ctx,
                        id,
                        "failed",
                        Some("couldn't save the file in the downloads folder".into()),
                        None,
                    );
                }
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            match e {
                FetchError::Cancelled => finish(ctx, id, "cancelled", None, None),
                FetchError::Link(p) => finish(ctx, id, "refused", Some(p.reason().into()), None),
                e => finish(ctx, id, "failed", Some(e.reason()), None),
            }
        }
    }
}

/// The user here answered the question about `id`: allowed (`always`:
/// from that device from now on, without asking) or not.
pub fn answer<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    allow: bool,
    always: bool,
) -> crate::Result<()> {
    let node = {
        let mut jobs = state(ctx).jobs.lock();
        let job = jobs
            .get_mut(id)
            .filter(|j| j.item.here && j.item.state == "asking")
            .ok_or_else(|| {
                CoreError::InvalidInput("that download isn't waiting for an answer".into())
            })?;
        job.answer = Some(allow);
        job.item.node_id.clone()
    };
    if allow && always {
        let mut s = settings(ctx);
        if !s.allowed.contains(&node) {
            s.allowed.push(node);
            set_settings(ctx, s)?;
        }
    }
    state(ctx).answered.notify_all();
    Ok(())
}

/// Stop `id`, either way, and tell the other device. False if it isn't a
/// download this device knows.
pub fn cancel<P: Platform>(ctx: &NetCtx<P>, id: &str) -> bool {
    let (here, node, finished) = match state(ctx).jobs.lock().get(id) {
        Some(j) => {
            j.cancel.store(true, Ordering::Relaxed);
            (j.item.here, j.item.node_id.clone(), j.item.finished())
        }
        None => return false,
    };
    if finished {
        return true;
    }
    state(ctx).queue.lock().retain(|q| q != id);
    finish(ctx, id, "cancelled", None, None);
    if !here {
        let msg = json!({ "type": "dl_cancel", "from": me(ctx), "id": id });
        crate::engine::send_reliably(ctx, &node, &msg.to_string());
    }
    true
}

pub struct RemoteDownloadHandler;

impl<P: Platform> ControlHandler<P> for RemoteDownloadHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["dl_request", "dl_status", "dl_cancel"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        // Whoever sends these knows them.
        remember(ctx, &from, &json!(VERSION));
        match msg.kind() {
            "dl_request" => on_request(ctx, msg, &from),
            "dl_status" => on_status(ctx, msg, &from),
            "dl_cancel" => {
                let id = msg.str("id");
                let ours = state(ctx)
                    .jobs
                    .lock()
                    .get(&id)
                    .is_some_and(|j| j.item.here && j.item.node_id == from);
                if ours {
                    cancel(ctx, &id);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(state: &str, here: bool) -> RemoteDownloadItem {
        RemoteDownloadItem {
            id: "dl1".into(),
            node_id: "n".into(),
            here,
            name: "a.zip".into(),
            size: 10,
            done: 4,
            state: state.into(),
            error: Some("boom".into()),
            path: Some("/d/a.zip".into()),
            ..RemoteDownloadItem::default()
        }
    }

    #[test]
    fn states_map_to_transfer_events() {
        assert!(matches!(
            transfer_event(&it("downloading", true)),
            CoreEvent::FileProgress {
                incoming: true,
                done: 4,
                total: 10,
                ..
            }
        ));
        assert!(matches!(
            transfer_event(&it("asking", false)),
            CoreEvent::FileProgress {
                incoming: false,
                ..
            }
        ));
        assert!(matches!(
            transfer_event(&it("done", true)),
            CoreEvent::FileReceived { ref path, size: 10, clipboard: false, sync: false, .. } if path == "/d/a.zip"
        ));
        assert!(matches!(
            transfer_event(&it("done", false)),
            CoreEvent::FileSent { .. }
        ));
        assert!(matches!(
            transfer_event(&it("cancelled", true)),
            CoreEvent::FileFailed { ref reason, incoming: true, .. } if reason == "cancelled"
        ));
        assert!(matches!(
            transfer_event(&it("refused", false)),
            CoreEvent::FileFailed { ref reason, incoming: false, .. } if reason == "boom"
        ));
    }

    #[test]
    fn finished_states() {
        for s in ["done", "failed", "cancelled", "refused"] {
            assert!(it(s, true).finished());
        }
        for s in ["requested", "checking", "asking", "queued", "downloading"] {
            assert!(!it(s, true).finished());
        }
    }
}
