//! Each device's wallpaper look, for your other devices to draw behind its
//! icon (the big device tile), the way a phone shows its lock screen.
//!
//! The host app hands the core its look (`set_own`): two or three dominant
//! colours (`#rrggbb`) and, when it could read it without asking for
//! anything, a small picture (JPEG, WebP or PNG, at most `MAX_IMAGE`
//! bytes, about 320 px on the long side). `None` means "don't share it"
//! (the setting is off, or there's nothing to share).
//!
//! It rides the status (`features::device`) only as a short hash, so the
//! look itself goes out only when it changes:
//!
//!   status          { ..., wallpaper: "<hash>" | "" }       device → asker
//!   wallpaper_get   { from, h }                             asker → device
//!   wallpaper       { from, h, colors[], image? }           device → asker
//!   wallpaper       { from, h: "" }                         "forget mine"
//!
//! - A device whose status hash differs from the one kept here is asked
//!   for its look (`wallpaper_get`, at most once a minute per hash), over
//!   the live link: the picture is too big for a push message.
//! - `""` means it shares nothing: what was kept for it is dropped.
//! - A device that changes its look tells the devices with a live link at
//!   once (`wallpaper`); turning sharing off tells every device it heard
//!   from, the reliable way, to forget it.
//! - Old apps never send the field and never get these messages: only a
//!   device that said `wallpaper` in its status is sent them (an older one
//!   would show the unknown message as chat). A status without the field
//!   leaves what's kept alone.
//!
//! The hash is worked out from the content on both sides (`hash_of`), so a
//! look that doesn't match its hash is dropped. What each device shared is
//! kept across restarts (`kv`, key `wallpapers`), and a change makes a
//! `CoreEvent::Wallpaper`. This device's own look is kept too
//! (`wallpaper_own`), so its status says the same hash right after a
//! restart instead of "nothing" until the host reads it again.
//!
//! `dominant_colors` finds the colours from pixels, for a host that has a
//! picture but no colour API (the desktop).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::Wallpaper;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status field carrying the hash (`""`: nothing shared).
pub const FLAG: &str = "wallpaper";
/// Largest picture, in bytes (before base64).
pub const MAX_IMAGE: usize = 48 * 1024;
/// Most colours kept.
pub const MAX_COLORS: usize = 4;
/// How long before the same hash is asked for again.
const ASK_EVERY: Duration = Duration::from_secs(60);
const MAP_KEY: &str = "wallpapers";
const OWN_KEY: &str = "wallpaper_own";
/// The most devices remembered.
const MAX_DEVICES: usize = 64;

// ---------------------------------------------------------------------------
// Pure parts
// ---------------------------------------------------------------------------

/// `#rrggbb` (any case, `#` optional) as lowercase `#rrggbb`.
pub fn tidy_color(c: &str) -> Option<String> {
    let h = c.trim().trim_start_matches('#');
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("#{}", h.to_ascii_lowercase()))
}

/// The picture's type from its first bytes: JPEG, WebP or PNG only.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else {
        None
    }
}

