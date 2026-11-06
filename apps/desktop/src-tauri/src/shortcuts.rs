// Keyboard shortcuts: keys you choose, pressed in any app, that make
// my-domain do something (Settings → "Keyboard shortcuts").
//
//  - each one is a binding: an action (`ACTIONS`), the device it acts on when
//    it needs one (its node id; the name shown is the nickname, if any), or
//    the automation it runs, the keys, and on/off;
//  - kept in `<app config dir>/shortcuts.json`, written a moment after a
//    change (`SAVE_DELAY`), through a temporary file and a rename;
//  - the first time (no file yet), the old "send what I copy to my phone"
//    setting (`send_shortcut` / `send_shortcut_keys` in `prefs.json`) becomes
//    the first binding, the same keys and on/off as before;
//  - registered with the OS through Tauri's global-shortcut plugin, here
//    only (the UI never registers anything), at startup and after every
//    change. When the OS won't take some keys (another app holds them) the
//    binding says so and the others still work;
//  - a press runs off the OS's event thread, and a notification says how it
//    went (`notify::post_always`: it answers something you just did).
//
// Keys are written the plugin's way ("CmdOrCtrl+Shift+Y"). Checked before
// they're kept: Cmd, Ctrl or Alt plus one key; not one the system or every
// app already uses (Cmd+Q, Alt+Tab, Ctrl+Alt+Delete…); not one another
// binding here uses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::{notify, Eng};

const FILE: &str = "shortcuts.json";
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(1);
/// The old switch and keys in `prefs.json` (carried over once).
const OLD_ON: &str = "send_shortcut";
const OLD_KEYS: &str = "send_shortcut_keys";
/// The id of the binding carried over from the old setting.
const FIRST_ID: &str = "send-clipboard";

/// "Send what I copied to my phone", out of the box.
pub const DEFAULT_KEYS: &str = "CmdOrCtrl+Shift+Y";

/// What a binding asks of the device it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// Nothing: it acts here.
    Nothing,
    /// A device of yours.
    Device,
    /// A device, or none for "your phone" (picked when pressed).
    DeviceOrPhone,
    /// One of your automations.
    Automation,
}

/// The actions a binding can run, and what each needs.
pub const ACTIONS: &[(&str, Needs)] = &[
    ("send_clipboard", Needs::DeviceOrPhone),
    ("ring", Needs::Device),
    ("lock", Needs::Device),
    ("locate", Needs::Device),
    ("send_file", Needs::Device),
    ("palette", Needs::Nothing),
    ("window", Needs::Nothing),
    ("phone_audio", Needs::Device),
    ("mute", Needs::Device),
    ("media", Needs::Device),
    ("wake", Needs::Device),
    ("automation", Needs::Automation),
];

fn needs(action: &str) -> Option<Needs> {
    ACTIONS.iter().find(|(a, _)| *a == action).map(|(_, n)| *n)
}

// --- keys ---

/// The computer's kind, for which keys mean what (Cmd is ⌘ on a Mac, the
/// Windows key on Windows, Super on Linux) and which are taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn here() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    fn name(self) -> &'static str {
        match self {
            Os::Mac => "macOS",
            Os::Windows => "Windows",
            Os::Linux => "Linux",
        }
    }
}

/// Keys as written: the modifiers and one key (its name as the plugin
/// writes it: "Y", "5", "F5", "Space", "ArrowUp", "Comma"…).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accel {
    /// Cmd on a Mac, Ctrl elsewhere.
    pub cmd_or_ctrl: bool,
    /// Cmd on a Mac, the Windows / Super key elsewhere.
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

/// The keys actually held on one kind of computer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Combo {
    pub meta: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

/// Named keys: what may be written (upper case) and the name kept.
const NAMED: &[(&str, &str)] = &[
    ("SPACE", "Space"),
    ("ENTER", "Enter"),
    ("RETURN", "Enter"),
    ("TAB", "Tab"),
    ("BACKSPACE", "Backspace"),
    ("DELETE", "Delete"),
    ("ESCAPE", "Escape"),
    ("ESC", "Escape"),
    ("HOME", "Home"),
    ("END", "End"),
    ("PAGEUP", "PageUp"),
    ("PAGEDOWN", "PageDown"),
    ("INSERT", "Insert"),
    ("ARROWUP", "ArrowUp"),
    ("UP", "ArrowUp"),
    ("ARROWDOWN", "ArrowDown"),
    ("DOWN", "ArrowDown"),
    ("ARROWLEFT", "ArrowLeft"),
    ("LEFT", "ArrowLeft"),
    ("ARROWRIGHT", "ArrowRight"),
    ("RIGHT", "ArrowRight"),
    ("COMMA", "Comma"),
    (",", "Comma"),
    ("PERIOD", "Period"),
    (".", "Period"),
    ("SLASH", "Slash"),
    ("/", "Slash"),
    ("BACKSLASH", "Backslash"),
    ("\\", "Backslash"),
    ("SEMICOLON", "Semicolon"),
    (";", "Semicolon"),
    ("QUOTE", "Quote"),
    ("'", "Quote"),
    ("BRACKETLEFT", "BracketLeft"),
    ("[", "BracketLeft"),
    ("BRACKETRIGHT", "BracketRight"),
    ("]", "BracketRight"),
    ("MINUS", "Minus"),
    ("-", "Minus"),
    ("EQUAL", "Equal"),
    ("=", "Equal"),
    ("BACKQUOTE", "Backquote"),
    ("`", "Backquote"),
];

