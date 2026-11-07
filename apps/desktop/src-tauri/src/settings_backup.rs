// Back up and restore this app's settings (Settings → "Back up settings…"
// and "Restore settings…"), so a reinstall or a new computer doesn't mean
// setting everything up again.
//
//  - the backup is one JSON file: `{"app": "my-domain", "kind":
//    "settings-backup", "version": 1, "platform", "created_at", "devices",
//    "sections": {…}}`. `devices` are your devices when it was made (id and
//    name), so a restore can tell which sections name devices it doesn't
//    know;
//  - what goes in is decided by an explicit allow-list (`KV_SECTIONS`,
//    `FILE_SECTIONS`): named keys of named files in the app config dir, plus
//    the core's sharing switches and the synced folders' definitions (folder,
//    device, mode: not their file indexes). Anything not listed never leaves:
//    the session, device key, encryption keys, the webhook token, FCM
//    credentials, `dev-secrets.json`, passwords, and every message,
//    notification, clipboard or note;
//  - a restore checks the file strictly (size, app, kind, version, platform,
//    every section's shape, every key against the allow-list) before it
//    shows what would change; the sections picked are written, the sharing
//    switches and synced folders apply at once, the rest after a restart
//    (each module reads its file at startup).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use mdcore::model::BatteryAlertSettings;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::Eng;

pub const APP: &str = "my-domain";
pub const KIND: &str = "settings-backup";
pub const VERSION: u64 = 1;
/// The biggest backup file read.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The longest value of one setting.
const MAX_VALUE: usize = 1024 * 1024;

/// A section kept as some keys of a flat key/value file (`FileKv`) in the
/// app config dir. Only `keys` are read or written.
pub struct KvSection {
    pub id: &'static str,
    pub file: &'static str,
    pub keys: &'static [&'static str],
}

/// The allow-list: these keys, of these files, and nothing else.
pub const KV_SECTIONS: [KvSection; 6] = [
    KvSection {
        id: "preferences",
        file: "prefs.json",
        keys: &[
            "notifications",
            "follow_dnd",
            "quick_panel",
            "tray_battery",
            "tray_battery_badge",
            "away_lock",
            "away_lock_minutes",
            "away_lock_phones",
            "calls",
            "calls_device",
            "calls_answer_audio",
            "chat_history",
            "clip_history",
            "clip_hide_concealed",
            "clip_excluded_apps",
            "muted_apps",
            "notif_hide_sensitive",
            "notif_only_locked",
            "share_location",
            "computer_notifs",
            "computer_notif_apps",
            "wake_lan",
            "wifi_direct",
            "phone_hotspot",
            "phone_audio_volume",
            "nearby_media",
            "share_wallpaper",
            "kvm_enabled",
            "kvm_hotkey",
            "kvm_swap_mods",
            "print_allow",
            "print_ask",
            "updates_auto",
        ],
    },
    KvSection {
        id: "automations",
        file: "automations.json",
        keys: &["automations"],
    },
    KvSection {
        id: "schedules",
        file: "schedules.json",
        keys: &["schedules"],
    },
    KvSection {
        id: "commands",
        file: "commands.json",
        keys: &["commands"],
    },
    // The webhook token lives in the OS secure store, never here.
    KvSection {
        id: "webhooks",
        file: "webhooks.json",
        keys: &["enabled", "port"],
    },
    KvSection {
        id: "device_labels",
        file: "device-labels.json",
        keys: &["labels"],
    },
];

/// Sections kept as a whole JSON file in the app config dir.
pub const FILE_SECTIONS: [(&str, &str); 2] = [
    ("shortcuts", "shortcuts.json"),
    ("page_layout", "page-layout.json"),
];

/// Every section, in the order the restore lists them.
pub const SECTION_IDS: [&str; 11] = [
    "appearance",
    "preferences",
    "sharing",
    "automations",
    "schedules",
    "commands",
    "shortcuts",
    "webhooks",
    "device_labels",
    "page_layout",
    "sync_folders",
];

/// Whether `key` of section `section` may go in a backup (and come back
/// from one): on the allow-list and not secret-looking.
pub fn allowed(section: &str, key: &str) -> bool {
    !looks_secret(key)
        && KV_SECTIONS
            .iter()
            .any(|s| s.id == section && s.keys.contains(&key))
}

/// A belt to the allow-list's braces: a key named like a secret is never
/// exported or imported, whatever list it's on.
pub fn looks_secret(key: &str) -> bool {
    const WORDS: [&str; 11] = [
        "token",
        "secret",
        "password",
        "passwd",
        "credential",
        "creds",
        "identity_key",
        "device_key",
        "private",
        "session",
        "fcm",
    ];
    let k = key.to_ascii_lowercase();
    WORDS.iter().any(|w| k.contains(w))
}

/// Only the allowed keys of `map` (a file of section `section`).
pub fn pick(section: &str, map: BTreeMap<String, String>) -> BTreeMap<String, String> {
    map.into_iter()
        .filter(|(k, _)| allowed(section, k))
        .collect()
}

// --- the file ---

/// The theme and language: kept by the UI (localStorage), passed in.
#[derive(Serialize, Deserialize, specta::Type, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// The core's sharing switches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sharing {
    /// The global switches from before per-device permissions: read from
    /// an older backup, never applied (a restore doesn't turn on for every
    /// device what the upgrade turned off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_share: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_input: Option<bool>,
    /// Which device may control this computer, see its screen, take over
    /// its mouse and keyboard (missing in a backup from before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<mdcore::features::perms::PeerPermissions>>,
    pub labels_share: bool,
    pub battery_alerts: BatteryAlertSettings,
    /// "Downloads from my devices" (missing in a backup from before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_downloads: Option<mdcore::model::RemoteDownloadSettings>,
}

