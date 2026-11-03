//! Browsing another device's shared folders and pulling a file from one.
//! Each device chooses what it shares (`Engine::set_shared_folders`, e.g. the
//! phone's Download/my-domain and Pictures, the computer's Downloads); only
//! those folders and what's under them can be listed or fetched.
//!
//!   fs_list        { from, req, root, path, offset }   asker → device
//!   fs_listing     { from, req, root, path, entries[], next?, error? }
//!                                                     device → asker
//!   fs_get         { from, id, root, path }            asker → device
//!   fs_get_failed  { from, id, name, error }           device → asker
//!
//! An empty `root` lists the shared folders themselves. `path` is relative
//! to the root, folders separated by `/`. Listings come in pages of `PAGE`
//! entries, folders first; `next` is the offset of the following page.
//!
//! A download is a normal file push the other way (`features::files`), so
//! it's resumable too: the asker picks the transfer id, the device starts
//! sending under it, and the asker sees the usual `FileProgress` /
//! `FileReceived` / `FileFailed` for that id.
//!
//! What a device never gives out: anything outside a shared folder (`..`,
//! absolute paths, symbolic links that lead out), hidden files (names
//! starting with `.`, which includes the partial-download folder).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::json;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::RemoteEntry;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// Entries per listing page (keeps a reply well inside a frame).
pub const PAGE: usize = 200;
/// Longest relative path accepted.
const MAX_PATH: usize = 4096;

/// A valid shared-folder name: what other devices see as the root.
pub fn is_folder_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// The parts of a relative path, or `None` if it could leave its folder or
/// names something hidden. `""` and `"/"` are the folder itself.
pub fn path_parts(path: &str) -> Option<Vec<&str>> {
    if path.len() > MAX_PATH || path.contains('\\') || path.chars().any(char::is_control) {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // `.`, `..` and hidden names; on Windows also `C:` (a drive) and
    // `name:stream` (an alternate data stream).
    if parts
        .iter()
        .any(|p| p.starts_with('.') || (cfg!(windows) && p.contains(':')))
    {
        return None;
    }
    Some(parts)
}

/// The real path of `path` inside shared folder `root`, if it exists and is
/// inside it (after following any links).
fn resolve<P: Platform>(ctx: &NetCtx<P>, root: &str, path: &str) -> Result<PathBuf, String> {
    let base = ctx
        .files
        .shared
        .lock()
        .iter()
        .find(|(name, _)| name == root)
        .map(|(_, p)| p.clone())
        .ok_or("that folder isn't shared")?;
    let parts = path_parts(path).ok_or("not found")?;
    let base = std::fs::canonicalize(&base).map_err(|_| "the shared folder is missing")?;
    let mut full = base.clone();
    for p in parts {
        full.push(p);
    }
    let real = std::fs::canonicalize(&full).map_err(|_| "not found")?;
    if !real.starts_with(&base) {
        return Err("not found".into());
    }
    Ok(real)
}

fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Folders first, then by name (case-insensitive).
fn sort(entries: &mut [RemoteEntry]) {
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// What's in `dir`: no hidden entries, no links, no names that aren't
/// valid UTF-8 (they couldn't be asked for again).
fn read_folder(dir: &Path) -> Result<Vec<RemoteEntry>, String> {
    let rd = std::fs::read_dir(dir).map_err(|_| "can't read the folder")?;
    let mut entries = Vec::new();
    for entry in rd.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        entries.push(RemoteEntry {
            name,
            dir: kind.is_dir(),
            size: if kind.is_file() { meta.len() } else { 0 },
            modified: modified(&meta),
        });
    }
    sort(&mut entries);
    Ok(entries)
}

/// The shared folders themselves (the top of the tree).
fn roots<P: Platform>(ctx: &NetCtx<P>) -> Vec<RemoteEntry> {
    let shared = ctx.files.shared.lock().clone();
    shared
        .into_iter()
        .filter_map(|(name, path)| {
            let meta = std::fs::metadata(&path).ok().filter(|m| m.is_dir())?;
            Some(RemoteEntry {
                name,
                dir: true,
                size: 0,
                modified: modified(&meta),
            })
        })
        .collect()
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Answer `fs_list`: one page of the folder, or why not.
fn on_list<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let (root, path) = (msg.str("root"), msg.str("path"));
    let offset = msg.json("offset").as_u64().unwrap_or(0) as usize;
    let listed = if root.is_empty() {
        Ok(roots(ctx))
    } else {
        resolve(ctx, &root, &path).and_then(|dir| {
            if dir.is_dir() {
                read_folder(&dir)
            } else {
                Err("not a folder".into())
            }
        })
    };
    let mut reply = json!({
        "type": "fs_listing", "from": me(ctx), "req": msg.str("req"),
        "root": root, "path": path,
    });
    match listed {
        Ok(all) => {
            let page: Vec<&RemoteEntry> = all.iter().skip(offset).take(PAGE).collect();
            reply["entries"] = json!(page);
            if offset + PAGE < all.len() {
                reply["next"] = json!(offset + PAGE);
            }
        }
        Err(e) => {
            reply["entries"] = json!([]);
            reply["error"] = e.into();
        }
    }
    // Reliably: a reply lost on a link that just died would otherwise
    // never come (the resent request is a repeat).
    crate::engine::send_reliably(ctx, from, &reply.to_string());
}

/// Answer `fs_get`: start sending the file under the asker's id.
fn on_get<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    if !super::files::is_transfer_id(&id) {
        return;
    }
    let path = msg.str("path");
    let started = resolve(ctx, &msg.str("root"), &path).and_then(|file| {
        if !file.is_file() {
            return Err("not a file".to_string());
        }
        super::files::start_with_id(ctx, from, &file, &id).map_err(|e| e.to_string())
    });
    if let Err(error) = started {
        let name = path.rsplit('/').next().unwrap_or_default();
        let reply = json!({
            "type": "fs_get_failed", "from": me(ctx), "id": id,
            "name": super::files::safe_name(name), "error": error,
        });
        crate::engine::send_reliably(ctx, from, &reply.to_string());
    }
}

