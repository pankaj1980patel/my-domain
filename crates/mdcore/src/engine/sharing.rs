//! Broadcasting feature messages to peers: notifications, calls, call
//! history, and the app-notification pub/sub.

use super::*;

impl<P: Platform> Engine<P> {
    // --- feature sharing (broadcast a control message to all peers) ---

    /// Share an OS notification with all of the user's other devices.
    pub fn share_notification(&self, title: &str, body: &str, app: Option<&str>) {
        let from = self.ctx.identity.lock().node_id.clone();
        let msg = serde_json::json!({
            "type": "notification", "title": title, "body": body, "app": app, "from": from,
        })
        .to_string();
        transport::broadcast_control(&self.net_ctx(), &msg);
    }

    /// Share an incoming/missed/ended call notification.
    pub fn share_call_notification(&self, caller: &str, number: Option<&str>, state: &str) {
        let from = self.ctx.identity.lock().node_id.clone();
        let msg = serde_json::json!({
            "type": "call_notification", "caller": caller, "number": number, "state": state, "from": from,
        })
        .to_string();
        transport::broadcast_control(&self.net_ctx(), &msg);
    }

    /// Share call-history entries (an opaque JSON array, passed through as-is).
    pub fn share_call_history(&self, entries_json: &str) {
        let from = self.ctx.identity.lock().node_id.clone();
        let entries: serde_json::Value =
            serde_json::from_str(entries_json).unwrap_or(serde_json::Value::Null);
        let msg = serde_json::json!({ "type": "call_history", "entries": entries, "from": from })
            .to_string();
        transport::broadcast_control(&self.net_ctx(), &msg);
    }

    // --- app-notification pub/sub ---

    /// Set this device's shareable app list (JSON array of {pkg,label}) — pushed
    /// by the host (e.g. android PackageManager). Producers answer `apps_request`
    /// with this.
    pub fn set_installed_apps(&self, apps_json: &str) {
        *self.ctx.installed_apps.lock() = apps_json.to_string();
    }

    /// Consumer: ask a producer for its app list (reply arrives as CoreEvent::AppsList).
    pub fn request_apps(&self, node_id: &str) -> crate::Result<()> {
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "apps_request", "from": me }).to_string();
        self.send_control(node_id, &m)
    }

    /// Consumer: set the full enabled package set on a producer.
    pub fn subscribe_apps(&self, node_id: &str, apps_json: &str) -> crate::Result<()> {
        let me = self.ctx.identity.lock().node_id.clone();
        let apps: serde_json::Value =
            serde_json::from_str(apps_json).unwrap_or(serde_json::Value::Array(vec![]));
        let m =
            serde_json::json!({ "type": "subscribe_apps", "from": me, "apps": apps }).to_string();
        self.send_control(node_id, &m)
    }

    /// Producer: a local app posted a notification — forward it to every peer
    /// subscribed to that package (as a normal `notification`).
    pub fn share_app_notification(&self, pkg: &str, app: &str, title: &str, body: &str) {
        let me = self.ctx.identity.lock().node_id.clone();
        let targets: Vec<String> = self
            .ctx
            .subs
            .lock()
            .iter()
            .filter(|(_, pkgs)| pkgs.iter().any(|p| p == pkg))
            .map(|(n, _)| n.clone())
            .collect();
        for n in targets {
            let m = serde_json::json!({
                "type": "notification", "from": me, "title": title, "body": body, "app": app,
            })
            .to_string();
            let _ = self.send_control(&n, &m);
        }
    }
}
