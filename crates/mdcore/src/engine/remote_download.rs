//! "Download on <device>": a link shared here is downloaded by another of
//! your devices (see `features::remote_download`).

use super::*;
use crate::features::remote_download;
use crate::model::{RemoteDownloadItem, RemoteDownloadSettings};

impl<P: Platform> Engine<P> {
    /// Ask `node_id` to download the first web link in `text`. Answers with
    /// the download as it starts (its id is also its transfer id); what
    /// follows comes as `RemoteDownload` and the File* events. Only to a
    /// device that downloads links (`remote_download_supported`).
    pub fn request_remote_download(
        &self,
        node_id: &str,
        text: &str,
    ) -> crate::Result<RemoteDownloadItem> {
        remote_download::request(&self.ctx, node_id, text)
    }

    /// `node_id` said it downloads links for your other devices.
    pub fn remote_download_supported(&self, node_id: &str) -> bool {
        remote_download::supports(&self.ctx, node_id)
    }

    /// Answer a download waiting here (`state` "asking"): `allow` it or
    /// not; `always`: allow that device from now on without asking.
    pub fn answer_remote_download(&self, id: &str, allow: bool, always: bool) -> crate::Result<()> {
        remote_download::answer(&self.ctx, id, allow, always)
    }

    /// Stop a download, either way (as `cancel_file` does too).
    pub fn cancel_remote_download(&self, id: &str) -> bool {
        remote_download::cancel(&self.ctx, id)
    }

    /// Downloads either way since the app started, the newest first.
    pub fn remote_downloads(&self) -> Vec<RemoteDownloadItem> {
        remote_download::list(&self.ctx)
    }

    /// What this device does when another of yours asks it to download.
    pub fn remote_download_settings(&self) -> RemoteDownloadSettings {
        remote_download::settings(&self.ctx)
    }

    pub fn set_remote_download_settings(&self, s: RemoteDownloadSettings) -> crate::Result<()> {
        remote_download::set_settings(&self.ctx, s)
    }
}
