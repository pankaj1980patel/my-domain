// How each device's page is arranged: which tiles come first and which are
// hidden ("Edit page" on a device's page).
//
//  - one layout per kind of device ("phone" / "computer"), and optionally one
//    for a single device ("Only for this device", by node id) that wins over
//    its kind's;
//  - a layout lists tile ids; the page passes the tiles it has now and gets
//    them back in the saved order (`merge`): a tile the layout doesn't know
//    (one added in a later version) appears at its default place, and ids
//    the page no longer has are ignored, so nothing is ever lost;
//  - kept in `<app config dir>/page-layout.json`, written shortly after a
//    change (`SAVE_DELAY`, temporary file then rename) so a drag that moves
//    a tile several places writes once.
//
// Local to this computer; never synced.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager, State};

const FILE: &str = "page-layout.json";
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_millis(800);
/// The most tiles a layout lists.
const MAX_TILES: usize = 100;
/// The longest tile id kept.
const MAX_ID: usize = 64;
/// The most devices with a layout of their own.
const MAX_DEVICES: usize = 200;

/// A page's arrangement: tile ids in order, and the hidden ones.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct PageLayout {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

/// Where the layout a page shows came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LayoutScope {
    /// Nothing saved: the page's own order.
    Default,
    /// The layout for every device of this kind.
    Kind,
    /// This device's own layout ("Only for this device").
    Device,
}

/// What a device's page shows: every tile it passed, in order, the hidden
/// ones, and where that came from.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct PageLayoutView {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub scope: LayoutScope,
}

/// The file: per kind, and per device.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Saved {
    #[serde(default)]
    kinds: BTreeMap<String, PageLayout>,
    #[serde(default)]
    devices: BTreeMap<String, PageLayout>,
}

/// "phone" stays "phone"; anything else is a computer.
pub fn kind_of(kind: &str) -> &'static str {
    if kind == "phone" {
        "phone"
    } else {
        "computer"
    }
}

/// `layout` made safe to keep: blank, overlong and repeated ids dropped,
/// at most `MAX_TILES` of each list.
pub fn clean(layout: PageLayout) -> PageLayout {
    let tidy = |ids: Vec<String>| {
        let mut seen = HashSet::new();
        ids.into_iter()
            .filter(|id| !id.trim().is_empty() && id.len() <= MAX_ID)
            .filter(|id| seen.insert(id.clone()))
            .take(MAX_TILES)
            .collect::<Vec<_>>()
    };
    PageLayout {
        order: tidy(layout.order),
        hidden: tidy(layout.hidden),
    }
}

/// `tiles` (the page's tiles, in their default order) arranged by `saved`:
/// the saved order first (ids not in `tiles` skipped), each tile `saved`
/// doesn't list placed right after the tile that comes before it by default
/// (first if none does). Hidden: those of `saved.hidden` that are tiles.
pub fn merge(saved: &PageLayout, tiles: &[String]) -> PageLayout {
    let known: HashSet<&str> = tiles.iter().map(String::as_str).collect();
    let mut order: Vec<String> = Vec::with_capacity(tiles.len());
    for id in &saved.order {
        if known.contains(id.as_str()) && !order.contains(id) {
            order.push(id.clone());
        }
    }
    for (i, id) in tiles.iter().enumerate() {
        if order.contains(id) {
            continue;
        }
        // The nearest tile before it by default that's already placed.
        let at = tiles[..i]
            .iter()
            .rev()
            .find_map(|prev| order.iter().position(|o| o == prev))
            .map_or(0, |p| p + 1);
        order.insert(at, id.clone());
    }
    let mut hidden: Vec<String> = Vec::new();
    for id in &saved.hidden {
        if known.contains(id.as_str()) && !hidden.contains(id) {
            hidden.push(id.clone());
        }
    }
    PageLayout { order, hidden }
}

impl Saved {
    /// The layout `node` (of `kind`) uses, and where it came from.
    fn pick(&self, kind: &str, node: &str) -> (Option<&PageLayout>, LayoutScope) {
        if let Some(l) = self.devices.get(node) {
            (Some(l), LayoutScope::Device)
        } else if let Some(l) = self.kinds.get(kind_of(kind)) {
            (Some(l), LayoutScope::Kind)
        } else {
            (None, LayoutScope::Default)
        }
    }

    /// What `node`'s page shows, given its `tiles` in default order.
    fn view(&self, kind: &str, node: &str, tiles: &[String]) -> PageLayoutView {
        let (layout, scope) = self.pick(kind, node);
        let merged = merge(layout.unwrap_or(&PageLayout::default()), tiles);
        PageLayoutView {
            order: merged.order,
            hidden: merged.hidden,
            scope,
        }
    }

