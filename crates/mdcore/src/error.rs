//! The core's error type.
//!
//! Before: every API returned `Result<_, String>`, so callers could only show
//! the text; they couldn't tell "not logged in" from "peer unreachable".
//! Now: an enum with one variant per failure a caller may want to handle.
//! `thiserror` writes the `Display`/`Error` impls from the `#[error("...")]`
//! attributes (Node analogy: `class NotLoggedInError extends Error`, with the
//! message declared next to the class).
//!
//! The `Display` texts deliberately match the old strings, so the desktop and
//! Android UIs show exactly what they showed before.
//!
//! Migration aid: `From<String>` / `From<&str>` map into `Other`, so code that
//! still produces string errors keeps compiling with `?`. New code should use a
//! specific variant.

use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("not logged in")]
    NotLoggedIn,

    #[error("set your encryption key first")]
    NoKey,

    #[error("peer not found")]
    PeerNotFound,

    /// The operation needs a live link (WS/punch/WebRTC) that doesn't exist.
    #[error("{0}")]
    NoLink(String),

    /// The platform lacks a capability (e.g. no clipboard on this device).
    #[error("{0}")]
    Unsupported(&'static str),

    /// Bad input from the caller (invalid IP, empty username, …).
    #[error("{0}")]
    InvalidInput(String),

    /// A peer or server didn't answer in time.
    #[error("{0}")]
    Timeout(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The registry server returned an error or couldn't be reached.
    #[error("{0}")]
    Http(String),

    /// Anything not (yet) given its own variant.
    #[error("{0}")]
    Other(String),
}

impl From<String> for CoreError {
    fn from(s: String) -> Self {
        CoreError::Other(s)
    }
}

impl From<&str> for CoreError {
    fn from(s: &str) -> Self {
        CoreError::Other(s.to_string())
    }
}

/// Adapters that still speak strings (JNI "ERROR: …", Tauri `Result<_, String>`).
impl From<CoreError> for String {
    fn from(e: CoreError) -> Self {
        e.to_string()
    }
}

/// Serialize as the display text, so a Tauri command can return
/// `Result<T, CoreError>` and the JS side receives the same message string.
impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_matches_the_old_string_errors() {
        assert_eq!(CoreError::NotLoggedIn.to_string(), "not logged in");
        assert_eq!(
            CoreError::NoKey.to_string(),
            "set your encryption key first"
        );
        assert_eq!(CoreError::from("boom").to_string(), "boom");
    }

    #[test]
    fn serializes_as_its_message() {
        assert_eq!(
            serde_json::to_string(&CoreError::PeerNotFound).unwrap(),
            "\"peer not found\""
        );
    }
}
