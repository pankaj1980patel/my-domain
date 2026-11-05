// Device labels: a nickname, an icon and an accent colour you give each of
// your devices. Kept here (what this computer shows), and shared with your
// other devices while "Share names and icons with my other devices" is on
// (mdcore `features::labels`, on until turned off): a label set here goes
// to the core (`Engine::set_shared_label`), and labels set on your other
// devices come back as `CoreEvent::LabelsChanged` (`apply_shared`), which
// changes the labels here without sending anything back. With the switch
// off the core neither sends nor takes them.
//
// Kept in `<app config dir>/device-labels.json`, key `labels`, as a JSON
// object node id -> label. A blank nickname means the device's own name;
// icons and colours come from small fixed sets the UI draws (anything else
// is refused). A label with nothing set is dropped.
//
// Everything on this side that names a device goes through `display_name`
// (via `notify::device_name`): the tray menu, notifications, schedules.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mdcore::kv::FileKv;
use mdcore::model::SharedLabel;
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

const KEY: &str = "labels";
/// The longest nickname, in characters.
pub const MAX_NICKNAME: usize = 40;
/// The most labels kept.
const MAX_LABELS: usize = 200;
/// Icons the UI can draw.
pub const ICONS: [&str; 6] = ["phone", "tablet", "laptop", "desktop", "tv", "watch"];
/// Accent colours the UI can draw (each maps to a theme colour).
pub const COLORS: [&str; 7] = ["cyan", "blue", "sky", "green", "gold", "rose", "grey"];

/// What you call one of your devices here.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, Default, PartialEq)]
pub struct DeviceLabel {
    /// Shown instead of its own name; none: its own name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// One of `ICONS`; none: the initial of its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// One of `COLORS`; none: the theme's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl DeviceLabel {
    fn is_empty(&self) -> bool {
        self.nickname.is_none() && self.icon.is_none() && self.color.is_none()
    }
}

/// A label as given: the nickname trimmed (blank: none, cut to
/// `MAX_NICKNAME`), the icon and colour checked against their sets.
pub fn checked(
    nickname: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<DeviceLabel, CoreError> {
    let nickname = nickname
        .map(|n| {
            n.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_NICKNAME)
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|n| !n.is_empty());
    let icon = icon.filter(|i| !i.is_empty());
    if let Some(i) = &icon {
        if !ICONS.contains(&i.as_str()) {
            return Err(CoreError::InvalidInput(format!("no such icon: {i}")));
        }
    }
    let color = color.filter(|c| !c.is_empty());
    if let Some(c) = &color {
        if !COLORS.contains(&c.as_str()) {
            return Err(CoreError::InvalidInput(format!("no such colour: {c}")));
        }
    }
    Ok(DeviceLabel {
        nickname,
        icon,
        color,
    })
}

/// A label another device set: like `checked`, but an icon or colour this
/// app can't draw is left out instead of refusing the label.
fn lenient(l: &SharedLabel) -> DeviceLabel {
    let icon = l.icon.clone().filter(|i| ICONS.contains(&i.as_str()));
    let color = l.color.clone().filter(|c| COLORS.contains(&c.as_str()));
    checked(l.nickname.clone(), icon, color).unwrap_or_default()
}

/// Set (or, when empty, drop) `node`'s label in `map`.
fn put(
    map: &mut BTreeMap<String, DeviceLabel>,
    node: &str,
    label: DeviceLabel,
) -> Result<(), CoreError> {
    if node.trim().is_empty() {
        return Err(CoreError::InvalidInput("which device?".into()));
    }
    if label.is_empty() {
        map.remove(node);
        return Ok(());
    }
    if !map.contains_key(node) && map.len() >= MAX_LABELS {
        return Err(CoreError::InvalidInput(format!(
            "at most {MAX_LABELS} labels"
        )));
    }
    map.insert(node.to_string(), label);
    Ok(())
}

/// The name to show for a device: its nickname, else its own name tidied
/// (see `notify::nice_name`).
pub fn name_for(labels: &BTreeMap<String, DeviceLabel>, node: &str, own: &str) -> String {
    labels
        .get(node)
        .and_then(|l| l.nickname.clone())
        .unwrap_or_else(|| crate::notify::nice_name(own))
}

/// The labels, loaded once at startup.
pub struct DeviceLabels {
    kv: Option<FileKv>,
    map: Mutex<BTreeMap<String, DeviceLabel>>,
}

impl DeviceLabels {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("device-labels.json")));
        let map = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        DeviceLabels {
            kv,
            map: Mutex::new(map),
        }
    }

    fn all(&self) -> BTreeMap<String, DeviceLabel> {
        self.map.lock().map(|m| m.clone()).unwrap_or_default()
    }

    fn set(&self, node: &str, label: DeviceLabel) -> Result<(), CoreError> {
        self.set_all(&[(node.to_string(), label)])
    }

    /// Set several labels and save once. One that can't be set is skipped
    /// (the first such error is the answer).
    fn set_all(&self, labels: &[(String, DeviceLabel)]) -> Result<(), CoreError> {
        let mut map = self
            .map
            .lock()
            .map_err(|_| CoreError::Other("labels unavailable".into()))?;
        let mut result = Ok(());
        for (node, label) in labels {
            if let Err(e) = put(&mut map, node, label.clone()) {
                if result.is_ok() {
                    result = Err(e);
                }
            }
        }
        if let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(&*map)) {
            if let Err(e) = kv.set(KEY, &json) {
                eprintln!("[labels] couldn't save the labels: {e}");
            }
        }
        result
    }
}

