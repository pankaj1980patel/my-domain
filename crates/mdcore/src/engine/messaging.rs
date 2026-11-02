//! Sending chat and pulling/syncing the clipboard.

use super::*;

impl<P: Platform> Engine<P> {
    // --- messaging ---

    /// Send a chat message over the peer's live link, opening a QUIC link
    /// first if there is none. With no way to reach the peer right now, the
    /// connect ladder starts in the background and this returns `NoLink`.
    /// Returns the transport used so the UI can report it.
    pub fn send(&self, node_id: &str, text: &str) -> crate::Result<String> {
        let from = self
            .username
            .lock()
            .clone()
            .unwrap_or_else(|| self.ctx.identity.lock().name.clone());
        if self.ctx.key.lock().is_none() {
            return Err(CoreError::NoKey);
        }
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        let linked = self.ensure_link(node_id);
        // A hand-entered peer may just have been re-listed under its real id.
        let node_id = &self.resolve_manual(node_id);
        if !linked && !self.ctx.ws_conns.lock().contains_key(node_id.as_str()) {
            let _ = self.connect(node_id);
            return Err(CoreError::NoLink(
                "not connected yet; connecting in the background".into(),
            ));
        }
        transport::send_payload(&self.ctx.ws_conns, &self.ctx.key, &from, node_id, text)?;
        Ok(self.display_transport(node_id))
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
        if let Err(e) =
            transport::send_payload(&self.ctx.ws_conns, &self.ctx.key, &from, node_id, &req)
        {
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
        Ok(content)
    }

    /// `manual:<ip>` placeholders are replaced by the real node id once a
    /// QUIC link reveals it; map a stale placeholder to that id.
    fn resolve_manual(&self, node_id: &str) -> String {
        let Some(ip) = node_id.strip_prefix("manual:") else {
            return node_id.to_string();
        };
        let peers = self.ctx.peers.lock();
        if peers.contains_key(node_id) {
            return node_id.to_string();
        }
        peers
            .values()
            .find(|p| p.source == Source::Manual && p.ip == ip)
            .map_or_else(|| node_id.to_string(), |p| p.node_id.clone())
    }
}
