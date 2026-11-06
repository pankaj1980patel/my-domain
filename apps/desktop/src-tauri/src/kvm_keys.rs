// Keys for "Share mouse and keyboard" (`kvm.rs`): every key goes between
// computers as its USB HID usage code (keyboard page 0x07), the same on
// every OS, and each end turns it into its own:
//  - macOS: the virtual key code (`kVK_*`, a CGKeyCode);
//  - Windows: the virtual-key code (`VK_*`); the low-level hook's
//    "extended" flag tells the keypad's Enter from the main one;
//  - Linux: the evdev code (X11 key codes are these plus 8).
// A key with no code on one side doesn't go (media keys, Fn).
//
// Cmd ↔ Ctrl: between a Mac and a Windows or Linux computer, the keys you
// press for the same shortcut differ (Cmd+C, Ctrl+C), so by default the
// controlling end swaps the Command (GUI) and Control keys when the two
// computers' systems differ ("Swap Cmd and Ctrl" in Settings).

// Each OS uses its own column (and the tests all of them): the others are
// dead code there.
#![allow(dead_code)]

/// No code on that OS.
const NO: u16 = u16::MAX;

/// (HID usage, macOS kVK, Windows VK, Linux evdev).
const KEYS: &[(u16, u16, u16, u16)] = &[
    // Letters a..z
    (0x04, 0x00, 0x41, 30),
    (0x05, 0x0B, 0x42, 48),
    (0x06, 0x08, 0x43, 46),
    (0x07, 0x02, 0x44, 32),
    (0x08, 0x0E, 0x45, 18),
    (0x09, 0x03, 0x46, 33),
    (0x0A, 0x05, 0x47, 34),
    (0x0B, 0x04, 0x48, 35),
    (0x0C, 0x22, 0x49, 23),
    (0x0D, 0x26, 0x4A, 36),
    (0x0E, 0x28, 0x4B, 37),
    (0x0F, 0x25, 0x4C, 38),
    (0x10, 0x2E, 0x4D, 50),
    (0x11, 0x2D, 0x4E, 49),
    (0x12, 0x1F, 0x4F, 24),
    (0x13, 0x23, 0x50, 25),
    (0x14, 0x0C, 0x51, 16),
    (0x15, 0x0F, 0x52, 19),
    (0x16, 0x01, 0x53, 31),
    (0x17, 0x11, 0x54, 20),
    (0x18, 0x20, 0x55, 22),
    (0x19, 0x09, 0x56, 47),
    (0x1A, 0x0D, 0x57, 17),
    (0x1B, 0x07, 0x58, 45),
    (0x1C, 0x10, 0x59, 21),
    (0x1D, 0x06, 0x5A, 44),
    // Digits 1..9, 0
    (0x1E, 0x12, 0x31, 2),
    (0x1F, 0x13, 0x32, 3),
    (0x20, 0x14, 0x33, 4),
    (0x21, 0x15, 0x34, 5),
    (0x22, 0x17, 0x35, 6),
    (0x23, 0x16, 0x36, 7),
    (0x24, 0x1A, 0x37, 8),
    (0x25, 0x1C, 0x38, 9),
    (0x26, 0x19, 0x39, 10),
    (0x27, 0x1D, 0x30, 11),
    // Enter, Esc, Backspace, Tab, Space
    (0x28, 0x24, 0x0D, 28),
    (0x29, 0x35, 0x1B, 1),
    (0x2A, 0x33, 0x08, 14),
    (0x2B, 0x30, 0x09, 15),
    (0x2C, 0x31, 0x20, 57),
    // - = [ ] \ ; ' ` , . /
    (0x2D, 0x1B, 0xBD, 12),
    (0x2E, 0x18, 0xBB, 13),
    (0x2F, 0x21, 0xDB, 26),
    (0x30, 0x1E, 0xDD, 27),
    (0x31, 0x2A, 0xDC, 43),
    (0x33, 0x29, 0xBA, 39),
    (0x34, 0x27, 0xDE, 40),
    (0x35, 0x32, 0xC0, 41),
    (0x36, 0x2B, 0xBC, 51),
    (0x37, 0x2F, 0xBE, 52),
    (0x38, 0x2C, 0xBF, 53),
    // Caps Lock
    (0x39, 0x39, 0x14, 58),
    // F1..F12
    (0x3A, 0x7A, 0x70, 59),
    (0x3B, 0x78, 0x71, 60),
    (0x3C, 0x63, 0x72, 61),
    (0x3D, 0x76, 0x73, 62),
    (0x3E, 0x60, 0x74, 63),
    (0x3F, 0x61, 0x75, 64),
    (0x40, 0x62, 0x76, 65),
    (0x41, 0x64, 0x77, 66),
    (0x42, 0x65, 0x78, 67),
    (0x43, 0x6D, 0x79, 68),
    (0x44, 0x67, 0x7A, 87),
    (0x45, 0x6F, 0x7B, 88),
    // Print Screen, Scroll Lock, Pause (no Mac keys)
    (0x46, NO, 0x2C, 99),
    (0x47, NO, 0x91, 70),
    (0x48, NO, 0x13, 119),
    // Insert (a Mac's Help), Home, Page Up, Delete, End, Page Down
    (0x49, 0x72, 0x2D, 110),
    (0x4A, 0x73, 0x24, 102),
    (0x4B, 0x74, 0x21, 104),
    (0x4C, 0x75, 0x2E, 111),
    (0x4D, 0x77, 0x23, 107),
    (0x4E, 0x79, 0x22, 109),
    // Arrows: right, left, down, up
    (0x4F, 0x7C, 0x27, 106),
    (0x50, 0x7B, 0x25, 105),
    (0x51, 0x7D, 0x28, 108),
    (0x52, 0x7E, 0x26, 103),
    // Num Lock (a Mac's Clear), keypad / * - + Enter 1..9 0 .
    (0x53, 0x47, 0x90, 69),
    (0x54, 0x4B, 0x6F, 98),
    (0x55, 0x43, 0x6A, 55),
    (0x56, 0x4E, 0x6D, 74),
    (0x57, 0x45, 0x6B, 78),
    (0x58, 0x4C, 0x0D, 96),
    (0x59, 0x53, 0x61, 79),
    (0x5A, 0x54, 0x62, 80),
    (0x5B, 0x55, 0x63, 81),
    (0x5C, 0x56, 0x64, 75),
    (0x5D, 0x57, 0x65, 76),
    (0x5E, 0x58, 0x66, 77),
    (0x5F, 0x59, 0x67, 71),
    (0x60, 0x5B, 0x68, 72),
    (0x61, 0x5C, 0x69, 73),
    (0x62, 0x52, 0x60, 82),
    (0x63, 0x41, 0x6E, 83),
    // The key next to left Shift on ISO keyboards, the menu key, keypad =
    (0x64, 0x0A, 0xE2, 86),
    (0x65, 0x6E, 0x5D, 127),
    (0x67, 0x51, NO, 117),
    // F13..F20
    (0x68, 0x69, 0x7C, 183),
    (0x69, 0x6B, 0x7D, 184),
    (0x6A, 0x71, 0x7E, 185),
    (0x6B, 0x6A, 0x7F, 186),
    (0x6C, 0x40, 0x80, 187),
    (0x6D, 0x4F, 0x81, 188),
    (0x6E, 0x50, 0x82, 189),
    (0x6F, 0x5A, 0x83, 190),
    // Mute, volume up, volume down
    (0x7F, 0x4A, 0xAD, 113),
    (0x80, 0x48, 0xAF, 115),
    (0x81, 0x49, 0xAE, 114),
    // Modifiers: left Ctrl, Shift, Alt (Option), GUI (Cmd, Windows), then right
    (0xE0, 0x3B, 0xA2, 29),
    (0xE1, 0x38, 0xA0, 42),
    (0xE2, 0x3A, 0xA4, 56),
    (0xE3, 0x37, 0x5B, 125),
    (0xE4, 0x3E, 0xA3, 97),
    (0xE5, 0x3C, 0xA1, 54),
    (0xE6, 0x3D, 0xA5, 100),
    (0xE7, 0x36, 0x5C, 126),
];

