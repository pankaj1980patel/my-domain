// Lock this computer when your phone goes away: with "Lock this computer when
// my phone goes away" on in Settings, once none of your phones has had a live
// link for a few minutes (2, 5 or 10; 5 by default), the screen locks, once.
//
//  - off until turned on (`<app config dir>/prefs.json`, keys `away_lock`,
//    `away_lock_minutes`);
//  - a phone is a device whose status says `"platform": "android"`; the ones
//    seen are remembered (`away_lock_phones`), so it works right after a
//    restart too;
//  - it locks once per absence: it waits for a phone to come back before it
//    can lock again, so unlocking with the phone still away doesn't loop;
//  - right after the app starts (or the switch is turned on), the phone gets
//    the whole wait to show up first;
//  - a notification says why it locked.
//
// Only where this computer can lock its screen (`device::screen::can_lock`).

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Manager, State};

use crate::device::screen;
use crate::Eng;

const KEY_ON: &str = "away_lock";
const KEY_MINUTES: &str = "away_lock_minutes";
const KEY_PHONES: &str = "away_lock_phones";

/// The waits on offer, in minutes.
const CHOICES: [u32; 3] = [2, 5, 10];
const DEFAULT_MINUTES: u32 = 5;

/// How often the links are looked at.
const TICK: Duration = Duration::from_secs(15);

/// The setting, as the UI sees it.
#[derive(serde::Serialize, specta::Type)]
pub struct AwayLock {
    on: bool,
    /// Minutes without a phone before locking: 2, 5 or 10.
    minutes: u32,
    /// This computer can lock its screen at all (else the row is hidden).
    can_lock: bool,
}

/// What `should_lock` decides from.
pub struct Check {
    /// The setting is on.
    pub on: bool,
    pub minutes: u32,
    /// There's a phone to wait for at all.
    pub has_phone: bool,
    /// When a phone last had a live link (this run).
    pub last_seen: Option<Instant>,
    pub now: Instant,
    /// Already locked for this absence.
    pub locked: bool,
    /// When waiting started: the app's start, or the switch being turned on.
    pub armed_at: Instant,
}

/// Lock now? Only with the setting on, a phone to wait for, not already
/// locked for this absence, and `minutes` gone by since the phone was last
/// live, counted from no earlier than `armed_at`.
pub fn should_lock(c: &Check) -> bool {
    if !c.on || !c.has_phone || c.locked || c.minutes == 0 {
        return false;
    }
    let since = match c.last_seen {
        Some(seen) => seen.max(c.armed_at),
        None => c.armed_at,
    };
    c.now.saturating_duration_since(since) >= Duration::from_secs(u64::from(c.minutes) * 60)
}

struct Inner {
    on: bool,
    minutes: u32,
    /// Your phones (node ids) seen so far.
    phones: BTreeSet<String>,
    armed_at: Instant,
    last_seen: Option<Instant>,
    locked: bool,
    /// Live devices already asked for their status this run.
    asked: BTreeSet<String>,
}

/// The "Lock this computer when my phone goes away" setting and what it's
/// tracking.
pub struct AwayLockState {
    kv: Option<FileKv>,
    inner: Mutex<Inner>,
}

impl AwayLockState {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let get = |key: &str| kv.as_ref().and_then(|kv| kv.get(key));
        let on = get(KEY_ON).as_deref() == Some("on");
        let minutes = get(KEY_MINUTES)
            .and_then(|m| m.parse().ok())
            .filter(|m| CHOICES.contains(m))
            .unwrap_or(DEFAULT_MINUTES);
        let phones = get(KEY_PHONES)
            .and_then(|p| serde_json::from_str(&p).ok())
            .unwrap_or_default();
        AwayLockState {
            kv,
            inner: Mutex::new(Inner {
                on,
                minutes,
                phones,
                armed_at: Instant::now(),
                last_seen: None,
                locked: false,
                asked: BTreeSet::new(),
            }),
        }
    }

    fn get(&self) -> AwayLock {
        let (on, minutes) = self
            .inner
            .lock()
            .map(|i| (i.on, i.minutes))
            .unwrap_or((false, DEFAULT_MINUTES));
        AwayLock {
            on,
            minutes,
            can_lock: screen::can_lock(),
        }
    }

    fn set(&self, on: bool, minutes: u32) {
        let minutes = if CHOICES.contains(&minutes) {
            minutes
        } else {
            DEFAULT_MINUTES
        };
        if let Ok(mut i) = self.inner.lock() {
            // Turned on or changed: the phone gets the whole wait again.
            if on && (!i.on || i.minutes != minutes) {
                i.armed_at = Instant::now();
                i.locked = false;
            }
            i.on = on;
            i.minutes = minutes;
        }
        if let Some(kv) = &self.kv {
            let saved = kv
                .set(KEY_ON, if on { "on" } else { "off" })
                .and_then(|()| kv.set(KEY_MINUTES, &minutes.to_string()));
            if let Err(e) = saved {
                eprintln!("[away-lock] couldn't save the setting: {e}");
            }
        }
    }

    /// A device reported its status: remember it if it's a phone.
    fn saw_status(&self, from: &str, status: &serde_json::Value) {
        if status.get("platform").and_then(|p| p.as_str()) != Some("android") {
            return;
        }
        let Ok(mut i) = self.inner.lock() else {
            return;
        };
        if !i.phones.insert(from.to_string()) {
            return;
        }
        let saved = serde_json::to_string(&i.phones).ok();
        drop(i);
        if let (Some(kv), Some(json)) = (&self.kv, saved) {
            if let Err(e) = kv.set(KEY_PHONES, &json) {
                eprintln!("[away-lock] couldn't remember the phone: {e}");
            }
        }
    }
}

