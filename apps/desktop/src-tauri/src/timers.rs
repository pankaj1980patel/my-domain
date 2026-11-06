// Timers (the core's `features::timers`): countdown timers shared between
// your devices. Thin commands for the Timers page, and this computer's
// alarm: timers started or changed on your other devices arrive by
// themselves as the `timers-changed` event (every timer shown), straight
// from the core.
//
// The alarm (the same rules as the phone's `TimerPlan.kt`):
//  - a thread wakes when the next timer that rings here ends (and at least
//    every 30 s, so a clock change or a sleep doesn't throw it off), and
//    whenever the timers change (`changed`: here, or on another device);
//  - a timer rings here when it ends if this computer is one of the
//    devices it rings on: the window comes up, an OS notification says so
//    and a `timer-ring` event lets the window ring and offer Stop and
//    "+1 min";
//  - it rings once per end time: what rang (timer id -> its end) is kept
//    in memory; a timer given more time (or started again) rings again at
//    its new end;
//  - one that ended over `LATE_MS` ago (the app was closed, the computer
//    asleep) doesn't ring: it shows as ended until dismissed.
//
// Times go to the UI as f64 (TypeScript has no u64).

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use mdcore::model::Timer;
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

/// A timer that ended this long ago doesn't ring any more.
pub const LATE_MS: u64 = 10 * 60 * 1000;
/// The longest the thread sleeps between looks.
const MAX_SLEEP_MS: u64 = 30_000;

/// One timer, as the UI shows it (the core's `Timer`).
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct TimerView {
    pub id: String,
    pub label: String,
    /// How long it was set for, in ms.
    pub duration_ms: f64,
    /// "running" or "paused".
    pub state: String,
    /// Running: when it ends (unix ms; past: ended, ringing until
    /// dismissed). Paused: 0.
    pub ends_at: f64,
    /// Paused: what's left, in ms.
    pub remaining_ms: f64,
    /// The devices (node ids) that ring when it ends.
    pub ring_on: Vec<String>,
    /// Unix ms.
    pub created_at: f64,
    pub updated_at: f64,
    /// The device that last changed it (node id) and its name then.
    pub by: String,
    pub by_name: String,
}

impl From<Timer> for TimerView {
    fn from(t: Timer) -> Self {
        TimerView {
            id: t.id,
            label: t.label,
            duration_ms: t.duration_ms as f64,
            state: t.state,
            ends_at: t.ends_at as f64,
            remaining_ms: t.remaining_ms as f64,
            ring_on: t.ring_on,
            created_at: t.created_at as f64,
            updated_at: t.updated_at as f64,
            by: t.by,
            by_name: t.by_name,
        }
    }
}

/// What a look at the timers found.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Timers to ring now.
    pub ring: Vec<String>,
    /// When the next one that rings here ends, if any.
    pub next: Option<u64>,
    /// What has rung (id -> its end) after this look: rung and too-late
    /// ones added; ones gone, paused or given a new end dropped.
    pub rung: BTreeMap<String, u64>,
}

/// Look at every timer at `now` for this device (`me`), given what rang
/// before.
pub fn plan(timers: &[Timer], me: &str, rung: &BTreeMap<String, u64>, now: u64) -> Plan {
    let mut out = Plan::default();
    for t in timers {
        if t.state != "running" || !t.ring_on.iter().any(|n| n == me) {
            continue;
        }
        if t.ends_at > now {
            out.next = Some(out.next.map_or(t.ends_at, |x| x.min(t.ends_at)));
            continue;
        }
        if rung.get(&t.id) != Some(&t.ends_at) && now - t.ends_at < LATE_MS {
            out.ring.push(t.id.clone());
        }
        out.rung.insert(t.id.clone(), t.ends_at);
    }
    out
}

/// How long to sleep after a look at `now`: until the next end, at most
/// `MAX_SLEEP_MS`.
pub fn sleep_for(next: Option<u64>, now: u64) -> Duration {
    let ms = next.map_or(MAX_SLEEP_MS, |at| at.saturating_sub(now).min(MAX_SLEEP_MS));
    Duration::from_millis(ms.max(1))
}