/// The kept name of a key as written ("y", "KeyY", "Digit5", "f5", "up"…).
fn key_name(token: &str) -> Option<String> {
    let up = token.trim().to_ascii_uppercase();
    let single = |s: &str| {
        let mut c = s.chars();
        match (c.next(), c.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        }
    };
    if let Some(ch) = single(&up).filter(|c| c.is_ascii_alphanumeric()) {
        return Some(ch.to_string());
    }
    if let Some(ch) = up.strip_prefix("KEY").and_then(single) {
        if ch.is_ascii_alphabetic() {
            return Some(ch.to_string());
        }
    }
    if let Some(ch) = up.strip_prefix("DIGIT").and_then(single) {
        if ch.is_ascii_digit() {
            return Some(ch.to_string());
        }
    }
    if let Some(n) = up.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=24).contains(&n) && !up.starts_with("F0") {
            return Some(format!("F{n}"));
        }
    }
    NAMED
        .iter()
        .find(|(k, _)| *k == up)
        .map(|(_, v)| v.to_string())
}

/// Read keys as written ("CmdOrCtrl+Shift+Y"): modifiers, then one key.
/// Cmd, Ctrl or Alt is needed (Shift alone would steal typing).
pub fn parse_accel(keys: &str) -> Result<Accel, String> {
    let keys = keys.trim();
    let example = "like CmdOrCtrl+Shift+Y";
    if keys.is_empty() {
        return Err(format!("Press some keys, {example}."));
    }
    let tokens: Vec<&str> = keys.split('+').map(str::trim).collect();
    let mut a = Accel::default();
    let (last, mods) = tokens.split_last().unwrap_or((&"", &[]));
    for m in mods {
        match m.to_ascii_uppercase().as_str() {
            "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                a.cmd_or_ctrl = true
            }
            "COMMAND" | "CMD" | "SUPER" | "META" => a.cmd = true,
            "CONTROL" | "CTRL" => a.ctrl = true,
            "OPTION" | "ALT" => a.alt = true,
            "SHIFT" => a.shift = true,
            _ => {
                return Err(format!(
                    "\u{201c}{keys}\u{201d} isn't a shortcut. Write it {example}."
                ))
            }
        }
    }
    a.key = key_name(last).ok_or_else(|| {
        if last.is_empty() || is_modifier(last) {
            format!("Add a key after the modifiers, {example}.")
        } else {
            format!("\u{201c}{last}\u{201d} isn't a key a shortcut can use.")
        }
    })?;
    if !(a.cmd_or_ctrl || a.cmd || a.ctrl || a.alt) {
        return Err(format!("Add Cmd, Ctrl or Alt, {example}."));
    }
    Ok(a)
}

fn is_modifier(token: &str) -> bool {
    matches!(
        token.to_ascii_uppercase().as_str(),
        "COMMANDORCONTROL"
            | "COMMANDORCTRL"
            | "CMDORCTRL"
            | "CMDORCONTROL"
            | "COMMAND"
            | "CMD"
            | "SUPER"
            | "META"
            | "CONTROL"
            | "CTRL"
            | "OPTION"
            | "ALT"
            | "SHIFT"
    )
}

impl Accel {
    /// Written the way it's kept (and the plugin reads it).
    pub fn canonical(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.cmd_or_ctrl {
            parts.push("CmdOrCtrl");
        }
        if self.cmd {
            parts.push("Cmd");
        }
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// The keys held on `os`.
    pub fn resolve(&self, os: Os) -> Combo {
        let mac = os == Os::Mac;
        Combo {
            meta: self.cmd || (self.cmd_or_ctrl && mac),
            ctrl: self.ctrl || (self.cmd_or_ctrl && !mac),
            alt: self.alt,
            shift: self.shift,
            key: self.key.clone(),
        }
    }

    /// As people read them on `os`: "⇧⌘Y" on a Mac, "Ctrl+Shift+Y" elsewhere.
    pub fn display(&self, os: Os) -> String {
        self.resolve(os).display(os)
    }
}

impl Combo {
    pub fn display(&self, os: Os) -> String {
        if os == Os::Mac {
            // Apple's order: Control, Option, Shift, Command.
            let mut s = String::new();
            for (on, sym) in [
                (self.ctrl, '\u{2303}'),
                (self.alt, '\u{2325}'),
                (self.shift, '\u{21e7}'),
                (self.meta, '\u{2318}'),
            ] {
                if on {
                    s.push(sym);
                }
            }
            s.push_str(key_label(&self.key, os));
            return s;
        }
        let meta = if os == Os::Windows { "Win" } else { "Super" };
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, meta),
        ] {
            if on {
                parts.push(name);
            }
        }
        parts.push(key_label(&self.key, os));
        parts.join("+")
    }
}

/// A key's name as people read it.
fn key_label(key: &str, os: Os) -> &str {
    let mac = os == Os::Mac;
    match key {
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Minus" => "-",
        "Equal" => "=",
        "Backquote" => "`",
        "ArrowUp" if mac => "\u{2191}",
        "ArrowDown" if mac => "\u{2193}",
        "ArrowLeft" if mac => "\u{2190}",
        "ArrowRight" if mac => "\u{2192}",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "Enter" if mac => "\u{21a9}",
        "Tab" if mac => "\u{21e5}",
        "Escape" if mac => "\u{238b}",
        "Escape" => "Esc",
        "Backspace" if mac => "\u{232b}",
        "Delete" if mac => "\u{2326}",
        "PageUp" => "Page Up",
        "PageDown" => "Page Down",
        other => other,
    }
}

