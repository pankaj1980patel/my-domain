//! Sending chat and pulling/syncing the clipboard.

use super::*;

impl<P: Platform> Engine<P> {
    // --- messaging ---

    pub fn connect_ws(&self, node_id: &str) -> crate::Result<()> {
        let peer = self
            .ctx
            .peers
            .lock()
            .get(node_id)
            .cloned()
            .ok_or(CoreError::PeerNotFound)?;
        if peer.ws_port == 0 {
            return Err("peer has no WebSocket port".into());
        }
        if self.ctx.ws_conns.lock().contains_key(node_id) {
            return Ok(());
        }
        ws::ws_connect(self.net_ctx(), &peer.ip, peer.ws_port).map_err(Into::into)
    }

    /// Send a chat message, auto-selecting the transport: the active persistent
    /// connection (WS/punch/WebRTC) if one is live, else the directed transport
    /// (UDP/TCP). Returns the protocol used so the UI can report it.
    pub fn send(&self, node_id: &str, text: &str) -> crate::Result<String> {
        let from = self
            .username
            .lock()
            .clone()
            .unwrap_or_else(|| self.ctx.identity.lock().name.clone());
        self.ensure_link(node_id);
        let proto = self.proto_for(node_id);
        transport::send_payload(
            &self.ctx.peers,
            &self.ctx.ws_conns,
            &self.ctx.key,
            &from,
            node_id,
            proto,
            text,
        )?;
        Ok(self.display_transport(node_id, proto))
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
        let proto = self.proto_for(node_id);

        if let Err(e) = transport::send_payload(
            &self.ctx.peers,
            &self.ctx.ws_conns,
            &self.ctx.key,
            &from,
            node_id,
            proto,
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
        Ok(content)
    }
}