/// One synced folder: which folder, with which device, how. Not its files.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SyncDef {
    pub peer: String,
    pub name: String,
    pub path: String,
    pub mode: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SyncSection {
    pub max_size_mb: u32,
    pub pairs: Vec<SyncDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sections {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<Appearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferences: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharing: Option<Sharing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automations: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedules: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhooks: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_labels: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_layout: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_folders: Option<SyncSection>,
}

impl Sections {
    fn kv(&self, id: &str) -> Option<&BTreeMap<String, String>> {
        match id {
            "preferences" => self.preferences.as_ref(),
            "automations" => self.automations.as_ref(),
            "schedules" => self.schedules.as_ref(),
            "commands" => self.commands.as_ref(),
            "webhooks" => self.webhooks.as_ref(),
            "device_labels" => self.device_labels.as_ref(),
            _ => None,
        }
    }

    fn kv_slot(&mut self, id: &str) -> Option<&mut Option<BTreeMap<String, String>>> {
        match id {
            "preferences" => Some(&mut self.preferences),
            "automations" => Some(&mut self.automations),
            "schedules" => Some(&mut self.schedules),
            "commands" => Some(&mut self.commands),
            "webhooks" => Some(&mut self.webhooks),
            "device_labels" => Some(&mut self.device_labels),
            _ => None,
        }
    }

    fn file(&self, id: &str) -> Option<&serde_json::Value> {
        match id {
            "shortcuts" => self.shortcuts.as_ref(),
            "page_layout" => self.page_layout.as_ref(),
            _ => None,
        }
    }

    fn file_slot(&mut self, id: &str) -> Option<&mut Option<serde_json::Value>> {
        match id {
            "shortcuts" => Some(&mut self.shortcuts),
            "page_layout" => Some(&mut self.page_layout),
            _ => None,
        }
    }

    /// Section `id` as JSON text (for finding the devices it names), or
    /// `None` when the backup has no such section.
    fn text(&self, id: &str) -> Option<String> {
        let v = match id {
            "appearance" => serde_json::to_value(self.appearance.as_ref()?),
            "sharing" => serde_json::to_value(self.sharing.as_ref()?),
            "sync_folders" => serde_json::to_value(self.sync_folders.as_ref()?),
            "shortcuts" | "page_layout" => Ok(self.file(id)?.clone()),
            _ => serde_json::to_value(self.kv(id)?),
        };
        v.ok().map(|v| v.to_string())
    }
}

/// One of your devices when the backup was made.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BackupDevice {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub app: String,
    pub kind: String,
    pub version: u64,
    /// "macos", "windows" or "linux".
    pub platform: String,
    /// Unix seconds.
    pub created_at: u64,
    #[serde(default)]
    pub devices: Vec<BackupDevice>,
    pub sections: Sections,
}

/// This computer's OS, as backups name it.
pub fn this_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- the config dir ---

/// A `FileKv` file as a map (missing or unreadable: empty).
fn read_kv(path: &Path) -> BTreeMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The file-kept sections of the config dir `dir`, through the allow-list.
pub fn collect_files(dir: &Path) -> Sections {
    let mut s = Sections::default();
    for sec in &KV_SECTIONS {
        let picked = pick(sec.id, read_kv(&dir.join(sec.file)));
        if let Some(slot) = s.kv_slot(sec.id) {
            *slot = Some(picked);
        }
    }
    for (id, file) in FILE_SECTIONS {
        let value = std::fs::read_to_string(dir.join(file))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .filter(|v| check_file(id, v).is_ok());
        if let Some(slot) = s.file_slot(id) {
            *slot = value;
        }
    }
    s
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Write the file-kept sections `chosen` of `s` into `dir`: a key/value
/// section sets its allowed keys as in the backup and clears the allowed
/// keys the backup hasn't (its defaults then); other keys of the file stay.
/// Returns the sections written.
pub fn apply_files(
    dir: &Path,
    s: &Sections,
    chosen: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    for sec in &KV_SECTIONS {
        let Some(map) = s.kv(sec.id).filter(|_| chosen.contains(sec.id)) else {
            continue;
        };
        let kv = FileKv::new(dir.join(sec.file));
        for key in sec.keys {
            let r = match map.get(*key) {
                Some(v) => kv.set(key, v),
                None => kv.remove(key),
            };
            r.map_err(|e| format!("{}: {e}", sec.file))?;
        }
        done.push(sec.id.to_string());
    }
    for (id, file) in FILE_SECTIONS {
        let Some(v) = s.file(id).filter(|_| chosen.contains(id)) else {
            continue;
        };
        let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
        write_atomic(&dir.join(file), &text).map_err(|e| format!("{file}: {e}"))?;
        done.push(id.to_string());
    }
    Ok(done)
}

// --- checking a file ---

/// What a backup file holds, checked; or why it can't be used.
pub fn parse(bytes: &[u8]) -> Result<Backup, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("This file is too big to be a settings backup (over 5 MB).".into());
    }
    let raw: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| {
        "This file isn\u{2019}t a settings backup: it isn\u{2019}t valid JSON.".to_string()
    })?;
    let field = |k: &str| raw.get(k).and_then(|v| v.as_str()).unwrap_or("");
    if field("app") != APP || field("kind") != KIND {
        return Err("This file isn\u{2019}t a my-domain settings backup.".into());
    }
    match raw.get("version").and_then(|v| v.as_u64()) {
        Some(VERSION) => {}
        Some(v) if v > VERSION => {
            return Err(format!(
                "This backup was made by a newer version of my-domain (format {v}). Update the app, then try again."
            ))
        }
        _ => return Err("This backup\u{2019}s format isn\u{2019}t one this app knows.".into()),
    }
    match field("platform") {
        "macos" | "windows" | "linux" => {}
        "android" => {
            return Err(
                "This backup is from the phone app: restore it on your phone (Settings \u{2192} Restore settings).".into(),
            )
        }
        _ => return Err("This backup doesn\u{2019}t say which kind of computer it\u{2019}s from.".into()),
    }
    let b: Backup = serde_json::from_value(raw)
        .map_err(|e| format!("This backup is damaged and can\u{2019}t be used ({e})."))?;
    check(&b.sections)
        .map_err(|e| format!("This backup is damaged and can\u{2019}t be used ({e})."))?;
    Ok(b)
}

