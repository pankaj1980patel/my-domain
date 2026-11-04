// "Quiet when my phone is quiet": which of your phones are on Do Not Disturb
// right now, as far as this computer can tell, for "Stay quiet while my
// phone is on Do Not Disturb" (the switch is `notify::Prefs::follow_dnd`).
//
// A phone says so two ways, and the newest one counts:
//  - `dnd: true|false` in its status (pushed when it changes);
//  - mdcore's own Do Not Disturb message (`CoreEvent::Dnd`).
//
// What was heard goes stale: a phone counts as quiet only while it has a
// live link and what it said is under `STALE_AFTER` old. So a phone that
// went away, or went silent, doesn't keep this computer quiet. While the
// switch is on, a live phone that's quiet is asked for a fresh status once
// what it said is `REFRESH_AFTER` old, so a long Do Not Disturb doesn't
// lapse by itself.
//
// When "quiet" starts or ends (also by going stale), the held pop-ups are
// summed up (`notify::dnd_changed`), the tray's tooltip says so, and the
// window hears `phone-quiet` (the phones keeping it quiet; empty: none).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::Eng;

/// What a phone said is forgotten after this long.
pub const STALE_AFTER: Duration = Duration::from_secs(30 * 60);
/// A live, quiet phone is asked again after this long.
pub const REFRESH_AFTER: Duration = Duration::from_secs(20 * 60);
/// How often staleness is looked at.
const TICK: Duration = Duration::from_secs(60);

/// What a phone last said about its Do Not Disturb, and when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    pub dnd: bool,
    pub at: Instant,
}

/// Is this phone quiet now? Only with a live link and a fresh "on".
pub fn is_quiet(heard: Option<Heard>, live: bool, now: Instant) -> bool {
    match heard {
        Some(h) => live && h.dnd && now.saturating_duration_since(h.at) < STALE_AFTER,
        None => false,
    }
}

/// The phones keeping this computer quiet (sorted node ids).
pub fn quiet_phones(heard: &HashMap<String, Heard>, live: &[String], now: Instant) -> Vec<String> {
    let mut out: Vec<String> = heard
        .iter()
        .filter(|(node, h)| is_quiet(Some(**h), live.contains(node), now))
        .map(|(node, _)| node.clone())
        .collect();
    out.sort();
    out
}

/// Quiet phones whose word is getting old enough to ask again.
pub fn to_refresh(heard: &HashMap<String, Heard>, live: &[String], now: Instant) -> Vec<String> {
    let mut out: Vec<String> = heard
        .iter()
        .filter(|(node, h)| {
            is_quiet(Some(**h), live.contains(node), now)
                && now.saturating_duration_since(h.at) >= REFRESH_AFTER
        })
        .map(|(node, _)| node.clone())
        .collect();
    out.sort();
    out
}

/// The phones' last word, and what "quiet" was last time it was looked at.
#[derive(Default)]
pub struct Quiet {
    heard: Mutex<HashMap<String, Heard>>,
    /// The phones keeping it quiet last time (switch on), to spot changes.
    shown: Mutex<Vec<String>>,
    /// Phones already asked for a fresh status, and when.
    asked: Mutex<HashMap<String, Instant>>,
}

impl Quiet {
    fn heard(&self, from: &str, dnd: bool) {
        if let Ok(mut h) = self.heard.lock() {
            h.insert(
                from.to_string(),
                Heard {
                    dnd,
                    at: Instant::now(),
                },
            );
        }
        if let Ok(mut a) = self.asked.lock() {
            a.remove(from);
        }
    }
}

/// A device's status arrived: note its `dnd`, if it sent one.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(dnd) = status.get("dnd").and_then(|d| d.as_bool()) else {
        return;
    };
    let Some(q) = app.try_state::<Quiet>() else {
        return;
    };
    q.heard(from, dnd);
    changed_soon(app);
}

/// mdcore heard a phone go into or out of Do Not Disturb.
pub fn on_dnd(app: &AppHandle, from: &str, on: bool) {
    if let Some(q) = app.try_state::<Quiet>() {
        q.heard(from, on);
    }
    changed_soon(app);
}

/// A link came up or went away (a phone without one isn't quiet).
pub fn on_link(app: &AppHandle) {
    changed_soon(app);
}

/// The phones on Do Not Disturb now, switch or not.
pub fn phones(app: &AppHandle) -> Vec<String> {
    let (Some(q), Some(engine)) = (app.try_state::<Quiet>(), app.try_state::<Eng>()) else {
        return Vec::new();
    };
    let live = engine.connected_peers();
    q.heard
        .lock()
        .map(|h| quiet_phones(&h, &live, Instant::now()))
        .unwrap_or_default()
}

