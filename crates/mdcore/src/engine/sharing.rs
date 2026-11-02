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

    /// Share a verification code (found by the host in a notification) with
    /// your other devices; those with a clipboard copy it
    /// (`CoreEvent::CodeCopied`). Sent the reliable way (it connects if
    /// there's no link yet), stamped with the time: a device drops a code
    /// older than `features::code::MAX_AGE_SECS` instead of pasting a stale
    /// one late.
    pub fn share_code(&self, code: &str, app: Option<&str>) -> crate::Result<()> {
        if !crate::features::code::is_code(code) {
            return Err("that doesn't look like a code".to_string().into());
        }
        let from = self.ctx.identity.lock().node_id.clone();
        let msg = serde_json::json!({
            "type": "code", "code": code, "app": app, "from": from,
            "ts": crate::transport::now_secs(),
        })
        .to_string();
        let peers: Vec<String> = self.ctx.peers.lock().keys().cloned().collect();
        for node in peers {
            crate::engine::send_reliably(&self.ctx, &node, &msg);
        }
        Ok(())
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

    // --- quick device actions (see features::device) ---

    /// Set this device's status, a JSON object such as
    /// `{"battery": 80, "charging": true}`, pushed by the host whenever it
    /// changes. Sent to devices that call `request_status`.
    pub fn set_device_status(&self, status_json: &str) {
        *self.ctx.device_status.lock() = status_json.to_string();
    }

    // These go like chat messages (`send`): held until the device is
    // reachable (a sleeping phone is woken by push), resent until it
    // acknowledges, and acted on once. Each returns the transport used, or
    // `"queued"`.

    /// Ask a device for its status (reply arrives as `CoreEvent::DeviceStatus`).
    pub fn request_status(&self, node_id: &str) -> crate::Result<String> {
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "status_request", "from": me }).to_string();
        self.send(node_id, &m)
    }

    /// Make a device ring (`CoreEvent::Ring` on it).
    pub fn ring(&self, node_id: &str) -> crate::Result<String> {
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "ring", "from": me }).to_string();
        self.send(node_id, &m)
    }

    /// Open a web link on a device (`CoreEvent::OpenUrl` on it). Only
    /// http(s) links; the receiving device checks again.
    pub fn open_url(&self, node_id: &str, url: &str) -> crate::Result<String> {
        if !crate::features::device::is_openable(url) {
            return Err("only http and https links can be opened".to_string().into());
        }
        let me = self.ctx.identity.lock().node_id.clone();
        let m =
            serde_json::json!({ "type": "open_url", "from": me, "url": url.trim() }).to_string();
        self.send(node_id, &m)
    }

    /// Ask a device to run an action it listed in its status (`actions`),
    /// e.g. `run_action(phone, "flashlight", r#"{"on":true}"#)`. Arrives as
    /// `CoreEvent::Action` there.
    pub fn run_action(&self, node_id: &str, name: &str, args_json: &str) -> crate::Result<String> {
        if !crate::features::device::is_action_name(name) {
            return Err(format!("not an action name: {name}").into());
        }
        let args: serde_json::Value = if args_json.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(args_json).map_err(|e| format!("bad action arguments: {e}"))?
        };
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "action", "from": me, "name": name, "args": args })
            .to_string();
        self.send(node_id, &m)
    }

    // --- files (see features::files) ---

    /// Where files from your other devices are saved. Until this is set,
    /// offers are declined.
    pub fn set_download_dir(&self, dir: &str) {
        *self.ctx.files.download_dir.lock() = Some(dir.into());
    }

    /// Send a file. Returns the transfer id at once; progress comes as
    /// `FileProgress`, then `FileSent` or `FileFailed`. Like a message, the
    /// offer waits for the device to be reachable.
    pub fn send_file(&self, node_id: &str, path: &str) -> crate::Result<String> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        crate::features::files::start(&self.ctx, node_id, std::path::Path::new(path))
    }

    /// Stop a transfer (either way); the other device is told.
    pub fn cancel_file(&self, id: &str) {
        crate::features::files::cancel(&self.ctx, id);
    }

    /// Press button `index` of notification `key` on `node_id` (with `text`
    /// for a reply button), or dismiss it with `index` -1. Arrives there as
    /// `CoreEvent::NotificationAction`.
    pub fn notification_action(
        &self,
        node_id: &str,
        key: &str,
        index: i64,
        text: Option<&str>,
    ) -> crate::Result<String> {
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({
            "type": "notif_action", "from": me, "key": key, "index": index, "text": text,
        })
        .to_string();
        self.send(node_id, &m)
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
        self.share_app_notification_with(pkg, app, title, body, None, "")
    }

    /// `share_app_notification`, plus the notification's `key` and its
    /// buttons (`actions_json`: `[{"title": "Reply", "reply": true}, ...]`),
    /// so the other device can act on it (`notification_action`).
    pub fn share_app_notification_with(
        &self,
        pkg: &str,
        app: &str,
        title: &str,
        body: &str,
        key: Option<&str>,
        actions_json: &str,
    ) {
        let actions: serde_json::Value = serde_json::from_str(actions_json).unwrap_or_default();
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
            let mut m = serde_json::json!({
                "type": "notification", "from": me, "title": title, "body": body, "app": app,
            });
            if let Some(key) = key {
                m["key"] = key.into();
                m["actions"] = actions.clone();
            }
            let _ = self.send_control(&n, &m.to_string());
        }
    }
}
