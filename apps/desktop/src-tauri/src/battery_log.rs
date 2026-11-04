// Battery history: the last two days of each device's battery, for the small
// graph in its "This device" tile.
//
//  - a sample is `(unix seconds, percent, charging)`, taken from the device's
//    status as it arrives (`CoreEvent::DeviceStatus`);
//  - at most one sample per device per 5 minutes, plus one at every change
//    between charging and not;
//  - samples older than 48 h are dropped;
//  - kept in `<app config dir>/battery_history.json`, written at most once a
//    minute (by a background thread), so the graph survives a restart.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

/// At most one sample per this many seconds (unless charging changed).
const MIN_GAP_SECS: u32 = 5 * 60;
/// Samples older than this many seconds are dropped.
const KEEP_SECS: u32 = 48 * 60 * 60;
/// A device can't grow the file without bound (e.g. a flapping charger).
const MAX_PER_DEVICE: usize = 1000;
/// How often unsaved samples are written.
const SAVE_EVERY: Duration = Duration::from_secs(60);

const FILE: &str = "battery_history.json";

/// One reading of a device's battery.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct BatterySample {
    /// Unix seconds.
    pub at: u32,
    /// Percent, 0-100.
    pub level: u8,
    pub charging: bool,
}

/// Add `s` to a device's samples (oldest first), unless the last one is
/// under 5 minutes older and has the same charging state. A sample from
/// before the last one (the clock went back) replaces the ones after it.
/// True if it was added.
pub fn add(list: &mut Vec<BatterySample>, s: BatterySample) -> bool {
    list.retain(|x| x.at <= s.at);
    if let Some(last) = list.last() {
        if last.charging == s.charging && s.at < last.at.saturating_add(MIN_GAP_SECS) {
            return false;
        }
    }
    list.push(s);
    if list.len() > MAX_PER_DEVICE {
        list.drain(..list.len() - MAX_PER_DEVICE);
    }
    true
}

/// Drop the samples more than 48 h older than `now`. True if any went.
pub fn prune(list: &mut Vec<BatterySample>, now: u32) -> bool {
    let before = list.len();
    let oldest = now.saturating_sub(KEEP_SECS);
    list.retain(|x| x.at >= oldest);
    list.len() != before
}

struct Inner {
    /// node id -> its samples, oldest first.
    devices: HashMap<String, Vec<BatterySample>>,
    /// Something changed since the last save.
    dirty: bool,
}

/// Every device's battery history, and where it's saved.
pub struct BatteryLog {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl BatteryLog {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_config_dir().ok().map(|dir| dir.join(FILE));
        let mut devices: HashMap<String, Vec<BatterySample>> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        let now = now_secs();
        devices.retain(|_, list| {
            prune(list, now);
            !list.is_empty()
        });
        BatteryLog {
            path,
            inner: Mutex::new(Inner {
                devices,
                dirty: false,
            }),
        }
    }

    fn record(&self, node: &str, s: BatterySample) {
        let Ok(mut i) = self.inner.lock() else {
            return;
        };
        let list = i.devices.entry(node.to_string()).or_default();
        let changed = add(list, s) | prune(list, s.at);
        if changed {
            i.dirty = true;
        }
    }

    fn history(&self, node: &str) -> Vec<BatterySample> {
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.devices.get(node).cloned())
            .unwrap_or_default()
    }

    /// Write the file if something changed since the last time.
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let json = {
            let Ok(mut i) = self.inner.lock() else {
                return;
            };
            if !i.dirty {
                return;
            }
            let now = now_secs();
            i.devices.retain(|_, list| {
                prune(list, now);
                !list.is_empty()
            });
            i.dirty = false;
            match serde_json::to_string(&i.devices) {
                Ok(json) => json,
                Err(_) => return,
            }
        };
        // Write beside it, then swap it in, so a crash can't leave half a file.
        let tmp = path.with_extension("json.tmp");
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&tmp, json))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            eprintln!("[battery_log] couldn't save the battery history: {e}");
        }
    }
}

/// A device's status arrived: note its battery, if it has one.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(log) = app.try_state::<BatteryLog>() else {
        return;
    };
    let Some(level) = status.get("battery").and_then(|b| b.as_u64()) else {
        return;
    };
    let charging = status
        .get("charging")
        .and_then(|c| c.as_bool())
        .unwrap_or(false);
    log.record(
        from,
        BatterySample {
            at: now_secs(),
            level: level.min(100) as u8,
            charging,
        },
    );
}

/// Save new samples once a minute.
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("battery-log".into())
        .spawn(move || loop {
            std::thread::sleep(SAVE_EVERY);
            if let Some(log) = app.try_state::<BatteryLog>() {
                log.save();
            }
        })
        .ok();
}

fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

/// A device's battery over the last 48 hours, oldest first.
#[tauri::command]
#[specta::specta]
pub fn battery_history(node: String, state: State<BatteryLog>) -> Vec<BatterySample> {
    state.history(&node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: u32, level: u8, charging: bool) -> BatterySample {
        BatterySample {
            at,
            level,
            charging,
        }
    }

    #[test]
    fn one_sample_per_five_minutes() {
        let mut list = Vec::new();
        assert!(add(&mut list, s(1000, 80, false)));
        assert!(!add(&mut list, s(1060, 79, false)));
        assert!(!add(&mut list, s(1000 + MIN_GAP_SECS - 1, 79, false)));
        assert!(add(&mut list, s(1000 + MIN_GAP_SECS, 78, false)));
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].level, 78);
    }

    #[test]
    fn charging_change_is_always_kept() {
        let mut list = Vec::new();
        add(&mut list, s(1000, 40, false));
        assert!(add(&mut list, s(1010, 40, true)));
        assert!(add(&mut list, s(1020, 41, false)));
        assert!(!add(&mut list, s(1030, 41, false)));
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn clock_going_back_replaces_later_samples() {
        let mut list = Vec::new();
        add(&mut list, s(1000, 90, false));
        add(&mut list, s(2000, 85, false));
        assert!(add(&mut list, s(1500, 88, false)));
        assert_eq!(list, vec![s(1000, 90, false), s(1500, 88, false)]);
    }

    #[test]
    fn capped_per_device() {
        let mut list = Vec::new();
        for n in 0..(MAX_PER_DEVICE as u32 + 10) {
            add(&mut list, s(n * MIN_GAP_SECS, 50, false));
        }
        assert_eq!(list.len(), MAX_PER_DEVICE);
        assert_eq!(list[0].at, 10 * MIN_GAP_SECS);
    }

    #[test]
    fn prune_drops_older_than_two_days() {
        let now = 1_000_000;
        let mut list = vec![
            s(now - KEEP_SECS - 1, 10, false),
            s(now - KEEP_SECS, 20, false),
            s(now - 60, 30, false),
        ];
        assert!(prune(&mut list, now));
        assert_eq!(
            list,
            vec![s(now - KEEP_SECS, 20, false), s(now - 60, 30, false)]
        );
        assert!(!prune(&mut list, now));
        // Early clocks don't underflow.
        assert!(!prune(&mut list, 5));
    }
}
