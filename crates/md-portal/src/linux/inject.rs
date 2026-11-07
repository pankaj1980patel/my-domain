//! Input played on this computer on Wayland: through the RemoteDesktop
//! portal, else `/dev/uinput`. The pointer's position isn't something an
//! app may read on Wayland, so it's followed here: from where it was last
//! put (an absolute move through the portal's screens) by every move since.

use super::remote::RemoteDesktop;
use crate::caps::InputWay;
use crate::geom::{self, Rect};
use crate::keys::{self, code};
use crate::uinput;

enum Backend {
    Portal(RemoteDesktop),
    Uinput(uinput::Device),
}

pub struct Injector {
    backend: Backend,
    /// The screens (portal streams): node and place.
    screens: Vec<(u32, Rect)>,
    /// Where the pointer is, as far as moves here know.
    at: Option<(f64, f64)>,
}

impl Injector {
    /// Open `way` (the portal asks the user once; `restore` is the token
    /// from last time). Also the token to keep.
    pub fn open(
        way: InputWay,
        restore: Option<&str>,
    ) -> Result<(Injector, Option<String>), String> {
        match way {
            InputWay::Portal => {
                let (rd, token) = RemoteDesktop::start(restore, true)?;
                let screens = rd
                    .streams
                    .iter()
                    .filter_map(|s| s.rect.map(|r| (s.node, r)))
                    .collect();
                Ok((
                    Injector {
                        backend: Backend::Portal(rd),
                        screens,
                        at: None,
                    },
                    token,
                ))
            }
            InputWay::Uinput => {
                let dev = uinput::Device::new("my-domain remote input")
                    .map_err(|e| format!("couldn\u{2019}t use /dev/uinput: {e}"))?;
                Ok((
                    Injector {
                        backend: Backend::Uinput(dev),
                        screens: Vec::new(),
                        at: None,
                    },
                    None,
                ))
            }
        }
    }

    /// The screens, when known (the portal gave them).
    pub fn screens(&self) -> Vec<Rect> {
        self.screens.iter().map(|(_, r)| *r).collect()
    }

    /// Where the pointer is, when known.
    pub fn cursor(&self) -> Option<(f64, f64)> {
        self.at
    }

    /// Put the pointer at (`x`, `y`): exactly when the portal knows the
    /// screen there, else by a move from where it's thought to be.
    pub fn warp(&mut self, x: f64, y: f64) -> bool {
        let on = self.screens.iter().find(|(_, r)| r.contains(x, y)).copied();
        if let (Backend::Portal(rd), Some((node, r))) = (&self.backend, on) {
            let ok = rd.motion_to(node, x - r.x, y - r.y);
            if ok {
                self.at = Some((x, y));
            }
            return ok;
        }
        let Some((ax, ay)) = self.at else {
            return false;
        };
        let ok = self
            .motion((x - ax).round() as i32, (y - ay).round() as i32)
            .is_ok();
        if ok {
            self.at = Some((x, y));
        }
        ok
    }

    pub fn motion(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let ok = match &mut self.backend {
            Backend::Portal(rd) => rd.motion(f64::from(dx), f64::from(dy)),
            Backend::Uinput(d) => d.motion(dx, dy).is_ok(),
        };
        if let Some(at) = self.at {
            let screens = self.screens();
            self.at = Some(geom::moved(&screens, at, f64::from(dx), f64::from(dy)));
        }
        ok.then_some(()).ok_or_else(|| "move".into())
    }

    /// 1 left, 2 right, 3 middle, 4 back, 5 forward.
    pub fn button(&mut self, button: u8, down: bool) -> Result<(), String> {
        let Some(c) = keys::button_code(button) else {
            return Ok(());
        };
        self.key(c, down)
    }

    /// Wheel clicks; `dy` positive scrolls down.
    pub fn wheel(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let ok = match &mut self.backend {
            Backend::Portal(rd) => (dy == 0 || rd.wheel(0, dy)) && (dx == 0 || rd.wheel(1, dx)),
            Backend::Uinput(d) => {
                (dy == 0 || d.rel(uinput::REL_WHEEL, -dy).is_ok())
                    && (dx == 0 || d.rel(uinput::REL_HWHEEL, dx).is_ok())
            }
        };
        ok.then_some(()).ok_or_else(|| "wheel".into())
    }

    /// An evdev key or button.
    pub fn key(&mut self, c: u16, down: bool) -> Result<(), String> {
        let ok = match &mut self.backend {
            Backend::Portal(rd) if c >= code::BTN_LEFT => rd.button(c, down),
            Backend::Portal(rd) => rd.key(c, down),
            Backend::Uinput(d) => d.key(c, down).is_ok(),
        };
        ok.then_some(()).ok_or_else(|| "key".into())
    }

    /// Hold `mods`, press `key`, let go (in reverse), even half way.
    pub fn combo(&mut self, mods: &[u16], key: u16) -> Result<(), String> {
        let mut held = Vec::new();
        let mut result = Ok(());
        for m in mods {
            match self.key(*m, true) {
                Ok(()) => held.push(*m),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        if result.is_ok() {
            result = self.key(key, true).and_then(|()| self.key(key, false));
        }
        for m in held.iter().rev() {
            let _ = self.key(*m, false);
        }
        result
    }

    /// Type `text`: any character through the portal; uinput can only
    /// press keys, so there what a US keyboard types.
    pub fn text(&mut self, text: &str) -> Result<(), String> {
        for c in text.chars() {
            let ok = if let Backend::Portal(rd) = &self.backend {
                let sym = keys::keysym(c);
                rd.keysym(sym, true) && rd.keysym(sym, false)
            } else {
                match keys::us_key(c) {
                    Some((k, true)) => self.combo(&[code::LEFTSHIFT], k).is_ok(),
                    Some((k, false)) => self.combo(&[], k).is_ok(),
                    None => true,
                }
            };
            if !ok {
                return Err("text".into());
            }
        }
        Ok(())
    }
}
