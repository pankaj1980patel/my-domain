// Automations: "when this happens, do that", e.g. "when my phone connects,
// set its ringer to normal" or "when my phone's battery drops below 20%,
// stop keeping it awake". Set up in Settings (Automations).
//
//  - kept in `<app config dir>/automations.json` (key `automations`, a JSON
//    list);
//  - what can start one: a device connects or disconnects, its battery drops
//    below a level, it starts or stops charging, or this computer wakes from
//    sleep;
//  - "disconnects" waits a minute without the link first, so a blip doesn't
//    count (and the link coming back within it isn't a "connects" either);
//  - battery and charging fire on the change (above → below the level,
//    not charging → charging), not again while it stays that way; the first
//    status seen after the app starts only sets where things stand;
//  - waking is noticed by the wall clock jumping ahead between two looks a
//    few seconds apart (the looks stop while the computer sleeps);
//  - what it does is a schedule's action, run the same way
//    (`schedule::run_action`): here for this computer, else sent, and kept
//    for a device that isn't reachable; or just a notification ("notify");
//  - one automation fires at most once a minute, and a notification says
//    each time it does (or why it couldn't);
//  - only while my-domain is running.
//
// "Lock this computer when my phone goes away" (`away_lock.rs`) is its own
// setting, with a longer wait and any phone counting; an automation can do
// much the same for one device ("disconnects" → lock this computer).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Local;
use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

const KEY: &str = "automations";
/// The most automations kept.
const MAX_AUTOMATIONS: usize = 100;
/// The longest label.
const MAX_LABEL: usize = 80;
/// How long a link must stay down to count as "disconnects".
const AWAY_AFTER: Duration = Duration::from_secs(60);
/// The least time between two firings of one automation.
const COOL_DOWN: Duration = Duration::from_secs(60);
/// How often the links (and the clock) are looked at.
const TICK: Duration = Duration::from_secs(15);
/// The wall clock moving this much more than a tick between two looks
/// means the computer slept.
const SLEPT_SECS: u64 = 60;

/// What can start an automation (`Trigger::kind`).
pub const KINDS: [&str; 6] = [
    "connects",
    "disconnects",
    "battery_below",
    "charging_starts",
    "charging_stops",
    "wakes",
];

/// The action that only posts a notification.
const NOTIFY: &str = "notify";

/// What starts an automation.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, PartialEq)]
pub struct Trigger {
    /// One of `KINDS`.
    pub kind: String,
    /// The device it watches ("" for "wakes": this computer).
    #[serde(default)]
    pub node: String,
    /// For "battery_below": the percent (1–99).
    #[serde(default)]
    pub level: Option<u8>,
}

/// One automation.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, PartialEq)]
pub struct Automation {
    pub id: String,
    pub trigger: Trigger,
    /// The device the action runs on.
    pub node: String,
    /// An action the device lists in its status, "ring", or "notify".
    pub action: String,
    /// The action's arguments, as JSON text ("" for none).
    #[serde(default)]
    pub args: String,
    /// What it does, for people ("Set the ringer to normal").
    #[serde(default)]
    pub label: String,
    pub enabled: bool,
    /// The last time it fired, local "YYYY-MM-DD HH:MM".
    #[serde(default)]
    pub last_fired: Option<String>,
}

/// A new automation, from the UI.
#[derive(serde::Deserialize, specta::Type, Clone, Debug)]
pub struct NewAutomation {
    pub trigger: Trigger,
    pub node: String,
    pub action: String,
    pub args: String,
    pub label: String,
    pub enabled: bool,
}

/// Something that happened, that an automation may be waiting for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Connected,
    /// The link has been down for `AWAY_AFTER`.
    Disconnected,
    /// The battery went from one percent to another.
    Battery {
        from: u8,
        to: u8,
    },
    ChargingStarted,
    ChargingStopped,
    /// This computer woke from sleep.
    Woke,
}

/// A device's link, as far as automations go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Link {
    /// Not seen up since the app started.
    #[default]
    Unknown,
    Up,
    /// Down since then, not long enough to count yet.
    Down(Instant),
    /// Down long enough: "disconnects" fired.
    Away,
}

/// What's known about one device, to tell when something changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub link: Link,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
}

