// Scheduled actions: run an action on one of your devices at a set time, e.g.
// "every weekday at 08:00, set my phone's ringer to normal" or "once at 18:30,
// ring my phone". Set up on a device's page (Schedules tile), or for this
// computer in Settings ("lock this computer at 23:00").
//
//  - kept in `<app config dir>/schedules.json` (key `schedules`, a JSON list);
//  - the time is this computer's local time ("HH:MM"), on the chosen days
//    (1 = Monday … 7 = Sunday); no days means once, and a one-off turns
//    itself off after it runs;
//  - a check runs just after each minute starts and covers the stretch of
//    local time since the last one, so each time runs once, and a check that
//    comes a little late doesn't miss it. When the clock jumps forward an
//    hour (DST), a time inside the skipped hour runs right after the jump;
//    when it goes back, a time in the repeated hour doesn't run twice;
//  - only while my-domain is running: a time that passes while the computer
//    is asleep or off (or the app closed) is skipped, not run later;
//  - it goes the way a button press on the device's page goes
//    (`run_action`, or `ring`): to a device that isn't reachable it's kept
//    and sent once it is, which the notification says;
//  - a notification says it ran (or why it couldn't).
//
// A schedule for this computer itself (its own node id) runs here directly
// (`device::on_action`): the core only sends actions to other devices.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

const KEY: &str = "schedules";
/// The most schedules kept.
const MAX_SCHEDULES: usize = 100;
/// The longest label.
const MAX_LABEL: usize = 80;
/// A gap between two checks longer than this means the computer slept (or
/// the clock was set): only the last minute counts then.
const MAX_GAP_SECS: i64 = 3 * 60;
/// The most local time one check covers: an hour for a DST jump, and a bit.
const MAX_SPAN_MINUTES: i64 = 65;

/// One scheduled action.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, PartialEq)]
pub struct Schedule {
    pub id: String,
    /// The device it runs on.
    pub node: String,
    /// An action the device lists in its status (`actions`), or "ring".
    pub action: String,
    /// The action's arguments, as JSON text ("" for none).
    #[serde(default)]
    pub args: String,
    /// What it does, for people ("Set the ringer to normal").
    #[serde(default)]
    pub label: String,
    /// Local time, "HH:MM".
    pub time: String,
    /// 1 = Monday … 7 = Sunday; empty = once.
    #[serde(default)]
    pub days: Vec<u8>,
    pub enabled: bool,
    /// The last time it ran, as local "YYYY-MM-DDTHH:MM" (its slot).
    #[serde(default)]
    pub last_run: Option<String>,
}

/// A new schedule, from the UI.
#[derive(serde::Deserialize, specta::Type, Clone, Debug)]
pub struct NewSchedule {
    pub node: String,
    pub action: String,
    pub args: String,
    pub label: String,
    pub time: String,
    pub days: Vec<u8>,
    pub enabled: bool,
}

/// "HH:MM" into a time of day.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// The stretch of local time a check covers, `(from, to]`: from the last
/// check (`prev`) to now. After a long gap (asleep, or the clock was set)
/// only the last minute, so missed times are skipped. When the local clock
/// went back (DST), `from` is after `to` and nothing is due.
pub fn window(
    prev_local: NaiveDateTime,
    prev_utc: i64,
    now_local: NaiveDateTime,
    now_utc: i64,
) -> (NaiveDateTime, NaiveDateTime) {
    let gap = now_utc - prev_utc;
    let from = if !(0..=MAX_GAP_SECS).contains(&gap) {
        now_local - chrono::Duration::minutes(1)
    } else {
        prev_local.max(now_local - chrono::Duration::minutes(MAX_SPAN_MINUTES))
    };
    (from, now_local)
}

/// A slot's key, for `last_run`.
fn slot_key(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M").to_string()
}

/// Is `s` due in `(from, to]`? Its slot key if so: on, a valid time inside
/// the stretch, on one of its days (any day for a one-off), and not already
/// run for that slot.
pub fn due(s: &Schedule, from: NaiveDateTime, to: NaiveDateTime) -> Option<String> {
    if !s.enabled {
        return None;
    }
    let time = parse_time(&s.time)?;
    // The stretch is at most a little over an hour, so today and yesterday
    // (past midnight) are all the dates it can touch.
    let today = to.date();
    let dates: [NaiveDate; 2] = [today, today.pred_opt()?];
    dates.into_iter().find_map(|date| {
        let at = date.and_time(time);
        let day = date.weekday().number_from_monday() as u8;
        let on_day = s.days.is_empty() || s.days.contains(&day);
        let key = slot_key(at);
        (from < at && at <= to && on_day && s.last_run.as_deref() != Some(key.as_str()))
            .then_some(key)
    })
}

