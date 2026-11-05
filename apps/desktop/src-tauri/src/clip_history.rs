// Clipboard history: the text this computer and your other devices sent each
// other as clipboard (`send_text` with mode "clipboard", either way), kept so
// it can be copied or sent again later.
//
//  - the last `MAX_ENTRIES` (all devices together), newest first, in
//    `<app config dir>/clipboard-history.json`, written a few seconds after a
//    change (`SAVE_DELAY`) rather than on every clip;
//  - each clip's text is kept up to `MAX_TEXT` bytes; a longer one is cut
//    (on a character boundary) and marked `truncated`;
//  - "Clear" on a device's page forgets that device's clips;
//  - the window hears `clip-history-changed` after each change, and the
//    tray's "Recent clips" shows the last few received.
//
// Copied pictures sent or received (`send_clipboard_image`, or a file that
// arrives with `clipboard: true`) are kept too, as a clip whose text names
// them ("Picture (image), 800 × 600", which is what search finds) plus:
//
//  - the picture itself in `<app config dir>/clip-images/<id>.png` and a
//    thumbnail (at most `THUMB_MAX` px a side) in `<id>.thumb.png`, their
//    size and dimensions in `clip-images/index.json` (`PictureMeta`);
//  - only the newest `MAX_PICTURES` pictures, and no more than
//    `MAX_PICTURE_BYTES` of them in all, whichever comes first; a picture's
//    files go when its clip does (pushed out, cleared, or history off).
//
// "Keep a clipboard history" in Settings turns it off (which also forgets
// everything kept); on by default. In `<app config dir>/prefs.json`, key
// `clip_history` ("on" / "off").
//
// This is separate from the core's in-memory list of synced clipboard values
// (`clipboard_history` in lib.rs), which lasts until the app restarts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager, State};

const FILE: &str = "clipboard-history.json";
/// The kept pictures' folder, in the app config dir.
const PICTURE_DIR: &str = "clip-images";
/// Their sizes, in that folder.
const PICTURE_INDEX: &str = "index.json";
/// The most pictures kept.
pub const MAX_PICTURES: usize = 50;
/// The most bytes of pictures kept, all together.
pub const MAX_PICTURE_BYTES: u64 = 100 * 1024 * 1024;
/// A thumbnail's longer side, at most, in pixels.
pub const THUMB_MAX: u32 = 256;
const PREF_KEY: &str = "clip_history";
/// What the window hears when the history changed.
pub const CHANGED_EVENT: &str = "clip-history-changed";
/// The most clips kept.
pub const MAX_ENTRIES: usize = 200;
/// The most text kept of one clip, in bytes.
pub const MAX_TEXT: usize = 10 * 1024;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// Which way a clip went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ClipDirection {
    /// Another device sent it here.
    In,
    /// This computer sent it.
    Out,
}

/// One clip as kept.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ClipRecord {
    pub id: u32,
    /// Up to `MAX_TEXT` bytes of it.
    pub text: String,
    /// The text was longer and was cut.
    #[serde(default)]
    pub truncated: bool,
    /// The device it came from (node id), for one received.
    #[serde(default)]
    pub from_node: Option<String>,
    /// The device it went to (node id), for one sent.
    #[serde(default)]
    pub to_node: Option<String>,
    pub direction: ClipDirection,
    /// When, in milliseconds since 1970.
    pub at: f64,
}

impl ClipRecord {
    /// The other device: where it came from or went to.
    pub fn node(&self) -> Option<&str> {
        match self.direction {
            ClipDirection::In => self.from_node.as_deref(),
            ClipDirection::Out => self.to_node.as_deref(),
        }
    }
}

/// What a device's clipboard tile shows: the first matches, how many there
/// are, and how many this device has kept in all.
/// `pictures`: the ones among `entries` that are pictures.
#[derive(serde::Serialize, specta::Type)]
pub struct ClipPage {
    pub entries: Vec<ClipRecord>,
    pub total: u32,
    pub kept: u32,
    pub pictures: Vec<ClipPicture>,
}

/// A kept picture's size, as in `clip-images/index.json`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PictureMeta {
    /// The clip's id.
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// The PNG's size on disk.
    pub bytes: u64,
}

/// A kept picture as a clipboard tile shows it.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct ClipPicture {
    /// The clip's id.
    pub id: u32,
    pub width: u32,
    pub height: u32,
    /// The PNG's size on disk.
    pub bytes: f64,
    /// Its thumbnail as a `data:image/png` URL, if there is one.
    pub thumb: Option<String>,
}