/// Hand the labels this computer had before labels were shared to the core
/// (at start): ones it doesn't know yet, which anything set since wins over.
pub fn seed(app: &AppHandle, engine: &Eng) {
    let Some(state) = app.try_state::<DeviceLabels>() else {
        return;
    };
    for (node, l) in state.all() {
        engine.seed_label(
            &node,
            l.nickname.as_deref(),
            l.icon.as_deref(),
            l.color.as_deref(),
        );
    }
}

/// Labels set on your other devices (`CoreEvent::LabelsChanged`): change
/// them here (a cleared one forgets the label), refresh the tray and tell
/// the window (`device-labels`, every label). Nothing goes back to the core.
pub fn apply_shared(app: &AppHandle, labels: &[SharedLabel]) {
    let Some(state) = app.try_state::<DeviceLabels>() else {
        return;
    };
    let changes: Vec<(String, DeviceLabel)> = labels
        .iter()
        .map(|l| (l.node.clone(), lenient(l)))
        .collect();
    if let Err(e) = state.set_all(&changes) {
        eprintln!("[labels] couldn't take a shared label: {e}");
    }
    crate::tray::refresh(app);
    let _ = app.emit("device-labels", state.all());
}

/// The nickname you gave `node` here, if any.
pub fn nickname(app: &AppHandle, node: &str) -> Option<String> {
    app.try_state::<DeviceLabels>()?
        .map
        .lock()
        .ok()?
        .get(node)?
        .nickname
        .clone()
}

/// The name to show for a device whose own name is `own`.
pub fn display_name(app: &AppHandle, node: &str, own: &str) -> String {
    match app.try_state::<DeviceLabels>() {
        Some(state) => match state.map.lock() {
            Ok(map) => name_for(&map, node, own),
            Err(_) => crate::notify::nice_name(own),
        },
        None => crate::notify::nice_name(own),
    }
}

/// Every label you gave your devices here (node id -> label).
#[tauri::command]
#[specta::specta]
pub fn get_device_labels(state: State<DeviceLabels>) -> BTreeMap<String, DeviceLabel> {
    state.all()
}

/// Give a device a nickname, an icon and a colour (each optional; all
/// blank clears its label). Answers with every label.
#[tauri::command]
#[specta::specta]
pub fn set_device_label(
    node: String,
    nickname: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    app: AppHandle,
    state: State<DeviceLabels>,
) -> Result<BTreeMap<String, DeviceLabel>, CoreError> {
    let label = checked(nickname, icon, color)?;
    state.set(&node, label.clone())?;
    // Your other devices see it too, while sharing is on.
    if let Some(engine) = app.try_state::<Eng>() {
        if let Err(e) = engine.set_shared_label(
            &node,
            label.nickname.as_deref(),
            label.icon.as_deref(),
            label.color.as_deref(),
        ) {
            eprintln!("[labels] couldn't share the label: {e}");
        }
    }
    crate::tray::refresh(&app);
    Ok(state.all())
}