/// The short hash of a look: its colours and picture bytes.
pub fn hash_of(colors: &[String], image: Option<&[u8]>) -> String {
    let mut h = Sha256::new();
    for c in colors {
        h.update(c.as_bytes());
        h.update(b",");
    }
    h.update(b"|");
    if let Some(img) = image {
        h.update(img);
    }
    h.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn is_hash(h: &str) -> bool {
    h.len() == 16 && h.chars().all(|c| c.is_ascii_hexdigit())
}

/// A look as given or as it arrived: colours tidied (bad ones left out),
/// the picture only if it's a known type and small enough. `None` when
/// nothing is left.
pub fn build(colors: &[String], image: Option<&[u8]>) -> Option<Wallpaper> {
    let colors: Vec<String> = colors
        .iter()
        .filter_map(|c| tidy_color(c))
        .take(MAX_COLORS)
        .collect();
    let image = image.filter(|b| !b.is_empty() && b.len() <= MAX_IMAGE);
    let mime = image.and_then(image_mime);
    let image = mime.and(image);
    if colors.is_empty() && image.is_none() {
        return None;
    }
    Some(Wallpaper {
        hash: hash_of(&colors, image),
        colors,
        image: image.zip(mime).map(|(b, m)| {
            format!(
                "data:{m};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(b)
            )
        }),
    })
}

/// The picture's bytes back from its `data:` URL.
fn image_bytes(w: &Wallpaper) -> Option<Vec<u8>> {
    let b64 = w.image.as_deref()?.split_once(";base64,")?.1;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

/// A `wallpaper` message's look, checked against its hash.
pub fn parse(msg: &Value) -> Option<Wallpaper> {
    let h = msg["h"].as_str()?;
    let colors: Vec<String> = msg["colors"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let image = match msg["image"].as_str() {
        Some(s) if s.len() <= MAX_IMAGE * 4 / 3 + 8 => {
            Some(base64::engine::general_purpose::STANDARD.decode(s).ok()?)
        }
        Some(_) => return None,
        None => None,
    };
    let w = build(&colors, image.as_deref())?;
    (w.hash == h).then_some(w)
}

/// The `wallpaper` message for `w` (`None`: "forget mine").
fn message(me: &str, w: Option<&Wallpaper>) -> String {
    let mut m = json!({ "type": "wallpaper", "from": me, "h": "" });
    if let Some(w) = w {
        m["h"] = w.hash.clone().into();
        m["colors"] = w.colors.clone().into();
        if let Some(img) = image_bytes(w) {
            m["image"] = base64::engine::general_purpose::STANDARD.encode(img).into();
        }
    }
    m.to_string()
}

/// Up to `n` colours that stand out in a picture, most of it first, as
/// `#rrggbb`. `rgb` is packed 8-bit RGB pixels (any order, any size; it's
/// sampled). Colours too close to one already picked are skipped, so a
/// mostly blue sky gives blue and then whatever else is there.
pub fn dominant_colors(rgb: &[u8], n: usize) -> Vec<String> {
    let pixels = rgb.len() / 3;
    if pixels == 0 || n == 0 {
        return Vec::new();
    }
    // About 4096 samples whatever the size.
    let step = (pixels / 4096).max(1);
    // 4 bits a channel: 4096 boxes, each with its pixels' sum.
    let mut boxes: HashMap<u16, (u32, [u64; 3])> = HashMap::new();
    let mut i = 0;
    while i < pixels {
        let p = &rgb[i * 3..i * 3 + 3];
        let key = ((p[0] as u16 >> 4) << 8) | ((p[1] as u16 >> 4) << 4) | (p[2] as u16 >> 4);
        let e = boxes.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        for (sum, v) in e.1.iter_mut().zip(p) {
            *sum += *v as u64;
        }
        i += step;
    }
    let mut ranked: Vec<(u32, [u8; 3])> = boxes
        .into_values()
        .map(|(count, sum)| {
            let avg = sum.map(|s| (s / count as u64) as u8);
            (count, avg)
        })
        .collect();
    // Most first; ties by colour so the answer doesn't depend on the map.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let far = |a: [u8; 3], b: [u8; 3]| {
        let d: i32 = (0..3).map(|c| (a[c] as i32 - b[c] as i32).pow(2)).sum();
        d > 56 * 56
    };
    let mut picked: Vec<[u8; 3]> = Vec::new();
    for (_, c) in &ranked {
        if picked.iter().all(|p| far(*p, *c)) {
            picked.push(*c);
            if picked.len() == n {
                break;
            }
        }
    }
    picked
        .into_iter()
        .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
        .collect()
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Wallpapers {
    /// This device's look: outer `None` until read from `kv`, inner
    /// `None` when it shares nothing.
    own: Mutex<Option<Option<Wallpaper>>>,
    /// node -> what it shared; `None` until read from `kv`.
    peers: Mutex<Option<BTreeMap<String, Wallpaper>>>,
    /// Devices that said `wallpaper` in their status (they know the
    /// messages).
    knows: Mutex<BTreeSet<String>>,
    /// node -> the hash last asked for, and when.
    asked: Mutex<HashMap<String, (String, Instant)>>,
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

fn with_peers<P: Platform, R>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut BTreeMap<String, Wallpaper>) -> R,
) -> R {
    let mut guard = ctx.extras.wallpaper.peers.lock();
    let map = guard.get_or_insert_with(|| {
        ctx.platform
            .kv_get(MAP_KEY)
            .and_then(|s| serde_json::from_str::<BTreeMap<String, Wallpaper>>(&s).ok())
            .unwrap_or_default()
    });
    f(map)
}

fn save_peers<P: Platform>(ctx: &NetCtx<P>, map: &BTreeMap<String, Wallpaper>) {
    ctx.platform
        .kv_set(MAP_KEY, &serde_json::to_string(map).unwrap_or_default());
}

/// This device's own look (`None`: it shares nothing).
pub fn own<P: Platform>(ctx: &NetCtx<P>) -> Option<Wallpaper> {
    let mut guard = ctx.extras.wallpaper.own.lock();
    guard
        .get_or_insert_with(|| {
            ctx.platform
                .kv_get(OWN_KEY)
                .and_then(|s| serde_json::from_str::<Option<Wallpaper>>(&s).ok())
                .flatten()
        })
        .clone()
}

/// The host's look for this device (`None`: share nothing, and tell the
/// others to forget it). Only a change is said: the devices with a live
/// link hear it at once; the others see the new hash in the next status.
pub fn set_own<P: Platform>(ctx: &NetCtx<P>, w: Option<Wallpaper>) {
    let before = own(ctx);
    if before.as_ref().map(|w| &w.hash) == w.as_ref().map(|w| &w.hash) {
        return;
    }
    *ctx.extras.wallpaper.own.lock() = Some(w.clone());
    ctx.platform
        .kv_set(OWN_KEY, &serde_json::to_string(&w).unwrap_or_default());
    let msg = message(&me(ctx), w.as_ref());
    let knows: Vec<String> = ctx.extras.wallpaper.knows.lock().iter().cloned().collect();
    for node in knows {
        if !super::known(ctx, &node) {
            continue;
        }
        if w.is_none() {
            // Small: the reliable way, so a device asleep forgets it too.
            crate::engine::send_reliably(ctx, &node, &msg);
        } else if ctx.ws_conns.lock().contains_key(&node) {
            crate::transport::send_control_to(ctx, &node, &msg);
        }
    }
}

/// Add this device's hash to a status reply (`""`: nothing shared).
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    status[FLAG] = own(ctx).map(|w| w.hash).unwrap_or_default().into();
}

/// `node`'s status arrived; `field` is its `wallpaper` field.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, field: &Value) {
    let Some(h) = field.as_str() else {
        // An older app, or one that hasn't said: leave what's kept.
        return;
    };
    if !super::known(ctx, node) {
        return;
    }
    ctx.extras.wallpaper.knows.lock().insert(node.to_string());
    if h.is_empty() {
        forget(ctx, node);
        return;
    }
    if !is_hash(h) || with_peers(ctx, |m| m.get(node).map(|w| w.hash == h)) == Some(true) {
        return;
    }
    {
        let mut asked = ctx.extras.wallpaper.asked.lock();
        if let Some((last, at)) = asked.get(node) {
            if last == h && at.elapsed() < ASK_EVERY {
                return;
            }
        }
        asked.insert(node.to_string(), (h.to_string(), Instant::now()));
    }
    let m = json!({ "type": "wallpaper_get", "from": me(ctx), "h": h }).to_string();
    crate::transport::send_control_to(ctx, node, &m);
}