pub struct BrowseHandler;

impl<P: Platform> ControlHandler<P> for BrowseHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["fs_list", "fs_listing", "fs_get", "fs_get_failed"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        match msg.kind() {
            "fs_list" => on_list(ctx, msg, &from),
            "fs_get" => on_get(ctx, msg, &from),
            "fs_listing" => {
                let entries: Vec<RemoteEntry> =
                    serde_json::from_value(msg.json("entries")).unwrap_or_default();
                ctx.sink.emit(CoreEvent::FolderListing {
                    from,
                    req: msg.str("req"),
                    root: msg.str("root"),
                    path: msg.str("path"),
                    entries,
                    next: msg.json("next").as_u64(),
                    error: msg.opt("error"),
                });
            }
            "fs_get_failed" => {
                let id = msg.str("id");
                if id.is_empty() {
                    return;
                }
                ctx.sink.emit(CoreEvent::FileFailed {
                    id,
                    node_id: from,
                    name: msg.str("name"),
                    incoming: true,
                    reason: msg.str("error"),
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_cannot_leave_the_shared_folder() {
        assert_eq!(path_parts(""), Some(vec![]));
        assert_eq!(path_parts("/"), Some(vec![]));
        assert_eq!(path_parts("a/b c/d.txt"), Some(vec!["a", "b c", "d.txt"]));
        assert_eq!(path_parts("/a//b/"), Some(vec!["a", "b"]));
        for bad in ["..", "a/../../etc", "./a", "a/.hidden", "a\\..\\b", "a\nb"] {
            assert_eq!(path_parts(bad), None, "{bad}");
        }
        assert_eq!(path_parts(&"a/".repeat(3000)), None);
    }

    #[test]
    fn folder_names_are_plain() {
        assert!(is_folder_name("Downloads"));
        assert!(is_folder_name("My pictures"));
        for bad in ["", "  ", ".x", "a/b", "a\\b", "a\tb"] {
            assert!(!is_folder_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn folders_come_first_then_names() {
        let e = |name: &str, dir| RemoteEntry {
            name: name.into(),
            dir,
            size: 0,
            modified: 0,
        };
        let mut v = vec![
            e("b.txt", false),
            e("Zed", true),
            e("A.txt", false),
            e("apps", true),
        ];
        sort(&mut v);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["apps", "Zed", "A.txt", "b.txt"]);
    }
}