/// Keys the system, or every app, already uses on each kind of computer
/// (written with Cmd = ⌘ / the Windows key / Super).
const TAKEN_MAC: &[&str] = &[
    "Cmd+Q",
    "Cmd+W",
    "Cmd+Tab",
    "Cmd+Shift+Tab",
    "Cmd+Backquote",
    "Cmd+Space",
    "Ctrl+Space",
    "Cmd+Alt+Space",
    "Ctrl+Cmd+Space",
    "Cmd+Alt+Escape",
    "Ctrl+Cmd+Q",
    "Ctrl+Cmd+F",
    "Cmd+Shift+3",
    "Cmd+Shift+4",
    "Cmd+Shift+5",
    "Cmd+H",
    "Cmd+Alt+H",
    "Cmd+M",
    "Cmd+Alt+D",
    "Cmd+C",
    "Cmd+V",
    "Cmd+X",
    "Cmd+Z",
    "Cmd+Shift+Z",
    "Cmd+A",
    "Cmd+S",
    "Cmd+F",
    "Cmd+N",
    "Cmd+O",
    "Cmd+P",
    "Cmd+T",
    "Cmd+Comma",
];
const TAKEN_WINDOWS: &[&str] = &[
    "Ctrl+Alt+Delete",
    "Ctrl+Shift+Escape",
    "Ctrl+Escape",
    "Alt+Tab",
    "Alt+Shift+Tab",
    "Ctrl+Alt+Tab",
    "Alt+F4",
    "Alt+Space",
    "Alt+Escape",
    "Cmd+L",
    "Cmd+D",
    "Cmd+E",
    "Cmd+R",
    "Cmd+Tab",
    "Cmd+Shift+S",
    "Ctrl+C",
    "Ctrl+V",
    "Ctrl+X",
    "Ctrl+Z",
    "Ctrl+Y",
    "Ctrl+A",
    "Ctrl+S",
    "Ctrl+F",
    "Ctrl+N",
    "Ctrl+O",
    "Ctrl+P",
    "Ctrl+T",
    "Ctrl+W",
];
const TAKEN_LINUX: &[&str] = &[
    "Ctrl+Alt+Delete",
    "Ctrl+Alt+Backspace",
    "Ctrl+Alt+T",
    "Ctrl+Alt+L",
    "Ctrl+Alt+ArrowUp",
    "Ctrl+Alt+ArrowDown",
    "Ctrl+Alt+ArrowLeft",
    "Ctrl+Alt+ArrowRight",
    "Alt+Tab",
    "Alt+Shift+Tab",
    "Alt+F2",
    "Alt+F4",
    "Alt+Space",
    "Cmd+L",
    "Cmd+Tab",
    "Cmd+Space",
    "Ctrl+C",
    "Ctrl+V",
    "Ctrl+X",
    "Ctrl+Z",
    "Ctrl+Shift+Z",
    "Ctrl+A",
    "Ctrl+S",
    "Ctrl+F",
    "Ctrl+N",
    "Ctrl+O",
    "Ctrl+P",
    "Ctrl+Q",
    "Ctrl+T",
    "Ctrl+W",
];

/// Why `a` can't be used on `os`, when the system or every app has it.
pub fn reserved(a: &Accel, os: Os) -> Option<String> {
    let list = match os {
        Os::Mac => TAKEN_MAC,
        Os::Windows => TAKEN_WINDOWS,
        Os::Linux => TAKEN_LINUX,
    };
    let held = a.resolve(os);
    list.iter()
        .filter_map(|k| parse_accel(k).ok())
        .any(|t| t.resolve(os) == held)
        .then(|| {
            format!(
                "{} is used by {} or by every app. Choose other keys.",
                held.display(os),
                os.name()
            )
        })
}

/// The other binding (not `id`) whose keys are the same on `os`, if any.
pub fn clash<'a>(list: &'a [Binding], id: &str, a: &Accel, os: Os) -> Option<&'a Binding> {
    let held = a.resolve(os);
    list.iter().filter(|b| b.id != id).find(|b| {
        parse_accel(&b.keys)
            .map(|o| o.resolve(os) == held)
            .unwrap_or(false)
    })
}

// --- bindings ---

/// One keyboard shortcut.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, PartialEq)]
pub struct Binding {
    /// "" for a new one (an id is given when it's kept).
    #[serde(default)]
    pub id: String,
    /// One of `ACTIONS`.
    pub action: String,
    /// The device it acts on (node id); "" when it needs none, or for
    /// "send the clipboard" to your phone, whichever it is then.
    #[serde(default)]
    pub node: String,
    /// The automation it runs (id), for "automation".
    #[serde(default)]
    pub automation: String,
    /// "CmdOrCtrl+Shift+Y".
    pub keys: String,
    #[serde(default = "on")]
    pub enabled: bool,
}

fn on() -> bool {
    true
}

/// `shortcuts.json`.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Stored {
    version: u32,
    bindings: Vec<Binding>,
}