impl Track {
    /// The link is `up` or not at `now`. "Connects" when it comes up for
    /// the first time or after counting as gone; "disconnects" once it's
    /// been down for `AWAY_AFTER` (a blip in between is neither).
    pub fn link(&mut self, up: bool, now: Instant) -> Option<Edge> {
        match (self.link, up) {
            (Link::Unknown | Link::Away, true) => {
                self.link = Link::Up;
                Some(Edge::Connected)
            }
            (Link::Down(_), true) => {
                self.link = Link::Up;
                None
            }
            (Link::Up, false) => {
                self.link = Link::Down(now);
                None
            }
            (Link::Down(since), false) if now.saturating_duration_since(since) >= AWAY_AFTER => {
                self.link = Link::Away;
                Some(Edge::Disconnected)
            }
            _ => None,
        }
    }

    /// A status with this battery and charging (either may be missing).
    /// Changes from the last known values; nothing on the first one.
    pub fn status(&mut self, battery: Option<u8>, charging: Option<bool>) -> Vec<Edge> {
        let mut out = Vec::new();
        if let (Some(from), Some(to)) = (self.battery, battery) {
            if from != to {
                out.push(Edge::Battery { from, to });
            }
        }
        match (self.charging, charging) {
            (Some(false), Some(true)) => out.push(Edge::ChargingStarted),
            (Some(true), Some(false)) => out.push(Edge::ChargingStopped),
            _ => {}
        }
        self.battery = battery.or(self.battery);
        self.charging = charging.or(self.charging);
        out
    }
}

/// Does `edge`, on `node`, start an automation with trigger `t`? Battery:
/// only when it goes from the level or above to below it.
pub fn matches(t: &Trigger, node: &str, edge: Edge) -> bool {
    if t.kind != "wakes" && t.node != node {
        return false;
    }
    match (t.kind.as_str(), edge) {
        ("connects", Edge::Connected)
        | ("disconnects", Edge::Disconnected)
        | ("charging_starts", Edge::ChargingStarted)
        | ("charging_stops", Edge::ChargingStopped)
        | ("wakes", Edge::Woke) => true,
        ("battery_below", Edge::Battery { from, to }) => {
            t.level.is_some_and(|n| from >= n && to < n)
        }
        _ => false,
    }
}

/// May an automation that last fired at `last` fire at `now`?
pub fn cooled(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= COOL_DOWN)
}

/// Two looks at the wall clock (seconds) a tick apart: did it sleep?
pub fn slept(prev: u64, now: u64) -> bool {
    now > prev + TICK.as_secs() + SLEPT_SECS
}

/// Check what the UI sent, and tidy it.
fn checked(mut a: Automation) -> Result<Automation, CoreError> {
    let bad = |why: &str| Err(CoreError::InvalidInput(why.to_string()));
    let t = &mut a.trigger;
    if !KINDS.contains(&t.kind.as_str()) {
        return bad("not something that can start an automation");
    }
    if t.kind == "wakes" {
        t.node.clear();
    } else if t.node.trim().is_empty() {
        return bad("choose a device to watch");
    }
    if t.kind == "battery_below" {
        if !t.level.is_some_and(|n| (1..=99).contains(&n)) {
            return bad("the battery level is 1 to 99%");
        }
    } else {
        t.level = None;
    }
    if a.action == NOTIFY {
        if a.node.trim().is_empty() {
            return bad("choose a device");
        }
    } else {
        crate::schedule::check_action(&a.node, &a.action, &a.args)?;
    }
    a.label = a.label.trim().chars().take(MAX_LABEL).collect();
    Ok(a)
}

struct Inner {
    list: Vec<Automation>,
    /// node id -> what's known about it.
    tracks: HashMap<String, Track>,
    /// automation id -> when it last fired (this run), for the cool-down.
    fired: HashMap<String, Instant>,
}

/// The automations, where they're saved, and what they're watching.
pub struct Automations {
    kv: Option<FileKv>,
    inner: Mutex<Inner>,
}