    /// Keep `layout` for `node` alone (`only_this`) or for every device of
    /// `kind` (then `node` follows it too: its own layout goes). `None`
    /// resets: with `only_this`, `node` follows its kind's layout again;
    /// otherwise the kind's layout and `node`'s own both go (the default).
    fn apply(&mut self, kind: &str, node: &str, only_this: bool, layout: Option<PageLayout>) {
        let layout = layout.map(clean);
        if only_this {
            match layout {
                Some(l) => {
                    if !self.devices.contains_key(node) && self.devices.len() >= MAX_DEVICES {
                        return;
                    }
                    self.devices.insert(node.to_string(), l);
                }
                None => {
                    self.devices.remove(node);
                }
            }
        } else {
            self.devices.remove(node);
            match layout {
                Some(l) => {
                    self.kinds.insert(kind_of(kind).to_string(), l);
                }
                None => {
                    self.kinds.remove(kind_of(kind));
                }
            }
        }
    }
}

/// The saved layouts.
pub struct PageLayouts {
    /// `<app config dir>/page-layout.json`.
    path: Option<PathBuf>,
    saved: Mutex<Saved>,
    /// A write is already on its way.
    save_pending: AtomicBool,
}

impl PageLayouts {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_config_dir().ok().map(|d| d.join(FILE));
        let saved = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        PageLayouts {
            path,
            saved: Mutex::new(saved),
            save_pending: AtomicBool::new(false),
        }
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
            if let Some(h) = app.try_state::<PageLayouts>() {
                h.save_pending.store(false, Ordering::Release);
                h.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let json = match self.saved.lock() {
            Ok(s) => serde_json::to_string_pretty(&*s),
            Err(_) => return,
        };
        let Ok(json) = json else {
            return;
        };
        if let Err(e) = write_atomic(path, &json) {
            eprintln!("[layout] couldn't save the page layout: {e}");
        }
    }
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

/// How `node`'s page (a device of `kind`: "phone" or "computer") is
/// arranged: `tiles` (every tile it has, in default order) in the saved
/// order, the hidden ones, and whether that's its own layout, its kind's,
/// or the default.
#[tauri::command]
#[specta::specta]
pub fn get_page_layout(
    kind: String,
    node: String,
    tiles: Vec<String>,
    store: State<PageLayouts>,
) -> PageLayoutView {
    let saved = store.saved.lock().map(|s| s.clone()).unwrap_or_default();
    saved.view(&kind, &node, &tiles)
}

/// Save `node`'s page arrangement for it alone (`only_this`) or for every
/// device of its `kind`; `layout` null resets (see `Saved::apply`). Answers
/// with what the page shows now.
#[tauri::command]
#[specta::specta]
pub fn set_page_layout(
    kind: String,
    node: String,
    only_this: bool,
    layout: Option<PageLayout>,
    tiles: Vec<String>,
    app: AppHandle,
    store: State<PageLayouts>,
) -> PageLayoutView {
    let view = match store.saved.lock() {
        Ok(mut s) => {
            s.apply(&kind, &node, only_this, layout);
            s.view(&kind, &node, &tiles)
        }
        Err(_) => Saved::default().view(&kind, &node, &tiles),
    };
    store.save_soon(&app);
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn layout(order: &[&str], hidden: &[&str]) -> PageLayout {
        PageLayout {
            order: ids(order),
            hidden: ids(hidden),
        }
    }

    #[test]
    fn nothing_saved_is_the_default_order() {
        let tiles = ids(&["chat", "files", "info"]);
        assert_eq!(
            merge(&PageLayout::default(), &tiles),
            layout(&["chat", "files", "info"], &[])
        );
    }

    #[test]
    fn saved_order_wins() {
        let tiles = ids(&["chat", "files", "info"]);
        let saved = layout(&["info", "chat", "files"], &["files"]);
        assert_eq!(merge(&saved, &tiles), saved);
    }

    #[test]
    fn a_new_tile_lands_after_its_default_neighbour() {
        // "connection" came after "info" by default; the saved layout
        // predates it and moved "info" first.
        let tiles = ids(&["chat", "files", "info", "connection", "apps"]);
        let saved = layout(&["info", "chat", "files", "apps"], &[]);
        assert_eq!(
            merge(&saved, &tiles).order,
            ids(&["info", "connection", "chat", "files", "apps"])
        );
    }

    #[test]
    fn a_new_first_tile_goes_first() {
        let tiles = ids(&["new", "chat", "files"]);
        let saved = layout(&["files", "chat"], &[]);
        assert_eq!(merge(&saved, &tiles).order, ids(&["new", "files", "chat"]));
    }

    #[test]
    fn several_new_tiles_keep_their_default_order() {
        let tiles = ids(&["a", "b", "c", "d", "e"]);
        let saved = layout(&["e", "a"], &[]);
        assert_eq!(merge(&saved, &tiles).order, ids(&["e", "a", "b", "c", "d"]));
    }

    #[test]
    fn gone_and_repeated_ids_are_skipped() {
        let tiles = ids(&["chat", "files"]);
        let saved = layout(&["old", "files", "files", "chat"], &["old", "chat", "chat"]);
        assert_eq!(merge(&saved, &tiles), layout(&["files", "chat"], &["chat"]));
    }

    #[test]
    fn every_tile_comes_back_once() {
        let tiles = ids(&["a", "b", "c", "d"]);
        for saved in [
            layout(&[], &[]),
            layout(&["d"], &[]),
            layout(&["c", "x", "a"], &["x"]),
            layout(&["d", "c", "b", "a"], &[]),
        ] {
            let mut got = merge(&saved, &tiles).order;
            got.sort();
            assert_eq!(got, tiles);
        }
    }

    #[test]
    fn clean_drops_bad_ids() {
        let long = "x".repeat(MAX_ID + 1);
        let dirty = PageLayout {
            order: vec!["a".into(), " ".into(), long.clone(), "a".into(), "b".into()],
            hidden: vec![long, "b".into(), "b".into()],
        };
        assert_eq!(clean(dirty), layout(&["a", "b"], &["b"]));
        let many = PageLayout {
            order: (0..MAX_TILES + 10).map(|i| i.to_string()).collect(),
            hidden: vec![],
        };
        assert_eq!(clean(many).order.len(), MAX_TILES);
    }

    #[test]
    fn device_layout_wins_over_kind() {
        let tiles = ids(&["a", "b", "c"]);
        let mut s = Saved::default();
        assert_eq!(s.view("phone", "n1", &tiles).scope, LayoutScope::Default);

        s.apply("phone", "n1", false, Some(layout(&["c", "b", "a"], &[])));
        let v = s.view("phone", "n2", &tiles);
        assert_eq!(
            (v.scope, v.order),
            (LayoutScope::Kind, ids(&["c", "b", "a"]))
        );
        // A computer isn't affected.
        assert_eq!(s.view("computer", "n3", &tiles).scope, LayoutScope::Default);

        s.apply("phone", "n1", true, Some(layout(&["b", "a", "c"], &["c"])));
        let v = s.view("phone", "n1", &tiles);
        assert_eq!(v.scope, LayoutScope::Device);
        assert_eq!(v.order, ids(&["b", "a", "c"]));
        assert_eq!(v.hidden, ids(&["c"]));
        // Other phones keep the kind's.
        assert_eq!(s.view("phone", "n2", &tiles).order, ids(&["c", "b", "a"]));
    }

    #[test]
    fn resetting() {
        let tiles = ids(&["a", "b"]);
        let mut s = Saved::default();
        s.apply("phone", "n1", false, Some(layout(&["b", "a"], &[])));
        s.apply("phone", "n1", true, Some(layout(&["a", "b"], &["a"])));
        // Reset "only for this device": back to the phone layout.
        s.apply("phone", "n1", true, None);
        let v = s.view("phone", "n1", &tiles);
        assert_eq!((v.scope, v.order), (LayoutScope::Kind, ids(&["b", "a"])));
        // Its own again, then reset for every phone: all gone.
        s.apply("phone", "n1", true, Some(layout(&["a", "b"], &[])));
        s.apply("phone", "n1", false, None);
        assert_eq!(s.view("phone", "n1", &tiles).scope, LayoutScope::Default);
        assert_eq!(s, Saved::default());
    }

    #[test]
    fn saving_for_the_kind_drops_the_device_layout() {
        let tiles = ids(&["a", "b"]);
        let mut s = Saved::default();
        s.apply("computer", "n1", true, Some(layout(&["b", "a"], &[])));
        s.apply("computer", "n1", false, Some(layout(&["a", "b"], &["b"])));
        let v = s.view("computer", "n1", &tiles);
        assert_eq!((v.scope, v.hidden), (LayoutScope::Kind, ids(&["b"])));
        assert!(s.devices.is_empty());
    }

    #[test]
    fn unknown_kind_is_a_computer() {
        let tiles = ids(&["a", "b"]);
        let mut s = Saved::default();
        s.apply("toaster", "n1", false, Some(layout(&["b", "a"], &[])));
        assert_eq!(s.view("computer", "n2", &tiles).order, ids(&["b", "a"]));
    }

    #[test]
    fn the_file_round_trips() {
        let mut s = Saved::default();
        s.apply("phone", "n1", false, Some(layout(&["b", "a"], &["a"])));
        s.apply("computer", "n2", true, Some(layout(&["x"], &[])));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&json).unwrap(), s);
        // An older or partial file still reads.
        assert_eq!(
            serde_json::from_str::<Saved>("{}").unwrap(),
            Saved::default()
        );
    }

    #[test]
    fn written_atomically() {
        let dir = std::env::temp_dir().join(format!("md-layout-{}", std::process::id()));
        let path = dir.join(FILE);
        write_atomic(&path, "{}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