/// The text a picture's clip is kept with: what search finds it by.
pub fn picture_text(width: u32, height: u32) -> String {
    format!("Picture (image), {width} \u{d7} {height}")
}

/// The pictures in `pics` (newest first) past the caps: beyond the newest
/// `max_count`, or once together they'd be over `max_bytes`.
pub fn over_cap(pics: &[PictureMeta], max_count: usize, max_bytes: u64) -> Vec<u32> {
    let mut total = 0u64;
    let mut out = Vec::new();
    for (i, p) in pics.iter().enumerate() {
        total = total.saturating_add(p.bytes);
        if i >= max_count || total > max_bytes {
            out.push(p.id);
        }
    }
    out
}

/// Keep `list` and `pics` in step: pictures whose clip is gone go, and so do
/// pictures (and their clips) past the caps. The ids of pictures dropped,
/// whose files are to go.
pub fn prune(
    list: &mut Vec<ClipRecord>,
    pics: &mut Vec<PictureMeta>,
    max_count: usize,
    max_bytes: u64,
) -> Vec<u32> {
    let mut dropped: Vec<u32> = pics
        .iter()
        .filter(|p| !list.iter().any(|e| e.id == p.id))
        .map(|p| p.id)
        .collect();
    pics.retain(|p| !dropped.contains(&p.id));
    let over = over_cap(pics, max_count, max_bytes);
    pics.retain(|p| !over.contains(&p.id));
    list.retain(|e| !over.contains(&e.id));
    dropped.extend(over);
    dropped
}

/// The kept pictures' files: `<dir>/<id>.png`, `<dir>/<id>.thumb.png`, and
/// `<dir>/index.json`.
pub struct PictureStore {
    dir: PathBuf,
}

impl PictureStore {
    pub fn new(dir: PathBuf) -> Self {
        PictureStore { dir }
    }