/// Every section's shape, and every key against the allow-list.
fn check(s: &Sections) -> Result<(), String> {
    if let Some(a) = &s.appearance {
        check_appearance(a)?;
    }
    for sec in &KV_SECTIONS {
        let Some(map) = s.kv(sec.id) else { continue };
        for (k, v) in map {
            if !allowed(sec.id, k) {
                return Err(format!(
                    "{}: unexpected setting \u{201c}{k}\u{201d}",
                    sec.id
                ));
            }
            if v.len() > MAX_VALUE {
                return Err(format!("{}: \u{201c}{k}\u{201d} is too long", sec.id));
            }
            check_value(sec.id, k, v).map_err(|e| format!("{}: {k}: {e}", sec.id))?;
        }
    }
    for (id, _) in FILE_SECTIONS {
        if let Some(v) = s.file(id) {
            check_file(id, v).map_err(|e| format!("{id}: {e}"))?;
        }
    }
    if let Some(sync) = &s.sync_folders {
        for p in &sync.pairs {
            if p.peer.is_empty() || p.path.is_empty() || p.peer.len() > 200 || p.path.len() > 4096 {
                return Err("sync_folders: a folder without a device or path".into());
            }
            if !matches!(p.mode.as_str(), "both" | "send" | "receive") {
                return Err(format!(
                    "sync_folders: unknown mode \u{201c}{}\u{201d}",
                    p.mode
                ));
            }
        }
    }
    Ok(())
}

