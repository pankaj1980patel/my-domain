//! First-run setup, the same short stepper on both apps (after signing in
//! or joining):
//!
//!   1. name this device        (`Name`)
//!   2. add your other devices  (`Devices`)
//!   3. choose features         (`Features`: the groups in `GROUPS`)
//!   4. allow what they need    (`Permissions`: the app opens each OS page)
//!   5. test the connection     (`Connection`: live from the links map)
//!
//! Every step can be skipped, and the whole thing put off ("Later": the
//! app just closes it; it carries on at the same step next time) or ended
//! ("Skip setup"). Where it stands is kept in this device's store
//! (`onboarding`), so it's remembered per device and survives restarts.
//! The state machine is pure (`Onboarding::apply`) and unit-tested; the
//! apps only draw the step and send what was tapped.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::engine::NetCtx;
use crate::platform::Platform;

/// The feature groups the user picks from, in the order shown.
pub const GROUPS: [&str; 5] = ["clipboard", "notifications", "files", "media", "find"];
const KEY: &str = "onboarding";
/// The longest device name taken, in characters.
pub const MAX_NAME: usize = 40;

/// A step of the stepper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Name,
    Devices,
    Features,
    Permissions,
    Connection,
    Done,
}

/// The steps in order (`Done` isn't one).
pub const STEPS: [Step; 5] = [
    Step::Name,
    Step::Devices,
    Step::Features,
    Step::Permissions,
    Step::Connection,
];

impl Step {
    pub fn parse(s: &str) -> Option<Step> {
        Some(match s.trim() {
            "name" => Step::Name,
            "devices" => Step::Devices,
            "features" => Step::Features,
            "permissions" => Step::Permissions,
            "connection" => Step::Connection,
            "done" => Step::Done,
            _ => return None,
        })
    }

    fn pos(self) -> usize {
        STEPS.iter().position(|s| *s == self).unwrap_or(STEPS.len())
    }
}

/// What was tapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// "Next" (the step was done).
    Next,
    Back,
    /// "Skip" this step.
    Skip,
    /// Jump to a step (the stepper's dots).
    Goto(Step),
    /// "Skip setup": ends it; the Health check is still there.
    SkipAll,
    /// "Done" on the last step.
    Finish,
    /// "Run setup again" (Settings): from the start, choices kept.
    Restart,
}

impl Action {
    /// `action` as the apps send it: "next", "back", "skip", "goto" (with
    /// `step`), "skip_all", "finish", "restart".
    pub fn parse(action: &str, step: Option<&str>) -> Option<Action> {
        Some(match action.trim() {
            "next" => Action::Next,
            "back" => Action::Back,
            "skip" => Action::Skip,
            "goto" => Action::Goto(Step::parse(step?)?),
            "skip_all" => Action::SkipAll,
            "finish" => Action::Finish,
            "restart" => Action::Restart,
            _ => return None,
        })
    }
}

/// Where setup stands on this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Onboarding {
    pub step: Step,
    /// Ended: by finishing or "Skip setup". Not shown by itself again.
    pub finished: bool,
    /// Ended with "Skip setup".
    pub skipped_all: bool,
    /// Steps skipped (each once, in the order skipped).
    pub skipped: Vec<Step>,
    /// The feature groups picked; none: not picked yet (all count).
    pub features: Option<Vec<String>>,
}

impl Default for Onboarding {
    fn default() -> Self {
        Onboarding {
            step: Step::Name,
            finished: false,
            skipped_all: false,
            skipped: Vec::new(),
            features: None,
        }
    }
}