pub const HID_LCTRL: u16 = 0xE0;
pub const HID_LSHIFT: u16 = 0xE1;
pub const HID_LALT: u16 = 0xE2;
pub const HID_LGUI: u16 = 0xE3;
pub const HID_RCTRL: u16 = 0xE4;
pub const HID_RSHIFT: u16 = 0xE5;
pub const HID_RALT: u16 = 0xE6;
pub const HID_RGUI: u16 = 0xE7;
pub const HID_ESC: u16 = 0x29;
pub const HID_F12: u16 = 0x45;
pub const HID_SCROLL_LOCK: u16 = 0x47;
pub const HID_ENTER: u16 = 0x28;
pub const HID_KP_ENTER: u16 = 0x58;

fn find(f: impl Fn(&(u16, u16, u16, u16)) -> bool) -> Option<&'static (u16, u16, u16, u16)> {
    KEYS.iter().find(|k| f(k))
}

/// A Mac's key code as a HID usage.
pub fn from_mac(code: u16) -> Option<u16> {
    find(|k| k.1 == code && code != NO).map(|k| k.0)
}

/// A Windows virtual key as a HID usage; `extended` (the hook's flag)
/// tells the keypad's Enter.
pub fn from_win(vk: u16, extended: bool) -> Option<u16> {
    if vk == 0x0D && extended {
        return Some(HID_KP_ENTER);
    }
    // The generic Shift/Ctrl/Alt some hooks report: the left one.
    let vk = match vk {
        0x10 => 0xA0,
        0x11 => 0xA2,
        0x12 => 0xA4,
        v => v,
    };
    find(|k| k.2 == vk && vk != NO).map(|k| k.0)
}

