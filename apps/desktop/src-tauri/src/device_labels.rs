// Device labels: a nickname, an icon and an accent colour you give each of
// your devices, on this computer only (like a contact name: the device
// itself and your other devices never see it; nothing goes on the wire).
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
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

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
        let mut map = self
            .map
            .lock()
            .map_err(|_| CoreError::Other("labels unavailable".into()))?;
        put(&mut map, node, label)?;
        if let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(&*map)) {
            if let Err(e) = kv.set(KEY, &json) {
                eprintln!("[labels] couldn't save the labels: {e}");
            }
        }
        Ok(())
    }
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
    state.set(&node, checked(nickname, icon, color)?)?;
    crate::tray::refresh(&app);
    Ok(state.all())
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