fn check_appearance(a: &Appearance) -> Result<(), String> {
    let ok = |v: &Option<String>| {
        v.as_deref().is_none_or(|t| {
            !t.is_empty()
                && t.len() <= 40
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    };
    if ok(&a.theme) && ok(&a.language) {
        Ok(())
    } else {
        Err("appearance: not a theme or language".into())
    }
}

/// The values whose shape the app relies on: the lists are what their
/// module reads.
fn check_value(section: &str, key: &str, v: &str) -> Result<(), String> {
    let e = |e: serde_json::Error| e.to_string();
    match (section, key) {
        ("automations", _) => serde_json::from_str::<Vec<crate::automations::Automation>>(v)
            .map(drop)
            .map_err(e),
        ("schedules", _) => serde_json::from_str::<Vec<crate::schedule::Schedule>>(v)
            .map(drop)
            .map_err(e),
        ("commands", _) => serde_json::from_str::<Vec<crate::commands_run::ShellCommand>>(v)
            .map(drop)
            .map_err(e),
        ("device_labels", _) => {
            serde_json::from_str::<BTreeMap<String, crate::device_labels::DeviceLabel>>(v)
                .map(drop)
                .map_err(e)
        }
        ("webhooks", "port") => v.parse::<u16>().map(drop).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

/// A whole-file section is what its module reads.
fn check_file(id: &str, v: &serde_json::Value) -> Result<(), String> {
    let ok = match id {
        "shortcuts" => crate::shortcuts::read_file(&v.to_string()).is_some(),
        "page_layout" => serde_json::from_value::<crate::page_layout::Saved>(v.clone()).is_ok(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err("not the expected settings".into())
    }
}

// --- what a restore would change ---

/// The devices (names) section text `text` names that aren't `known` here.
/// Only the backup's own devices are looked for.
pub fn unknown_devices(
    text: &str,
    devices: &[BackupDevice],
    known: &BTreeSet<String>,
) -> Vec<String> {
    devices
        .iter()
        .filter(|d| !d.id.is_empty() && text.contains(&d.id) && !known.contains(&d.id))
        .map(|d| {
            if d.name.is_empty() {
                d.id.clone()
            } else {
                d.name.clone()
            }
        })
        .collect()
}

/// The keys of a key/value section that differ between `now` and `then`.
pub fn kv_changes(now: &BTreeMap<String, String>, then: &BTreeMap<String, String>) -> Vec<String> {
    let keys: BTreeSet<&String> = now.keys().chain(then.keys()).collect();
    keys.into_iter()
        .filter(|k| now.get(*k) != then.get(*k))
        .map(|k| k.to_string())
        .collect()
}

/// How many things section `id` of `s` holds (automations, labels, …).
fn items(s: &Sections, id: &str) -> u32 {
    let list_len = |text: Option<&String>| -> usize {
        text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .map(|v| match v {
                serde_json::Value::Array(a) => a.len(),
                serde_json::Value::Object(o) => o.len(),
                _ => 0,
            })
            .unwrap_or(0)
    };
    let n = match id {
        "appearance" => s.appearance.as_ref().map_or(0, |a| {
            usize::from(a.theme.is_some()) + usize::from(a.language.is_some())
        }),
        "sharing" => usize::from(s.sharing.is_some()) * 4,
        "sync_folders" => s.sync_folders.as_ref().map_or(0, |x| x.pairs.len()),
        "automations" => list_len(s.automations.as_ref().and_then(|m| m.get("automations"))),
        "schedules" => list_len(s.schedules.as_ref().and_then(|m| m.get("schedules"))),
        "commands" => list_len(s.commands.as_ref().and_then(|m| m.get("commands"))),
        "device_labels" => list_len(s.device_labels.as_ref().and_then(|m| m.get("labels"))),
        "shortcuts" => s
            .shortcuts
            .as_ref()
            .and_then(|v| crate::shortcuts::read_file(&v.to_string()))
            .map_or(0, |l| l.len()),
        "page_layout" => usize::from(s.page_layout.is_some()),
        _ => s.kv(id).map_or(0, |m| m.len()),
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// What one section of a restore would do.
#[derive(Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct SectionPreview {
    /// One of `SECTION_IDS`.
    pub id: String,
    /// It differs from what's here now.
    pub changed: bool,
    /// The settings that would change (named sections), or the synced
    /// folders that would be added.
    pub keys: Vec<String>,
    /// How many things the backup holds there.
    pub items: u32,
    /// Devices it names that aren't yours here (their names).
    pub unknown_devices: Vec<String>,
    /// Ticked to start with: nothing unknown in it.
    pub on: bool,
}

/// A checked backup, ready to restore.
#[derive(Serialize, specta::Type, Clone, Debug)]
pub struct RestorePreview {
    pub platform: String,
    /// Unix seconds.
    pub created_at: f64,
    pub sections: Vec<SectionPreview>,
    /// The theme and language it holds, for the UI to apply.
    pub appearance: Option<Appearance>,
}

/// What the live parts are now, for comparing.
pub struct Current {
    pub appearance: Appearance,
    pub sharing: Option<Sharing>,
    pub sync: Vec<SyncDef>,
    pub max_size_mb: u32,
    /// Your devices here (this computer too).
    pub known: BTreeSet<String>,
}

/// The synced folders of `then` that aren't here yet (same device and
/// folder).
fn new_pairs<'a>(then: &'a SyncSection, now: &[SyncDef]) -> Vec<&'a SyncDef> {
    then.pairs
        .iter()
        .filter(|p| !now.iter().any(|n| n.peer == p.peer && n.path == p.path))
        .collect()
}

/// What restoring `b` into config dir `dir` would change.
pub fn preview(dir: &Path, b: &Backup, cur: &Current) -> Vec<SectionPreview> {
    let here = collect_files(dir);
    let s = &b.sections;
    let mut out = Vec::new();
    for id in SECTION_IDS {
        let Some(text) = s.text(id) else { continue };
        let keys: Vec<String> = match id {
            "appearance" => {
                let a = s.appearance.clone().unwrap_or_default();
                let mut k = Vec::new();
                if a.theme.is_some() && a.theme != cur.appearance.theme {
                    k.push("theme".to_string());
                }
                if a.language.is_some() && a.language != cur.appearance.language {
                    k.push("language".to_string());
                }
                k
            }
            "sharing" => {
                let then = s.sharing.as_ref();
                let now = cur.sharing.as_ref();
                let mut k = Vec::new();
                if let (Some(t), Some(n)) = (then, now) {
                    if t.permissions.is_some() && t.permissions != n.permissions {
                        k.push("permissions".into());
                    }
                    if t.labels_share != n.labels_share {
                        k.push("labels_share".into());
                    }
                    if t.battery_alerts != n.battery_alerts {
                        k.push("battery_alerts".into());
                    }
                    if t.remote_downloads.is_some() && t.remote_downloads != n.remote_downloads {
                        k.push("remote_downloads".into());
                    }
                }
                k
            }
            "sync_folders" => {
                let sync = s.sync_folders.as_ref();
                let mut k: Vec<String> = sync
                    .map(|x| {
                        new_pairs(x, &cur.sync)
                            .into_iter()
                            .map(|p| p.name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if sync.is_some_and(|x| x.max_size_mb != cur.max_size_mb) {
                    k.push("max_size".into());
                }
                k
            }
            "shortcuts" | "page_layout" => {
                if s.file(id) != here.file(id) {
                    vec![id.to_string()]
                } else {
                    Vec::new()
                }
            }
            _ => kv_changes(
                here.kv(id).unwrap_or(&BTreeMap::new()),
                s.kv(id).unwrap_or(&BTreeMap::new()),
            ),
        };
        let unknown = unknown_devices(&text, &b.devices, &cur.known);
        // A folder path from another OS means nothing here.
        let foreign = id == "sync_folders" && b.platform != this_platform();
        out.push(SectionPreview {
            id: id.to_string(),
            changed: !keys.is_empty(),
            keys,
            items: items(s, id),
            on: unknown.is_empty() && !foreign,
            unknown_devices: unknown,
        });
    }
    out
}

// --- the commands ---

/// The backup picked for restoring, between the preview and the apply.
#[derive(Default)]
pub struct Pending(Mutex<Option<Backup>>);

/// What a restore did.
#[derive(Serialize, specta::Type, Clone, Debug)]
pub struct RestoreResult {
    pub applied: Vec<String>,
    /// What couldn't be done, for people.
    pub failed: Vec<String>,
    /// Some of it takes effect only after the app restarts.
    pub restart: bool,
}

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|_| "This computer gave the app no settings folder.".to_string())
}

fn sharing_now(engine: &Eng) -> Sharing {
    Sharing {
        screen_share: None,
        remote_input: None,
        permissions: Some(engine.all_permissions()),
        labels_share: engine.labels_share(),
        battery_alerts: engine.battery_alerts(),
        remote_downloads: Some(engine.remote_download_settings()),
    }
}

/// Make the per-device permissions what the backup says: what it allows,
/// allowed; anything else allowed now, turned off.
fn restore_permissions(engine: &Eng, list: &[mdcore::features::perms::PeerPermissions]) {
    let mut nodes: BTreeSet<String> = list.iter().map(|p| p.node_id.clone()).collect();
    nodes.extend(engine.all_permissions().into_iter().map(|p| p.node_id));
    for node in nodes {
        let want = list.iter().find(|p| p.node_id == node);
        for (perm, on) in [
            ("control", want.is_some_and(|p| p.control)),
            ("screen", want.is_some_and(|p| p.screen)),
            ("kvm", want.is_some_and(|p| p.kvm)),
        ] {
            let _ = engine.set_peer_permission(&node, perm, on);
        }
    }
}

fn sync_now(engine: &Eng) -> Vec<SyncDef> {
    engine
        .sync_pairs()
        .into_iter()
        // An invitation waiting here has no folder yet: nothing to restore.
        .filter(|p| !p.path.is_empty())
        .map(|p| SyncDef {
            peer: p.peer,
            name: p.name,
            path: p.path,
            mode: p.mode,
        })
        .collect()
}

fn max_size_mb(engine: &Eng) -> u32 {
    u32::try_from(engine.sync_max_size() >> 20).unwrap_or(u32::MAX)
}

/// The backup of this computer's settings now.
fn build(app: &AppHandle, appearance: Appearance) -> Result<Backup, String> {
    check_appearance(&appearance)?;
    let mut sections = collect_files(&config_dir(app)?);
    sections.appearance = Some(appearance);
    let mut devices = Vec::new();
    if let Some(engine) = app.try_state::<Eng>() {
        sections.sharing = Some(sharing_now(&engine));
        sections.sync_folders = Some(SyncSection {
            max_size_mb: max_size_mb(&engine),
            pairs: sync_now(&engine),
        });
        devices = engine
            .get_peers()
            .into_iter()
            .map(|p| BackupDevice {
                name: crate::device_labels::display_name(app, &p.node_id, &p.name),
                id: p.node_id,
            })
            .collect();
    }
    Ok(Backup {
        app: APP.into(),
        kind: KIND.into(),
        version: VERSION,
        platform: this_platform().into(),
        created_at: now_secs(),
        devices,
        sections,
    })
}

/// Save a backup of this app's settings where the user picks. `appearance`
/// is the UI's theme and language. The path, or `None` when the dialog was
/// closed.
#[tauri::command]
#[specta::specta]
pub async fn settings_backup_save(
    appearance: Appearance,
    app: AppHandle,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let backup = build(&app, appearance)?;
    let text = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    let day = chrono::Local::now().format("%Y-%m-%d");
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_file_name(format!("my-domain settings {day}.json"))
        .add_filter("JSON", &["json"])
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
    std::fs::write(&dest, text).map_err(|e| format!("Couldn\u{2019}t save the backup: {e}"))?;
    Ok(Some(dest.to_string_lossy().into_owned()))
}

/// Pick a backup file and check it: what restoring it would change, or
/// `None` when the dialog was closed. `appearance` is the UI's now.
#[tauri::command]
#[specta::specta]
pub async fn settings_restore_open(
    appearance: Appearance,
    app: AppHandle,
) -> Result<Option<RestorePreview>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .pick_file(move |file| {
            let _ = tx.send(file.and_then(|f| f.into_path().ok()));
        });
    let path = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten();
    let Some(path) = path else {
        return Ok(None);
    };
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("Couldn\u{2019}t read the file: {e}"))?
        .len();
    if size > MAX_BYTES {
        return Err("This file is too big to be a settings backup (over 5 MB).".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn\u{2019}t read the file: {e}"))?;
    let backup = parse(&bytes)?;
    let mut cur = Current {
        appearance,
        sharing: None,
        sync: Vec::new(),
        max_size_mb: 0,
        known: BTreeSet::new(),
    };
    if let Some(engine) = app.try_state::<Eng>() {
        cur.sharing = Some(sharing_now(&engine));
        cur.sync = sync_now(&engine);
        cur.max_size_mb = max_size_mb(&engine);
        cur.known = engine.get_peers().into_iter().map(|p| p.node_id).collect();
        cur.known.insert(engine.identity().node_id);
    }
    let sections = preview(&config_dir(&app)?, &backup, &cur);
    let out = RestorePreview {
        platform: backup.platform.clone(),
        created_at: backup.created_at as f64,
        sections,
        appearance: backup.sections.appearance.clone(),
    };
    if let Ok(mut p) = app.state::<Pending>().0.lock() {
        *p = Some(backup);
    }
    Ok(Some(out))
}

/// Restore `sections` (ids) of the backup `settings_restore_open` checked.
/// The theme and language are the UI's to apply.
#[tauri::command]
#[specta::specta]
pub fn settings_restore_apply(
    sections: Vec<String>,
    app: AppHandle,
    pending: State<Pending>,
) -> Result<RestoreResult, String> {
    let backup = pending
        .0
        .lock()
        .ok()
        .and_then(|mut p| p.take())
        .ok_or_else(|| "Pick the backup file again.".to_string())?;
    let chosen: BTreeSet<String> = sections.into_iter().collect();
    let s = &backup.sections;
    let mut applied = apply_files(&config_dir(&app)?, s, &chosen)?;
    let restart = !applied.is_empty();
    let mut failed = Vec::new();
    if chosen.contains("appearance") && s.appearance.is_some() {
        applied.push("appearance".into());
    }
    let engine = app.try_state::<Eng>();
    if let (Some(sh), true) = (&s.sharing, chosen.contains("sharing")) {
        match &engine {
            Some(engine) => {
                if let Some(list) = &sh.permissions {
                    restore_permissions(engine, list);
                }
                engine.set_labels_share(sh.labels_share);
                if let Err(e) = engine.set_battery_alerts(sh.battery_alerts.clone()) {
                    failed.push(format!("Battery alerts: {e}"));
                }
                if let Some(rd) = &sh.remote_downloads {
                    if let Err(e) = engine.set_remote_download_settings(rd.clone()) {
                        failed.push(format!("Downloads from my devices: {e}"));
                    }
                }
                applied.push("sharing".into());
                // The others see what this computer offers now.
                let app = app.clone();
                std::thread::spawn(move || crate::device::refresh_status(&app));
            }
            None => failed.push("Sharing: the app isn\u{2019}t signed in".into()),
        }
    }
    if let (Some(sync), true) = (&s.sync_folders, chosen.contains("sync_folders")) {
        match &engine {
            Some(engine) => {
                if let Err(e) = engine.sync_set_max_size(u64::from(sync.max_size_mb) << 20) {
                    failed.push(format!("Synced folders: {e}"));
                }
                let now = sync_now(engine);
                let known: BTreeSet<String> =
                    engine.get_peers().into_iter().map(|p| p.node_id).collect();
                for p in new_pairs(sync, &now) {
                    if !known.contains(&p.peer) {
                        failed.push(format!(
                            "{}: that device isn\u{2019}t one of yours here",
                            p.name
                        ));
                    } else if let Err(e) = engine.sync_create(&p.peer, &p.path, &p.mode) {
                        failed.push(format!("{}: {e}", p.name));
                    }
                }
                applied.push("sync_folders".into());
            }
            None => failed.push("Synced folders: the app isn\u{2019}t signed in".into()),
        }
    }
    Ok(RestoreResult {
        applied,
        failed,
        restart,
    })
}

/// Restart the app (after a restore).
#[tauri::command]
#[specta::specta]
pub fn restart_app(app: AppHandle) {
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("md-settings-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, file: &str, pairs: &[(&str, &str)]) {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        std::fs::write(dir.join(file), serde_json::to_vec(&map).unwrap()).unwrap();
    }

    const AUTOMATIONS: &str = r#"[{"id":"a1","trigger":{"kind":"connects","node":"node-phone"},"node":"","action":"notify","args":"","label":"Hi","enabled":true}]"#;
    const SCHEDULES: &str = r#"[{"id":"s1","node":"node-phone","action":"ring","time":"07:30","days":[1,2],"enabled":true}]"#;
    const COMMANDS: &str = r#"[{"id":"c1","name":"Backup","command":"echo hi","enabled":true}]"#;
    const LABELS: &str = r#"{"node-phone":{"nickname":"Pixel","icon":"phone"}}"#;
    const SHORTCUTS: &str = r#"{"version":1,"bindings":[{"id":"send-clipboard","action":"send_clipboard","node":"","automation":"","keys":"CmdOrCtrl+Shift+Y","enabled":true}]}"#;
    const LAYOUT: &str = r#"{"kinds":{"phone":{"order":["battery"],"hidden":[]}},"devices":{}}"#;

    /// A config dir like a real one: settings, plus secrets and contents
    /// that must never leave.
    fn seeded(name: &str) -> PathBuf {
        let d = temp_dir(name);
        put(
            &d,
            "prefs.json",
            &[
                ("notifications", "off"),
                ("away_lock", "on"),
                ("away_lock_minutes", "10"),
                ("muted_apps", r#"["com.spam"]"#),
                ("quick_panel", "off"),
                ("calls_device", "node-phone"),
                ("send_to_last", "node-phone"),
                ("calls_mac_audio", "refused"),
                ("phone_audio_output", "Speakers"),
                ("session_token", "SECRET-SESSION"),
            ],
        );
        put(&d, "automations.json", &[("automations", AUTOMATIONS)]);
        put(&d, "schedules.json", &[("schedules", SCHEDULES)]);
        put(&d, "commands.json", &[("commands", COMMANDS)]);
        put(
            &d,
            "webhooks.json",
            &[
                ("enabled", "on"),
                ("port", "47821"),
                ("webhook-token", "SECRET-WEBHOOK"),
            ],
        );
        put(&d, "device-labels.json", &[("labels", LABELS)]);
        std::fs::write(d.join("shortcuts.json"), SHORTCUTS).unwrap();
        std::fs::write(d.join("page-layout.json"), LAYOUT).unwrap();
        // Never read by the backup.
        put(
            &d,
            "session.json",
            &[
                ("identity_key", "SECRET-IDENTITY"),
                ("notes", "SECRET-NOTE"),
                ("username", "alice"),
            ],
        );
        put(
            &d,
            "dev-secrets.json",
            &[("device_key", "SECRET-DEVICE-KEY")],
        );
        std::fs::write(
            d.join("fcm_creds_android.json"),
            r#"{"token":"SECRET-FCM"}"#,
        )
        .unwrap();
        std::fs::write(d.join("chat-history.json"), r#"[{"text":"SECRET-CHAT"}]"#).unwrap();
        std::fs::write(
            d.join("clipboard-history.json"),
            r#"[{"text":"SECRET-CLIP"}]"#,
        )
        .unwrap();
        std::fs::write(
            d.join("notification-history.json"),
            r#"[{"text":"SECRET-NOTIF"}]"#,
        )
        .unwrap();
        d
    }

    fn backup_of(dir: &Path) -> Backup {
        let mut sections = collect_files(dir);
        sections.appearance = Some(Appearance {
            theme: Some("nebula".into()),
            language: Some("hi".into()),
        });
        sections.sharing = Some(Sharing {
            screen_share: None,
            remote_input: None,
            permissions: Some(vec![mdcore::features::perms::PeerPermissions {
                node_id: "node-phone".into(),
                control: true,
                screen: false,
                kvm: false,
            }]),
            labels_share: true,
            battery_alerts: BatteryAlertSettings {
                enabled: true,
                low: 20,
                charged: 80,
            },
            remote_downloads: Some(mdcore::model::RemoteDownloadSettings {
                policy: "always".into(),
                ..Default::default()
            }),
        });
        sections.sync_folders = Some(SyncSection {
            max_size_mb: 512,
            pairs: vec![SyncDef {
                peer: "node-phone".into(),
                name: "Photos".into(),
                path: "/Users/a/Photos".into(),
                mode: "both".into(),
            }],
        });
        Backup {
            app: APP.into(),
            kind: KIND.into(),
            version: VERSION,
            platform: this_platform().into(),
            created_at: 1_700_000_000,
            devices: vec![BackupDevice {
                id: "node-phone".into(),
                name: "Pixel".into(),
            }],
            sections,
        }
    }

    #[test]
    fn the_allow_list_has_no_secrets() {
        for sec in &KV_SECTIONS {
            for key in sec.keys {
                assert!(!looks_secret(key), "{}: {key}", sec.id);
                assert!(allowed(sec.id, key));
            }
            for f in [
                "session.json",
                "dev-secrets.json",
                "fcm_creds_android.json",
                "fcm_config.json",
            ] {
                assert_ne!(sec.file, f);
            }
        }
        for key in [
            "webhook-token",
            "identity_key",
            "device_key",
            "session_token",
            "fcm_token",
            "automation_apps_key",
            "password",
            "send_shortcut_keys",
            "send_to_last",
        ] {
            for sec in &KV_SECTIONS {
                assert!(!allowed(sec.id, key), "{} let {key} through", sec.id);
            }
        }
    }

    #[test]
    fn secrets_and_contents_never_go_in_a_backup() {
        let d = seeded("secrets");
        let text = serde_json::to_string(&backup_of(&d)).unwrap();
        assert!(!text.contains("SECRET"), "{text}");
        for key in [
            "webhook-token",
            "session_token",
            "identity_key",
            "send_to_last",
            "calls_mac_audio",
            "phone_audio_output",
        ] {
            assert!(!text.contains(key), "{key} leaked");
        }
        // The settings themselves are there.
        assert!(text.contains("away_lock_minutes"));
        assert!(text.contains("com.spam"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn export_then_import_gives_the_same_settings() {
        let from = seeded("rt-from");
        let b = backup_of(&from);
        let text = serde_json::to_string_pretty(&b).unwrap();
        let back = parse(text.as_bytes()).unwrap();
        assert_eq!(back, b);

        let to = temp_dir("rt-to");
        // Here already: a key the backup clears, and a secret it keeps.
        put(
            &to,
            "prefs.json",
            &[("follow_dnd", "on"), ("send_to_last", "x")],
        );
        put(&to, "webhooks.json", &[("webhook-token", "KEEP-ME")]);
        let all: BTreeSet<String> = SECTION_IDS.iter().map(|s| s.to_string()).collect();
        let done = apply_files(&to, &back.sections, &all).unwrap();
        assert_eq!(done.len(), KV_SECTIONS.len() + FILE_SECTIONS.len());

        let again = collect_files(&to);
        let mut want = b.sections.clone();
        want.appearance = None;
        want.sharing = None;
        want.sync_folders = None;
        assert_eq!(again, want);
        // Not on the list: left alone.
        assert_eq!(
            read_kv(&to.join("webhooks.json"))
                .get("webhook-token")
                .map(String::as_str),
            Some("KEEP-ME")
        );
        assert_eq!(
            read_kv(&to.join("prefs.json"))
                .get("send_to_last")
                .map(String::as_str),
            Some("x")
        );
        // On the list but not in the backup: back to its default.
        assert_eq!(read_kv(&to.join("prefs.json")).get("follow_dnd"), None);
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn only_chosen_sections_are_written() {
        let from = seeded("chosen-from");
        let b = backup_of(&from);
        let to = temp_dir("chosen-to");
        let chosen: BTreeSet<String> = ["schedules".to_string()].into();
        assert_eq!(
            apply_files(&to, &b.sections, &chosen).unwrap(),
            vec!["schedules"]
        );
        assert!(to.join("schedules.json").exists());
        assert!(!to.join("prefs.json").exists());
        assert!(!to.join("shortcuts.json").exists());
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    fn with(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let d = seeded(&format!("edit-{}", rand_suffix()));
        let mut v = serde_json::to_value(backup_of(&d)).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        edit(&mut v);
        serde_json::to_vec(&v).unwrap()
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn bad_files_are_refused_with_a_reason() {
        assert!(parse(b"{ not json").unwrap_err().contains("valid JSON"));
        assert!(parse(b"[]")
            .unwrap_err()
            .contains("isn\u{2019}t a my-domain"));
        let newer = with(|v| v["version"] = 2.into());
        assert!(parse(&newer).unwrap_err().contains("newer version"));
        let zero = with(|v| v["version"] = 0.into());
        assert!(parse(&zero).unwrap_err().contains("format"));
        let kind = with(|v| v["kind"] = "notes".into());
        assert!(parse(&kind).is_err());
        let phone = with(|v| v["platform"] = "android".into());
        assert!(parse(&phone).unwrap_err().contains("phone app"));
        let extra = with(|v| v["sections"]["passwords"] = serde_json::json!({"x": "y"}));
        assert!(parse(&extra).is_err());
        // A secret slipped into an allowed section is refused, not written.
        let sneaky = with(|v| v["sections"]["webhooks"]["webhook-token"] = "abc".into());
        assert!(parse(&sneaky).unwrap_err().contains("webhook-token"));
        let unlisted = with(|v| v["sections"]["preferences"]["send_to_last"] = "x".into());
        assert!(parse(&unlisted).is_err());
        let broken = with(|v| v["sections"]["automations"]["automations"] = "[{\"id\":1}]".into());
        assert!(parse(&broken).is_err());
        let mode = with(|v| v["sections"]["sync_folders"]["pairs"][0]["mode"] = "mirror".into());
        assert!(parse(&mode).is_err());
        let theme = with(|v| v["sections"]["appearance"]["theme"] = "<script>".into());
        assert!(parse(&theme).is_err());
        let big = vec![b' '; (MAX_BYTES + 1) as usize];
        assert!(parse(&big).unwrap_err().contains("too big"));
    }

    #[test]
    fn sections_naming_unknown_devices_start_unticked() {
        let from = seeded("pv-from");
        let b = backup_of(&from);
        let to = temp_dir("pv-to");
        let cur = Current {
            appearance: Appearance {
                theme: Some("nebula".into()),
                language: None,
            },
            sharing: b.sections.sharing.clone(),
            sync: Vec::new(),
            max_size_mb: 512,
            known: BTreeSet::from(["node-laptop".to_string()]),
        };
        let p = preview(&to, &b, &cur);
        let get = |id: &str| p.iter().find(|s| s.id == id).unwrap().clone();
        // Names the phone, which isn't known here.
        for id in [
            "automations",
            "schedules",
            "device_labels",
            "sync_folders",
            "preferences",
        ] {
            let s = get(id);
            assert_eq!(s.unknown_devices, vec!["Pixel".to_string()], "{id}");
            assert!(!s.on, "{id}");
        }
        let cmds = get("commands");
        assert!(cmds.on && cmds.changed && cmds.unknown_devices.is_empty());
        assert_eq!(cmds.items, 1);
        assert_eq!(get("appearance").keys, vec!["language".to_string()]);
        assert!(!get("sharing").changed);
        assert_eq!(get("sync_folders").keys, vec!["Photos".to_string()]);

        // Known here: ticked.
        let mut cur = cur;
        cur.known.insert("node-phone".into());
        let p = preview(&to, &b, &cur);
        assert!(p.iter().all(|s| s.on));
        // Restored already: nothing changes.
        let all: BTreeSet<String> = SECTION_IDS.iter().map(|s| s.to_string()).collect();
        apply_files(&to, &b.sections, &all).unwrap();
        let p = preview(&to, &b, &cur);
        for s in p
            .iter()
            .filter(|s| !["appearance", "sync_folders"].contains(&s.id.as_str()))
        {
            assert!(!s.changed, "{}: {:?}", s.id, s.keys);
        }
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn an_older_backups_global_switches_are_read_never_applied() {
        let old: Sharing = serde_json::from_value(serde_json::json!({
            "screen_share": true,
            "remote_input": true,
            "labels_share": false,
            "battery_alerts": {"enabled": true, "low": 20, "charged": 80},
        }))
        .unwrap();
        assert_eq!(old.permissions, None);
        let again = serde_json::to_value(Sharing {
            screen_share: None,
            remote_input: None,
            ..old
        })
        .unwrap();
        assert!(again.get("screen_share").is_none());
        assert!(again.get("permissions").is_none());
    }

    #[test]
    fn changes_are_per_key() {
        let now = BTreeMap::from([
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ]);
        let then = BTreeMap::from([
            ("a".to_string(), "1".to_string()),
            ("c".to_string(), "3".to_string()),
        ]);
        assert_eq!(
            kv_changes(&now, &then),
            vec!["b".to_string(), "c".to_string()]
        );
    }
}
