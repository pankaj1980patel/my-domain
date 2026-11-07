//! Keys: evdev codes, X keysyms for typed text, and the GlobalShortcuts
//! portal's trigger names.

/// evdev codes used here (linux/input-event-codes.h).
pub mod code {
    pub const ESC: u16 = 1;
    pub const BACKSPACE: u16 = 14;
    pub const TAB: u16 = 15;
    pub const ENTER: u16 = 28;
    pub const LEFTCTRL: u16 = 29;
    pub const LEFTSHIFT: u16 = 42;
    pub const LEFTALT: u16 = 56;
    pub const SPACE: u16 = 57;
    pub const UP: u16 = 103;
    pub const LEFT: u16 = 105;
    pub const RIGHT: u16 = 106;
    pub const DOWN: u16 = 108;
    pub const LEFTMETA: u16 = 125;
    pub const BTN_LEFT: u16 = 0x110;
    pub const BTN_RIGHT: u16 = 0x111;
    pub const BTN_MIDDLE: u16 = 0x112;
    pub const BTN_SIDE: u16 = 0x113;
    pub const BTN_EXTRA: u16 = 0x114;
}

/// The evdev button for 1 left, 2 right, 3 middle, 4 back, 5 forward.
pub fn button_code(button: u8) -> Option<u16> {
    Some(match button {
        1 => code::BTN_LEFT,
        2 => code::BTN_RIGHT,
        3 => code::BTN_MIDDLE,
        4 => code::BTN_SIDE,
        5 => code::BTN_EXTRA,
        _ => return None,
    })
}

/// The other way: an evdev button as 1 left ... 5 forward.
pub fn button_number(code: u32) -> Option<u8> {
    (1..=5).find(|b| button_code(*b).map(u32::from) == Some(code))
}

/// The X keysym that types `c` (what the portal's NotifyKeyboardKeysym
/// takes): Latin-1 is itself, everything else Unicode's keysym range.
pub fn keysym(c: char) -> u32 {
    let cp = u32::from(c);
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        _ if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) => cp,
        _ => 0x0100_0000 + cp,
    }
}

/// The evdev key (and whether Shift is held) that types `c` on a US
/// keyboard layout: uinput can only press keys, so only these are typed.
pub fn us_key(c: char) -> Option<(u16, bool)> {
    const ROW: &[(&str, u16)] = &[
        ("1234567890", 2),
        ("qwertyuiop", 16),
        ("asdfghjkl", 30),
        ("zxcvbnm", 44),
    ];
    const SHIFTED: &str = "!@#$%^&*()";
    const PUNCT: &[(char, char, u16)] = &[
        ('-', '_', 12),
        ('=', '+', 13),
        ('[', '{', 26),
        (']', '}', 27),
        (';', ':', 39),
        ('\'', '"', 40),
        ('`', '~', 41),
        ('\\', '|', 43),
        (',', '<', 51),
        ('.', '>', 52),
        ('/', '?', 53),
    ];
    match c {
        ' ' => return Some((code::SPACE, false)),
        '\n' | '\r' => return Some((code::ENTER, false)),
        '\t' => return Some((code::TAB, false)),
        _ => {}
    }
    let lower = c.to_ascii_lowercase();
    for (keys, first) in ROW {
        if let Some(i) = keys.find(lower) {
            let shift = c.is_ascii_uppercase();
            return Some((first + i as u16, shift));
        }
    }
    if let Some(i) = SHIFTED.find(c) {
        return Some((2 + i as u16, true));
    }
    PUNCT.iter().find_map(|(plain, shifted, k)| {
        if c == *plain {
            Some((*k, false))
        } else if c == *shifted {
            Some((*k, true))
        } else {
            None
        }
    })
}

