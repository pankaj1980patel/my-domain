//! First-run setup, the health check, this device's name and app version
//! (see `features::onboarding`, `features::health`, `features::app_info`).

use std::collections::BTreeMap;

use super::*;
use crate::features::app_info::{self, PeerApp};
use crate::features::health::{self, HealthReport, PlatformFacts};
use crate::features::onboarding::{self, Action, Onboarding};

/// kv key holding the name the user gave this device (else the
/// platform's).
pub(super) const NAME_KEY: &str = "device_name";

impl<P: Platform> Engine<P> {
    /// This app's version ("0.4.1"): said to your other devices (status,
    /// `addr_update`) for their health check. Call once at start.
    pub fn set_app_version(&self, version: &str) {
        app_info::set_own(&self.ctx, version);
    }

    /// What `node_id` said about its app (none: nothing yet).
    pub fn peer_app(&self, node_id: &str) -> Option<PeerApp> {
        app_info::peer(&self.ctx, node_id)
    }

    /// Every device's app that's known, by node id.
    pub fn peer_apps(&self) -> BTreeMap<String, PeerApp> {
        app_info::all(&self.ctx)
    }

    /// A push reached this device (the desktop's own push self-test answer,
    /// which isn't a signal; signals note it themselves).
    pub fn push_seen(&self) {
        app_info::push_seen(&self.ctx);
    }

    /// The health check: what the core knows plus what the app read
    /// (`facts`). `check_server` asks the server (blocks up to 5 s: call
    /// off the UI thread).
    pub fn health(&self, facts: &PlatformFacts, check_server: bool) -> HealthReport {
        let url = self.server_url.lock().clone();
        let signed_in = url.is_some() && self.token.lock().is_some();
        let server_ok = match (&url, check_server) {
            (Some(u), true) => Some(crate::registry::health_ok(u)),
            _ => None,
        };
        let features = onboarding::get(&self.ctx).enabled();
        let core = health::core_facts(&self.ctx, signed_in, server_ok, features);
        health::evaluate(&core, facts)
    }

    /// Where first-run setup stands on this device.
    pub fn onboarding(&self) -> Onboarding {
        onboarding::get(&self.ctx)
    }

    /// The user tapped `action` ("next", "back", "skip", "goto" with
    /// `step`, "skip_all", "finish", "restart") in first-run setup.
    pub fn onboarding_act(&self, action: &str, step: Option<&str>) -> crate::Result<Onboarding> {
        let a = Action::parse(action, step)
            .ok_or_else(|| CoreError::InvalidInput(format!("unknown setup action: {action}")))?;
        Ok(onboarding::update(&self.ctx, |s| s.apply(a)))
    }

    /// The feature groups picked in first-run setup (`onboarding::GROUPS`;
    /// unknown ones dropped). The health check counts permissions only for
    /// these.
    pub fn set_onboarding_features(&self, groups: &[String]) -> Onboarding {
        onboarding::update(&self.ctx, |s| s.with_features(groups))
    }

    /// Name this device (first-run setup): kept across restarts, used in
    /// place of the platform's name, and told to the server so your other
    /// devices list it by it (in the background). Returns the name as
    /// kept (spaces tidied).
    pub fn set_device_name(&self, name: &str) -> crate::Result<String> {
        let name = onboarding::clean_name(name).ok_or_else(|| {
            CoreError::InvalidInput(format!(
                "a name of 1 to {} characters",
                onboarding::MAX_NAME
            ))
        })?;
        self.ctx.platform.kv_set(NAME_KEY, &name);
        self.ctx.identity.lock().name = name.clone();
        let url = self.server_url.lock().clone();
        let token = self.token.lock().clone();
        if let (Some(url), Some(token)) = (url, token) {
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                if let Err(e) = peers::register_self(&ctx, &url, &token) {
                    eprintln!("[setup] couldn't tell the server the new name: {e}");
                }
            });
        }
        Ok(name)
    }
}