impl Automations {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("automations.json")));
        let list = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Automations {
            kv,
            inner: Mutex::new(Inner {
                list,
                tracks: HashMap::new(),
                fired: HashMap::new(),
            }),
        }
    }

    fn save(&self, list: &[Automation]) {
        let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(list)) else {
            return;
        };
        if let Err(e) = kv.set(KEY, &json) {
            eprintln!("[automations] couldn't save the automations: {e}");
        }
    }

    fn list(&self) -> Vec<Automation> {
        self.inner
            .lock()
            .map(|i| i.list.clone())
            .unwrap_or_default()
    }

    fn add(&self, new: NewAutomation) -> Result<Automation, CoreError> {
        let a = checked(Automation {
            id: uuid::Uuid::new_v4().to_string(),
            trigger: new.trigger,
            node: new.node,
            action: new.action,
            args: new.args,
            label: new.label,
            enabled: new.enabled,
            last_fired: None,
        })?;
        let mut i = self.inner.lock().map_err(|_| busy())?;
        if i.list.len() >= MAX_AUTOMATIONS {
            return Err(CoreError::InvalidInput(format!(
                "at most {MAX_AUTOMATIONS} automations"
            )));
        }
        i.list.push(a.clone());
        self.save(&i.list);
        Ok(a)
    }

    /// Change one; when it last fired is kept.
    fn update(&self, a: Automation) -> Result<Automation, CoreError> {
        let mut a = checked(a)?;
        let mut i = self.inner.lock().map_err(|_| busy())?;
        let old = i.list.iter_mut().find(|x| x.id == a.id).ok_or_else(gone)?;
        a.last_fired = old.last_fired.clone();
        *old = a.clone();
        self.save(&i.list);
        Ok(a)
    }

    fn delete(&self, id: &str) {
        let Ok(mut i) = self.inner.lock() else {
            return;
        };
        let before = i.list.len();
        i.list.retain(|a| a.id != id);
        i.fired.remove(id);
        if i.list.len() != before {
            self.save(&i.list);
        }
    }

    /// The links now (`live`, of `nodes`): what changed.
    fn links(&self, nodes: &[String], live: &[String], now: Instant) -> Vec<(String, Edge)> {
        let Ok(mut i) = self.inner.lock() else {
            return Vec::new();
        };
        nodes
            .iter()
            .filter_map(|node| {
                let up = live.contains(node);
                let edge = i.tracks.entry(node.clone()).or_default().link(up, now)?;
                Some((node.clone(), edge))
            })
            .collect()
    }

    fn status(&self, node: &str, battery: Option<u8>, charging: Option<bool>) -> Vec<Edge> {
        let Ok(mut i) = self.inner.lock() else {
            return Vec::new();
        };
        i.tracks
            .entry(node.to_string())
            .or_default()
            .status(battery, charging)
    }

    /// The automations `edge` on `node` starts, past their cool-down,
    /// marked as fired (and saved) before they run.
    fn take_fired(&self, node: &str, edge: Edge, now: Instant) -> Vec<Automation> {
        let Ok(mut i) = self.inner.lock() else {
            return Vec::new();
        };
        let Inner { list, fired, .. } = &mut *i;
        let when = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let mut out = Vec::new();
        for a in list.iter_mut() {
            if a.enabled
                && matches(&a.trigger, node, edge)
                && cooled(fired.get(&a.id).copied(), now)
            {
                fired.insert(a.id.clone(), now);
                a.last_fired = Some(when.clone());
                out.push(a.clone());
            }
        }
        if !out.is_empty() {
            self.save(list);
        }
        out
    }
}

fn busy() -> CoreError {
    CoreError::Other("automations unavailable".into())
}

fn gone() -> CoreError {
    CoreError::InvalidInput("that automation is gone".into())
}

/// A device's status arrived (`CoreEvent::DeviceStatus`): battery and
/// charging changes.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(state) = app.try_state::<Automations>() else {
        return;
    };
    let battery = status
        .get("battery")
        .and_then(|b| b.as_u64())
        .map(|b| b.min(100) as u8);
    let charging = status.get("charging").and_then(|c| c.as_bool());
    let edges = state.status(from, battery, charging);
    if edges.is_empty() {
        return;
    }
    let (app, from) = (app.clone(), from.to_string());
    std::thread::spawn(move || {
        for edge in edges {
            fire(&app, &from, edge);
        }
    });
}

/// A link came up or went down (`CoreEvent::PeerConnected` and the like):
/// look now rather than at the next tick. Off the core's thread.
pub fn on_link(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || check_links(&app));
}

/// Compare the live links with what's known, and fire what changed.
fn check_links(app: &AppHandle) {
    let (Some(state), Some(engine)) = (app.try_state::<Automations>(), app.try_state::<Eng>())
    else {
        return;
    };
    let live = engine.connected_peers();
    let nodes: Vec<String> = engine.get_peers().into_iter().map(|p| p.node_id).collect();
    for (node, edge) in state.links(&nodes, &live, Instant::now()) {
        fire(app, &node, edge);
    }
}