/// It ran for `slot`: remember that, and turn a one-off off.
pub fn ran(s: &mut Schedule, slot: String) {
    s.last_run = Some(slot);
    if s.days.is_empty() {
        s.enabled = false;
    }
}

/// Days as a clean set: 1–7, sorted, no repeats.
fn clean_days(days: &[u8]) -> Result<Vec<u8>, CoreError> {
    if days.iter().any(|d| !(1..=7).contains(d)) {
        return Err(CoreError::InvalidInput(
            "days are 1 (Monday) to 7 (Sunday)".into(),
        ));
    }
    let mut days = days.to_vec();
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

/// A device, an action it can be sent ("ring", or a listed action's name)
/// and arguments that are JSON (or none). Automations check theirs the same.
pub(crate) fn check_action(node: &str, action: &str, args: &str) -> Result<(), CoreError> {
    let bad = |why: &str| Err(CoreError::InvalidInput(why.to_string()));
    if node.trim().is_empty() {
        return bad("choose a device");
    }
    if action != "ring" && !mdcore::features::device::is_action_name(action) {
        return bad("not an action");
    }
    if !args.trim().is_empty() && serde_json::from_str::<serde_json::Value>(args).is_err() {
        return bad("bad action arguments");
    }
    Ok(())
}

/// Check what the UI sent, and tidy it.
fn checked(mut s: Schedule) -> Result<Schedule, CoreError> {
    let bad = |why: &str| Err(CoreError::InvalidInput(why.to_string()));
    check_action(&s.node, &s.action, &s.args)?;
    let Some(time) = parse_time(&s.time) else {
        return bad("the time is HH:MM");
    };
    s.time = time.format("%H:%M").to_string();
    s.days = clean_days(&s.days)?;
    s.label = s.label.trim().chars().take(MAX_LABEL).collect();
    Ok(s)
}

/// The schedules, and where they're saved.
pub struct Schedules {
    kv: Option<FileKv>,
    list: Mutex<Vec<Schedule>>,
}

impl Schedules {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("schedules.json")));
        let list = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Schedules {
            kv,
            list: Mutex::new(list),
        }
    }

    fn save(&self, list: &[Schedule]) {
        let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(list)) else {
            return;
        };
        if let Err(e) = kv.set(KEY, &json) {
            eprintln!("[schedule] couldn't save the schedules: {e}");
        }
    }

    fn list(&self) -> Vec<Schedule> {
        self.list.lock().map(|l| l.clone()).unwrap_or_default()
    }

    fn add(&self, new: NewSchedule) -> Result<Schedule, CoreError> {
        let s = checked(Schedule {
            id: uuid::Uuid::new_v4().to_string(),
            node: new.node,
            action: new.action,
            args: new.args,
            label: new.label,
            time: new.time,
            days: new.days,
            enabled: new.enabled,
            last_run: None,
        })?;
        let mut list = self.list.lock().map_err(|_| busy())?;
        if list.len() >= MAX_SCHEDULES {
            return Err(CoreError::InvalidInput(format!(
                "at most {MAX_SCHEDULES} schedules"
            )));
        }
        list.push(s.clone());
        self.save(&list);
        Ok(s)
    }

    /// Change one; when it last ran is kept.
    fn update(&self, s: Schedule) -> Result<Schedule, CoreError> {
        let mut s = checked(s)?;
        let mut list = self.list.lock().map_err(|_| busy())?;
        let old = list.iter_mut().find(|x| x.id == s.id).ok_or_else(gone)?;
        s.last_run = old.last_run.clone();
        *old = s.clone();
        self.save(&list);
        Ok(s)
    }

    fn delete(&self, id: &str) {
        let Ok(mut list) = self.list.lock() else {
            return;
        };
        let before = list.len();
        list.retain(|s| s.id != id);
        if list.len() != before {
            self.save(&list);
        }
    }

    fn get(&self, id: &str) -> Option<Schedule> {
        self.list.lock().ok()?.iter().find(|s| s.id == id).cloned()
    }

    /// The ones due in `(from, to]`, marked as run (and saved) before they
    /// go, so a slow send can't make one run twice.
    fn take_due(&self, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Schedule> {
        let Ok(mut list) = self.list.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for s in list.iter_mut() {
            if let Some(slot) = due(s, from, to) {
                ran(s, slot);
                out.push(s.clone());
            }
        }
        if !out.is_empty() {
            self.save(&list);
        }
        out
    }
}