/// Check one binding and tidy it (keys as kept, nothing it doesn't need).
/// Keys taken or used twice are checked separately (`check_keys`).
pub fn checked(mut b: Binding) -> Result<Binding, String> {
    let needs = needs(&b.action).ok_or("That isn't something a shortcut can do.")?;
    b.keys = parse_accel(&b.keys)?.canonical();
    b.node = b.node.trim().to_string();
    b.automation = b.automation.trim().to_string();
    match needs {
        Needs::Nothing => {
            b.node.clear();
            b.automation.clear();
        }
        Needs::Device | Needs::DeviceOrPhone => {
            b.automation.clear();
            if needs == Needs::Device && b.node.is_empty() {
                return Err("Choose a device.".into());
            }
        }
        Needs::Automation => {
            b.node.clear();
            if b.automation.is_empty() {
                return Err("Choose an automation.".into());
            }
        }
    }
    Ok(b)
}

/// Keys for binding `id` on `os`: shortcut keys, not taken by the system,
/// not used by another binding. `name` describes a binding for the message.
pub fn check_keys(
    list: &[Binding],
    id: &str,
    keys: &str,
    os: Os,
    name: impl Fn(&Binding) -> String,
) -> Result<Accel, String> {
    let a = parse_accel(keys)?;
    if let Some(why) = reserved(&a, os) {
        return Err(why);
    }
    if let Some(other) = clash(list, id, &a, os) {
        return Err(format!(
            "{} is already the shortcut for \u{201c}{}\u{201d}.",
            a.display(os),
            name(other)
        ));
    }
    Ok(a)
}

/// The bindings from the old setting (`send_shortcut` "on"/"off",
/// `send_shortcut_keys`): the same keys, on or off as it was.
pub fn migrate(on: Option<&str>, keys: Option<&str>) -> Vec<Binding> {
    let keys = keys
        .and_then(|k| parse_accel(k).ok())
        .map(|a| a.canonical())
        .unwrap_or_else(|| DEFAULT_KEYS.to_string());
    vec![Binding {
        id: FIRST_ID.into(),
        action: "send_clipboard".into(),
        node: String::new(),
        automation: String::new(),
        keys,
        enabled: on != Some("off"),
    }]
}

/// The bindings in `shortcuts.json`'s text; ones that don't check out are
/// left out. `None` when it isn't the file.
pub fn read_file(json: &str) -> Option<Vec<Binding>> {
    let stored: Stored = serde_json::from_str(json).ok()?;
    let mut out: Vec<Binding> = Vec::new();
    for b in stored.bindings {
        if b.id.is_empty() || out.iter().any(|o| o.id == b.id) {
            continue;
        }
        if let Ok(b) = checked(b) {
            out.push(b);
        }
    }
    Some(out)
}

fn to_file(list: &[Binding]) -> String {
    serde_json::to_string_pretty(&Stored {
        version: 1,
        bindings: list.to_vec(),
    })
    .unwrap_or_default()
}

// --- state ---

pub struct Shortcuts {
    /// `<app config dir>/shortcuts.json`.
    path: Option<PathBuf>,
    list: Mutex<Vec<Binding>>,
    /// Why a binding isn't working (binding id -> why).
    problems: Mutex<HashMap<String, String>>,
    /// What the OS has (shortcut id -> binding id).
    registered: Mutex<HashMap<u32, String>>,
    /// No global shortcuts on this computer.
    unavailable: AtomicBool,
    /// Taken off while Settings records new keys.
    paused: AtomicBool,
    save_pending: AtomicBool,
    /// Whether each device said it's muted (for "mute / unmute").
    muted: Mutex<HashMap<String, bool>>,
}