/// The phones keeping this computer quiet: none while the switch is off.
pub fn in_effect(app: &AppHandle) -> Vec<String> {
    let on = app
        .try_state::<crate::notify::Prefs>()
        .is_some_and(|p| p.follow_dnd());
    if on {
        phones(app)
    } else {
        Vec::new()
    }
}

/// Look again off the caller's thread (it may be the core's).
fn changed_soon(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || changed(&app));
}

/// Look again; when "quiet" started, ended or moved to other phones, tell
/// the pop-ups, the tray and the window.
pub fn changed(app: &AppHandle) {
    let now = in_effect(app);
    let Some(q) = app.try_state::<Quiet>() else {
        return;
    };
    let moved = match q.shown.lock() {
        Ok(mut shown) if *shown != now => {
            *shown = now.clone();
            true
        }
        _ => false,
    };
    if !moved {
        return;
    }
    crate::notify::dnd_changed(app);
    crate::tray::set_quiet(app, !now.is_empty());
    let _ = app.emit("phone-quiet", &now);
}

/// Every minute: notice what went stale, and ask quiet phones again
/// before it does.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        changed(&app);
        refresh(&app);
    });
}

fn refresh(app: &AppHandle) {
    let on = app
        .try_state::<crate::notify::Prefs>()
        .is_some_and(|p| p.follow_dnd());
    let (Some(q), Some(engine)) = (app.try_state::<Quiet>(), app.try_state::<Eng>()) else {
        return;
    };
    if !on {
        return;
    }
    let live = engine.connected_peers();
    let now = Instant::now();
    let due = q
        .heard
        .lock()
        .map(|h| to_refresh(&h, &live, now))
        .unwrap_or_default();
    for node in due {
        // Once per `REFRESH_AFTER` at most, if it doesn't answer.
        let ask = q.asked.lock().is_ok_and(|mut a| {
            let fresh = a
                .get(&node)
                .is_some_and(|at| now.saturating_duration_since(*at) < REFRESH_AFTER);
            if !fresh {
                a.insert(node.clone(), now);
            }
            !fresh
        });
        if ask {
            let _ = engine.request_status(&node);
        }
    }
}

/// The phones keeping this computer quiet right now (node ids; empty when
/// "Stay quiet while my phone is on Do Not Disturb" is off or none is).
#[tauri::command]
#[specta::specta]
pub fn phone_quiet(app: AppHandle) -> Vec<String> {
    in_effect(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(dnd: bool, ago_min: u64, now: Instant) -> Heard {
        Heard {
            dnd,
            at: now - Duration::from_secs(ago_min * 60),
        }
    }

    #[test]
    fn a_live_phone_on_dnd_is_quiet() {
        let now = Instant::now() + Duration::from_secs(3 * 60 * 60);
        assert!(is_quiet(Some(heard(true, 0, now)), true, now));
        assert!(is_quiet(Some(heard(true, 29, now)), true, now));
        assert!(!is_quiet(Some(heard(false, 0, now)), true, now));
        assert!(!is_quiet(None, true, now));
    }

    #[test]
    fn stale_or_gone_counts_as_not_quiet() {
        let now = Instant::now() + Duration::from_secs(3 * 60 * 60);
        // Older than 30 minutes: unknown.
        assert!(!is_quiet(Some(heard(true, 30, now)), true, now));
        assert!(!is_quiet(Some(heard(true, 90, now)), true, now));
        // No live link: unknown, however fresh.
        assert!(!is_quiet(Some(heard(true, 0, now)), false, now));
    }

    #[test]
    fn any_of_several_phones_keeps_it_quiet() {
        let now = Instant::now() + Duration::from_secs(3 * 60 * 60);
        let mut h = HashMap::new();
        h.insert("a".to_string(), heard(false, 1, now));
        h.insert("b".to_string(), heard(true, 5, now));
        h.insert("c".to_string(), heard(true, 45, now)); // stale
        h.insert("d".to_string(), heard(true, 2, now)); // not live
        let live = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(quiet_phones(&h, &live, now), ["b"]);
        // b comes out of it: nobody keeps it quiet.
        h.insert("b".to_string(), heard(false, 0, now));
        assert!(quiet_phones(&h, &live, now).is_empty());
        // d comes back, still fresh.
        let live = vec!["d".to_string()];
        assert_eq!(quiet_phones(&h, &live, now), ["d"]);
    }

    #[test]
    fn quiet_phones_are_asked_again_before_it_goes_stale() {
        let now = Instant::now() + Duration::from_secs(3 * 60 * 60);
        let mut h = HashMap::new();
        h.insert("fresh".to_string(), heard(true, 5, now));
        h.insert("old".to_string(), heard(true, 25, now));
        h.insert("off".to_string(), heard(false, 25, now));
        h.insert("away".to_string(), heard(true, 25, now));
        let live: Vec<String> = ["fresh", "old", "off"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(to_refresh(&h, &live, now), ["old"]);
    }
}