/// "1:05:03", "4:32", "0:07": hours only when there are some.
pub fn clock(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The OS notification: the timer's label ("Timer" when blank) and how
/// long it was set for.
pub fn text(t: &Timer) -> (String, String) {
    let label = t.label.trim();
    let title = if label.is_empty() { "Timer" } else { label };
    (
        format!("⏰ {title}"),
        format!("Time's up ({})", clock(t.duration_ms)),
    )
}

/// What rang, and the thread's wake-up call.
#[derive(Default)]
pub struct TimerAlarm {
    rung: Mutex<BTreeMap<String, u64>>,
    /// Set by `changed`; the thread waits on it.
    poke: (Mutex<bool>, Condvar),
}

impl TimerAlarm {
    fn look(&self, timers: &[Timer], me: &str, now: u64) -> (Vec<String>, Option<u64>) {
        let Ok(mut rung) = self.rung.lock() else {
            return (Vec::new(), None);
        };
        let p = plan(timers, me, &rung, now);
        *rung = p.rung;
        (p.ring, p.next)
    }

    fn wait(&self, d: Duration) {
        let (lock, cv) = &self.poke;
        let Ok(mut poked) = lock.lock() else {
            return;
        };
        if !*poked {
            poked = match cv.wait_timeout(poked, d) {
                Ok((g, _)) => g,
                Err(_) => return,
            };
        }
        *poked = false;
    }
}

/// The timers changed (here, or on another device): look again now.
pub fn changed(app: &AppHandle) {
    if let Some(state) = app.try_state::<TimerAlarm>() {
        let (lock, cv) = &state.poke;
        if let Ok(mut poked) = lock.lock() {
            *poked = true;
            cv.notify_all();
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Ring timers as they end (see the top of this file).
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("timer-alarm".into())
        .spawn(move || loop {
            let (Some(state), Some(engine)) =
                (app.try_state::<TimerAlarm>(), app.try_state::<Eng>())
            else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let timers = engine.timers();
            let me = engine.identity().node_id;
            let now = now_ms();
            let (ring, next) = state.look(&timers, &me, now);
            for id in ring {
                if let Some(t) = timers.iter().find(|t| t.id == id) {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let (title, body) = text(t);
                    // Asked for on purpose: shown even while pop-ups are
                    // held for Do Not Disturb.
                    crate::notify::post_always(&app, &title, &body);
                    let _ = app.emit("timer-ring", TimerView::from(t.clone()));
                }
            }
            state.wait(sleep_for(next, now));
        })
        .ok();
}

fn done(app: &AppHandle, r: Result<Timer, CoreError>) -> Result<TimerView, CoreError> {
    let t = r?;
    changed(app);
    Ok(t.into())
}

fn ms(v: f64) -> Result<u64, CoreError> {
    if v.is_finite() && v > 0.0 && v < 1e15 {
        Ok(v as u64)
    } else {
        Err(CoreError::InvalidInput("how long?".into()))
    }
}

/// Every timer shown: running ones by when they end (ended ones first),
/// then paused ones.
#[tauri::command]
#[specta::specta]
pub fn timers_list(engine: State<Eng>) -> Vec<TimerView> {
    engine.timers().into_iter().map(Into::into).collect()
}

/// Start a timer for `duration_ms`, ringing on `ring_on` (node ids) when it
/// ends. Goes to your other devices.
#[tauri::command]
#[specta::specta]
pub fn timer_start(
    label: String,
    duration_ms: f64,
    ring_on: Vec<String>,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<TimerView, CoreError> {
    done(&app, engine.start_timer(&label, ms(duration_ms)?, &ring_on))
}

/// Pause, resume, cancel, dismiss (stop ringing) or restart a timer, on
/// every device.
#[tauri::command]
#[specta::specta]
pub fn timer_action(
    id: String,
    action: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<TimerView, CoreError> {
    let r = match action.as_str() {
        "pause" => engine.pause_timer(&id),
        "resume" => engine.resume_timer(&id),
        "cancel" => engine.cancel_timer(&id),
        "dismiss" => engine.dismiss_timer(&id),
        "restart" => engine.restart_timer(&id),
        _ => return Err(CoreError::InvalidInput("what to do?".into())),
    };
    done(&app, r)
}

/// Add `ms` to a timer (one that ended starts again for that long).
#[tauri::command]
#[specta::specta]
pub fn timer_add(
    id: String,
    ms_more: f64,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<TimerView, CoreError> {
    done(&app, engine.add_timer_time(&id, ms(ms_more)?))
}

/// Ring on these devices when the timer ends.
#[tauri::command]
#[specta::specta]
pub fn timer_set_ring(
    id: String,
    ring_on: Vec<String>,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<TimerView, CoreError> {
    done(&app, engine.set_timer_ring(&id, &ring_on))
}

/// Your devices that share timers (node ids), this computer not included.
#[tauri::command]
#[specta::specta]
pub fn timer_devices(engine: State<Eng>) -> Vec<String> {
    engine
        .get_peers()
        .into_iter()
        .map(|p| p.node_id)
        .filter(|n| engine.timers_supported(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn timer(id: &str, state: &str, ends_at: u64, ring_on: &[&str]) -> Timer {
        Timer {
            id: id.into(),
            state: state.into(),
            ends_at,
            duration_ms: 5 * MIN,
            ring_on: ring_on.iter().map(|s| s.to_string()).collect(),
            ..Timer::default()
        }
    }

    #[test]
    fn rings_here_only_when_this_device_was_picked() {
        let now = 100 * MIN;
        let timers = [
            timer("mine", "running", now - 1, &["me", "phone"]),
            timer("theirs", "running", now - 1, &["phone"]),
            timer("paused", "paused", 0, &["me"]),
            timer("later", "running", now + MIN, &["me"]),
            timer("later-theirs", "running", now + 10, &["phone"]),
        ];
        let p = plan(&timers, "me", &BTreeMap::new(), now);
        assert_eq!(p.ring, ["mine"]);
        assert_eq!(p.next, Some(now + MIN), "only what rings here wakes it");
        assert_eq!(p.rung.get("mine"), Some(&(now - 1)));
    }

    #[test]
    fn rings_once_per_end_and_again_after_more_time() {
        let now = 100 * MIN;
        let t = [timer("t", "running", now, &["me"])];
        let first = plan(&t, "me", &BTreeMap::new(), now);
        assert_eq!(first.ring, ["t"]);
        let again = plan(&t, "me", &first.rung, now + 1_000);
        assert!(again.ring.is_empty(), "not twice");
        // "+1 min": a new end, rings again then.
        let more = [timer("t", "running", now + MIN, &["me"])];
        let p = plan(&more, "me", &again.rung, now + 2_000);
        assert!(p.ring.is_empty());
        assert_eq!(p.next, Some(now + MIN));
        assert!(p.rung.is_empty(), "the old end is forgotten");
        assert_eq!(plan(&more, "me", &p.rung, now + MIN).ring, ["t"]);
    }

    #[test]
    fn too_late_does_not_ring() {
        let now = 100 * MIN;
        let t = [timer("old", "running", now - LATE_MS, &["me"])];
        let p = plan(&t, "me", &BTreeMap::new(), now);
        assert!(p.ring.is_empty());
        assert_eq!(p.rung.get("old"), Some(&(now - LATE_MS)));
    }

    #[test]
    fn sleeps_until_the_next_end_at_most_half_a_minute() {
        assert_eq!(sleep_for(Some(1_500), 1_000), Duration::from_millis(500));
        assert_eq!(sleep_for(None, 0), Duration::from_millis(MAX_SLEEP_MS));
        assert_eq!(
            sleep_for(Some(10 * MIN), 0),
            Duration::from_millis(MAX_SLEEP_MS)
        );
        assert_eq!(sleep_for(Some(5), 10), Duration::from_millis(1));
    }

    #[test]
    fn the_clock_and_the_notification() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(7_000), "0:07");
        assert_eq!(clock(6_001), "0:07", "rounds up, like a timer");
        assert_eq!(clock(4 * MIN + 32_000), "4:32");
        assert_eq!(clock(3_903_000), "1:05:03");
        let t = Timer {
            label: "  ".into(),
            duration_ms: 10 * MIN,
            ..Timer::default()
        };
        assert_eq!(text(&t), ("⏰ Timer".into(), "Time's up (10:00)".into()));
        let t = Timer {
            label: "Pasta".into(),
            ..t
        };
        assert_eq!(text(&t).0, "⏰ Pasta");
    }
}
