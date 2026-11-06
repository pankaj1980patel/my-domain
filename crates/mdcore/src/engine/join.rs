//! Add a device without typing (see `crate::join`): the signed-in device
//! makes an invite (a QR code of a one-time join link, plus a 6-digit code)
//! and watches for the new device; the new device joins with the link or
//! the code and ends up signed in to the same account with the same key.

use super::*;
use crate::join::{normalize_code, open_code_box, random_code, seal_for_code, JoinLink};
use crate::qr::QrCode;
use crate::registry::{pair_cancel, pair_claim, pair_create, pair_status};

/// What "Add a device" shows.
#[derive(Serialize, Clone, Debug)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct JoinInvite {
    /// The `mydomain://join?…` link the QR code holds.
    pub link: String,
    /// The same invite as 6 digits, for a device that can't scan.
    pub code: String,
    /// Seconds it stays valid.
    pub expires_in: u32,
    /// The QR code: modules per side, and its dark modules as an SVG path
    /// with a 4-module margin (`viewBox="0 0 {qr_size+8} {qr_size+8}"`).
    pub qr_size: u32,
    pub qr_path: String,
}

/// Has the new device joined yet?
#[derive(Serialize, Clone, Debug)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct JoinStatus {
    /// "waiting", "joined" (then `name` says who), or "none" (expired or
    /// cancelled).
    pub state: String,
    pub name: Option<String>,
}

impl<P: Platform> Engine<P> {
    fn signed_in_session(&self) -> crate::Result<(String, String)> {
        let url = self.server_url.lock().clone();
        let token = self.token.lock().clone();
        match (url, token) {
            (Some(u), Some(t)) => Ok((u, t)),
            _ => Err(CoreError::NotLoggedIn),
        }
    }

    /// Start adding a device: a fresh single-use invite, valid 5 minutes.
    /// Replaces any earlier one.
    pub fn join_invite(&self) -> crate::Result<JoinInvite> {
        let (url, token) = self.signed_in_session()?;
        let username = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let key = (*self.ctx.key.lock()).ok_or(CoreError::NoKey)?;
        // A code another account is using right now: pick another.
        for _ in 0..5 {
            let link = JoinLink::new(&url, &username, &key).ok_or("couldn't make a join link")?;
            let code = random_code();
            let code_box = seal_for_code(&code, &key).ok_or("couldn't make a join link")?;
            let Some(created) = pair_create(&url, &token, &link.claim(), &code, &code_box)? else {
                continue;
            };
            let uri = link.to_uri();
            let qr =
                QrCode::encode(uri.as_bytes()).ok_or("the join link is too long for a QR code")?;
            return Ok(JoinInvite {
                link: uri,
                code: created.code,
                expires_in: u32::try_from(created.expires_in).unwrap_or(u32::MAX),
                qr_size: qr.size as u32,
                qr_path: qr.svg_path(),
            });
        }
        Err(CoreError::Other(
            "couldn't start adding a device, try again".into(),
        ))
    }

    /// Whether the invite was used (and by which device).
    pub fn join_status(&self) -> crate::Result<JoinStatus> {
        let (url, token) = self.signed_in_session()?;
        let s = pair_status(&url, &token)?;
        Ok(JoinStatus {
            state: s.state,
            name: s.name,
        })
    }

    /// Withdraw the invite (the QR was closed).
    pub fn join_cancel(&self) -> crate::Result<()> {
        let (url, token) = self.signed_in_session()?;
        Ok(pair_cancel(&url, &token)?)
    }

    /// Join with a scanned link: signs in to its account with its key.
    pub fn join_with_link(&self, link: &str) -> crate::Result<()> {
        let link = JoinLink::parse(link)
            .ok_or_else(|| CoreError::InvalidInput("that isn't a my-domain join code".into()))?;
        // Checked before the server is asked, so a damaged link doesn't
        // use up the invite.
        let key = link
            .open_key()
            .ok_or_else(|| CoreError::InvalidInput("that join code is damaged".into()))?;
        let name = self.identity().name;
        let claimed = pair_claim(&link.server, Some(&link.claim()), None, &name)?;
        self.adopt_session(&link.server, claimed.token, claimed.username);
        self.adopt_key(key);
        Ok(())
    }

    /// Join with the 6-digit code typed from the other device's screen.
    pub fn join_with_code(&self, server_url: &str, code: &str) -> crate::Result<()> {
        if server_url.trim().is_empty() {
            return Err("server URL is required".into());
        }
        let code = normalize_code(code)
            .ok_or_else(|| CoreError::InvalidInput("type the 6-digit code".into()))?;
        let name = self.identity().name;
        let claimed = pair_claim(server_url, None, Some(&code), &name)?;
        let key = claimed
            .code_box
            .as_deref()
            .and_then(|b| open_code_box(&code, b))
            .ok_or("the code worked but the device key didn't come through: sign in with your password instead")?;
        self.adopt_session(server_url, claimed.token, claimed.username);
        self.adopt_key(key);
        Ok(())
    }
}