fn busy() -> CoreError {
    CoreError::Other("schedules unavailable".into())
}

fn gone() -> CoreError {
    CoreError::InvalidInput("that schedule is gone".into())
}

/// Check just after each minute starts, and run what's due.
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("schedules".into())
        .spawn(move || {
            // Times before the app started don't count.
            let start = Local::now();
            let mut prev = (start.naive_local(), start.timestamp());
            loop {
                let now = Local::now();
                // A second into the next minute.
                let wait = 61 - u64::from(now.second().min(59));
                std::thread::sleep(Duration::from_secs(wait));
                let now = Local::now();
                let (local, utc) = (now.naive_local(), now.timestamp());
                let (from, to) = window(prev.0, prev.1, local, utc);
                prev = (local, utc);
                let Some(state) = app.try_state::<Schedules>() else {
                    continue;
                };
                for s in state.take_due(from, to) {
                    let result = run(&app, &s);
                    tell(&app, &s, &result);
                }
            }
        })
        .ok();
}

/// Where a schedule goes.
#[derive(Debug, PartialEq)]
enum Route {
    /// This computer (`me`): run it here.
    Here,
    /// Another device: ring it.
    Ring,
    /// Another computer: wake it from sleep (Wake-on-LAN, from here).
    Wake,
    /// Another device: send it the action.
    Send,
}

fn route(node: &str, action: &str, me: &str) -> Route {
    if action == WAKE {
        Route::Wake
    } else if node == me {
        Route::Here
    } else if action == "ring" {
        Route::Ring
    } else {
        Route::Send
    }
}

/// Run one now, the way the device page's buttons do. Returns the
/// transport, "queued" (sent once the device is reachable) or "here" (this
/// computer).
fn run(app: &AppHandle, s: &Schedule) -> Result<String, CoreError> {
    run_action(app, &s.node, &s.action, &s.args)
}

/// Run `action` on `node` now: here for this computer, else sent the way
/// the device page's buttons send it. Shared with automations.
pub(crate) fn run_action(
    app: &AppHandle,
    node: &str,
    action: &str,
    args: &str,
) -> Result<String, CoreError> {
    let engine = app
        .try_state::<Eng>()
        .ok_or_else(|| CoreError::Other("not started yet".into()))?;
    match route(node, action, &engine.identity().node_id) {
        Route::Here => {
            // `on_action` skips what it doesn't know: say so instead.
            if !crate::device::local_actions(app)
                .iter()
                .any(|a| a == action)
            {
                return Err(CoreError::InvalidInput(
                    "this computer can\u{2019}t do that right now".into(),
                ));
            }
            let args = serde_json::from_str(args).unwrap_or(serde_json::Value::Null);
            // Off this thread, as a sibling's action is: "Run now" comes
            // from the UI, and locking can take a moment.
            let (app, action) = (app.clone(), action.to_string());
            let me = node.to_string();
            std::thread::spawn(move || crate::device::on_action(&app, &me, &action, &args));
            Ok("here".into())
        }
        Route::Ring => engine.ring(node),
        Route::Wake => {
            check_wake(node, &engine.identity().node_id, engine.can_wake(node))?;
            // A few UDP sends and queued messages: never blocks.
            engine.wake_device(node).map(|_| "woken".into())
        }
        Route::Send => engine.run_action(node, action, args),
    }
}

/// The "wake" action: run here (it's this computer that sends the magic
/// packet), never sent to the computer itself, which is asleep.
const WAKE: &str = "wake";

/// A new schedule's or automation's target, when it wakes one: checked as
/// it's added, so a "wake" never goes to this computer or a phone.
pub(crate) fn check_target(app: &AppHandle, node: &str, action: &str) -> Result<(), CoreError> {
    if action != WAKE {
        return Ok(());
    }
    let engine = app
        .try_state::<Eng>()
        .ok_or_else(|| CoreError::Other("not started yet".into()))?;
    check_wake(node, &engine.identity().node_id, engine.can_wake(node))
}