impl Shortcuts {
    fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let path = dir.as_ref().map(|d| d.join(FILE));
        let from_file = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| read_file(&json));
        let (list, carried) = match from_file {
            Some(list) => (list, false),
            None => {
                let kv = dir.map(|d| FileKv::new(d.join("prefs.json")));
                let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
                (
                    migrate(get(OLD_ON).as_deref(), get(OLD_KEYS).as_deref()),
                    true,
                )
            }
        };
        let s = Shortcuts {
            path,
            list: Mutex::new(list),
            problems: Mutex::new(HashMap::new()),
            registered: Mutex::new(HashMap::new()),
            unavailable: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            save_pending: AtomicBool::new(false),
            muted: Mutex::new(HashMap::new()),
        };
        if carried {
            s.save_now();
        }
        s
    }

    fn list(&self) -> Vec<Binding> {
        self.list.lock().map(|l| l.clone()).unwrap_or_default()
    }

    fn get(&self, id: &str) -> Option<Binding> {
        self.list.lock().ok()?.iter().find(|b| b.id == id).cloned()
    }

    fn save_soon(&self, app: &AppHandle) {
        if self.save_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            if let Some(s) = app.try_state::<Shortcuts>() {
                s.save_pending.store(false, Ordering::Release);
                s.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let json = to_file(&self.list());
        if let Err(e) = write_atomic(path, &json) {
            eprintln!("[shortcuts] couldn't save the shortcuts: {e}");
        }
    }

    fn view(&self, app: &AppHandle) -> ShortcutList {
        let os = Os::here();
        let problems = self.problems.lock().map(|p| p.clone()).unwrap_or_default();
        let unavailable = self.unavailable.load(Ordering::Relaxed);
        ShortcutList {
            bindings: self
                .list()
                .into_iter()
                .map(|b| ShortcutView {
                    label: parse_accel(&b.keys)
                        .map(|a| a.display(os))
                        .unwrap_or_else(|_| b.keys.clone()),
                    problem: problems.get(&b.id).cloned(),
                    name: describe(app, &b),
                    binding: b,
                })
                .collect(),
            unavailable,
            mac: os == Os::Mac,
            saved: String::new(),
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

/// Load the bindings, add the plugin, and register them.
pub fn init(app: &AppHandle) {
    app.manage(Shortcuts::load(app));
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(on_event)
        .build();
    if let Err(e) = app.plugin(plugin) {
        eprintln!("[shortcuts] no global shortcuts here: {e}");
        if let Some(s) = app.try_state::<Shortcuts>() {
            s.unavailable.store(true, Ordering::Relaxed);
        }
        return;
    }
    apply(app);
}

/// Register every binding that's on (and none while paused); note the ones
/// the OS wouldn't take.
fn apply(app: &AppHandle) {
    let Some(s) = app.try_state::<Shortcuts>() else {
        return;
    };
    if s.unavailable.load(Ordering::Relaxed) {
        return;
    }
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        eprintln!("[shortcuts] couldn't clear the shortcuts: {e}");
    }
    let mut problems = HashMap::new();
    let mut registered: HashMap<u32, String> = HashMap::new();
    if !s.paused.load(Ordering::Relaxed) {
        let os = Os::here();
        for b in s.list().into_iter().filter(|b| b.enabled) {
            let accel = match parse_accel(&b.keys) {
                Ok(a) => a,
                Err(e) => {
                    problems.insert(b.id, e);
                    continue;
                }
            };
            let label = accel.display(os);
            let shortcut = match Shortcut::from_str(&accel.canonical()) {
                Ok(sc) => sc,
                Err(_) => {
                    problems.insert(b.id, format!("{label} can't be used here."));
                    continue;
                }
            };
            if registered.contains_key(&shortcut.id()) {
                problems.insert(
                    b.id,
                    format!("Another my-domain shortcut already uses {label}."),
                );
                continue;
            }
            match gs.register(shortcut) {
                Ok(()) => {
                    registered.insert(shortcut.id(), b.id);
                }
                Err(e) => {
                    eprintln!("[shortcuts] couldn't register {}: {e}", b.keys);
                    problems.insert(
                        b.id,
                        format!("{label} couldn't be used: another app has it. Choose other keys."),
                    );
                }
            }
        }
    }
    if let Ok(mut p) = s.problems.lock() {
        *p = problems;
    }
    if let Ok(mut r) = s.registered.lock() {
        *r = registered;
    };
}

/// A press (from the OS's event thread): run its binding on another.
fn on_event(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let Some(s) = app.try_state::<Shortcuts>() else {
        return;
    };
    let id = s
        .registered
        .lock()
        .ok()
        .and_then(|r| r.get(&shortcut.id()).cloned());
    let Some(b) = id.and_then(|id| s.get(&id)) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || run(&app, &b));
}

/// A device's status arrived: remember whether it's muted.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(muted) = status.get("muted").and_then(|m| m.as_bool()) else {
        return;
    };
    if let Some(s) = app.try_state::<Shortcuts>() {
        if let Ok(mut m) = s.muted.lock() {
            m.insert(from.to_string(), muted);
        }
    }
}

// --- what a press does ---

fn say(app: &AppHandle, title: &str, body: &str) {
    notify::post_always(app, title, body);
}

/// An action sent to a device: say it's done, or that it goes once the
/// device is reachable.
pub(crate) fn sent(app: &AppHandle, what: &str, name: &str, result: Result<String, CoreError>) {
    match result {
        Ok(via) if via == "queued" => say(
            app,
            what,
            &format!("{name} isn\u{2019}t reachable right now. It\u{2019}ll happen once it is."),
        ),
        Ok(_) => say(app, what, &format!("Sent to {name}.")),
        Err(e) => say(
            app,
            &format!("Couldn\u{2019}t do \u{201c}{what}\u{201d}"),
            &e.to_string(),
        ),
    }
}

