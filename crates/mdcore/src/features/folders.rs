//! Sending a whole folder: a batch (`features::batches`) whose files each
//! go as their own transfer (`features::files`, so resumable like any
//! other), with their place in the folder in the offer:
//!
//!   file_offer { ..., folder: "Trip/day 1", batch: {...} }
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
//! anyway). Empty subfolders aren't recreated. Sending a folder the receiver
//! already has merges into it: files with the same name and content are
//! skipped, others get a numbered name.

use std::path::Path;

use crate::engine::NetCtx;
use crate::platform::Platform;

pub use super::batches::MAX_FILES;

/// Start sending the folder at `path` to `node`. Returns how many files it
/// holds; the batch and each file show up as transfers.
pub fn start<P: Platform>(ctx: &NetCtx<P>, node: &str, path: &Path) -> crate::Result<usize> {
    if !path.is_dir() {
        return Err("not a folder".to_string().into());
    }
    let started = super::batches::start(ctx, node, &[path.to_path_buf()])?;
    Ok(started.count as usize)
}

/// How many folders (batches) are still being sent.
pub fn sending<P: Platform>(ctx: &NetCtx<P>) -> usize {
    super::batches::sending(ctx)
}

/// Outgoing transfer `id` ended (`why` = the failure, if it failed).
pub fn outgoing_ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>) {
    super::batches::outgoing_ended(ctx, id, why);
}