/// Drop what `node` shared (it shares nothing now).
fn forget<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let gone = with_peers(ctx, |m| {
        let gone = m.remove(node).is_some();
        if gone {
            save_peers(ctx, m);
        }
        gone
    });
    if gone {
        ctx.sink.emit(CoreEvent::Wallpaper {
            node: node.to_string(),
            wallpaper: None,
        });
    }
}

/// Keep what `node` shared.
fn learn<P: Platform>(ctx: &NetCtx<P>, node: &str, w: Wallpaper) {
    let changed = with_peers(ctx, |m| {
        if m.get(node).map(|o| &o.hash) == Some(&w.hash) {
            return false;
        }
        m.insert(node.to_string(), w.clone());
        while m.len() > MAX_DEVICES {
            let Some(first) = m.keys().find(|k| k.as_str() != node).cloned() else {
                break;
            };
            m.remove(&first);
        }
        save_peers(ctx, m);
        true
    });
    if changed {
        ctx.sink.emit(CoreEvent::Wallpaper {
            node: node.to_string(),
            wallpaper: Some(w),
        });
    }
}

/// What `node` shared, if anything.
pub fn of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<Wallpaper> {
    with_peers(ctx, |m| m.get(node).cloned())
}

/// What each of your devices shared, by node id (devices still in the
/// list only).
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> BTreeMap<String, Wallpaper> {
    let mut map = with_peers(ctx, |m| m.clone());
    map.retain(|node, _| super::known(ctx, node));
    map
}