impl Onboarding {
    /// The state after `action`.
    pub fn apply(&self, action: Action) -> Onboarding {
        let mut s = self.clone();
        let next = |p: usize| STEPS.get(p + 1).copied().unwrap_or(Step::Done);
        match action {
            Action::Next => {
                if s.step != Step::Done {
                    s.skipped.retain(|x| *x != s.step);
                    s.step = next(s.step.pos());
                }
            }
            Action::Skip => {
                if s.step != Step::Done {
                    if !s.skipped.contains(&s.step) {
                        s.skipped.push(s.step);
                    }
                    s.step = next(s.step.pos());
                }
            }
            Action::Back => {
                let p = s.step.pos();
                if p > 0 {
                    s.step = STEPS[p - 1];
                }
            }
            Action::Goto(step) => {
                if step != Step::Done {
                    s.step = step;
                }
            }
            Action::SkipAll => {
                s.step = Step::Done;
                s.finished = true;
                s.skipped_all = true;
            }
            Action::Finish => {
                s.step = Step::Done;
                s.finished = true;
            }
            Action::Restart => {
                s.step = Step::Name;
                s.finished = false;
                s.skipped_all = false;
                s.skipped.clear();
            }
        }
        // Past the last step: done.
        if s.step == Step::Done {
            s.finished = true;
        }
        s
    }

    /// The step's number, 1 to `STEPS.len()` (`Done`: past the last).
    pub fn number(&self) -> usize {
        self.step.pos() + 1
    }

    /// Show the "Finish setting up" card: not ended yet.
    pub fn resumable(&self) -> bool {
        !self.finished
    }

    /// The feature groups in use: those picked, else all of them.
    pub fn enabled(&self) -> Vec<String> {
        match &self.features {
            Some(f) => f.clone(),
            None => GROUPS.map(String::from).to_vec(),
        }
    }

    /// Picked these groups (unknown ones dropped, `GROUPS` order).
    pub fn with_features(&self, groups: &[String]) -> Onboarding {
        let mut s = self.clone();
        s.features = Some(clean_features(groups));
        s
    }
}

/// Known groups only, once each, in `GROUPS` order.
pub fn clean_features(groups: &[String]) -> Vec<String> {
    GROUPS
        .iter()
        .filter(|g| groups.iter().any(|x| x.trim() == **g))
        .map(|g| g.to_string())
        .collect()
}

/// A device name as typed: spaces collapsed, no control characters, 1 to
/// `MAX_NAME` characters.
pub fn clean_name(name: &str) -> Option<String> {
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let n = name.chars().count();
    (n > 0 && n <= MAX_NAME).then_some(name)
}

/// The kept state, read once.
#[derive(Default)]
pub struct OnboardingState(Mutex<Option<Onboarding>>);

