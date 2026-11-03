//! my-domain over Bluetooth LE, desktop side.
//!
//! The phone advertises a my-domain GATT service and accepts connections
//! (Android: `BleLink.kt`); a computer finds it and connects ([`central`]).
//! Over the connection go the core's frames (`Engine::ble_opened` and
//! friends), so messages stay end-to-end encrypted and every feature works,
//! slowly, with no shared network and no internet.
//!
//! The service, both ends must agree:
//! - [`SERVICE`] with two characteristics: [`TO_PHONE`] (the computer
//!   writes, with response) and [`FROM_PHONE`] (the phone notifies).
//! - A frame is sent as chunks: one header byte (1 = the frame's last
//!   chunk, 0 = more follow), then up to the packet size minus one bytes.

pub mod central;

use uuid::Uuid;

/// The my-domain service.
pub const SERVICE: Uuid = Uuid::from_u128(0x7a3e0001_5f3c_4b8e_9d1a_6d79646f6d61);
/// The computer writes frames here.
pub const TO_PHONE: Uuid = Uuid::from_u128(0x7a3e0002_5f3c_4b8e_9d1a_6d79646f6d61);
/// The phone notifies frames here.
pub const FROM_PHONE: Uuid = Uuid::from_u128(0x7a3e0003_5f3c_4b8e_9d1a_6d79646f6d61);

/// Largest frame put back together (the same cap as the QUIC link's).
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// Split `frame` into chunks of at most `packet` bytes, header included.
pub fn split(frame: &[u8], packet: usize) -> Vec<Vec<u8>> {
    let body = packet.saturating_sub(1).max(1);
    let mut parts: Vec<&[u8]> = frame.chunks(body).collect();
    if parts.is_empty() {
        parts.push(&[]);
    }
    let last = parts.len() - 1;
    parts
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let mut chunk = Vec::with_capacity(part.len() + 1);
            chunk.push(u8::from(i == last));
            chunk.extend_from_slice(part);
            chunk
        })
        .collect()
}

/// Puts chunks back together into frames.
#[derive(Default)]
pub struct Joiner {
    buf: Vec<u8>,
}

impl Joiner {
    /// Add one chunk; returns the frame when it was the last one.
    pub fn push(&mut self, chunk: &[u8]) -> Option<Vec<u8>> {
        let (&last, body) = chunk.split_first()?;
        if self.buf.len() + body.len() > MAX_FRAME {
            self.buf.clear(); // too big: drop it
            return None;
        }
        self.buf.extend_from_slice(body);
        (last == 1).then(|| std::mem::take(&mut self.buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_being_split() {
        for len in [0, 1, 19, 20, 21, 1000] {
            let frame: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let mut j = Joiner::default();
            let chunks = split(&frame, 20);
            assert!(chunks.iter().all(|c| c.len() <= 20));
            let mut out = None;
            for (i, c) in chunks.iter().enumerate() {
                out = j.push(c);
                assert_eq!(out.is_some(), i == chunks.len() - 1);
            }
            assert_eq!(out.unwrap(), frame);
        }
    }

    #[test]
    fn an_oversized_frame_is_dropped() {
        let mut j = Joiner::default();
        let big = vec![0u8; 1024 * 1024];
        for _ in 0..4 {
            assert_eq!(j.push(&[&[0u8][..], &big].concat()), None);
        }
        assert_eq!(j.push(&[1, 7]), None, "over the cap: dropped");
        assert_eq!(j.push(&[1, 7]), Some(vec![7]), "the next frame is fine");
    }
}