fn run(app: &AppHandle, b: &Binding) {
    let node = b.node.as_str();
    let name = || notify::device_name(app, node);
    match b.action.as_str() {
        "send_clipboard" if node.is_empty() => crate::shortcut::send_copied(app),
        "send_clipboard" => {
            if let Some(engine) = app.try_state::<Eng>() {
                crate::shortcut::send_clipboard_to(app, &engine, node);
            }
        }
        "ring" => crate::tray::ring(app, node),
        "locate" => crate::tray::locate(app, node),
        "send_file" => crate::tray::send_file(app, node),
        "lock" => sent(
            app,
            &format!("Lock {}", name()),
            &name(),
            crate::schedule::run_action(app, node, "lock", ""),
        ),
        "media" => sent(
            app,
            &format!("Play / pause on {}", name()),
            &name(),
            crate::schedule::run_action(app, node, "media", r#"{"key":"play_pause"}"#),
        ),
        "wake" => sent(
            app,
            &format!("Wake {}", name()),
            &name(),
            crate::schedule::run_action(app, node, "wake", ""),
        ),
        "mute" => {
            let muted = app
                .try_state::<Shortcuts>()
                .and_then(|s| s.muted.lock().ok().and_then(|m| m.get(node).copied()))
                .unwrap_or(false);
            let what = if muted {
                format!("Unmute {}", name())
            } else {
                format!("Mute {}", name())
            };
            let args = serde_json::json!({ "on": !muted }).to_string();
            let result = crate::schedule::run_action(app, node, "mute", &args);
            if result.is_ok() {
                // Pressed again before its status says so: flip it back.
                if let Some(s) = app.try_state::<Shortcuts>() {
                    if let Ok(mut m) = s.muted.lock() {
                        m.insert(node.to_string(), !muted);
                    }
                }
            }
            sent(app, &what, &name(), result);
        }
        "phone_audio" => {
            let status = crate::phone_audio::phone_audio_status(app.clone());
            let playing = status.node.as_deref() == Some(node) && status.state != "idle";
            if playing {
                crate::phone_audio::phone_audio_stop(app.clone());
                say(
                    app,
                    "my-domain",
                    &format!("Stopped playing {}\u{2019}s audio here", name()),
                );
            } else {
                match crate::phone_audio::phone_audio_start(node.to_string(), app.clone()) {
                    Ok(()) => say(
                        app,
                        "my-domain",
                        &format!(
                            "Asked {} to play its audio here. Say yes on the phone.",
                            name()
                        ),
                    ),
                    Err(e) => say(
                        app,
                        &format!("Couldn\u{2019}t play {}\u{2019}s audio here", name()),
                        &e,
                    ),
                }
            }
        }
        "palette" => {
            crate::tray::show_window(app);
            let _ = app.emit("open-palette", ());
        }
        "window" => {
            let Some(w) = app.get_webview_window("main") else {
                return;
            };
            let shown = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
            if shown && w.is_focused().unwrap_or(false) {
                let _ = w.hide();
            } else {
                crate::tray::show_window(app);
            }
        }
        "automation" => match crate::automations::run_now(app, &b.automation) {
            Ok((what, via)) if via == "queued" => say(
                app,
                &format!("Ran \u{201c}{what}\u{201d}"),
                "It\u{2019}ll happen once the device is reachable.",
            ),
            Ok((what, _)) => say(app, &format!("Ran \u{201c}{what}\u{201d}"), "Done."),
            Err(e) => say(app, "Couldn\u{2019}t run the automation", &e),
        },
        _ => {}
    }
}

/// What a binding does, for people ("Ring Pixel 8"), in Rust's messages.
fn describe(app: &AppHandle, b: &Binding) -> String {
    let name = || notify::device_name(app, &b.node);
    match b.action.as_str() {
        "send_clipboard" if b.node.is_empty() => "Send what I copied to my phone".into(),
        "send_clipboard" => format!("Send what I copied to {}", name()),
        "ring" => format!("Ring {}", name()),
        "lock" => format!("Lock {}", name()),
        "locate" => format!("Locate {}", name()),
        "send_file" => format!("Send a file to {}", name()),
        "palette" => "Open the command palette".into(),
        "window" => "Show or hide my-domain".into(),
        "phone_audio" => format!("Play {}\u{2019}s audio here", name()),
        "mute" => format!("Mute or unmute {}", name()),
        "media" => format!("Play / pause on {}", name()),
        "wake" => format!("Wake {}", name()),
        "automation" => {
            let label = app
                .try_state::<crate::automations::Automations>()
                .map(crate::automations::list_automations)
                .and_then(|l| l.into_iter().find(|a| a.id == b.automation))
                .map(|a| {
                    if a.label.is_empty() {
                        a.action
                    } else {
                        a.label
                    }
                })
                .unwrap_or_else(|| b.automation.clone());
            format!("Run the automation \u{201c}{label}\u{201d}")
        }
        other => other.to_string(),
    }
}

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("k{nanos:x}")
}

// --- commands ---

/// One binding as Settings shows it.
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct ShortcutView {
    pub binding: Binding,
    /// The keys for people ("⇧⌘Y" on a Mac, "Ctrl+Shift+Y" elsewhere).
    pub label: String,
    /// What it does, in English (for messages; the UI says it itself).
    pub name: String,
    /// Why it isn't working (the OS wouldn't take the keys…).
    pub problem: Option<String>,
}

/// Every binding, and whether shortcuts work here at all.
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct ShortcutList {
    pub bindings: Vec<ShortcutView>,
    /// Global shortcuts aren't available on this computer.
    pub unavailable: bool,
    /// This is a Mac (Cmd is ⌘).
    pub mac: bool,
    /// The id of the binding just added or changed ("" otherwise).
    pub saved: String,
}

/// Keys that were checked: as kept, and for people.
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct CheckedKeys {
    pub keys: String,
    pub label: String,
}

/// Your keyboard shortcuts.
#[tauri::command]
#[specta::specta]
pub fn list_shortcuts(app: AppHandle, state: State<Shortcuts>) -> ShortcutList {
    state.view(&app)
}

/// Whether `keys` can be the keys of binding `id` ("" for a new one): the
/// keys as kept and for people, or why not.
#[tauri::command]
#[specta::specta]
pub fn check_shortcut_keys(
    keys: String,
    id: String,
    app: AppHandle,
    state: State<Shortcuts>,
) -> Result<CheckedKeys, CoreError> {
    let os = Os::here();
    let a = check_keys(&state.list(), &id, &keys, os, |b| describe(&app, b))
        .map_err(CoreError::InvalidInput)?;
    Ok(CheckedKeys {
        keys: a.canonical(),
        label: a.display(os),
    })
}

/// Add a binding (id "") or change one; registered at once. The list comes
/// back, with a problem on it if the OS wouldn't take its keys.
#[tauri::command]
#[specta::specta]
pub fn save_shortcut(
    binding: Binding,
    app: AppHandle,
    state: State<Shortcuts>,
) -> Result<ShortcutList, CoreError> {
    let mut b = checked(binding).map_err(CoreError::InvalidInput)?;
    let os = Os::here();
    let saved = {
        let mut list = state
            .list
            .lock()
            .map_err(|_| CoreError::Other("busy".into()))?;
        check_keys(&list, &b.id, &b.keys, os, |o| describe(&app, o))
            .map_err(CoreError::InvalidInput)?;
        match list.iter_mut().find(|o| !b.id.is_empty() && o.id == b.id) {
            Some(old) => {
                *old = b;
                old.id.clone()
            }
            None => {
                b.id = new_id();
                let id = b.id.clone();
                list.push(b);
                id
            }
        }
    };
    state.save_soon(&app);
    apply(&app);
    Ok(ShortcutList {
        saved,
        ..state.view(&app)
    })
}

