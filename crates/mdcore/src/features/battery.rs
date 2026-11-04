//! Battery alerts: your other devices hear when this one's battery runs low
//! or has finished charging.
//!
//!   battery_alert { from, kind: "low" | "full" | "charged", level }   device → all
//!
//! Worked out from the status the host pushes (`Engine::set_device_status`,
//! `{"battery": 0-100, "charging": bool}`), so the phone and laptops get it
//! with no extra code. Once per crossing:
//!
//! - "low": not charging and at or below the threshold (default 15%). Armed
//!   again once charging, or 5 points above the threshold.
//! - "full": charging and at 100%. Armed again once it's down to 97% or
//!   less (so unplugging and plugging back in at 100% stays quiet).
//! - "charged": charging and at or above the chosen level (`charged`, e.g.
//!   80%; off by default). Sent instead of "full", once per charge: armed
//!   again once it's 5 points below the level (not on unplugging, so a
//!   phone that holds its charge at the level, flickering between charging
//!   and not, stays quiet). At 100% it goes out as "full", which older
//!   versions show; they drop a "charged" they don't know.
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
/// Points below the chosen level before "charged" is armed again.
const CHARGED_REARM: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Armed {
    low: bool,
    /// The charge alert ("full" or "charged"): one per charge either way.
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
    // The charge alert: "full" at 100% unless a lower level was chosen.
    let (target, kind, rearm) = match s.charged {
        1..=99 => (
            s.charged,
            "charged",
            s.charged.saturating_sub(CHARGED_REARM),
        ),
        _ => (100, "full", FULL_REARM),
    };
    if charging || level >= s.low.saturating_add(LOW_REARM) {
        next.low = true;
    }
    if level <= rearm {
        next.full = true;
    }
    if !charging && level <= s.low && next.low {
        next.low = false;
        alert = Some("low");
    } else if charging && level >= target && next.full {
        next.full = false;
        alert = Some(kind);
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

/// Change the settings; `low` must be 1-99, `charged` 0 (off) or 50-100.
pub fn set_settings<P: Platform>(ctx: &NetCtx<P>, s: BatteryAlertSettings) -> crate::Result<()> {
    if !(1..=99).contains(&s.low) {
        return Err("the low-battery level must be 1 to 99%".to_string().into());
    }
    if s.charged != 0 && !(50..=100).contains(&s.charged) {
        return Err("the charged level must be 50 to 100%, or off"
            .to_string()
            .into());
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
        if from.is_empty() || !matches!(kind.as_str(), "low" | "full" | "charged") {
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
        run_with(&BatteryAlertSettings::default(), readings)
    }

    /// The same, with the charge alert at `charged` percent.
    fn run_charged(charged: u8, readings: &[(u8, bool)]) -> Vec<&'static str> {
        let s = BatteryAlertSettings {
            charged,
            ..BatteryAlertSettings::default()
        };
        run_with(&s, readings)
    }

    fn run_with(s: &BatteryAlertSettings, readings: &[(u8, bool)]) -> Vec<&'static str> {
        let mut armed = Armed::default();
        let mut out = Vec::new();
        for &(level, charging) in readings {
            let (alert, next) = step(s, armed, level, charging);
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

    #[test]
    fn charged_once_per_charge_at_the_chosen_level() {
        // Crossing 80 while charging: once, and not "full" on top.
        assert_eq!(
            run_charged(
                80,
                &[(70, true), (79, true), (80, true), (85, true), (100, true)]
            ),
            ["charged"]
        );
        // Jumping past the level still counts.
        assert_eq!(run_charged(90, &[(85, true), (93, true)]), ["charged"]);
        // At the level but not charging: nothing.
        assert!(run_charged(80, &[(70, false), (85, false), (90, false)]).is_empty());
        // Unplugged and plugged back in near the level (or a phone holding
        // its charge there, flickering): quiet.
        assert_eq!(
            run_charged(
                80,
                &[(80, true), (80, false), (78, false), (80, true), (81, true)]
            ),
            ["charged"]
        );
        // Down 5 points below the level and charged again: a new charge.
        assert_eq!(
            run_charged(80, &[(80, true), (75, false), (76, true), (80, true)]),
            ["charged", "charged"]
        );
        // Low still works alongside.
        assert_eq!(
            run_charged(80, &[(15, false), (50, true), (80, true)]),
            ["low", "charged"]
        );
    }

    #[test]
    fn settings_saved_before_charged_existed_still_load() {
        let s = serde_json::from_str::<BatteryAlertSettings>(r#"{"enabled":false,"low":20}"#);
        assert_eq!(
            s.ok(),
            Some(BatteryAlertSettings {
                enabled: false,
                low: 20,
                charged: 0
            })
        );
    }

    #[test]
    fn charged_at_100_goes_out_as_full() {
        assert_eq!(
            run_charged(100, &[(99, true), (100, true), (100, true)]),
            ["full"]
        );
        assert_eq!(
            run_charged(100, &[(100, true), (97, false), (100, true)]),
            ["full", "full"]
        );
    }
}