/// Look at the links every few seconds ("disconnects" waits a minute), and
/// notice the computer waking up.
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("automations".into())
        .spawn(move || {
            let mut prev = wall_secs();
            loop {
                std::thread::sleep(TICK);
                let now = wall_secs();
                if slept(prev, now) {
                    fire(&app, "", Edge::Woke);
                }
                prev = now;
                check_links(&app);
            }
        })
        .ok();
}

fn wall_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run the automations `edge` on `node` starts.
fn fire(app: &AppHandle, node: &str, edge: Edge) {
    let Some(state) = app.try_state::<Automations>() else {
        return;
    };
    for a in state.take_fired(node, edge, Instant::now()) {
        let result = if a.action == NOTIFY {
            Ok("here".into())
        } else {
            crate::schedule::run_action(app, &a.node, &a.action, &a.args)
        };
        tell(app, &a, &result);
    }
}

/// What happened, as a sentence start ("Pixel 8 connected").
fn why(app: &AppHandle, t: &Trigger) -> String {
    let device = name_of(app, &t.node);
    match t.kind.as_str() {
        "connects" => format!("{device} connected"),
        "disconnects" => format!("{device} disconnected"),
        "battery_below" => format!(
            "{device}\u{2019}s battery dropped below {}%",
            t.level.unwrap_or(0)
        ),
        "charging_starts" => format!("{device} started charging"),
        "charging_stops" => format!("{device} stopped charging"),
        _ => "This computer woke up".into(),
    }
}

/// A notification saying it fired, and how it went.
fn tell(app: &AppHandle, a: &Automation, result: &Result<String, CoreError>) {
    let why = why(app, &a.trigger);
    let what = if a.label.is_empty() {
        a.action.clone()
    } else {
        a.label.clone()
    };
    let device = name_of(app, &a.node);
    match result {
        Ok(_) if a.action == NOTIFY => crate::notify::post(
            app,
            &why,
            "You asked to be told in my-domain\u{2019}s Automations.",
        ),
        Ok(via) if via == "queued" => crate::notify::post(
            app,
            &format!("Automation: {what}"),
            &format!(
                "{why}. {device} isn\u{2019}t reachable right now; it\u{2019}ll happen once it is."
            ),
        ),
        Ok(_) => crate::notify::post(
            app,
            &format!("Automation: {what}"),
            &format!("{why}, so it\u{2019}s done on {device}."),
        ),
        Err(e) => crate::notify::post(
            app,
            &format!("Couldn\u{2019}t run \u{201c}{what}\u{201d}"),
            &format!("{why}, but on {device}: {e}"),
        ),
    }
}

/// A device's name for a notification; "this computer" for this one.
fn name_of(app: &AppHandle, node: &str) -> String {
    let me = app
        .try_state::<Eng>()
        .map(|e| e.identity().node_id)
        .unwrap_or_default();
    if node.is_empty() || node == me {
        "this computer".into()
    } else {
        crate::notify::device_name(app, node)
    }
}

/// The automation `which` names: its id, else its label (any case). One
/// that's switched off doesn't count.
pub fn pick<'a>(list: &'a [Automation], which: &str) -> Result<&'a Automation, String> {
    let which = which.trim();
    let found: Vec<&Automation> = match list.iter().find(|a| a.id == which) {
        Some(a) => vec![a],
        None => list
            .iter()
            .filter(|a| !a.label.is_empty() && a.label.eq_ignore_ascii_case(which))
            .collect(),
    };
    match found.as_slice() {
        [] => Err(format!(
            "there\u{2019}s no automation called \u{201c}{which}\u{201d}"
        )),
        [a] if !a.enabled => Err(format!("\u{201c}{which}\u{201d} is switched off")),
        [a] => Ok(a),
        _ => Err(format!(
            "more than one automation is called \u{201c}{which}\u{201d}; use its id"
        )),
    }
}

/// Do what an automation does now, without waiting for what starts it (a
/// webhook's `/v1/automation`). Returns what it did ("Lock this computer")
/// and how it went (`schedule::run_action`'s transport).
pub(crate) fn run_now(app: &AppHandle, which: &str) -> Result<(String, String), String> {
    let state = app.try_state::<Automations>().ok_or("not started yet")?;
    let a = pick(&state.list(), which)?.clone();
    let what = if a.label.is_empty() {
        a.action.clone()
    } else {
        a.label.clone()
    };
    if a.action == NOTIFY {
        crate::notify::post(app, &what, "A webhook ran this automation.");
        return Ok((what, "here".into()));
    }
    let via =
        crate::schedule::run_action(app, &a.node, &a.action, &a.args).map_err(|e| e.to_string())?;
    Ok((what, via))
}