pub struct WallpaperHandler;

impl<P: Platform> ControlHandler<P> for WallpaperHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["wallpaper_get", "wallpaper"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if !super::known(ctx, &from) {
            return;
        }
        // It knows these messages.
        ctx.extras.wallpaper.knows.lock().insert(from.clone());
        match msg.kind() {
            "wallpaper_get" => {
                let m = message(&me(ctx), own(ctx).as_ref());
                crate::transport::send_control_to(ctx, &from, &m);
            }
            "wallpaper" => {
                let v = msg.json("h");
                match v.as_str() {
                    Some("") => forget(ctx, &from),
                    Some(_) => {
                        let body = json!({
                            "h": v,
                            "colors": msg.json("colors"),
                            "image": msg.json("image"),
                        });
                        if let Some(w) = parse(&body) {
                            learn(ctx, &from, w);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4];

    #[test]
    fn colours_are_tidied() {
        assert_eq!(tidy_color("#A1B2C3").as_deref(), Some("#a1b2c3"));
        assert_eq!(tidy_color("a1b2c3").as_deref(), Some("#a1b2c3"));
        for bad in ["", "#abc", "#gggggg", "#a1b2c3d4", "red"] {
            assert_eq!(tidy_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_small_known_pictures_are_kept() {
        assert_eq!(image_mime(JPEG), Some("image/jpeg"));
        assert_eq!(image_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_mime(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(image_mime(b"<svg>"), None);
        let w = build(&["#112233".into()], Some(b"<svg onload=x>")).unwrap();
        assert_eq!(w.image, None);
        let big = [JPEG, &vec![0u8; MAX_IMAGE]].concat();
        assert_eq!(build(&["#112233".into()], Some(&big)).unwrap().image, None);
        assert_eq!(build(&[], Some(b"<svg>")), None);
        assert_eq!(build(&["nope".into()], None), None);
    }

    #[test]
    fn a_look_round_trips_and_its_hash_is_checked() {
        let w = build(&["#0A0B0C".into(), "#ffffff".into()], Some(JPEG)).unwrap();
        assert_eq!(w.colors, vec!["#0a0b0c", "#ffffff"]);
        assert!(w
            .image
            .as_deref()
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert!(is_hash(&w.hash));
        let m: Value = serde_json::from_str(&message("me", Some(&w))).unwrap();
        assert_eq!(m["type"], "wallpaper");
        let back = parse(&m).unwrap();
        assert_eq!(back, w);
        let mut tampered = m.clone();
        tampered["colors"] = json!(["#000000"]);
        assert_eq!(parse(&tampered), None);
        // Same content, same hash; any change, another.
        assert_eq!(hash_of(&w.colors, Some(JPEG)), w.hash);
        assert_ne!(hash_of(&w.colors, None), w.hash);
        let off: Value = serde_json::from_str(&message("me", None)).unwrap();
        assert_eq!(off["h"], "");
    }

    #[test]
    fn dominant_colours_come_most_first_and_apart() {
        let mut px = Vec::new();
        for _ in 0..700 {
            px.extend_from_slice(&[20, 40, 200]); // blue
        }
        for _ in 0..250 {
            px.extend_from_slice(&[230, 120, 30]); // orange
        }
        for _ in 0..50 {
            px.extend_from_slice(&[36, 56, 186]); // near the blue
        }
        let c = dominant_colors(&px, 3);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0], "#1428c8");
        assert_eq!(c[1], "#e6781e");
        assert!(dominant_colors(&[], 3).is_empty());
    }
}