/// Whether the names, icons and colours you give your devices are shared
/// with your other devices (and theirs taken here). On until turned off.
#[tauri::command]
#[specta::specta]
pub fn labels_share(engine: State<Eng>) -> bool {
    engine.labels_share()
}

#[tauri::command]
#[specta::specta]
pub fn set_labels_share(on: bool, engine: State<Eng>) {
    engine.set_labels_share(on);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn nicknames_are_tidied() {
        let l = checked(s("  Mum's   phone "), None, None).unwrap();
        assert_eq!(l.nickname.as_deref(), Some("Mum's phone"));
        assert_eq!(checked(s("   "), None, None).unwrap().nickname, None);
        let long = "x".repeat(100);
        let l = checked(Some(long), None, None).unwrap();
        assert_eq!(l.nickname.unwrap().chars().count(), MAX_NICKNAME);
    }

    #[test]
    fn icons_and_colours_come_from_the_sets() {
        let l = checked(None, s("laptop"), s("rose")).unwrap();
        assert_eq!(
            (l.icon.as_deref(), l.color.as_deref()),
            (Some("laptop"), Some("rose"))
        );
        assert!(checked(None, s("toaster"), None).is_err());
        assert!(checked(None, None, s("#ff0000")).is_err());
        // Blank means none.
        assert_eq!(checked(None, s(""), s("")).unwrap(), DeviceLabel::default());
    }

    #[test]
    fn empty_labels_are_dropped() {
        let mut map = BTreeMap::new();
        put(&mut map, "n1", checked(s("Den TV"), s("tv"), None).unwrap()).unwrap();
        assert_eq!(map.len(), 1);
        put(&mut map, "n1", checked(s(" "), None, None).unwrap()).unwrap();
        assert!(map.is_empty());
        assert!(put(&mut map, " ", checked(s("x"), None, None).unwrap()).is_err());
    }

    #[test]
    fn names_prefer_the_nickname() {
        let mut map = BTreeMap::new();
        assert_eq!(
            name_for(&map, "n1", "Pankajs-MacBook-Pro.local"),
            "Pankajs MacBook Pro"
        );
        put(&mut map, "n1", checked(s("Work-Mac"), None, None).unwrap()).unwrap();
        assert_eq!(
            name_for(&map, "n1", "Pankajs-MacBook-Pro.local"),
            "Work-Mac"
        );
        // An icon alone keeps the device's own name.
        put(&mut map, "n2", checked(None, s("phone"), None).unwrap()).unwrap();
        assert_eq!(name_for(&map, "n2", "pixel_8"), "pixel 8");
    }

    #[test]
    fn shared_labels_keep_only_what_this_app_draws() {
        let shared = SharedLabel {
            node: "n1".into(),
            nickname: s("  Den   TV "),
            icon: s("toaster"),
            color: s("gold"),
            updated_at: 5,
            by: "n2".into(),
        };
        let l = lenient(&shared);
        assert_eq!(l.nickname.as_deref(), Some("Den TV"));
        assert_eq!((l.icon, l.color.as_deref()), (None, Some("gold")));
        let cleared = SharedLabel {
            node: "n1".into(),
            ..SharedLabel::default()
        };
        assert!(lenient(&cleared).is_empty());
    }

    #[test]
    fn labels_round_trip_as_json() {
        let mut map = BTreeMap::new();
        put(
            &mut map,
            "n1",
            checked(s("Den"), s("tv"), s("gold")).unwrap(),
        )
        .unwrap();
        put(&mut map, "n2", checked(None, None, s("blue")).unwrap()).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(
            json,
            r#"{"n1":{"nickname":"Den","icon":"tv","color":"gold"},"n2":{"color":"blue"}}"#
        );
        let back: BTreeMap<String, DeviceLabel> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, map);
    }
}