/// Your automations.
#[tauri::command]
#[specta::specta]
pub fn list_automations(state: State<Automations>) -> Vec<Automation> {
    state.list()
}

/// Add an automation: when something happens, do something.
#[tauri::command]
#[specta::specta]
pub fn add_automation(
    automation: NewAutomation,
    app: AppHandle,
    state: State<Automations>,
) -> Result<Automation, CoreError> {
    crate::schedule::check_target(&app, &automation.node, &automation.action)?;
    state.add(automation)
}

/// Change an automation (what starts it, what it does, on/off…).
#[tauri::command]
#[specta::specta]
pub fn update_automation(
    automation: Automation,
    state: State<Automations>,
) -> Result<Automation, CoreError> {
    state.update(automation)
}

/// Remove an automation.
#[tauri::command]
#[specta::specta]
pub fn delete_automation(id: String, state: State<Automations>) {
    state.delete(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Duration = Duration::from_secs(1);

    fn trigger(kind: &str, level: Option<u8>) -> Trigger {
        Trigger {
            kind: kind.into(),
            node: "phone".into(),
            level,
        }
    }

    fn rule(kind: &str) -> Automation {
        Automation {
            id: "a".into(),
            trigger: trigger(kind, None),
            node: "me".into(),
            action: "lock".into(),
            args: "{}".into(),
            label: "Lock this computer".into(),
            enabled: true,
            last_fired: None,
        }
    }

    #[test]
    fn connects_when_it_first_comes_up() {
        let t0 = Instant::now();
        let mut t = Track::default();
        // Never up: nothing, however long.
        assert_eq!(t.link(false, t0), None);
        assert_eq!(t.link(false, t0 + SEC * 600), None);
        assert_eq!(t.link(true, t0 + SEC * 601), Some(Edge::Connected));
        assert_eq!(t.link(true, t0 + SEC * 615), None, "only once");
    }

    #[test]
    fn disconnects_after_a_minute_down() {
        let t0 = Instant::now();
        let mut t = Track::default();
        t.link(true, t0);
        assert_eq!(t.link(false, t0 + SEC * 10), None);
        assert_eq!(t.link(false, t0 + SEC * 69), None, "59 seconds down");
        assert_eq!(t.link(false, t0 + SEC * 70), Some(Edge::Disconnected));
        assert_eq!(t.link(false, t0 + SEC * 200), None, "only once");
        // Back after that: connects again.
        assert_eq!(t.link(true, t0 + SEC * 300), Some(Edge::Connected));
    }

    #[test]
    fn a_blip_is_neither() {
        let t0 = Instant::now();
        let mut t = Track::default();
        t.link(true, t0);
        assert_eq!(t.link(false, t0 + SEC), None);
        assert_eq!(t.link(true, t0 + SEC * 30), None);
        // The minute starts over at the next drop.
        assert_eq!(t.link(false, t0 + SEC * 40), None);
        assert_eq!(t.link(false, t0 + SEC * 90), None);
        assert_eq!(t.link(false, t0 + SEC * 100), Some(Edge::Disconnected));
    }

    #[test]
    fn battery_fires_on_the_crossing_only() {
        let below_20 = trigger("battery_below", Some(20));
        let mut t = Track::default();
        // The first status only sets where things stand.
        assert!(t.status(Some(15), Some(false)).is_empty());
        let mut t = Track::default();
        t.status(Some(22), None);
        let edges = t.status(Some(20), None);
        assert!(!edges.iter().any(|e| matches(&below_20, "phone", *e)));
        let edges = t.status(Some(19), None);
        assert!(edges.iter().any(|e| matches(&below_20, "phone", *e)));
        // Staying below: not again.
        let edges = t.status(Some(18), None);
        assert!(!edges.iter().any(|e| matches(&below_20, "phone", *e)));
        // A status without a battery doesn't forget it.
        assert!(t.status(None, None).is_empty());
        assert_eq!(t.battery, Some(18));
        // A big drop past the level counts too.
        let mut t = Track::default();
        t.status(Some(60), None);
        let edges = t.status(Some(10), None);
        assert!(edges.iter().any(|e| matches(&below_20, "phone", *e)));
    }

    #[test]
    fn charging_starts_and_stops() {
        let mut t = Track::default();
        assert!(t.status(None, Some(false)).is_empty());
        assert_eq!(t.status(None, Some(false)), vec![]);
        assert_eq!(t.status(Some(50), Some(true)), vec![Edge::ChargingStarted]);
        assert_eq!(t.status(Some(50), Some(true)), vec![]);
        assert_eq!(
            t.status(Some(51), Some(false)),
            vec![Edge::Battery { from: 50, to: 51 }, Edge::ChargingStopped]
        );
    }

    #[test]
    fn matches_the_device_and_kind() {
        let t = trigger("connects", None);
        assert!(matches(&t, "phone", Edge::Connected));
        assert!(!matches(&t, "tablet", Edge::Connected));
        assert!(!matches(&t, "phone", Edge::Disconnected));
        assert!(matches(
            &trigger("disconnects", None),
            "phone",
            Edge::Disconnected
        ));
        assert!(matches(
            &trigger("charging_starts", None),
            "phone",
            Edge::ChargingStarted
        ));
        assert!(matches(
            &trigger("charging_stops", None),
            "phone",
            Edge::ChargingStopped
        ));
        // Waking is this computer's, whatever the node.
        let wakes = Trigger {
            kind: "wakes".into(),
            node: String::new(),
            level: None,
        };
        assert!(matches(&wakes, "", Edge::Woke));
        assert!(!matches(&wakes, "", Edge::Connected));
        // A battery trigger without a level never fires.
        let no_level = trigger("battery_below", None);
        assert!(!matches(
            &no_level,
            "phone",
            Edge::Battery { from: 50, to: 1 }
        ));
    }

    #[test]
    fn cool_down_of_a_minute() {
        let t0 = Instant::now();
        assert!(cooled(None, t0));
        assert!(!cooled(Some(t0), t0 + SEC * 59));
        assert!(cooled(Some(t0), t0 + SEC * 60));
    }

    #[test]
    fn notices_sleep() {
        assert!(!slept(1_000, 1_015));
        assert!(!slept(1_000, 1_030), "a slow tick isn't sleep");
        assert!(slept(1_000, 1_000 + 15 + 61));
        assert!(slept(1_000, 1_000 + 3_600));
        // The clock set back isn't waking.
        assert!(!slept(1_000, 500));
    }

    #[test]
    fn checks_what_the_ui_sends() {
        assert!(checked(rule("connects")).is_ok());
        assert!(checked(rule("explodes")).is_err());
        let mut a = rule("connects");
        a.trigger.node = " ".into();
        assert!(checked(a).is_err(), "no device to watch");
        // Waking needs no device; a stray one is dropped.
        let a = checked(rule("wakes")).unwrap();
        assert_eq!(a.trigger.node, "");
        // Battery: a level, 1–99; others drop one.
        let mut a = rule("battery_below");
        assert!(checked(a.clone()).is_err());
        a.trigger.level = Some(100);
        assert!(checked(a.clone()).is_err());
        a.trigger.level = Some(20);
        assert!(checked(a).is_ok());
        let mut a = rule("connects");
        a.trigger.level = Some(20);
        assert_eq!(checked(a).unwrap().trigger.level, None);
        // Just a notification: no action to check.
        let mut a = rule("connects");
        a.action = NOTIFY.into();
        a.args = String::new();
        assert!(checked(a.clone()).is_ok());
        a.action = "Bad Name".into();
        assert!(checked(a).is_err());
    }

    #[test]
    fn a_webhook_picks_one_by_id_or_label() {
        let mut a = rule("connects");
        let mut b = rule("wakes");
        b.id = "b".into();
        b.label = "Ring my phone".into();
        let mut off = rule("wakes");
        off.id = "c".into();
        off.label = "Off one".into();
        off.enabled = false;
        let list = vec![a.clone(), b.clone(), off];
        assert_eq!(pick(&list, "b").unwrap().id, "b");
        assert_eq!(pick(&list, " ring MY phone ").unwrap().id, "b");
        assert!(pick(&list, "Off one").unwrap_err().contains("switched off"));
        assert!(pick(&list, "nothing")
            .unwrap_err()
            .contains("no automation"));
        // Two with one label: the id decides.
        a.label = b.label.clone();
        let list = vec![a, b];
        assert!(pick(&list, "ring my phone")
            .unwrap_err()
            .contains("more than one"));
        assert_eq!(pick(&list, "a").unwrap().id, "a");
    }
}
