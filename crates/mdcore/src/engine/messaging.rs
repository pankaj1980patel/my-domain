//! Sending chat and pulling/syncing the clipboard.

use super::*;

/// What `send` returns when the message is waiting for a link.
pub const QUEUED: &str = "queued";

/// `Engine::send` for code that only has the context (background threads):
/// hold `text` until `node_id` acknowledges it, sending it now over a live
/// link, or starting the connect ladder (which wakes a sleeping device).
pub(crate) fn send_reliably<P: Platform>(ctx: &NetCtx<P>, node_id: &str, text: &str) {
    let from = ctx.identity.lock().name.clone();
    let id = transport::outbox::new_id();
    transport::outbox::hold(ctx, node_id, &id, &from, text);
    if ctx.ws_conns.lock().contains_key(node_id) {
        transport::outbox::pump(ctx, node_id, false, Some(&id));
    } else {
        let (ctx, node) = (ctx.clone(), node_id.to_string());
        let (url, token) = (ctx.server_url.clone(), ctx.token.clone());
        std::thread::spawn(move || super::ladder::run_connect_ladder(ctx, url, token, node));
    }
}

impl<P: Platform> Engine<P> {
    // --- messaging ---

    /// Send a chat message over the peer's live link, opening a QUIC link
    /// first if there is none. The message is kept (`transport::outbox`) and
    /// resent on later links until the peer acknowledges it
    /// (`CoreEvent::MessageDelivered`). With no way to reach the peer right
    /// now, the connect ladder starts in the background and this returns
    /// `"queued"`: the message goes out as soon as a link is up. Otherwise
    /// returns the transport used.
    pub fn send(&self, node_id: &str, text: &str) -> crate::Result<String> {
        self.send_with_id(node_id, text, &transport::outbox::new_id())
    }

    /// `send`, with the message id chosen by the caller (the UI matches it
    /// against `MessageDelivered`).
    pub fn send_with_id(&self, node_id: &str, text: &str, id: &str) -> crate::Result<String> {
        let from = self
            .username
            .lock()
            .clone()
            .unwrap_or_else(|| self.ctx.identity.lock().name.clone());
        if self.ctx.key.lock().is_none() {
            return Err(CoreError::NoKey);
        }
        // A hand-entered peer is re-listed under its real id once a link
        // reveals it: before dialing (a later send), and after (this one).
        let node_id = &self.resolve_manual(node_id);
        if !self.ctx.peers.lock().contains_key(node_id.as_str()) {
            return Err(CoreError::PeerNotFound);
        }
        let linked = self.ensure_link(node_id);
        let node_id = &self.resolve_manual(node_id);
        transport::outbox::hold(&self.ctx, node_id, id, &from, text);
        let live = linked || self.ctx.ws_conns.lock().contains_key(node_id.as_str());
        if live && transport::outbox::pump(&self.ctx, node_id, false, Some(id)) {
            // Handed to a route that isn't confirmed live yet (a relay
            // route waiting for the peer's hello): still waiting, really.
            if self.ctx.p2p_conns.lock().contains_key(node_id.as_str()) {
                return Ok(self.display_transport(node_id));
            }
        } else {
            let _ = self.connect(node_id);
        }
        Ok(QUEUED.to_string())
    }

    // --- clipboard ---

    pub fn enable_clipboard_sync(&self) {
        if let Some(clip) = self.ctx.platform.clipboard() {
            if let Some(cur) = clip.get() {
                *self.ctx.clip_last.lock() = cur;
            }
        }
        self.ctx.clip_active.store(true, Ordering::Relaxed);
    }

    pub fn disable_clipboard_sync(&self) {
        self.ctx.clip_active.store(false, Ordering::Relaxed);
    }

    pub fn clipboard_sync_enabled(&self) -> bool {
        self.ctx.clip_active.load(Ordering::Relaxed)
    }

    /// Pull a peer's clipboard once, write it locally, and return it.
    pub fn get_clipboard(&self, node_id: &str) -> crate::Result<String> {
        let clip = self.ctx.platform.clipboard().ok_or(CoreError::Unsupported(
            "clipboard not supported on this device",
        ))?;
        let (tx, rx) = mpsc::channel::<String>();
        self.ctx.clip_pending.lock().insert(node_id.to_string(), tx);

        let me = self.ctx.identity.lock().node_id.clone();
        let from = self.ctx.identity.lock().name.clone();
        let req = serde_json::json!({ "type": "clipboard_request", "from": me }).to_string();
        self.ensure_link(node_id);
        if let Err(e) = transport::send_payload(
            &self.ctx.ws_conns,
            &self.ctx.p2p_conns,
            &self.ctx.key,
            &from,
            node_id,
            &req,
        ) {
            self.ctx.clip_pending.lock().remove(node_id);
            return Err(e);
        }

        let result = rx.recv_timeout(Duration::from_secs(6));
        self.ctx.clip_pending.lock().remove(node_id);
        let content = result.map_err(|_| {
            "no clipboard response from peer (is it reachable and using the same key?)".to_string()
        })?;
        clip.set(&content);
        *self.ctx.clip_last.lock() = content.clone();
        crate::features::clipboard::remember(&self.ctx, &content, Some(node_id));
        Ok(content)
    }

    /// Recent clipboard values from all your devices, newest first (also
    /// sent as `ClipboardHistory` whenever it changes).
    pub fn clipboard_history(&self) -> Vec<crate::model::ClipEntry> {
        self.ctx.clip_history.lock().iter().cloned().collect()
    }

    /// Forget the clipboard history.
    pub fn clear_clipboard_history(&self) {
        self.ctx.clip_history.lock().clear();
        self.ctx.sink.emit(CoreEvent::ClipboardHistory { entries: vec![] });
    }

    /// Put `text` (e.g. an entry from the history) on this device's
    /// clipboard. With auto-sync on, it then goes to your other devices too.
    pub fn copy_to_clipboard(&self, text: &str) -> crate::Result<()> {
        let clip = self.ctx.platform.clipboard().ok_or(CoreError::Unsupported(
            "clipboard not supported on this device",
        ))?;
        clip.set(text);
        Ok(())
    }

    /// `manual:<ip>` placeholders are replaced by the real node id once a
    /// QUIC link reveals it; map a replaced placeholder to that id.
    fn resolve_manual(&self, node_id: &str) -> String {
        self.ctx
            .manual_aliases
            .lock()
            .get(node_id)
            .cloned()
            .unwrap_or_else(|| node_id.to_string())
    }
}
