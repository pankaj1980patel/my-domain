//! A phone's recent photos and videos, for a computer to show as a grid and
//! fetch from. The core carries the messages; the host app (Android:
//! MediaStore) does the listing and makes the thumbnails.
//!
//!   gallery_list    { from, req, offset, limit, thumbs }   asker → phone
//!   gallery_listing { from, req, items[], next?, error? }  phone → asker
//!   gallery_thumb   { from, req, id, data }                phone → asker
//!   gallery_get     { from, id, item }                     asker → phone
//!
//! On the phone, `gallery_list` becomes `CoreEvent::GalleryRequest` for the
//! host, which answers with `Engine::gallery_reply` (items newest first,
//! each with its path and, if asked for, a small JPEG). A device whose host
//! never said it has a gallery (`Engine::set_gallery_enabled`) answers
//! "no photo gallery" itself, so the asker never waits for nothing.
//!
//! Thumbnails go one per message after the listing (a page of them would
//! be too big for one frame on some links), each as `GalleryThumb`.
//!
//! Paths never leave the phone: the core remembers which item id is which
//! file for items it listed, and `gallery_get` fetches only those (one the
//! phone hasn't listed since it started is refused: list again). The file
//! then comes as a normal resumable transfer under the asker's id, exactly
//! like `features::browse` downloads (failures as `fs_get_failed`).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::json;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::{GalleryEntry, GalleryItem};
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// Most items in one page.
pub const MAX_PAGE: u32 = 100;
/// A page when the asker didn't say.
pub const DEFAULT_PAGE: u32 = 60;
/// Largest thumbnail sent (bytes of JPEG); bigger ones are left out.
pub const MAX_THUMB: usize = 48 * 1024;
/// How many listed items the phone remembers the file of.
const REMEMBERED: usize = 5000;

/// The phone's side: whether the host lists a gallery, and which file each
/// listed item is.
#[derive(Default)]
pub struct Gallery {
    pub enabled: AtomicBool,
    paths: Mutex<Paths>,
}

#[derive(Default)]
struct Paths {
    by_id: HashMap<String, PathBuf>,
    order: VecDeque<String>,
}

impl Gallery {
    fn remember(&self, id: &str, path: &str) {
        let mut p = self.paths.lock();
        if p.by_id.insert(id.to_string(), path.into()).is_none() {
            p.order.push_back(id.to_string());
        }
        while p.order.len() > REMEMBERED {
            if let Some(old) = p.order.pop_front() {
                p.by_id.remove(&old);
            }
        }
    }

    fn path_of(&self, id: &str) -> Option<PathBuf> {
        self.paths.lock().by_id.get(id).cloned()
    }
}

/// Item ids are the host's (MediaStore row ids): letters, digits, `_`, `-`.
pub fn is_item_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Answer request `req` from `to`: the page, then each thumbnail.
pub fn reply<P: Platform>(
    ctx: &NetCtx<P>,
    to: &str,
    req: &str,
    items: Vec<GalleryItem>,
    next: Option<u64>,
    error: Option<&str>,
) {
    let mut entries: Vec<GalleryEntry> = Vec::with_capacity(items.len());
    let mut thumbs = Vec::new();
    for item in items.into_iter().take(MAX_PAGE as usize) {
        if !is_item_id(&item.entry.id) {
            continue;
        }
        ctx.extras.gallery.remember(&item.entry.id, &item.path);
        if let Some(t) = item.thumb.filter(|t| !t.is_empty() && t.len() <= MAX_THUMB) {
            thumbs.push((item.entry.id.clone(), t));
        }
        let mut e = item.entry;
        e.name = super::files::safe_name(&e.name);
        entries.push(e);
    }
    let mut msg = json!({
        "type": "gallery_listing", "from": me(ctx), "req": req, "items": entries,
    });
    if let Some(n) = next {
        msg["next"] = n.into();
    }
    if let Some(e) = error {
        msg["error"] = e.into();
    }
    crate::engine::send_reliably(ctx, to, &msg.to_string());
    // Over the live link the request came on, not the outbox (a page of
    // them would push other messages out of it). A thumbnail lost to a
    // link that just died is only a grey square; asking again fixes it.
    let name = ctx.identity.lock().name.clone();
    for (id, data) in thumbs {
        let m = json!({
            "type": "gallery_thumb", "from": me(ctx), "req": req, "id": id,
            "data": base64::engine::general_purpose::STANDARD.encode(&data),
        });
        let sent = crate::transport::send_payload(
            &ctx.ws_conns,
            &ctx.p2p_conns,
            &ctx.key,
            &name,
            to,
            &m.to_string(),
        );
        if sent.is_err() {
            break;
        }
    }
}

