//! Verification codes: the phone sees a one-time code in a notification
//! (its host app finds it) and shares it; a device with a clipboard copies
//! it there, so you can paste it on the computer you're signing in on.
//!
//!   code { from, code, app? }   phone → your other devices
//!
//! Best effort, to devices with a live link (`Engine::share_code`): a code
//! is useless a few minutes later, so it isn't queued or used to wake a
//! device.

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

pub struct CodeHandler;

/// A plausible one-time code: 4 to 10 letters or digits, at least 4 digits.
pub fn is_code(code: &str) -> bool {
    (4..=10).contains(&code.len())
        && code.chars().all(|c| c.is_ascii_alphanumeric())
        && code.chars().filter(char::is_ascii_digit).count() >= 4
}

impl<P: Platform> ControlHandler<P> for CodeHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["code"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let code = msg.str("code");
        let Some(clip) = ctx.platform.clipboard() else {
            return; // nowhere to paste it here
        };
        if !is_code(&code) {
            return;
        }
        let from = msg.sender();
        // Not sent on by the clipboard sync: it came from that device.
        *ctx.clip_last.lock() = code.clone();
        clip.set(&code);
        super::clipboard::remember(ctx, &code, Some(&from));
        ctx.sink.emit(CoreEvent::CodeCopied {
            from,
            code,
            app: msg.opt("app"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_short_and_mostly_digits() {
        assert!(is_code("123456"));
        assert!(is_code("G-1234".replace('-', "").as_str()));
        assert!(!is_code("123"));
        assert!(!is_code("abcdef"));
        assert!(!is_code("12 345"));
        assert!(!is_code("12345678901"));
    }
}
