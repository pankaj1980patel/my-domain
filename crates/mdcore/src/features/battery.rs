//! Battery alerts: your other devices hear when this one's battery runs low
//! or has finished charging.
//!
//!   battery_alert { from, kind: "low" | "full", level }   device → all
//!
//! Worked out from the status the host pushes (`Engine::set_device_status`,
//! `{"battery": 0-100, "charging": bool}`), so the phone and laptops get it
//! with no extra code. Once per crossing:
//!
//! - "low": not charging and at or below the threshold (default 15%). Armed
//!   again once charging, or 5 points above the threshold.
//! - "full": charging and at 100%. Armed again once it's down to 97% or
//!   less (so unplugging and plugging back in at 100% stays quiet).
//!
//! Which alerts have gone out is kept (`kv`), so a restart doesn't repeat
//! one. Sent like a chat message (held until each device is reachable, a
//! sleeping one woken by push); it arrives as `CoreEvent::BatteryAlert`.
//! `Engine::set_battery_alerts` turns them off or moves the threshold.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::BatteryAlertSettings;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

const SETTINGS_KEY: &str = "battery_alerts";
const ARMED_KEY: &str = "battery_alerts_armed";
/// Points above the threshold before "low" is armed again.
const LOW_REARM: u8 = 5;
/// "full" is armed again at or below this.
const FULL_REARM: u8 = 97;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Armed {
    low: bool,
    full: bool,
}

impl Default for Armed {
    fn default() -> Self {
        Armed {
            low: true,
            full: true,
        }
    }
}

#[derive(Default)]
struct State {
    loaded: bool,
    settings: BatteryAlertSettings,
    armed: Armed,
}

/// The device's alert settings and which alerts are armed.
#[derive(Default)]
pub struct BatteryAlerts {
    state: Mutex<State>,
}

/// What a reading means: the alert to send, if any, and the new arming.
fn step(
    s: &BatteryAlertSettings,
    armed: Armed,
    level: u8,
    charging: bool,
) -> (Option<&'static str>, Armed) {
    let mut next = armed;
    let mut alert = None;
    if charging || level >= s.low.saturating_add(LOW_REARM) {
        next.low = true;
    }
    if level <= FULL_REARM {
        next.full = true;
    }
    if !charging && level <= s.low && next.low {
        next.low = false;
        alert = Some("low");
    } else if charging && level >= 100 && next.full {
        next.full = false;
        alert = Some("full");
    }
    (alert, next)
}

fn load<P: Platform>(ctx: &NetCtx<P>, st: &mut State) {
    if st.loaded {
        return;
    }
    st.loaded = true;
    let get = |k: &str| ctx.platform.kv_get(k);
    st.settings = get(SETTINGS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    st.armed = get(ARMED_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
}

/// The current settings.
pub fn settings<P: Platform>(ctx: &NetCtx<P>) -> BatteryAlertSettings {
    let mut st = ctx.extras.battery.state.lock();
    load(ctx, &mut st);
    st.settings.clone()
}

/// Change the settings; `low` must be 1-99.
pub fn set_settings<P: Platform>(ctx: &NetCtx<P>, s: BatteryAlertSettings) -> crate::Result<()> {
    if !(1..=99).contains(&s.low) {
        return Err("the low-battery level must be 1 to 99%".to_string().into());
    }
    let mut st = ctx.extras.battery.state.lock();
    load(ctx, &mut st);
    if let Ok(json) = serde_json::to_string(&s) {
        ctx.platform.kv_set(SETTINGS_KEY, &json);
    }
    st.settings = s;
    Ok(())
}

/// A new status from the host: send an alert if a line was crossed.
pub fn on_status<P: Platform>(ctx: &NetCtx<P>, status_json: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(status_json) else {
        return;
    };
    let Some(level) = v["battery"].as_u64().map(|l| l.min(100) as u8) else {
        return;
    };
    let charging = v["charging"].as_bool().unwrap_or(false);
    let alert = {
        let mut st = ctx.extras.battery.state.lock();
        load(ctx, &mut st);
        let (alert, armed) = step(&st.settings, st.armed, level, charging);
        if armed != st.armed {
            st.armed = armed;
            if let Ok(json) = serde_json::to_string(&armed) {
                ctx.platform.kv_set(ARMED_KEY, &json);
            }
        }
        alert.filter(|_| st.settings.enabled)
    };
    let Some(kind) = alert else { return };
    let me = ctx.identity.lock().node_id.clone();
    let msg =
        json!({ "type": "battery_alert", "from": me, "kind": kind, "level": level }).to_string();
    let peers: Vec<String> = ctx.peers.lock().keys().cloned().collect();
    for node in peers.iter().filter(|n| !n.starts_with("manual:")) {
        crate::engine::send_reliably(ctx, node, &msg);
    }
}

pub struct BatteryHandler;

impl<P: Platform> ControlHandler<P> for BatteryHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["battery_alert"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let kind = msg.str("kind");
        let Some(level) = msg.json("level").as_u64().filter(|l| *l <= 100) else {
            return;
        };
        if from.is_empty() || !matches!(kind.as_str(), "low" | "full") {
            return;
        }
        ctx.sink.emit(CoreEvent::BatteryAlert {
            from,
            alert: kind,
            level: level as u8,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed readings in order; the alerts that came out.
    fn run(readings: &[(u8, bool)]) -> Vec<&'static str> {
        let s = BatteryAlertSettings::default();
        let mut armed = Armed::default();
        let mut out = Vec::new();
        for &(level, charging) in readings {
            let (alert, next) = step(&s, armed, level, charging);
            armed = next;
            out.extend(alert);
        }
        out
    }

    #[test]
    fn low_once_per_crossing() {
        assert_eq!(
            run(&[
                (20, false),
                (16, false),
                (15, false),
                (14, false),
                (9, false)
            ]),
            ["low"]
        );
        // Wobbling around the line doesn't repeat it.
        assert_eq!(run(&[(15, false), (16, false), (15, false)]), ["low"]);
        // Charged a little, then down again: a new crossing.
        assert_eq!(
            run(&[(15, false), (15, true), (16, false), (14, false)]),
            ["low", "low"]
        );
        assert_eq!(
            run(&[(15, false), (20, false), (15, false)]),
            ["low", "low"]
        );
        // Low while charging: no alert.
        assert!(run(&[(10, true), (12, true)]).is_empty());
    }

    #[test]
    fn full_once_until_it_drains_a_bit() {
        assert_eq!(
            run(&[(98, true), (99, true), (100, true), (100, true)]),
            ["full"]
        );
        // Unplugged and plugged back in at 100: quiet.
        assert_eq!(
            run(&[(100, true), (100, false), (99, false), (100, true)]),
            ["full"]
        );
        // Down to 97 and charged again: a new one.
        assert_eq!(
            run(&[(100, true), (97, false), (100, true)]),
            ["full", "full"]
        );
        // Unplugged at 100 isn't "finished charging".
        assert!(run(&[(100, false)]).is_empty());
    }
}
