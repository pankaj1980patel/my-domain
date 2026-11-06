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
    /// Send a picture copied on this device (a PNG or JPEG file at `path`)
    /// for `node_id` to put on its clipboard; it's also saved there like any
    /// received file. Returns the transfer id.
    pub fn send_clipboard_image(&self, node_id: &str, path: &str) -> crate::Result<String> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        crate::features::files::start_with_extra(
            &self.ctx,
            node_id,
            std::path::Path::new(path),
            &crate::transport::outbox::new_id(),
            Some(serde_json::json!({ "clipboard": true })),
        )
    }

    pub fn send_file(&self, node_id: &str, path: &str) -> crate::Result<String> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        crate::features::files::start(&self.ctx, node_id, std::path::Path::new(path))
    }

    /// Send a whole folder: each file inside goes as its own transfer (with
    /// the usual events), and the other device recreates the folder under
    /// its downloads. Returns how many files it holds. See
    /// `features::folders`.
    pub fn send_folder(&self, node_id: &str, path: &str) -> crate::Result<u32> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        let n = crate::features::folders::start(&self.ctx, node_id, std::path::Path::new(path))?;
        Ok(n as u32)
    }

    /// How many folders `send_folder` is still sending. 0 once every file
    /// of every folder has ended (or the folder was stopped), so a copy made
    /// just for sending can go.
    pub fn folders_sending(&self) -> u32 {
        crate::features::folders::sending(&self.ctx) as u32
    }

    /// Send several files and folders as one batch (one row, with each
    /// file inside it; folders are recreated under the device's downloads).
    /// See `features::batches`.
    pub fn send_batch(
        &self,
        node_id: &str,
        paths: &[String],
    ) -> crate::Result<crate::model::BatchStart> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        let paths: Vec<std::path::PathBuf> = paths.iter().map(Into::into).collect();
        crate::features::batches::start(&self.ctx, node_id, &paths)
    }

    /// `node_id` knows batches and pausing (`features::batches`): its
    /// transfers to here can be paused from here.
    pub fn transfer_batches_supported(&self, node_id: &str) -> bool {
        crate::features::batches::supports(&self.ctx, node_id)
    }

    /// Batches going either way, and the last ones that ended.
    pub fn transfer_batches(&self) -> Vec<crate::model::BatchItem> {
        crate::features::batches::list(&self.ctx)
    }

    /// Pause (`paused`) or resume a transfer or a batch, either way. A
    /// paused transfer keeps what it has and waits, across restarts too.
    /// Receiving, only from a device that knows how (else an error).
    pub fn pause_file(&self, id: &str, paused: bool) -> crate::Result<()> {
        crate::features::files::set_paused(&self.ctx, id, paused, None)
    }

    /// Send a failed transfer again: a batch's files that didn't go (same
    /// id), a file sent from here, or a link asked of a device. Returns the
    /// id to follow (new, except for a batch).
    pub fn retry_file(&self, id: &str) -> crate::Result<String> {
        crate::features::files::retry(&self.ctx, id)
    }

    /// Tests only: damage the next `n` file chunks this device sends.
    #[doc(hidden)]
    pub fn test_corrupt_file_chunks(&self, n: u32) {
        self.ctx
            .files
            .corrupt_chunks
            .store(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Stop a transfer (either way); the other device is told.
    pub fn cancel_file(&self, id: &str) {
        // A link downloaded on a device (either way) is a transfer too.
        if crate::features::remote_download::cancel(&self.ctx, id) {
            return;
        }
        crate::features::files::cancel(&self.ctx, id);
    }

    /// Tests only: answer file offers like a device from before transfers
    /// could resume (no offset in the accept, any partial file thrown
    /// away), to check such a peer still gets whole files.
    #[doc(hidden)]
    pub fn test_legacy_file_receiver(&self, on: bool) {
        self.ctx
            .files
            .legacy_receiver
            .store(on, std::sync::atomic::Ordering::Relaxed);
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
            resumed: false,
            batch: None,
        });
        Ok(id)
    }

    // --- sending text (see features::share) ---

    /// Send `text` to a device: onto its clipboard (`mode` "clipboard") or
    /// shown as a notification ("notify"). Arrives there as `TextShared`.
    /// Like a message, it waits for the device to be reachable; returns the
    /// transport used, or "queued".
    pub fn send_text(&self, node_id: &str, text: &str, mode: &str) -> crate::Result<String> {
        self.send_text_marked(node_id, text, mode, false)
    }

    /// `send_text`, marked `sensitive` (a password copied from a password
    /// manager, sent anyway): the device keeps it out of its history and
    /// notices. Older devices ignore the mark.
    pub fn send_text_marked(
        &self,
        node_id: &str,
        text: &str,
        mode: &str,
        sensitive: bool,
    ) -> crate::Result<String> {
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
        let m = crate::features::share::message(&me, text, mode, sensitive);
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

    /// Consumer: set the full enabled package set on a producer. Also asks
    /// to hear when a shared notification goes away there
    /// (`CoreEvent::NotificationRemoved`).
    pub fn subscribe_apps(&self, node_id: &str, apps_json: &str) -> crate::Result<()> {
        self.subscribe_apps_with(node_id, apps_json, false, false)
    }

    /// `subscribe_apps`, with this device's filters: `only_locked` (only
    /// while the phone isn't in use) and `private` (the content of a
    /// sensitive notification left out on the phone). A phone from before
    /// ignores them.
    pub fn subscribe_apps_with(
        &self,
        node_id: &str,
        apps_json: &str,
        only_locked: bool,
        private: bool,
    ) -> crate::Result<()> {
        let me = self.ctx.identity.lock().node_id.clone();
        let apps: serde_json::Value =
            serde_json::from_str(apps_json).unwrap_or(serde_json::Value::Array(vec![]));
        let m = serde_json::json!({
            "type": "subscribe_apps", "from": me, "apps": apps, "removals": true,
            "only_locked": only_locked, "private": private,
        })
        .to_string();
        self.send_control(node_id, &m)
    }

    /// `node_id` understands rich notifications (conversations, pictures,
    /// private mode, app icons).
    pub fn notif_rich_supported(&self, node_id: &str) -> bool {
        crate::features::notif_rich::supports(&self.ctx, node_id)
    }

    /// Producer: app `pkg`'s icon, a small JPEG in base64 (at most
    /// `notif_rich::MAX_ICON` bytes). Notifications from it then name it,
    /// and a subscriber that hasn't got it asks. False when it isn't one.
    pub fn set_app_icon(&self, pkg: &str, jpeg_b64: &str) -> bool {
        crate::features::notif_rich::set_icon(&self.ctx, pkg, jpeg_b64)
    }

    /// Subscriber: ask `node_id` for app `pkg`'s icon; it arrives as
    /// `CoreEvent::NotificationIcon`. Only to a device that understands
    /// rich notifications, for an app this device follows there.
    pub fn request_notification_icon(&self, node_id: &str, pkg: &str) -> crate::Result<()> {
        crate::features::notif_rich::request_icon(&self.ctx, node_id, pkg)
    }

    /// Consumer (a phone): show (`on`) or stop showing a computer's own
    /// notifications here. Only for a computer whose status says
    /// `notif_share`. They arrive as `CoreEvent::Notification` with a key,
    /// go away as `CoreEvent::NotificationRemoved`, and a dismiss here goes
    /// back with `notification_action` (index -1).
    pub fn subscribe_computer_notifications(&self, node_id: &str, on: bool) -> crate::Result<()> {
        let apps = if on {
            serde_json::json!([crate::features::notif_apps::ALL_APPS])
        } else {
            serde_json::json!([])
        };
        self.subscribe_apps(node_id, &apps.to_string())
    }

    /// Producer (a computer): the devices that asked for this computer's
    /// own notifications (`subscribe_computer_notifications`).
    pub fn computer_notification_subscribers(&self) -> Vec<String> {
        let all = crate::features::notif_apps::ALL_APPS;
        let mut nodes: Vec<String> = self
            .ctx
            .subs
            .lock()
            .iter()
            .filter(|(_, pkgs)| pkgs.iter().any(|p| p == all))
            .map(|(n, _)| n.clone())
            .collect();
        nodes.sort();
        nodes
    }

    /// Producer (a computer): one of its own apps (`app`, its name) posted
    /// a notification, `key` naming it here; it goes to the devices that
    /// asked for this computer's notifications. The computer decides which
    /// apps go out before calling this.
    pub fn share_computer_notification(&self, app: &str, title: &str, body: &str, key: &str) {
        if key.is_empty() {
            return;
        }
        let all = crate::features::notif_apps::ALL_APPS;
        self.share_app_notification_with(all, app, title, body, Some(key), "[]")
    }

    /// Producer (a computer): its notification `key`, shared with
    /// `share_computer_notification`, went away here.
    pub fn share_computer_notification_removed(&self, key: &str) {
        self.share_app_notification_removed(crate::features::notif_apps::ALL_APPS, key)
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
        let mut n = serde_json::json!({ "title": title, "body": body, "app": app });
        if let Some(key) = key {
            n["key"] = key.into();
            n["actions"] = actions;
        }
        crate::features::notif_rich::share(&self.ctx, pkg, &n);
    }

    /// Producer: share a notification of package `pkg` with everything it
    /// has. `notif_json`: `{title, body, app, key?, actions?, in_use?,
    /// conv?, conv_title?, group_chat?, sender?, messages?, avatar?,
    /// picture?, sensitive?, silent?, ts?}` (see `features::notif_rich`);
    /// `in_use`: the phone is unlocked with its screen on now. Each
    /// subscriber gets what it understands and asked for.
    pub fn share_app_notification_rich(&self, pkg: &str, notif_json: &str) {
        let n: serde_json::Value = serde_json::from_str(notif_json).unwrap_or_default();
        if n.is_object() {
            crate::features::notif_rich::share(&self.ctx, pkg, &n);
        }
    }

    /// Producer: the notification `key` from package `pkg`, shared with
    /// `share_app_notification_with`, went away here (swiped, opened, or
    /// cancelled by its app). Tells the subscribers of `pkg` that asked to
    /// hear it; they get `CoreEvent::NotificationRemoved`.
    pub fn share_app_notification_removed(&self, pkg: &str, key: &str) {
        if key.is_empty() {
            return;
        }
        let me = self.ctx.identity.lock().node_id.clone();
        let wanted = self.ctx.removal_subs.lock().clone();
        let targets: Vec<String> = self
            .ctx
            .subs
            .lock()
            .iter()
            .filter(|(n, pkgs)| wanted.contains(*n) && pkgs.iter().any(|p| p == pkg))
            .map(|(n, _)| n.clone())
            .collect();
        let m = serde_json::json!({ "type": "notif_removed", "from": me, "key": key }).to_string();
        for n in targets {
            let _ = self.send_control(&n, &m);
        }
    }
}