/// Remove a binding.
#[tauri::command]
#[specta::specta]
pub fn delete_shortcut(id: String, app: AppHandle, state: State<Shortcuts>) -> ShortcutList {
    if let Ok(mut list) = state.list.lock() {
        list.retain(|b| b.id != id);
    }
    state.save_soon(&app);
    apply(&app);
    state.view(&app)
}

/// Take every shortcut off while Settings records new keys (so pressing
/// one there is seen, not run), and put them back after.
#[tauri::command]
#[specta::specta]
pub fn pause_shortcuts(paused: bool, app: AppHandle, state: State<Shortcuts>) {
    if state.paused.swap(paused, Ordering::Relaxed) != paused {
        apply(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(id: &str, keys: &str) -> Binding {
        Binding {
            id: id.into(),
            action: "ring".into(),
            node: "n1".into(),
            automation: String::new(),
            keys: keys.into(),
            enabled: true,
        }
    }

    #[test]
    fn shortcuts_parse_keys() {
        let a = parse_accel("CmdOrCtrl+Shift+y").unwrap();
        assert!(a.cmd_or_ctrl && a.shift && !a.cmd && !a.ctrl && !a.alt);
        assert_eq!(a.key, "Y");
        assert_eq!(a.canonical(), "CmdOrCtrl+Shift+Y");
        // What the recorder sends (KeyboardEvent.code-like) is tidied.
        assert_eq!(
            parse_accel("Cmd+Alt+KeyK").unwrap().canonical(),
            "Cmd+Alt+K"
        );
        assert_eq!(parse_accel("ctrl+digit5").unwrap().canonical(), "Ctrl+5");
        assert_eq!(parse_accel("Alt+f5").unwrap().canonical(), "Alt+F5");
        assert_eq!(
            parse_accel("Ctrl+Shift+up").unwrap().canonical(),
            "Ctrl+Shift+ArrowUp"
        );
        assert_eq!(parse_accel("Option+,").unwrap().canonical(), "Alt+Comma");
        assert_eq!(
            parse_accel(" Super + Space ").unwrap().canonical(),
            "Cmd+Space"
        );
        // The plugin reads every kept form.
        for k in [
            "CmdOrCtrl+Shift+Y",
            "Cmd+Alt+K",
            "Ctrl+5",
            "Alt+F24",
            "Ctrl+Shift+ArrowUp",
            "Alt+Comma",
            "Ctrl+Backquote",
        ] {
            let kept = parse_accel(k).unwrap().canonical();
            assert!(Shortcut::from_str(&kept).is_ok(), "{kept}");
        }
    }

    #[test]
    fn shortcuts_need_a_modifier_and_a_key() {
        for bad in [
            "",
            "Y",
            "Shift+Y",
            "Ctrl+",
            "Ctrl+Shift",
            "Ctrl+Shift+Nope",
            "Hyper+Y",
            "Ctrl+F0",
            "Ctrl+F25",
            "Ctrl+Y+Z",
        ] {
            assert!(parse_accel(bad).is_err(), "{bad}");
        }
        assert!(parse_accel("Alt+Y").is_ok());
    }

    #[test]
    fn shortcuts_show_per_os() {
        let a = parse_accel("CmdOrCtrl+Shift+Y").unwrap();
        assert_eq!(a.display(Os::Mac), "\u{21e7}\u{2318}Y");
        assert_eq!(a.display(Os::Windows), "Ctrl+Shift+Y");
        assert_eq!(a.display(Os::Linux), "Ctrl+Shift+Y");
        let b = parse_accel("Ctrl+Alt+Cmd+ArrowLeft").unwrap();
        assert_eq!(b.display(Os::Mac), "\u{2303}\u{2325}\u{2318}\u{2190}");
        assert_eq!(b.display(Os::Windows), "Ctrl+Alt+Win+Left");
        assert_eq!(b.display(Os::Linux), "Ctrl+Alt+Super+Left");
        assert_eq!(
            parse_accel("Alt+Slash").unwrap().display(Os::Linux),
            "Alt+/"
        );
    }

    #[test]
    fn shortcuts_cmd_or_ctrl_resolves_per_os() {
        let a = parse_accel("CmdOrCtrl+K").unwrap();
        assert_eq!(
            a.resolve(Os::Mac),
            parse_accel("Cmd+K").unwrap().resolve(Os::Mac)
        );
        assert_eq!(
            a.resolve(Os::Windows),
            parse_accel("Ctrl+K").unwrap().resolve(Os::Windows)
        );
        assert_ne!(
            a.resolve(Os::Mac),
            parse_accel("Ctrl+K").unwrap().resolve(Os::Mac)
        );
    }

    #[test]
    fn shortcuts_reject_reserved_keys() {
        let r = |k: &str, os| reserved(&parse_accel(k).unwrap(), os);
        assert!(r("Cmd+Q", Os::Mac).is_some());
        assert!(r("CmdOrCtrl+Q", Os::Mac).is_some());
        assert!(r("Cmd+Tab", Os::Mac).is_some());
        assert!(r("Cmd+Space", Os::Mac).is_some());
        assert!(r("Cmd+Shift+4", Os::Mac).is_some());
        assert!(r("Ctrl+Alt+Delete", Os::Windows).is_some());
        assert!(r("Ctrl+Alt+Delete", Os::Linux).is_some());
        assert!(r("Alt+Tab", Os::Windows).is_some());
        assert!(r("Alt+F4", Os::Linux).is_some());
        assert!(r("CmdOrCtrl+C", Os::Windows).is_some());
        // Cmd+Q is a Mac thing: Ctrl+Q is free on Windows.
        assert!(r("CmdOrCtrl+Q", Os::Windows).is_none());
        assert!(r(DEFAULT_KEYS, Os::Mac).is_none());
        assert!(r(DEFAULT_KEYS, Os::Windows).is_none());
        assert!(r(DEFAULT_KEYS, Os::Linux).is_none());
        let why = r("Cmd+Q", Os::Mac).unwrap();
        assert!(why.contains("\u{2318}Q") && why.contains("macOS"), "{why}");
        // Every listed one reads.
        for k in TAKEN_MAC.iter().chain(TAKEN_WINDOWS).chain(TAKEN_LINUX) {
            assert!(parse_accel(k).is_ok(), "{k}");
        }
    }

    #[test]
    fn shortcuts_find_clashes() {
        let list = vec![bind("a", "CmdOrCtrl+Shift+Y"), bind("b", "Ctrl+Alt+R")];
        let a = |k: &str| parse_accel(k).unwrap();
        // The same keys written another way.
        assert_eq!(
            clash(&list, "", &a("Cmd+Shift+Y"), Os::Mac).map(|b| b.id.as_str()),
            Some("a")
        );
        assert_eq!(
            clash(&list, "", &a("Ctrl+Shift+Y"), Os::Windows).map(|b| b.id.as_str()),
            Some("a")
        );
        assert!(clash(&list, "", &a("Ctrl+Shift+Y"), Os::Mac).is_none());
        // A binding doesn't clash with itself.
        assert!(clash(&list, "a", &a("CmdOrCtrl+Shift+Y"), Os::Mac).is_none());
        let err = check_keys(&list, "x", "Alt+Ctrl+R", Os::Linux, |b| b.id.clone()).unwrap_err();
        assert!(
            err.contains("Ctrl+Alt+R") && err.contains("\u{201c}b\u{201d}"),
            "{err}"
        );
        assert!(check_keys(&list, "x", "Cmd+Q", Os::Mac, |b| b.id.clone()).is_err());
        assert!(check_keys(&list, "x", "Ctrl+Alt+P", Os::Mac, |b| b.id.clone()).is_ok());
    }

    #[test]
    fn shortcuts_migrate_the_old_setting() {
        let first = |v: Vec<Binding>| v.into_iter().next().unwrap();
        let b = first(migrate(None, None));
        assert_eq!(
            (b.id.as_str(), b.action.as_str(), b.node.as_str()),
            (FIRST_ID, "send_clipboard", "")
        );
        assert_eq!(b.keys, DEFAULT_KEYS);
        assert!(b.enabled);
        let b = first(migrate(Some("off"), Some("ctrl+alt+p")));
        assert_eq!(b.keys, "Ctrl+Alt+P");
        assert!(!b.enabled);
        assert!(first(migrate(Some("on"), None)).enabled);
        // Keys that never worked fall back, as they did.
        assert_eq!(
            first(migrate(None, Some("Ctrl+Shift+Nope"))).keys,
            DEFAULT_KEYS
        );
        // And it reads back from the file the same.
        let list = migrate(Some("off"), Some("Alt+K"));
        assert_eq!(read_file(&to_file(&list)), Some(list));
    }

    #[test]
    fn shortcuts_read_the_file() {
        assert_eq!(read_file("not json"), None);
        let json = r#"{"version":1,"bindings":[
            {"id":"a","action":"ring","node":"n1","keys":"cmdorctrl+shift+r"},
            {"id":"a","action":"ring","node":"n2","keys":"Ctrl+Alt+R"},
            {"id":"b","action":"teleport","keys":"Ctrl+Alt+T"},
            {"id":"c","action":"ring","node":"","keys":"Ctrl+Alt+U"},
            {"id":"d","action":"palette","node":"n1","keys":"Alt+Space","enabled":false},
            {"id":"e","action":"automation","automation":"x1","keys":"Alt+1"}
        ]}"#;
        let list = read_file(json).unwrap();
        let ids: Vec<&str> = list.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["a", "d", "e"]);
        assert_eq!(list[0].keys, "CmdOrCtrl+Shift+R");
        assert!(list[0].enabled);
        // What doesn't need a device names none.
        assert_eq!(list[1].node, "");
        assert!(!list[1].enabled);
        assert_eq!(list[2].automation, "x1");
    }

    #[test]
    fn shortcuts_check_what_they_need() {
        let mut b = bind("", "Alt+R");
        b.node.clear();
        assert!(checked(b.clone()).is_err());
        b.action = "send_clipboard".into();
        assert!(checked(b.clone()).is_ok());
        b.action = "automation".into();
        assert!(checked(b.clone()).is_err());
        b.automation = "x".into();
        assert!(checked(b.clone()).is_ok());
        b.action = "nope".into();
        assert!(checked(b).is_err());
    }
}
