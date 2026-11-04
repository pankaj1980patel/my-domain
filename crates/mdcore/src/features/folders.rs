//! Sending a whole folder: each file inside goes as its own transfer
//! (`features::files`, so resumable like any other), with its place in the
//! folder in the offer:
//!
//!   file_offer { ..., folder: "Trip/day 1" }
//!
//! `folder` is the folder's own name, then the subfolders down to the file,
//! joined with `/`. The receiver saves the file under `<downloads>/<folder>/`
//! (see `files::safe_folder`, which refuses `..`, absolute paths and the
//! like). A receiver from before folders ignores the field and saves every
//! file straight into its downloads.
//!
//! The sender walks the folder once, at the start: symlinks are skipped (never
//! followed, so nothing outside the folder goes), and so are hidden entries
//! (`.DS_Store`, `.git`: the receiver couldn't keep a name starting with a dot
//! anyway). Empty subfolders aren't recreated. At most `IN_FLIGHT` files of a
//! folder go at once; when one ends the next starts. A file that couldn't be
//! read or arrived damaged is skipped; any other failure (cancelled on either
//! side, the device unreachable, its storage full) stops the whole folder.
//! Sending a folder the receiver already has merges into it: files with the
//! same name and content are skipped, others get a numbered name.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::engine::NetCtx;
use crate::platform::Platform;

use super::files::{safe_name, start_with_extra};

/// Files of one folder being sent at once.
const IN_FLIGHT: usize = 4;
/// Most files in one folder.
pub const MAX_FILES: usize = 10_000;
/// Deepest subfolder walked into.
const MAX_DEPTH: usize = 32;

/// A folder being sent.
pub(crate) struct FolderSend {
    node: String,
    /// Files still to start: (path, folder for the offer).
    queue: VecDeque<(PathBuf, String)>,
    /// Transfers of it going now.
    going: HashSet<String>,
}

/// Start sending the folder at `path` to `node`. Returns how many files it
/// holds; each shows up as its own transfer as it starts.
pub fn start<P: Platform>(ctx: &NetCtx<P>, node: &str, path: &Path) -> crate::Result<usize> {
    // The real path, so `.` or a trailing `/` still has a name.
    let root = std::fs::canonicalize(path).map_err(|e| format!("can't read the folder: {e}"))?;
    if !root.is_dir() {
        return Err("not a folder".to_string().into());
    }
    let top = safe_name(&root.file_name().unwrap_or_default().to_string_lossy());
    let mut files = Vec::new();
    walk(&root, &top, 0, &mut files).map_err(|e| format!("can't read the folder: {e}"))?;
    if files.is_empty() {
        return Err("the folder has no files to send".to_string().into());
    }
    if files.len() > MAX_FILES {
        return Err(format!("the folder has more than {MAX_FILES} files").into());
    }
    let count = files.len();
    ctx.files.folders.lock().push(FolderSend {
        node: node.to_string(),
        queue: files.into(),
        going: HashSet::new(),
    });
    pump(ctx);
    Ok(count)
}

/// How many folders are still being sent (files going or still to start).
pub fn sending<P: Platform>(ctx: &NetCtx<P>) -> usize {
    ctx.files
        .folders
        .lock()
        .iter()
        .filter(|f| !f.queue.is_empty() || !f.going.is_empty())
        .count()
}

/// The files under `dir` (sorted by name, folder by folder), each with the
/// folder it goes into on the receiver (`rel`, then its subfolders).
fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<(PathBuf, String)>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() > MAX_FILES {
            break; // too many anyway; `start` says so
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // `file_type` doesn't follow symlinks: a link is skipped whatever
        // it points at.
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_file() {
            out.push((e.path(), rel.to_string()));
        } else if kind.is_dir() && depth < MAX_DEPTH {
            let sub = format!("{rel}/{}", safe_name(&name));
            // A subfolder we may not read is left out, not the whole send.
            let _ = walk(&e.path(), &sub, depth + 1, out);
        }
    }
    Ok(())
}

/// Start files of every folder until each has `IN_FLIGHT` going (or none
/// left); folders that are done are dropped.
fn pump<P: Platform>(ctx: &NetCtx<P>) {
    loop {
        let next = {
            let mut folders = ctx.files.folders.lock();
            folders.retain(|f| !f.queue.is_empty() || !f.going.is_empty());
            folders
                .iter_mut()
                .find(|f| f.going.len() < IN_FLIGHT && !f.queue.is_empty())
                .and_then(|f| {
                    let (path, rel) = f.queue.pop_front()?;
                    let id = crate::transport::outbox::new_id();
                    // Booked before it starts: it may end before `start`
                    // returns.
                    f.going.insert(id.clone());
                    Some((f.node.clone(), path, rel, id))
                })
        };
        let Some((node, path, rel, id)) = next else {
            return;
        };
        let extra = Some(json!({ "folder": rel }));
        if start_with_extra(ctx, &node, &path, &id, extra).is_err() {
            // Gone or changed since the walk: skip it.
            for f in ctx.files.folders.lock().iter_mut() {
                f.going.remove(&id);
            }
        }
    }
}

/// A failure that's about this one file: the rest of the folder still goes.
fn only_this_file(why: &str) -> bool {
    why.starts_with("can't read")
        || why == "the file couldn't be read"
        || why == "the file arrived damaged"
}

/// Outgoing transfer `id` ended (`why` = the failure, if it failed): if it
/// belongs to a folder, start the next files, or stop the folder.
pub fn outgoing_ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>) {
    let stop = {
        let mut folders = ctx.files.folders.lock();
        let Some(i) = folders.iter().position(|f| f.going.contains(id)) else {
            return;
        };
        folders[i].going.remove(id);
        match why {
            Some(why) if !only_this_file(why) => {
                // The others of it would fail the same way, or the user
                // wants it stopped.
                let f = folders.remove(i);
                f.going.into_iter().collect()
            }
            _ => Vec::new(),
        }
    };
    for other in stop {
        super::files::cancel(ctx, &other);
    }
    pump(ctx);
}