/// Whether `node` can be woken by a "wake" action: another computer (not
/// this one, `me`), that said it may be woken (`can_wake`).
pub(crate) fn check_wake(node: &str, me: &str, can_wake: bool) -> Result<(), CoreError> {
    let bad = |why: &str| Err(CoreError::InvalidInput(why.to_string()));
    if node.trim().is_empty() {
        bad("choose a computer to wake")
    } else if node == me {
        bad("this computer can\u{2019}t wake itself")
    } else if !can_wake {
        bad("that device hasn\u{2019}t said it can be woken: turn on \u{201c}Let my other devices wake this computer\u{201d} on it")
    } else {
        Ok(())
    }
}

/// A notification saying how it went.
fn tell(app: &AppHandle, s: &Schedule, result: &Result<String, CoreError>) {
    let device = device_name(app, &s.node);
    let what = if s.label.is_empty() {
        s.action.clone()
    } else {
        s.label.clone()
    };
    match result {
        Ok(via) if via == "queued" => crate::notify::post(
            app,
            &format!("Scheduled: {what}"),
            &format!("{device} isn\u{2019}t reachable right now. It\u{2019}ll happen once it is."),
        ),
        Ok(via) if via == "woken" => crate::notify::post(
            app,
            &format!("Scheduled: {what}"),
            &format!("Sent {device} a wake-up at {}.", s.time),
        ),
        Ok(_) => crate::notify::post(
            app,
            &format!("Scheduled: {what}"),
            &format!("Done on {device} at {}.", s.time),
        ),
        Err(e) => crate::notify::post(
            app,
            &format!("Couldn\u{2019}t run \u{201c}{what}\u{201d}"),
            &format!("On {device} at {}: {e}", s.time),
        ),
    }
}

fn device_name(app: &AppHandle, node: &str) -> String {
    let Some(engine) = app.try_state::<Eng>() else {
        return "your device".into();
    };
    if engine.identity().node_id == node {
        return "this computer".into();
    }
    engine
        .get_peers()
        .into_iter()
        .find(|p| p.node_id == node)
        .map(|p| crate::device_labels::display_name(app, node, &p.name))
        .unwrap_or_else(|| "your device".into())
}

/// Your scheduled actions.
#[tauri::command]
#[specta::specta]
pub fn list_schedules(state: State<Schedules>) -> Vec<Schedule> {
    state.list()
}

/// Schedule an action on a device.
#[tauri::command]
#[specta::specta]
pub fn add_schedule(
    schedule: NewSchedule,
    app: AppHandle,
    state: State<Schedules>,
) -> Result<Schedule, CoreError> {
    check_target(&app, &schedule.node, &schedule.action)?;
    state.add(schedule)
}

/// Change a schedule (its time, days, on/off…).
#[tauri::command]
#[specta::specta]
pub fn update_schedule(schedule: Schedule, state: State<Schedules>) -> Result<Schedule, CoreError> {
    state.update(schedule)
}

/// Remove a schedule.
#[tauri::command]
#[specta::specta]
pub fn delete_schedule(id: String, state: State<Schedules>) {
    state.delete(&id)
}

/// Run a schedule's action now (it still runs at its time too). Returns
/// the transport, "queued" or "here".
#[tauri::command]
#[specta::specta]
pub fn run_schedule_now(
    id: String,
    app: AppHandle,
    state: State<Schedules>,
) -> Result<String, CoreError> {
    let s = state.get(&id).ok_or_else(gone)?;
    run(&app, &s)
}

/// This computer's node id and the actions it can run on itself, for its
/// own schedules (Settings).
#[derive(serde::Serialize, specta::Type)]
pub struct LocalActions {
    pub node: String,
    pub actions: Vec<String>,
}

/// This computer's own id and actions. Async, off the main thread:
/// reading the volume runs a command.
#[tauri::command]
#[specta::specta]
pub async fn local_actions(app: AppHandle) -> Result<LocalActions, CoreError> {
    let engine = app
        .try_state::<Eng>()
        .ok_or_else(|| CoreError::Other("not started yet".into()))?;
    Ok(LocalActions {
        node: engine.identity().node_id,
        actions: crate::device::local_actions(&app),
    })
}