    pub fn image(&self, id: u32) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }

    pub fn thumb(&self, id: u32) -> PathBuf {
        self.dir.join(format!("{id}.thumb.png"))
    }

    /// Keep a copy of the PNG at `src` as picture `id`, with its thumbnail.
    pub fn put(&self, id: u32, src: &Path) -> Result<PictureMeta, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let (w, h, rgba) = crate::clip_image::read_png(src)?;
        let width = u32::try_from(w).map_err(|_| "picture too big")?;
        let height = u32::try_from(h).map_err(|_| "picture too big")?;
        let image = self.image(id);
        let bytes = std::fs::copy(src, &image).map_err(|e| e.to_string())?;
        let (tw, th, small) = crate::clip_image::thumbnail(width, height, &rgba, THUMB_MAX);
        if let Err(e) = crate::clip_image::write_png(&self.thumb(id), tw, th, &small) {
            // The list shows it without one.
            eprintln!("[clips] couldn't make a thumbnail: {e}");
        }
        Ok(PictureMeta {
            id,
            width,
            height,
            bytes,
        })
    }

    /// Forget picture `id`'s files.
    pub fn remove(&self, id: u32) {
        let _ = std::fs::remove_file(self.image(id));
        let _ = std::fs::remove_file(self.thumb(id));
    }

    /// Delete every file in the folder that isn't one of `keep`'s (the
    /// index stays).
    pub fn sweep(&self, keep: &[u32]) {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for f in dir.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if name == PICTURE_INDEX {
                continue;
            }
            let id = name.split('.').next().and_then(|n| n.parse::<u32>().ok());
            let ok = id.is_some_and(|id| {
                keep.contains(&id)
                    && (name == format!("{id}.png") || name == format!("{id}.thumb.png"))
            });
            if !ok {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }

    /// The sizes kept in the index (none when there's no index).
    pub fn load_index(&self) -> Vec<PictureMeta> {
        std::fs::read_to_string(self.dir.join(PICTURE_INDEX))
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save_index(&self, pics: &[PictureMeta]) -> std::io::Result<()> {
        let json = serde_json::to_string(pics).map_err(std::io::Error::other)?;
        write_atomic(&self.dir.join(PICTURE_INDEX), &json)
    }

    /// The file at `path` as a `data:image/png` URL.
    fn data_url(path: &Path) -> Option<String> {
        let bytes = std::fs::read(path).ok()?;
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

/// `text` cut to at most `max` bytes on a character boundary, and whether it
/// was cut.
pub fn cap_text(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// Keep `entry` first in `list`, dropping the oldest past `max`.
pub fn record(list: &mut Vec<ClipRecord>, entry: ClipRecord, max: usize) {
    list.insert(0, entry);
    list.truncate(max);
}

/// Whether `e` matches a search: every word of `query` appears in its text,
/// ignoring case. An empty query matches everything.
pub fn matches(e: &ClipRecord, query: &str) -> bool {
    let hay = e.text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| hay.contains(word))
}

/// `node`'s clips (either way) matching `query`, newest first.
pub fn search<'a>(
    list: &'a [ClipRecord],
    node: &'a str,
    query: &'a str,
) -> impl Iterator<Item = &'a ClipRecord> + 'a {
    list.iter()
        .filter(move |e| e.node() == Some(node))
        .filter(move |e| matches(e, query))
}

/// A clip as one short menu line: its first non-blank line, at most `max`
/// characters (an ellipsis when there's more).
pub fn preview(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or("");
    let more = line.chars().count() > max || text.trim().lines().nth(1).is_some();
    let mut out: String = line.chars().take(max).collect();
    if more {
        out.push('\u{2026}');
    }
    out
}

struct Inner {
    list: Vec<ClipRecord>,
    /// The pictures among `list`, newest first.
    pictures: Vec<PictureMeta>,
    next_id: u32,
}

/// The clipboard history and its on/off switch.
pub struct ClipHistory {
    /// `<app config dir>/clipboard-history.json`.
    path: Option<PathBuf>,
    /// `<app config dir>/clip-images`.
    store: Option<PictureStore>,
    prefs: Option<FileKv>,
    on: AtomicBool,
    inner: Mutex<Inner>,
    /// A write is already on its way.
    save_pending: AtomicBool,
}

impl ClipHistory {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let path = dir.as_ref().map(|d| d.join(FILE));
        let store = dir.as_ref().map(|d| PictureStore::new(d.join(PICTURE_DIR)));
        let prefs = dir.map(|d| FileKv::new(d.join("prefs.json")));
        let on = prefs.as_ref().and_then(|kv| kv.get(PREF_KEY)).as_deref() != Some("off");
        let mut list: Vec<ClipRecord> = if on {
            path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        list.truncate(MAX_ENTRIES);
        let mut pictures = Vec::new();
        if let Some(store) = &store {
            if on {
                pictures = store.load_index();
                pictures.retain(|p| store.image(p.id).is_file());
                prune(&mut list, &mut pictures, MAX_PICTURES, MAX_PICTURE_BYTES);
            }
            // Files of pictures no longer kept (or all, when it's off).
            let keep: Vec<u32> = pictures.iter().map(|p| p.id).collect();
            store.sweep(&keep);
        }
        let next_id = list.iter().map(|e| e.id).max().unwrap_or(0).wrapping_add(1);
        ClipHistory {
            path,
            store,
            prefs,
            on: AtomicBool::new(on),
            inner: Mutex::new(Inner {
                list,
                pictures,
                next_id,
            }),
            save_pending: AtomicBool::new(false),
        }
    }

    pub fn enabled(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// A copy of every kept clip, newest first (for search.rs).
    pub fn snapshot(&self) -> Vec<ClipRecord> {
        self.inner
            .lock()
            .map(|i| i.list.clone())
            .unwrap_or_default()
    }

    /// Write the file once `SAVE_DELAY` has passed, folding in whatever else
    /// changes meanwhile.
    fn save_soon(&self, app: &AppHandle) {
        if self.save_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            if let Some(h) = app.try_state::<ClipHistory>() {
                h.save_pending.store(false, Ordering::Release);
                h.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let (json, pictures) = match self.inner.lock() {
            Ok(i) => (serde_json::to_string(&i.list), i.pictures.clone()),
            Err(_) => return,
        };
        let Ok(json) = json else {
            return;
        };
        // The pictures first: a clip whose picture isn't in the index would
        // come back as just its text.
        if let Some(store) = &self.store {
            if let Err(e) = store.save_index(&pictures) {
                eprintln!("[clips] couldn't save the pictures' index: {e}");
            }
        }
        if let Err(e) = write_atomic(path, &json) {
            eprintln!("[clips] couldn't save the clipboard history: {e}");
        }
    }

    /// Keep a clip, if the history is on. Whether it was kept.
    fn add(&self, direction: ClipDirection, node: &str, text: &str) -> bool {
        if !self.enabled() || text.trim().is_empty() {
            return false;
        }
        let Ok(mut i) = self.inner.lock() else {
            return false;
        };
        let (text, truncated) = cap_text(text, MAX_TEXT);
        let id = i.next_id;
        i.next_id = i.next_id.wrapping_add(1);
        let node = Some(node.to_string());
        let (from_node, to_node) = match direction {
            ClipDirection::In => (node, None),
            ClipDirection::Out => (None, node),
        };
        let entry = ClipRecord {
            id,
            text,
            truncated,
            from_node,
            to_node,
            direction,
            at: now_ms(),
        };
        record(&mut i.list, entry, MAX_ENTRIES);
        let dropped = prune_inner(&mut i);
        drop(i);
        self.forget_files(&dropped);
        true
    }

    /// Keep a copy of the PNG at `src` as a picture clip, if the history is
    /// on. Whether it was kept. Copies and shrinks it: not on the main thread.
    fn add_picture(&self, direction: ClipDirection, node: &str, src: &Path) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        if !self.enabled() {
            return false;
        }
        let id = match self.inner.lock() {
            Ok(mut i) => {
                let id = i.next_id;
                i.next_id = i.next_id.wrapping_add(1);
                id
            }
            Err(_) => return false,
        };
        let meta = match store.put(id, src) {
            Ok(m) if m.bytes <= MAX_PICTURE_BYTES => m,
            Ok(_) => {
                store.remove(id);
                return false;
            }
            Err(e) => {
                eprintln!("[clips] couldn't keep the picture: {e}");
                store.remove(id);
                return false;
            }
        };
        let Ok(mut i) = self.inner.lock() else {
            store.remove(id);
            return false;
        };
        // Turned off meanwhile.
        if !self.enabled() {
            drop(i);
            store.remove(id);
            return false;
        }
        let node = Some(node.to_string());
        let (from_node, to_node) = match direction {
            ClipDirection::In => (node, None),
            ClipDirection::Out => (None, node),
        };
        let entry = ClipRecord {
            id,
            text: picture_text(meta.width, meta.height),
            truncated: false,
            from_node,
            to_node,
            direction,
            at: now_ms(),
        };
        record(&mut i.list, entry, MAX_ENTRIES);
        i.pictures.insert(0, meta);
        let dropped = prune_inner(&mut i);
        drop(i);
        self.forget_files(&dropped);
        true
    }

    fn forget_files(&self, ids: &[u32]) {
        if let Some(store) = &self.store {
            for &id in ids {
                store.remove(id);
            }
        }
    }

    /// Kept picture `id`'s PNG, if it's still kept.
    fn picture_file(&self, id: u32) -> Option<PathBuf> {
        let store = self.store.as_ref()?;
        let i = self.inner.lock().ok()?;
        i.pictures
            .iter()
            .any(|p| p.id == id)
            .then(|| store.image(id))
    }
}

/// `prune` with the real caps.
fn prune_inner(i: &mut Inner) -> Vec<u32> {
    prune(
        &mut i.list,
        &mut i.pictures,
        MAX_PICTURES,
        MAX_PICTURE_BYTES,
    )
}

/// Write to a temporary file, then rename it over `path`.
fn write_atomic(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Saved soon, the window told, and the tray's recent clips rebuilt.
fn changed(app: &AppHandle, h: &ClipHistory) {
    h.save_soon(app);
    let _ = app.emit(CHANGED_EVENT, ());
    crate::tray::refresh(app);
}

/// Text arrived from `from` as clipboard.
pub fn on_received(app: &AppHandle, from: &str, text: &str) {
    if let Some(h) = app.try_state::<ClipHistory>() {
        if h.add(ClipDirection::In, from, text) {
            changed(app, &h);
        }
    }
}

/// This computer sent `text` to `to` as clipboard.
pub fn on_sent(app: &AppHandle, to: &str, text: &str) {
    if let Some(h) = app.try_state::<ClipHistory>() {
        if h.add(ClipDirection::Out, to, text) {
            changed(app, &h);
        }
    }
}

/// A copied picture (the PNG at `path`) arrived from `from` (`In`) or this
/// computer sent it to `to` (`Out`): kept, off the calling thread.
pub fn on_picture(app: &AppHandle, direction: ClipDirection, node: &str, path: &Path) {
    let (app, node, path) = (app.clone(), node.to_string(), path.to_path_buf());
    std::thread::spawn(move || {
        if let Some(h) = app.try_state::<ClipHistory>() {
            if h.add_picture(direction, &node, &path) {
                changed(&app, &h);
            }
        }
    });
}

/// The kept picture clip `id`'s PNG, if it is one and still kept.
pub fn picture_of(app: &AppHandle, id: u32) -> Option<PathBuf> {
    app.try_state::<ClipHistory>()?.picture_file(id)
}

/// The last `n` clips received (from any device), newest first.
pub fn recent_received(app: &AppHandle, n: usize) -> Vec<ClipRecord> {
    newest(app, n, true)
}

/// The last `n` clips either way (any device), newest first: the quick
/// panel's "Recent".
pub fn recent(app: &AppHandle, n: usize) -> Vec<ClipRecord> {
    newest(app, n, false)
}

fn newest(app: &AppHandle, n: usize, received_only: bool) -> Vec<ClipRecord> {
    let Some(h) = app.try_state::<ClipHistory>() else {
        return Vec::new();
    };
    h.inner
        .lock()
        .map(|i| {
            i.list
                .iter()
                .filter(|e| !received_only || e.direction == ClipDirection::In)
                .take(n)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The kept clip `id`'s text, if it's still kept.
pub fn text_of(app: &AppHandle, id: u32) -> Option<String> {
    let h = app.try_state::<ClipHistory>()?;
    let i = h.inner.lock().ok()?;
    i.list.iter().find(|e| e.id == id).map(|e| e.text.clone())
}

/// Device `node_id`'s kept clips (sent and received), newest first: the
/// first `limit` matching `query` (every word in the text, ignoring case).
#[tauri::command]
#[specta::specta]
pub fn clip_history(
    node_id: String,
    query: String,
    limit: u32,
    history: State<ClipHistory>,
) -> ClipPage {
    let Ok(i) = history.inner.lock() else {
        return ClipPage {
            entries: Vec::new(),
            total: 0,
            kept: 0,
            pictures: Vec::new(),
        };
    };
    let mut entries = Vec::new();
    let mut total = 0u32;
    for e in search(&i.list, &node_id, &query) {
        if total < limit.min(MAX_ENTRIES as u32) {
            entries.push(e.clone());
        }
        total += 1;
    }
    let kept = i
        .list
        .iter()
        .filter(|e| e.node() == Some(node_id.as_str()))
        .count() as u32;
    let metas: Vec<PictureMeta> = i
        .pictures
        .iter()
        .filter(|p| entries.iter().any(|e| e.id == p.id))
        .cloned()
        .collect();
    drop(i);
    // Thumbnails read with the list let go.
    let pictures = metas
        .into_iter()
        .map(|p| ClipPicture {
            id: p.id,
            width: p.width,
            height: p.height,
            bytes: p.bytes as f64,
            thumb: history
                .store
                .as_ref()
                .and_then(|s| PictureStore::data_url(&s.thumb(p.id))),
        })
        .collect();
    ClipPage {
        entries,
        total,
        kept,
        pictures,
    }
}

/// The kept picture `id` (`clip_history`'s pictures), or why not.
fn kept_picture(history: &ClipHistory, id: u32) -> Result<PathBuf, String> {
    history
        .picture_file(id)
        .filter(|p| p.is_file())
        .ok_or_else(|| "That picture isn't kept any more.".to_string())
}

/// Kept picture `id` at full size, as a `data:image/png` URL (to look at it).
#[tauri::command]
#[specta::specta]
pub fn clip_picture(id: u32, history: State<ClipHistory>) -> Result<String, String> {
    let path = kept_picture(&history, id)?;
    PictureStore::data_url(&path).ok_or_else(|| "Couldn't read the picture.".to_string())
}

/// Put kept picture `id` on this computer's clipboard.
#[tauri::command]
#[specta::specta]
pub fn copy_clip_picture(id: u32, history: State<ClipHistory>) -> Result<(), String> {
    let path = kept_picture(&history, id)?;
    crate::clip_image::put_on_clipboard(&path)
}

/// Send kept picture `id` to device `node_id`'s clipboard again. Returns the
/// transfer id (progress as `file-progress`); kept again as sent.
#[tauri::command]
#[specta::specta]
pub fn send_clip_picture(
    node_id: String,
    id: u32,
    app: AppHandle,
    history: State<ClipHistory>,
    engine: State<crate::Eng>,
) -> Result<String, String> {
    let path = kept_picture(&history, id)?;
    let copy = crate::clip_image::temp_copy(&path)?;
    let sent = engine
        .send_clipboard_image(&node_id, &copy.to_string_lossy())
        .map_err(|e| e.to_string())?;
    on_picture(&app, ClipDirection::Out, &node_id, &copy);
    Ok(sent)
}

/// Save kept picture `id` where the user picks (the save dialog). The path
/// it went to, or `None` when the dialog was closed.
#[tauri::command]
#[specta::specta]
pub async fn save_clip_picture(id: u32, app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let src = {
        let history = app.state::<ClipHistory>();
        kept_picture(&history, id)?
    };
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_file_name(format!("Picture {id}.png"))
        .add_filter("PNG", &["png"])
        .save_file(move |file| {
            let _ = tx.send(file.and_then(|f| f.into_path().ok()));
        });
    let dest = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten();
    let Some(dest) = dest else {
        return Ok(None);
    };
    std::fs::copy(&src, &dest).map_err(|e| format!("Couldn't save the picture: {e}"))?;
    Ok(Some(dest.to_string_lossy().into_owned()))
}

/// Forget device `node_id`'s kept clips.
#[tauri::command]
#[specta::specta]
pub fn clear_clip_history(node_id: String, app: AppHandle, history: State<ClipHistory>) {
    let dropped = match history.inner.lock() {
        Ok(mut i) => {
            i.list.retain(|e| e.node() != Some(node_id.as_str()));
            prune_inner(&mut i)
        }
        Err(_) => Vec::new(),
    };
    history.forget_files(&dropped);
    changed(&app, &history);
}

/// Whether clips are kept ("Keep a clipboard history" in Settings).
#[tauri::command]
#[specta::specta]
pub fn clip_history_enabled(history: State<ClipHistory>) -> bool {
    history.enabled()
}

/// Turn the history on or off; off also forgets everything kept.
#[tauri::command]
#[specta::specta]
pub fn set_clip_history_enabled(on: bool, app: AppHandle, history: State<ClipHistory>) {
    history.on.store(on, Ordering::Relaxed);
    if let Some(kv) = &history.prefs {
        if let Err(e) = kv.set(PREF_KEY, if on { "on" } else { "off" }) {
            eprintln!("[clips] couldn't save the setting: {e}");
        }
    }
    if !on {
        if let Ok(mut i) = history.inner.lock() {
            i.list.clear();
            i.pictures.clear();
        }
        // Gone from the disk now, not in a few seconds.
        history.save_now();
        if let Some(store) = &history.store {
            store.sweep(&[]);
        }
        let _ = app.emit(CHANGED_EVENT, ());
        crate::tray::refresh(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u32, dir: ClipDirection, node: &str, text: &str) -> ClipRecord {
        let node = Some(node.to_string());
        let (from_node, to_node) = match dir {
            ClipDirection::In => (node, None),
            ClipDirection::Out => (None, node),
        };
        ClipRecord {
            id,
            text: text.into(),
            truncated: false,
            from_node,
            to_node,
            direction: dir,
            at: f64::from(id),
        }
    }

    #[test]
    fn keeps_the_newest_up_to_the_cap() {
        let mut list = Vec::new();
        for id in 1..=7 {
            record(&mut list, clip(id, ClipDirection::In, "p", "hi"), 5);
        }
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [7, 6, 5, 4, 3]);
    }

    #[test]
    fn the_real_cap_is_200() {
        let mut list = Vec::new();
        for id in 1..=250 {
            record(
                &mut list,
                clip(id, ClipDirection::Out, "p", "x"),
                MAX_ENTRIES,
            );
        }
        assert_eq!(list.len(), 200);
        assert_eq!(list[0].id, 250);
        assert_eq!(list[199].id, 51);
    }

    #[test]
    fn long_text_is_cut_and_marked() {
        assert_eq!(cap_text("short", 10), ("short".to_string(), false));
        assert_eq!(
            cap_text("exactly10!", 10),
            ("exactly10!".to_string(), false)
        );
        assert_eq!(
            cap_text("0123456789abc", 10),
            ("0123456789".to_string(), true)
        );
        let big = "a".repeat(MAX_TEXT + 1);
        let (kept, cut) = cap_text(&big, MAX_TEXT);
        assert!(cut);
        assert_eq!(kept.len(), 10 * 1024);
    }

    #[test]
    fn cutting_keeps_whole_characters() {
        // "é" is two bytes: a cut through it backs off to before it.
        let (kept, cut) = cap_text("aé", 2);
        assert_eq!((kept.as_str(), cut), ("a", true));
        // "🙂" is four bytes.
        let (kept, cut) = cap_text("🙂🙂", 6);
        assert_eq!((kept.as_str(), cut), ("🙂", true));
    }

    #[test]
    fn search_matches_every_word_ignoring_case() {
        let e = clip(1, ClipDirection::In, "p", "Meet at the Blue Door, 8pm");
        assert!(matches(&e, ""));
        assert!(matches(&e, "   "));
        assert!(matches(&e, "blue"));
        assert!(matches(&e, "DOOR blue"));
        assert!(!matches(&e, "blue window"));
    }

    #[test]
    fn search_keeps_to_the_device_either_way() {
        let list = vec![
            clip(5, ClipDirection::Out, "p", "hello sent"),
            clip(4, ClipDirection::In, "q", "hello other"),
            clip(3, ClipDirection::In, "p", "hello received"),
            clip(2, ClipDirection::In, "p", "bye"),
            // A sent clip names its device in `to_node` only.
            ClipRecord {
                from_node: Some("p".into()),
                ..clip(1, ClipDirection::Out, "q", "to q")
            },
        ];
        let ids = |it: Vec<&ClipRecord>| it.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(search(&list, "p", "").collect()), [5, 3, 2]);
        assert_eq!(ids(search(&list, "p", "hello").collect()), [5, 3]);
        assert_eq!(ids(search(&list, "q", "").collect()), [4, 1]);
        assert_eq!(ids(search(&list, "x", "").collect()), Vec::<u32>::new());
    }

    #[test]
    fn menu_preview_is_one_short_line() {
        assert_eq!(preview("hello", 10), "hello");
        assert_eq!(
            preview("  \n  first line \nsecond", 20),
            "first line\u{2026}"
        );
        assert_eq!(preview("abcdefghijkl", 5), "abcde\u{2026}");
        assert_eq!(preview("", 5), "");
    }

    fn pic(id: u32, bytes: u64) -> PictureMeta {
        PictureMeta {
            id,
            width: 1,
            height: 1,
            bytes,
        }
    }

    /// A fresh folder for one test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "md-clip-pictures-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `w` x `h` PNG at `path`.
    fn png_at(path: &Path, w: u32, h: u32) {
        let px = vec![90u8; (w * h * 4) as usize];
        crate::clip_image::write_png(path, w, h, &px).unwrap();
    }

    #[test]
    fn pictures_are_capped_by_count() {
        let pics: Vec<PictureMeta> = (1..=60).rev().map(|id| pic(id, 10)).collect();
        let over = over_cap(&pics, MAX_PICTURES, MAX_PICTURE_BYTES);
        // The newest 50 (60..=11) stay; 10..=1 go.
        assert_eq!(over, (1..=10).rev().collect::<Vec<_>>());
        assert!(over_cap(&pics[..50], MAX_PICTURES, MAX_PICTURE_BYTES).is_empty());
    }

    #[test]
    fn pictures_are_capped_by_bytes() {
        let mb = 1024 * 1024;
        // Newest first: 40 + 40 + 30 MB is over 100 MB: the third goes, and
        // every older one with it.
        let pics = vec![
            pic(5, 40 * mb),
            pic(4, 40 * mb),
            pic(3, 30 * mb),
            pic(2, mb),
            pic(1, mb),
        ];
        assert_eq!(over_cap(&pics, MAX_PICTURES, MAX_PICTURE_BYTES), [3, 2, 1]);
        // Exactly 100 MB is fine.
        let pics = vec![pic(2, 60 * mb), pic(1, 40 * mb)];
        assert!(over_cap(&pics, MAX_PICTURES, MAX_PICTURE_BYTES).is_empty());
        // Whichever comes first: two pictures, a cap of one.
        assert_eq!(over_cap(&pics, 1, MAX_PICTURE_BYTES), [1]);
    }

    #[test]
    fn prune_keeps_clips_and_pictures_in_step() {
        let mut list = vec![
            clip(4, ClipDirection::In, "p", "Picture (image), 1 × 1"),
            clip(3, ClipDirection::Out, "p", "text"),
            clip(2, ClipDirection::In, "p", "Picture (image), 1 × 1"),
        ];
        // Picture 1's clip already fell out of the list.
        let mut pics = vec![pic(4, 10), pic(2, 10), pic(1, 10)];
        let dropped = prune(&mut list, &mut pics, 1, MAX_PICTURE_BYTES);
        assert_eq!(dropped, [1, 2]);
        assert_eq!(pics, [pic(4, 10)]);
        // Picture 2 went past the count cap: its clip goes too.
        assert_eq!(list.iter().map(|e| e.id).collect::<Vec<_>>(), [4, 3]);
    }

    #[test]
    fn a_kept_picture_has_a_small_thumbnail() {
        let dir = temp_dir("thumb");
        let src = dir.join("src.png");
        png_at(&src, 1000, 400);
        let store = PictureStore::new(dir.join(PICTURE_DIR));
        let meta = store.put(7, &src).unwrap();
        assert_eq!((meta.width, meta.height), (1000, 400));
        assert_eq!(meta.bytes, std::fs::metadata(&src).unwrap().len());
        assert!(store.image(7).is_file());
        let (w, h, _) = crate::clip_image::read_png(&store.thumb(7)).unwrap();
        assert_eq!((w, h), (256, 102));
        assert!(PictureStore::data_url(&store.thumb(7))
            .unwrap()
            .starts_with("data:image/png;base64,"));
        // Not a PNG: nothing kept.
        std::fs::write(dir.join("bad.png"), b"nope").unwrap();
        assert!(store.put(8, &dir.join("bad.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_go_when_pictures_fall_out_or_are_cleared() {
        let dir = temp_dir("evict");
        let src = dir.join("src.png");
        png_at(&src, 4, 4);
        let store = PictureStore::new(dir.join(PICTURE_DIR));
        let mut list = Vec::new();
        let mut pics = Vec::new();
        for id in 1..=3 {
            let meta = store.put(id, &src).unwrap();
            record(
                &mut list,
                clip(id, ClipDirection::In, "p", &picture_text(4, 4)),
                MAX_ENTRIES,
            );
            pics.insert(0, meta);
            // A cap of two pictures.
            for gone in prune(&mut list, &mut pics, 2, MAX_PICTURE_BYTES) {
                store.remove(gone);
            }
        }
        assert!(!store.image(1).exists() && !store.thumb(1).exists());
        assert!(store.image(2).is_file() && store.image(3).is_file());
        assert_eq!(list.len(), 2);

        // Cleared ("Clear" on the device): its pictures' files go.
        list.retain(|e| e.node() != Some("p"));
        for gone in prune(&mut list, &mut pics, 2, MAX_PICTURE_BYTES) {
            store.remove(gone);
        }
        assert!(pics.is_empty());
        assert!(!store.image(2).exists() && !store.image(3).exists());
        assert!(!store.thumb(2).exists() && !store.thumb(3).exists());

        // History off (or files no one knows of on start): swept, the
        // index left alone.
        store.put(9, &src).unwrap();
        store.save_index(&[pic(9, 1)]).unwrap();
        std::fs::write(dir.join(PICTURE_DIR).join("stray.txt"), b"x").unwrap();
        store.sweep(&[9]);
        assert!(store.image(9).is_file());
        assert!(!dir.join(PICTURE_DIR).join("stray.txt").exists());
        store.sweep(&[]);
        assert!(!store.image(9).exists() && !store.thumb(9).exists());
        assert_eq!(store.load_index(), [pic(9, 1)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picture_clips_are_found_as_pictures_or_images() {
        let e = clip(1, ClipDirection::In, "p", &picture_text(800, 600));
        assert!(matches(&e, "picture"));
        assert!(matches(&e, "IMAGE"));
        assert!(!matches(&e, "photo"));
    }

    #[test]
    fn the_file_round_trips() {
        let list = vec![
            clip(2, ClipDirection::Out, "p", "sent"),
            ClipRecord {
                truncated: true,
                ..clip(1, ClipDirection::In, "q", "got")
            },
        ];
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"direction\":\"out\""));
        let back: Vec<ClipRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, list);
    }
}