/// A device's status arrived (`CoreEvent::DeviceStatus`).
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    if let Some(state) = app.try_state::<AwayLockState>() {
        state.saw_status(from, status);
    }
}

/// Look at the links every few seconds, and lock when it's time.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        if tick(&app) {
            screen::lock();
            crate::notify::post(
                &app,
                "Locked because your phone went away",
                "Turn this off under \u{201c}Lock this computer when my phone goes away\u{201d} in my-domain\u{2019}s Settings.",
            );
        }
    });
}

/// One look: true when the screen should lock now (and it's then counted
/// as locked for this absence).
fn tick(app: &AppHandle) -> bool {
    let (Some(state), Some(engine)) = (app.try_state::<AwayLockState>(), app.try_state::<Eng>())
    else {
        return false;
    };
    let live = engine.connected_peers();
    let peers: BTreeSet<String> = engine.get_peers().into_iter().map(|p| p.node_id).collect();
    let Ok(mut i) = state.inner.lock() else {
        return false;
    };
    // While on, live devices not yet known to be phones: ask once what
    // they are (the answer comes back as a status).
    let unknown: Vec<String> = live
        .iter()
        .filter(|n| i.on && !i.phones.contains(*n) && !i.asked.contains(*n))
        .cloned()
        .collect();
    i.asked.extend(unknown.iter().cloned());
    // Phones you've since removed don't count.
    let phones: Vec<&String> = i
        .phones
        .iter()
        .filter(|n| peers.is_empty() || peers.contains(*n))
        .collect();
    let now = Instant::now();
    let phone_live = phones.iter().any(|n| live.contains(n));
    let has_phone = !phones.is_empty();
    if phone_live {
        i.last_seen = Some(now);
        i.locked = false;
    }
    let lock = should_lock(&Check {
        on: i.on,
        minutes: i.minutes,
        has_phone,
        last_seen: i.last_seen,
        now,
        locked: i.locked,
        armed_at: i.armed_at,
    });
    if lock {
        i.locked = true;
    }
    drop(i);
    for node in unknown {
        let _ = engine.request_status(&node);
    }
    lock && screen::can_lock()
}

/// "Lock this computer when my phone goes away": on or off, how long to
/// wait, and whether this computer can lock at all.
#[tauri::command]
#[specta::specta]
pub fn away_lock(state: State<AwayLockState>) -> AwayLock {
    state.get()
}

/// Turn locking when your phone goes away on or off, and set the wait
/// (2, 5 or 10 minutes).
#[tauri::command]
#[specta::specta]
pub fn set_away_lock(on: bool, minutes: u32, state: State<AwayLockState>) {
    state.set(on, minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn check(start: Instant) -> Check {
        Check {
            on: true,
            minutes: 5,
            has_phone: true,
            last_seen: None,
            now: start,
            locked: false,
            armed_at: start,
        }
    }

    #[test]
    fn locks_after_the_wait() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.last_seen = Some(t0 + MIN);
        c.now = t0 + MIN * 5;
        assert!(!should_lock(&c), "only 4 minutes away");
        c.now = t0 + MIN * 6;
        assert!(should_lock(&c));
    }

    #[test]
    fn never_while_off() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.on = false;
        c.now = t0 + MIN * 60;
        assert!(!should_lock(&c));
    }

    #[test]
    fn once_per_absence() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.now = t0 + MIN * 60;
        c.locked = true;
        assert!(!should_lock(&c));
    }

    #[test]
    fn grace_after_start() {
        let t0 = Instant::now();
        let mut c = check(t0);
        // Never seen this run: the wait counts from the start.
        c.now = t0 + MIN * 4;
        assert!(!should_lock(&c));
        c.now = t0 + MIN * 5;
        assert!(should_lock(&c));
        // Seen long before the switch was turned on: counts from then.
        let mut c = check(t0 + MIN * 30);
        c.last_seen = Some(t0);
        c.now = t0 + MIN * 31;
        assert!(!should_lock(&c));
    }

    #[test]
    fn not_without_a_phone() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.has_phone = false;
        c.now = t0 + MIN * 60;
        assert!(!should_lock(&c));
    }

    #[test]
    fn not_while_the_phone_is_here() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.now = t0 + MIN * 60;
        c.last_seen = Some(c.now);
        assert!(!should_lock(&c));
    }

    #[test]
    fn the_wait_follows_the_setting() {
        let t0 = Instant::now();
        let mut c = check(t0);
        c.minutes = 2;
        c.now = t0 + MIN * 2;
        assert!(should_lock(&c));
        c.minutes = 10;
        assert!(!should_lock(&c));
    }
}