/// Run one of this computer's own actions now (the command palette).
/// Returns "here". Async, off the main thread: checking what this computer
/// can do may run a command.
#[tauri::command]
#[specta::specta]
pub async fn run_local_action(
    name: String,
    args_json: String,
    app: AppHandle,
) -> Result<String, CoreError> {
    let engine = app
        .try_state::<Eng>()
        .ok_or_else(|| CoreError::Other("not started yet".into()))?;
    let me = engine.identity().node_id;
    run_action(&app, &me, &name, &args_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-02 is a Monday.
    fn at(day: u32, h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, day)
            .unwrap()
            .and_hms_opt(h, m, s)
            .unwrap()
    }

    fn sched(time: &str, days: &[u8]) -> Schedule {
        Schedule {
            id: "a".into(),
            node: "phone".into(),
            action: "ringer".into(),
            args: r#"{"mode":"normal"}"#.into(),
            label: "Ringer to normal".into(),
            time: time.into(),
            days: days.to_vec(),
            enabled: true,
            last_run: None,
        }
    }

    const WEEKDAYS: [u8; 5] = [1, 2, 3, 4, 5];

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("08:00"), NaiveTime::from_hms_opt(8, 0, 0));
        assert_eq!(parse_time("23:59"), NaiveTime::from_hms_opt(23, 59, 0));
        for bad in ["8:00", "24:00", "12:60", "", "ab:cd", "12:3", "12-30"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn due_inside_the_stretch_only() {
        let s = sched("08:00", &[]);
        assert!(due(&s, at(2, 7, 59, 1), at(2, 8, 0, 1)).is_some());
        // Exactly at the end counts; exactly at the start doesn't (the
        // check before covered it).
        assert!(due(&s, at(2, 7, 59, 0), at(2, 8, 0, 0)).is_some());
        assert!(due(&s, at(2, 8, 0, 0), at(2, 8, 1, 0)).is_none());
        assert!(due(&s, at(2, 8, 0, 1), at(2, 8, 1, 1)).is_none());
        assert!(due(&s, at(2, 7, 58, 1), at(2, 7, 59, 1)).is_none());
    }

    #[test]
    fn a_late_check_still_catches_it() {
        let s = sched("08:00", &[]);
        // The check at 08:00 came at 07:59:59, the next at 08:01:02.
        assert!(due(&s, at(2, 7, 58, 59), at(2, 7, 59, 59)).is_none());
        assert!(due(&s, at(2, 7, 59, 59), at(2, 8, 1, 2)).is_some());
    }

    #[test]
    fn weekdays_only() {
        let s = sched("08:00", &WEEKDAYS);
        // Monday 2 … Friday 6 yes, Saturday 7 and Sunday 8 no.
        for day in 2..=6 {
            assert!(
                due(&s, at(day, 7, 59, 30), at(day, 8, 0, 30)).is_some(),
                "{day}"
            );
        }
        for day in 7..=8 {
            assert!(
                due(&s, at(day, 7, 59, 30), at(day, 8, 0, 30)).is_none(),
                "{day}"
            );
        }
        let sunday = sched("08:00", &[7]);
        assert!(due(&sunday, at(8, 7, 59, 30), at(8, 8, 0, 30)).is_some());
    }

    #[test]
    fn not_twice_for_one_slot() {
        let mut s = sched("08:00", &WEEKDAYS);
        let slot = due(&s, at(2, 7, 59, 30), at(2, 8, 0, 30)).unwrap();
        ran(&mut s, slot);
        assert!(s.enabled, "a repeating one stays on");
        // The same stretch again (e.g. the clock went back): not again.
        assert!(due(&s, at(2, 7, 59, 30), at(2, 8, 0, 30)).is_none());
        // The next day: yes.
        assert!(due(&s, at(3, 7, 59, 30), at(3, 8, 0, 30)).is_some());
    }

    #[test]
    fn a_one_off_turns_itself_off() {
        let mut s = sched("18:30", &[]);
        let slot = due(&s, at(4, 18, 29, 30), at(4, 18, 30, 30)).unwrap();
        assert_eq!(slot, "2026-11-04T18:30");
        ran(&mut s, slot);
        assert!(!s.enabled);
        assert_eq!(s.last_run.as_deref(), Some("2026-11-04T18:30"));
        assert!(due(&s, at(5, 18, 29, 30), at(5, 18, 30, 30)).is_none());
    }

    #[test]
    fn off_never_runs() {
        let mut s = sched("08:00", &WEEKDAYS);
        s.enabled = false;
        assert!(due(&s, at(2, 7, 59, 30), at(2, 8, 0, 30)).is_none());
    }

    #[test]
    fn across_midnight() {
        // 23:59 on Monday, checked just after midnight (Tuesday).
        let s = sched("23:59", &[1]);
        let slot = due(&s, at(2, 23, 58, 59), at(3, 0, 0, 1));
        assert_eq!(slot.as_deref(), Some("2026-11-02T23:59"));
        // Midnight itself belongs to the new day.
        let s = sched("00:00", &[2]);
        assert!(due(&s, at(2, 23, 59, 30), at(3, 0, 0, 30)).is_some());
        let s = sched("00:00", &[1]);
        assert!(due(&s, at(2, 23, 59, 30), at(3, 0, 0, 30)).is_none());
    }

    #[test]
    fn the_window_follows_the_checks() {
        let (from, to) = window(at(2, 8, 0, 1), 1_000, at(2, 8, 1, 1), 1_060);
        assert_eq!((from, to), (at(2, 8, 0, 1), at(2, 8, 1, 1)));
    }

    #[test]
    fn missed_while_asleep_is_skipped() {
        // Asleep from 07:00 to 09:00:30: 08:00 doesn't run on waking.
        let s = sched("08:00", &[]);
        let (from, to) = window(at(2, 7, 0, 1), 0, at(2, 9, 0, 30), 2 * 3600 + 29);
        assert!(due(&s, from, to).is_none());
        // But the minute it woke in still counts.
        let s = sched("09:00", &[]);
        assert!(due(&s, from, to).is_some());
    }

    #[test]
    fn dst_forward_runs_the_skipped_hour_once() {
        // 01:59:30 → 03:00:30 local, a minute of real time: 02:30 runs.
        let s = sched("02:30", &[]);
        let (from, to) = window(at(2, 1, 59, 30), 0, at(2, 3, 0, 30), 60);
        assert!(due(&s, from, to).is_some());
    }

    #[test]
    fn dst_back_does_not_repeat() {
        let mut s = sched("01:30", &WEEKDAYS);
        let slot = due(&s, at(2, 1, 29, 30), at(2, 1, 30, 30)).unwrap();
        ran(&mut s, slot);
        // 01:59:30 → 01:00:30 local, a minute of real time: nothing.
        let (from, to) = window(at(2, 1, 59, 30), 0, at(2, 1, 0, 30), 60);
        assert!(from > to);
        assert!(due(&s, from, to).is_none());
        // Then the repeated 01:30 comes round again: still not twice.
        let (from, to) = window(at(2, 1, 29, 30), 1_740, at(2, 1, 30, 30), 1_800);
        assert!(due(&s, from, to).is_none());
    }

    #[test]
    fn a_clock_set_far_ahead_only_counts_a_minute() {
        // The zone changed (travel): six hours of local time in a minute.
        let s = sched("10:00", &[]);
        let (from, to) = window(at(2, 8, 0, 0), 0, at(2, 14, 0, 0), 60);
        assert!(due(&s, from, to).is_none());
    }

    #[test]
    fn this_computers_own_run_here() {
        // Locking this computer runs here (`device::on_action`), not through
        // the core, which only sends to other devices.
        assert_eq!(route("me", "lock", "me"), Route::Here);
        assert_eq!(route("me", "ring", "me"), Route::Here);
        assert_eq!(route("phone", "ring", "me"), Route::Ring);
        assert_eq!(route("phone", "lock", "me"), Route::Send);
        // Waking goes from here, never to the sleeping computer itself.
        assert_eq!(route("office", "wake", "me"), Route::Wake);
        assert_eq!(route("me", "wake", "me"), Route::Wake);
    }

    #[test]
    fn wake_checks_its_target() {
        assert!(check_wake("office", "me", true).is_ok());
        // Not this computer, not one that never said it can be woken
        // (a phone, or the switch off), and not nothing.
        assert!(check_wake("me", "me", true).is_err());
        assert!(check_wake("phone", "me", false).is_err());
        assert!(check_wake("", "me", true).is_err());
        assert!(check_wake("  ", "me", true).is_err());
        // A schedule with it is otherwise checked like any other.
        let mut s = sched("08:55", &WEEKDAYS);
        s.node = "office".into();
        s.action = "wake".into();
        s.args = "{}".into();
        assert!(checked(s).is_ok());
    }

    #[test]
    fn checks_what_the_ui_sends() {
        let mut s = sched("8:00", &[]);
        assert!(checked(s.clone()).is_err());
        s.time = "08:05".into();
        s.days = vec![5, 1, 1, 3];
        assert_eq!(checked(s.clone()).unwrap().days, vec![1, 3, 5]);
        s.days = vec![0];
        assert!(checked(s.clone()).is_err());
        s.days = vec![];
        s.args = "{not json".into();
        assert!(checked(s.clone()).is_err());
        s.args = String::new();
        s.action = "ring".into();
        assert!(checked(s.clone()).is_ok());
        s.action = "Bad Name".into();
        assert!(checked(s).is_err());
    }
}
