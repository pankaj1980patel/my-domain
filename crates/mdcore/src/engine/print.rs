//! Print from the phone on a computer's printer (see `features::print`).

use super::*;
use crate::features::print::{self, PrintOptions, Printer};

impl<P: Platform> Engine<P> {
    /// Computer: it prints for your phones now (the user's "Let my phone
    /// print here" is on and it can print). Says `print` in its status
    /// while on; a change goes to the linked devices at once.
    pub fn set_print_offered(&self, on: bool) {
        print::set_offered(&self.ctx, on)
    }

    pub fn print_offered(&self) -> bool {
        print::offered(&self.ctx)
    }

    /// `node` said it prints for your phones (in its status or the
    /// `addr_update` a link starts with; kept across restarts).
    pub fn print_supported(&self, node_id: &str) -> bool {
        print::supports(&self.ctx, node_id)
    }

    /// Phone: ask computer `node_id` for its printers; they arrive as
    /// `Printers`. Only to a computer that prints for your phones.
    pub fn request_printers(&self, node_id: &str) -> crate::Result<()> {
        print::request_printers(&self.ctx, node_id)
    }

    /// Phone: send the PDF at `path` to computer `node_id` to print with
    /// `options`. Returns the job's id, which is its transfer's id (the
    /// usual `file-*` events say how sending goes; `PrintStatus` how
    /// printing goes). Only to a computer that prints for your phones.
    pub fn print_file(
        &self,
        node_id: &str,
        path: &str,
        options: &PrintOptions,
    ) -> crate::Result<String> {
        print::print_file(&self.ctx, node_id, std::path::Path::new(path), options)
    }

    /// Computer: answer a `PrintersRequested` from `node_id`.
    pub fn send_printers(&self, node_id: &str, printers: &[Printer]) -> crate::Result<()> {
        print::send_printers(&self.ctx, node_id, printers)
    }

    /// Computer: how print job `id` (from a `PrintJob`) goes: "waiting",
    /// "printing", "done" or "failed" (with `reason`). Goes to the phone
    /// that sent it.
    pub fn print_status(&self, id: &str, state: &str, reason: Option<&str>) -> crate::Result<()> {
        print::report(&self.ctx, id, state, reason)
    }
}
