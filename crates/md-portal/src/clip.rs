//! The clipboard on Wayland: a password manager's "concealed" mark, and
//! the wl-clipboard command lines.

/// KDE's and KeePassXC's mark on a copied password (its content "secret").
pub const SECRET_HINT: &str = "x-kde-passwordManagerHint";

/// A copy is concealed: it carries the hint, and the hint (if it could be
/// read) says "secret".
pub fn concealed(types: &[String], hint: Option<&[u8]>) -> bool {
    if !types.iter().any(|t| t == SECRET_HINT) {
        return false;
    }
    match hint {
        Some(h) => String::from_utf8_lossy(h)
            .trim()
            .eq_ignore_ascii_case("secret"),
        // Marked, but unreadable: keep it here to be safe.
        None => true,
    }
}

/// `wl-paste` arguments for the clipboard's text, as copied (no newline
/// added).
pub fn paste_args() -> [&'static str; 3] {
    ["--no-newline", "--type", "text/plain;charset=utf-8"]
}

/// `wl-copy` arguments for text (read from its stdin; it stays to serve
/// it until something else is copied).
pub fn copy_args() -> [&'static str; 2] {
    ["--type", "text/plain;charset=utf-8"]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_the_secret_hint_conceals() {
        let marked = types(&["text/plain", SECRET_HINT]);
        assert!(concealed(&marked, Some(b"secret")));
        assert!(concealed(&marked, Some(b"secret\n")));
        assert!(concealed(&marked, None));
        assert!(!concealed(&marked, Some(b"public")));
        assert!(!concealed(
            &types(&["text/plain", "UTF8_STRING"]),
            Some(b"secret")
        ));
        assert!(!concealed(&[], None));
    }

    #[test]
    fn wl_clipboard_reads_and_writes_text_as_is() {
        assert!(paste_args().contains(&"--no-newline"));
        assert_eq!(copy_args()[1], "text/plain;charset=utf-8");
    }
}