fn with<P: Platform, R>(ctx: &NetCtx<P>, f: impl FnOnce(&mut Onboarding) -> R) -> R {
    let mut guard = ctx.extras.onboarding.0.lock();
    let s = guard.get_or_insert_with(|| {
        ctx.platform
            .kv_get(KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    f(s)
}

/// Where setup stands.
pub fn get<P: Platform>(ctx: &NetCtx<P>) -> Onboarding {
    with(ctx, |s| s.clone())
}

/// Change it with `f`, keep it, and return it.
pub fn update<P: Platform>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&Onboarding) -> Onboarding,
) -> Onboarding {
    let (now, saved) = with(ctx, |s| {
        let next = f(s);
        let changed = next != *s;
        *s = next.clone();
        (next, changed)
    });
    if saved {
        if let Ok(json) = serde_json::to_string(&now) {
            ctx.platform.kv_set(KEY, &json);
        }
    }
    now
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(actions: &[Action]) -> Onboarding {
        actions
            .iter()
            .fold(Onboarding::default(), |s, a| s.apply(*a))
    }

    #[test]
    fn next_walks_every_step_then_finishes() {
        let mut s = Onboarding::default();
        assert_eq!((s.step, s.number(), s.resumable()), (Step::Name, 1, true));
        for want in [
            Step::Devices,
            Step::Features,
            Step::Permissions,
            Step::Connection,
        ] {
            s = s.apply(Action::Next);
            assert_eq!(s.step, want);
            assert!(!s.finished);
        }
        s = s.apply(Action::Next);
        assert_eq!(
            (s.step, s.finished, s.skipped_all),
            (Step::Done, true, false)
        );
        assert!(!s.resumable());
        // Nothing moves once done, but Next/Skip/Back are harmless.
        assert_eq!(s.apply(Action::Next), s);
        assert_eq!(s.apply(Action::Skip), s);
    }

    #[test]
    fn skip_and_back() {
        let s = walk(&[Action::Skip, Action::Next, Action::Skip]);
        assert_eq!(s.step, Step::Permissions);
        assert_eq!(s.skipped, vec![Step::Name, Step::Features]);
        // Back to a skipped step and done this time: no longer skipped.
        let s = s.apply(Action::Back).apply(Action::Next);
        assert_eq!(s.step, Step::Permissions);
        assert_eq!(s.skipped, vec![Step::Name]);
        // Back stops at the first step.
        let s = walk(&[Action::Back, Action::Back]);
        assert_eq!(s.step, Step::Name);
        // Skipping the same step twice lists it once.
        let s = walk(&[Action::Skip, Action::Back, Action::Skip]);
        assert_eq!(s.skipped, vec![Step::Name]);
    }

    #[test]
    fn skip_all_finish_goto_restart() {
        let s = walk(&[Action::Next, Action::SkipAll]);
        assert_eq!(
            (s.step, s.finished, s.skipped_all),
            (Step::Done, true, true)
        );
        let s = walk(&[Action::Finish]);
        assert_eq!((s.finished, s.skipped_all), (true, false));
        let s = walk(&[Action::Goto(Step::Connection)]);
        assert_eq!(s.step, Step::Connection);
        assert_eq!(
            walk(&[Action::Goto(Step::Done)]).step,
            Step::Name,
            "Done isn't a step to jump to"
        );
        // Restart keeps the choices, forgets the rest.
        let s = Onboarding::default()
            .with_features(&["files".into()])
            .apply(Action::Skip)
            .apply(Action::SkipAll)
            .apply(Action::Restart);
        assert_eq!(s.step, Step::Name);
        assert!(!s.finished && !s.skipped_all && s.skipped.is_empty());
        assert_eq!(s.features, Some(vec!["files".to_string()]));
    }

    #[test]
    fn actions_parse() {
        assert_eq!(Action::parse("next", None), Some(Action::Next));
        assert_eq!(
            Action::parse("goto", Some("features")),
            Some(Action::Goto(Step::Features))
        );
        assert_eq!(Action::parse("goto", None), None);
        assert_eq!(Action::parse("goto", Some("nowhere")), None);
        assert_eq!(Action::parse("skip_all", None), Some(Action::SkipAll));
        assert_eq!(Action::parse("explode", None), None);
    }

    #[test]
    fn features_are_known_groups_in_order() {
        let s = Onboarding::default();
        assert_eq!(s.enabled().len(), GROUPS.len(), "all until picked");
        let s = s.with_features(&[
            "find".into(),
            "bogus".into(),
            "clipboard".into(),
            "find".into(),
        ]);
        assert_eq!(
            s.features,
            Some(vec!["clipboard".to_string(), "find".to_string()])
        );
        assert_eq!(s.enabled(), vec!["clipboard", "find"]);
        assert!(
            s.with_features(&[]).enabled().is_empty(),
            "picked none: none"
        );
    }

    #[test]
    fn names_are_tidied() {
        assert_eq!(
            clean_name("  Pankaj's   Pixel ").as_deref(),
            Some("Pankaj's Pixel")
        );
        assert_eq!(clean_name("a\u{7}b").as_deref(), Some("ab"));
        assert_eq!(clean_name("   "), None);
        assert_eq!(
            clean_name(&"x".repeat(MAX_NAME)).map(|n| n.len()),
            Some(MAX_NAME)
        );
        assert_eq!(clean_name(&"x".repeat(MAX_NAME + 1)), None);
    }

    #[test]
    fn the_state_round_trips_as_json() {
        let s = walk(&[Action::Skip, Action::Next]).with_features(&["media".into()]);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"step\":\"features\""), "{json}");
        assert_eq!(serde_json::from_str::<Onboarding>(&json).unwrap(), s);
    }
}
