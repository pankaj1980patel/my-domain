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
        crate::features::battery::on_status(&self.ctx, status_json);
    }

    /// What's playing on this device, as a JSON object `{title, artist,
    /// album, app, state}` (`state`: "playing" / "paused" / "stopped"), or
    /// "" / "null" for nothing. Call it whenever it changes; devices that
    /// asked for this one's status lately hear the change
    /// (`CoreEvent::NowPlaying`), and status replies carry it.
    pub fn set_now_playing(&self, media_json: &str) {
        let v: serde_json::Value = serde_json::from_str(media_json).unwrap_or_default();
        crate::features::media::set(&self.ctx, &v);
    }

    /// Some device asked for this one's status in the last few minutes:
    /// worth reading what's playing (a desktop polls only then).
    pub fn now_playing_watched(&self) -> bool {
        crate::features::media::watched(&self.ctx)
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

    /// `open_url`, in a chosen app on the device (an Android package such
    /// as `com.google.android.youtube` or `com.spotify.music`); a device
    /// without that app (or a computer) opens it in the browser.
    pub fn open_url_in(&self, node_id: &str, url: &str, app: &str) -> crate::Result<String> {
        if !crate::features::device::is_openable(url) {
            return Err("only http and https links can be opened".to_string().into());
        }
        if !crate::features::device::is_package(app) {
            return Err(format!("not an app id: {app}").into());
        }
        let me = self.ctx.identity.lock().node_id.clone();
        let m =
            serde_json::json!({ "type": "open_url", "from": me, "url": url.trim(), "app": app })
                .to_string();
        self.send(node_id, &m)
    }

    /// Search for `query` in `app` ("youtube", "spotify" or "web") on a
    /// device: the `search` action, which a device lists in its `actions`
    /// (with the apps it has in `search_apps`). The phone opens the app's
    /// search (the browser if the app is missing); a computer opens the
    /// search page in the browser.
    pub fn search_in_app(&self, node_id: &str, app: &str, query: &str) -> crate::Result<String> {
        use crate::features::device::{MAX_QUERY, SEARCH_APPS};
        if !SEARCH_APPS.contains(&app) {
            return Err(format!("can't search in {app}").into());
        }
        let query = query.trim();
        if query.is_empty() || query.len() > MAX_QUERY {
            return Err("search for something shorter".to_string().into());
        }
        let args = serde_json::json!({ "app": app, "query": query }).to_string();
        self.run_action(node_id, "search", &args)
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

    // --- browsing another device's shared folders (see features::browse) ---

    /// The folders other devices may browse and download from, each shown
    /// under its `name`. Replaces the previous set; empty shares nothing.
    /// Folders that don't exist (yet) are kept but not listed.
    pub fn set_shared_folders(&self, folders: &[crate::model::SharedFolder]) -> crate::Result<()> {
        let mut list: Vec<(String, std::path::PathBuf)> = Vec::new();
        for f in folders {
            if !crate::features::browse::is_folder_name(&f.name) {
                return Err(format!("not a folder name: {}", f.name).into());
            }
            if list.iter().any(|(n, _)| *n == f.name) {
                return Err(format!("two folders named {}", f.name).into());
            }
            list.push((f.name.clone(), f.path.clone().into()));
        }
        *self.ctx.files.shared.lock() = list;
        Ok(())
    }

    /// What `set_shared_folders` last set.
    pub fn shared_folders(&self) -> Vec<crate::model::SharedFolder> {
        self.ctx
            .files
            .shared
            .lock()
            .iter()
            .map(|(name, path)| crate::model::SharedFolder {
                name: name.clone(),
                path: path.to_string_lossy().into_owned(),
            })
            .collect()
    }

    /// Ask a device what's in `path` of its shared folder `root` (`root` ""
    /// lists the shared folders), from entry `offset` on. Returns the
    /// request id; the answer comes as `FolderListing` with it. Like a
    /// message, the request waits for the device to be reachable.
    pub fn list_folder(
        &self,
        node_id: &str,
        root: &str,
        path: &str,
        offset: u64,
    ) -> crate::Result<String> {
        if crate::features::browse::path_parts(path).is_none() {
            return Err("not a path in a shared folder".to_string().into());
        }
        let req = transport::outbox::new_id();
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({
            "type": "fs_list", "from": me, "req": req,
            "root": root, "path": path, "offset": offset,
        })
        .to_string();
        self.send(node_id, &m)?;
        Ok(req)
    }

    /// Fetch file `path` from a device's shared folder `root` into this
    /// device's downloads folder. Returns the transfer id at once; the file
    /// then arrives like one the device sent (`FileProgress`, then
    /// `FileReceived` or `FileFailed`, all with this id), resuming if the
    /// link drops.
    pub fn download_file(&self, node_id: &str, root: &str, path: &str) -> crate::Result<String> {
        let parts = crate::features::browse::path_parts(path)
            .filter(|p| !p.is_empty() && !root.is_empty())
            .ok_or_else(|| CoreError::from("not a file in a shared folder".to_string()))?;
        if self.ctx.files.download_dir.lock().is_none() {
            return Err("this device has no downloads folder".to_string().into());
        }
        let id = transport::outbox::new_id();
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({
            "type": "fs_get", "from": me, "id": id, "root": root, "path": path,
        })
        .to_string();
        self.send(node_id, &m)?;
        // Shown as waiting until the device starts sending.
        self.ctx.sink.emit(CoreEvent::FileProgress {
            id: id.clone(),
            node_id: node_id.to_string(),
            name: crate::features::files::safe_name(parts.last().copied().unwrap_or_default()),
            done: 0,
            total: 0,
            incoming: true,
        });
        Ok(id)
    }

    // --- sending text (see features::share) ---

    /// Send `text` to a device: onto its clipboard (`mode` "clipboard") or
    /// shown as a notification ("notify"). Arrives there as `TextShared`.
    /// Like a message, it waits for the device to be reachable; returns the
    /// transport used, or "queued".
    pub fn send_text(&self, node_id: &str, text: &str, mode: &str) -> crate::Result<String> {
        if !crate::features::share::is_mode(mode) {
            return Err(format!("unknown mode {mode}").into());
        }
        if text.trim().is_empty() {
            return Err("nothing to send".to_string().into());
        }
        if text.len() > crate::features::share::MAX_TEXT {
            return Err("that text is too long; send it as a file"
                .to_string()
                .into());
        }
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "share_text", "from": me, "text": text, "mode": mode })
            .to_string();
        self.send(node_id, &m)
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