/// A Linux evdev code as a HID usage.
pub fn from_evdev(code: u16) -> Option<u16> {
    find(|k| k.3 == code && code != NO).map(|k| k.0)
}

pub fn to_mac(hid: u16) -> Option<u16> {
    find(|k| k.0 == hid).map(|k| k.1).filter(|c| *c != NO)
}

pub fn to_win(hid: u16) -> Option<u16> {
    find(|k| k.0 == hid).map(|k| k.2).filter(|c| *c != NO)
}

pub fn to_evdev(hid: u16) -> Option<u16> {
    find(|k| k.0 == hid).map(|k| k.3).filter(|c| *c != NO)
}

/// The key to send for `hid` pressed here: Command and Control swap when
/// `swap` is on and exactly one of the two computers is a Mac.
pub fn map_for(hid: u16, from_mac: bool, to_mac: bool, swap: bool) -> u16 {
    if !swap || from_mac == to_mac {
        return hid;
    }
    match hid {
        HID_LGUI => HID_LCTRL,
        HID_LCTRL => HID_LGUI,
        HID_RGUI => HID_RCTRL,
        HID_RCTRL => HID_RGUI,
        other => other,
    }
}

/// The hotkey that brings the mouse and keyboard back, whatever the
/// pointer is doing (a remembered setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    CtrlAltEsc,
    CtrlAltF12,
    ScrollLock,
}

impl Hotkey {
    pub const ALL: [Hotkey; 3] = [Hotkey::CtrlAltEsc, Hotkey::CtrlAltF12, Hotkey::ScrollLock];

    pub fn id(self) -> &'static str {
        match self {
            Hotkey::CtrlAltEsc => "ctrl+alt+esc",
            Hotkey::CtrlAltF12 => "ctrl+alt+f12",
            Hotkey::ScrollLock => "scroll_lock",
        }
    }

    pub fn parse(s: &str) -> Option<Hotkey> {
        Hotkey::ALL.into_iter().find(|h| h.id() == s)
    }

    fn key(self) -> u16 {
        match self {
            Hotkey::CtrlAltEsc => HID_ESC,
            Hotkey::CtrlAltF12 => HID_F12,
            Hotkey::ScrollLock => HID_SCROLL_LOCK,
        }
    }

    fn needs_ctrl_alt(self) -> bool {
        !matches!(self, Hotkey::ScrollLock)
    }
}

/// The keys held down on the controlling computer: repeats of a held key
/// don't go (the other computer repeats it itself), and the hotkey is
/// caught here, never sent.
#[derive(Debug, Default)]
pub struct Keys {
    held: Vec<u16>,
}

/// What to do with a key from the capture.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Send it.
    Send,
    /// A repeat of a held key, or a release of one never pressed: drop it.
    Skip,
    /// The hotkey: bring the mouse and keyboard back.
    Hotkey,
}

impl Keys {
    pub fn key(&mut self, hid: u16, down: bool, hotkey: Hotkey) -> KeyAction {
        if down {
            if self.held.contains(&hid) {
                return KeyAction::Skip;
            }
            let ctrl = self.held.iter().any(|k| *k == HID_LCTRL || *k == HID_RCTRL);
            let alt = self.held.iter().any(|k| *k == HID_LALT || *k == HID_RALT);
            if hid == hotkey.key() && (!hotkey.needs_ctrl_alt() || (ctrl && alt)) {
                return KeyAction::Hotkey;
            }
            self.held.push(hid);
            KeyAction::Send
        } else if let Some(i) = self.held.iter().position(|k| *k == hid) {
            self.held.remove(i);
            KeyAction::Send
        } else {
            KeyAction::Skip
        }
    }