/// A shortcut as the GlobalShortcuts portal's `preferred_trigger`
/// ("CTRL+SHIFT+y": modifiers, then the key's keysym name), from the
/// app's own key name ("Y", "F5", "Comma", "ArrowUp"...). None: a key the
/// portal can't be told.
pub fn trigger(ctrl: bool, alt: bool, shift: bool, logo: bool, key: &str) -> Option<String> {
    const NAMED: &[(&str, &str)] = &[
        ("Space", "space"),
        ("Enter", "Return"),
        ("Tab", "Tab"),
        ("Escape", "Escape"),
        ("Backspace", "BackSpace"),
        ("Delete", "Delete"),
        ("Home", "Home"),
        ("End", "End"),
        ("PageUp", "Page_Up"),
        ("PageDown", "Page_Down"),
        ("Insert", "Insert"),
        ("ArrowUp", "Up"),
        ("ArrowDown", "Down"),
        ("ArrowLeft", "Left"),
        ("ArrowRight", "Right"),
        ("Comma", "comma"),
        ("Period", "period"),
        ("Slash", "slash"),
        ("Backslash", "backslash"),
        ("Semicolon", "semicolon"),
        ("Quote", "apostrophe"),
        ("BracketLeft", "bracketleft"),
        ("BracketRight", "bracketright"),
        ("Minus", "minus"),
        ("Equal", "equal"),
        ("Backquote", "grave"),
    ];
    let mut chars = key.chars();
    let name = match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
        _ if key.len() > 1
            && key.starts_with('F')
            && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) =>
        {
            key.to_string()
        }
        _ => NAMED.iter().find(|(k, _)| *k == key)?.1.to_string(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for (on, m) in [
        (ctrl, "CTRL"),
        (alt, "ALT"),
        (shift, "SHIFT"),
        (logo, "LOGO"),
    ] {
        if on {
            parts.push(m);
        }
    }
    parts.push(&name);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_both_ways() {
        assert_eq!(button_code(1), Some(0x110));
        assert_eq!(button_code(2), Some(0x111));
        assert_eq!(button_code(9), None);
        assert_eq!(button_number(0x112), Some(3));
        assert_eq!(button_number(0x114), Some(5));
        assert_eq!(button_number(0x120), None);
    }

    #[test]
    fn text_becomes_keysyms() {
        assert_eq!(keysym('a'), 0x61);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('\n'), 0xff0d);
        assert_eq!(keysym('क'), 0x0100_0915);
        assert_eq!(keysym('€'), 0x0100_20ac);
    }

    #[test]
    fn us_keys_for_uinput_typing() {
        assert_eq!(us_key('a'), Some((30, false)));
        assert_eq!(us_key('A'), Some((30, true)));
        assert_eq!(us_key('1'), Some((2, false)));
        assert_eq!(us_key('0'), Some((11, false)));
        assert_eq!(us_key('!'), Some((2, true)));
        assert_eq!(us_key(')'), Some((11, true)));
        assert_eq!(us_key('?'), Some((53, true)));
        assert_eq!(us_key('m'), Some((50, false)));
        assert_eq!(us_key(' '), Some((code::SPACE, false)));
        assert_eq!(us_key('é'), None);
    }

    #[test]
    fn shortcuts_become_portal_triggers() {
        assert_eq!(
            trigger(true, false, true, false, "Y").as_deref(),
            Some("CTRL+SHIFT+y")
        );
        assert_eq!(
            trigger(false, true, false, true, "F12").as_deref(),
            Some("ALT+LOGO+F12")
        );
        assert_eq!(
            trigger(true, false, false, false, "ArrowUp").as_deref(),
            Some("CTRL+Up")
        );
        assert_eq!(
            trigger(true, false, false, false, "Comma").as_deref(),
            Some("CTRL+comma")
        );
        assert_eq!(
            trigger(true, false, false, false, "5").as_deref(),
            Some("CTRL+5")
        );
        assert_eq!(trigger(true, false, false, false, "Fn"), None);
        assert_eq!(trigger(true, false, false, false, "F30"), None);
    }
}