/// Answer `gallery_get`: send the listed item's file under the asker's id.
fn on_get<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    if !super::files::is_transfer_id(&id) {
        return;
    }
    let item = msg.str("item");
    let started = if !ctx.extras.gallery.enabled.load(Ordering::Relaxed) {
        Err("this device has no photo gallery".to_string())
    } else {
        match ctx
            .extras
            .gallery
            .path_of(&item)
            .filter(|_| is_item_id(&item))
        {
            None => Err("not in the gallery any more; list it again".to_string()),
            Some(path) if !path.is_file() => Err("not found".to_string()),
            Some(path) => {
                super::files::start_with_id(ctx, from, &path, &id).map_err(|e| e.to_string())
            }
        }
    };
    if let Err(error) = started {
        let reply = json!({
            "type": "fs_get_failed", "from": me(ctx), "id": id,
            "name": "photo", "error": error,
        });
        crate::engine::send_reliably(ctx, from, &reply.to_string());
    }
}

pub struct GalleryHandler;

impl<P: Platform> ControlHandler<P> for GalleryHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "gallery_list",
            "gallery_listing",
            "gallery_thumb",
            "gallery_get",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if !super::known(ctx, &from) {
            return;
        }
        match msg.kind() {
            "gallery_list" => {
                let req = msg.str("req");
                if !ctx.extras.gallery.enabled.load(Ordering::Relaxed) {
                    return reply(
                        ctx,
                        &from,
                        &req,
                        vec![],
                        None,
                        Some("this device has no photo gallery"),
                    );
                }
                let limit = msg
                    .json("limit")
                    .as_u64()
                    .map_or(DEFAULT_PAGE, |l| (l as u32).clamp(1, MAX_PAGE));
                ctx.sink.emit(CoreEvent::GalleryRequest {
                    from,
                    req,
                    offset: msg.json("offset").as_u64().unwrap_or(0),
                    limit,
                    thumbs: msg.json("thumbs").as_bool().unwrap_or(true),
                });
            }
            "gallery_listing" => {
                let items: Vec<GalleryEntry> =
                    serde_json::from_value(msg.json("items")).unwrap_or_default();
                ctx.sink.emit(CoreEvent::GalleryListing {
                    from,
                    req: msg.str("req"),
                    items,
                    next: msg.json("next").as_u64(),
                    error: msg.opt("error"),
                });
            }
            "gallery_thumb" => {
                let (id, data) = (msg.str("id"), msg.str("data"));
                if !is_item_id(&id) || data.is_empty() || data.len() > MAX_THUMB * 4 / 3 + 4 {
                    return;
                }
                ctx.sink.emit(CoreEvent::GalleryThumb {
                    from,
                    req: msg.str("req"),
                    id,
                    jpeg: data,
                });
            }
            "gallery_get" => on_get(ctx, msg, &from),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ids_are_plain() {
        assert!(is_item_id("1000034567"));
        assert!(is_item_id("img_1-2"));
        for bad in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            assert!(!is_item_id(bad), "{bad}");
        }
    }

    #[test]
    fn only_the_newest_listed_items_are_remembered() {
        let g = Gallery::default();
        for i in 0..REMEMBERED + 10 {
            g.remember(&i.to_string(), &format!("/p/{i}.jpg"));
        }
        assert_eq!(g.path_of("0"), None);
        assert_eq!(g.path_of("9"), None);
        assert_eq!(g.path_of("10"), Some(PathBuf::from("/p/10.jpg")));
        // Listed again: still one entry.
        g.remember("10", "/p/10.jpg");
        assert_eq!(g.paths.lock().order.len(), REMEMBERED);
    }
}