    pub fn clear(&mut self) {
        self.held.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_round_trips_on_every_os() {
        let mut seen = std::collections::HashSet::new();
        for &(hid, mac, win, evdev) in KEYS {
            assert!(seen.insert(hid), "{hid:#x} twice");
            if mac != NO {
                assert_eq!(from_mac(mac), Some(hid), "mac {mac:#x}");
                assert_eq!(to_mac(hid), Some(mac));
            }
            // Keypad Enter shares Enter's virtual key on Windows.
            if win != NO && hid != HID_KP_ENTER {
                assert_eq!(from_win(win, false), Some(hid), "win {win:#x}");
            }
            if evdev != NO {
                assert_eq!(from_evdev(evdev), Some(hid), "evdev {evdev}");
                assert_eq!(to_evdev(hid), Some(evdev));
            }
        }
        // A few well-known ones.
        assert_eq!(from_mac(0x08), Some(0x06)); // c
        assert_eq!(to_win(0x06), Some(0x43)); // C
        assert_eq!(to_evdev(0x06), Some(46)); // KEY_C
        assert_eq!(from_win(0x0D, true), Some(HID_KP_ENTER));
        assert_eq!(from_win(0x0D, false), Some(HID_ENTER));
        assert_eq!(to_win(HID_KP_ENTER), Some(0x0D));
        assert_eq!(from_win(0x11, false), Some(HID_LCTRL));
        assert_eq!(to_mac(HID_SCROLL_LOCK), None);
        assert_eq!(from_mac(NO), None);
    }

    #[test]
    fn cmd_and_ctrl_swap_only_between_a_mac_and_another_system() {
        // Mac to Windows: Cmd+C arrives as Ctrl+C.
        assert_eq!(map_for(HID_LGUI, true, false, true), HID_LCTRL);
        assert_eq!(map_for(HID_RGUI, true, false, true), HID_RCTRL);
        // Windows to Mac: Ctrl+C arrives as Cmd+C.
        assert_eq!(map_for(HID_LCTRL, false, true, true), HID_LGUI);
        // Same system, or the setting off: as pressed.
        assert_eq!(map_for(HID_LGUI, true, true, true), HID_LGUI);
        assert_eq!(map_for(HID_LCTRL, false, false, true), HID_LCTRL);
        assert_eq!(map_for(HID_LGUI, true, false, false), HID_LGUI);
        // Other keys never change.
        assert_eq!(map_for(0x06, true, false, true), 0x06);
        assert_eq!(map_for(HID_LALT, true, false, true), HID_LALT);
    }

    #[test]
    fn the_hotkey_is_caught_and_repeats_are_dropped() {
        let hk = Hotkey::CtrlAltEsc;
        let mut k = Keys::default();
        // Esc alone goes.
        assert_eq!(k.key(HID_ESC, true, hk), KeyAction::Send);
        assert_eq!(k.key(HID_ESC, true, hk), KeyAction::Skip, "a repeat");
        assert_eq!(k.key(HID_ESC, false, hk), KeyAction::Send);
        assert_eq!(k.key(HID_ESC, false, hk), KeyAction::Skip, "never pressed");
        // Ctrl+Alt+Esc (either Ctrl or Alt) is the hotkey, and isn't sent.
        assert_eq!(k.key(HID_RCTRL, true, hk), KeyAction::Send);
        assert_eq!(k.key(HID_LALT, true, hk), KeyAction::Send);
        assert_eq!(k.key(HID_ESC, true, hk), KeyAction::Hotkey);
        // Another choice.
        let mut k = Keys::default();
        assert_eq!(
            k.key(HID_SCROLL_LOCK, true, Hotkey::ScrollLock),
            KeyAction::Hotkey
        );
        assert_eq!(k.key(HID_ESC, true, Hotkey::ScrollLock), KeyAction::Send);
        k.clear();
        assert_eq!(k.key(HID_ESC, false, Hotkey::ScrollLock), KeyAction::Skip);
        for h in Hotkey::ALL {
            assert_eq!(Hotkey::parse(h.id()), Some(h));
        }
        assert_eq!(Hotkey::parse("ctrl+c"), None);
    }
}
