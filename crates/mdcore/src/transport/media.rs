//! Binary media frames: screen snapshots and audio as raw bytes, for the
//! QUIC links whose two ends read them.
//!
//! The JSON path carries a frame as a control message (`screen_frame`,
//! `audio_frame`) with the bytes in base64, inside an envelope whose
//! ciphertext is base64 again: about 1.8 times the bytes (a phone's 48 kHz
//! stereo, 1.5 Mbit/s of PCM, took about 2.8). A binary record is the bytes
//! themselves, plus 69 bytes:
//!
//! ```text
//! record    = [version u8 = 1][nonce 24][ciphertext(header ‖ payload) + tag 16]
//! header    = [kind u8][f0 u8][f1 u16][f2 u32][session u32][seq u64][ts_ms u64]   (28 bytes, big-endian)
//!   kind 1  = audio:  f0 channels, f1 0, f2 rate (Hz); payload s16le PCM, interleaved
//!   kind 2  = screen: f0 quality, f1 width, f2 height (0: not said); payload a JPEG
//! ```
//!
//! - Sealed with the same key and cipher as every other message
//!   (XChaCha20-Poly1305, `crypto::seal`), the header inside, and
//!   [`AAD`] authenticated so a record can never pass for an envelope. Media
//!   is never sent in the clear.
//! - Who sent it is the QUIC connection's peer (its device key proven in
//!   the handshake), as for every message over that link; over the relay,
//!   the device the server says it came from (one of the same account's).
//! - Over QUIC, audio goes as an unreliable DATAGRAM when the record
//!   fits one (late audio is noise anyway), otherwise, like every screen
//!   frame, on a long-lived unidirectional stream of its own kind
//!   (`[magic "MDM1"]` then `[u32 length][record]` repeated): ordered and
//!   reliable, and audio never waits behind a picture. A lane that's full
//!   (the link is slower than the frames) drops the frame: a stale frame is
//!   worth nothing.
//! - Over the relay, as a binary WebSocket frame (`transport::relay`), when
//!   the server says it forwards them (`"bin": 1`) and the peer's relayed
//!   `Hello` said `bin_media`; a full queue drops the frame, as above.
//! - Only to a peer whose `Hello` said `bin_media` (this version or later):
//!   an older app, or an older relay server, gets the JSON messages, as
//!   before. WebRTC and Bluetooth links always carry the JSON messages.
//! - The receiver checks a binary frame exactly as the JSON one and emits
//!   the same `CoreEvent::ScreenFrame` / `CoreEvent::AudioFrame`, so the
//!   apps don't know which way it came.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::crypto::{open, seal};
use crate::engine::NetCtx;
use crate::platform::Platform;

/// The record version this build writes and reads (said in the `Hello`).
pub const VERSION: u8 = 1;
/// Authenticated with every record, never sent.
const AAD: &[u8] = b"mydomain media v1";
/// What a media stream starts with.
pub(crate) const STREAM_MAGIC: &[u8; 4] = b"MDM1";
/// Bytes of header inside the ciphertext.
pub const HEADER_LEN: usize = 28;
/// Bytes a record adds to its payload: version, nonce, header, tag.
pub const OVERHEAD: usize = 1 + 24 + HEADER_LEN + 16;

/// Which stream a frame belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio = 1,
    Screen = 2,
}

/// What the payload is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// s16le PCM, `channels` interleaved, at `rate` Hz.
    Pcm { rate: u32, channels: u8 },
    /// A JPEG; 0 for what the sender didn't say.
    Jpeg {
        width: u16,
        height: u16,
        quality: u8,
    },
}

impl Format {
    pub fn kind(&self) -> MediaKind {
        match self {
            Format::Pcm { .. } => MediaKind::Audio,
            Format::Jpeg { .. } => MediaKind::Screen,
        }
    }
}

/// A frame's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The sending engine's run (random per start): a new one means the
    /// sender restarted, and its `seq` started again.
    pub session: u32,
    /// Counts up per kind, shared with the JSON messages' `seq`.
    pub seq: u64,
    /// When the sender captured or sent it, ms since the Unix epoch.
    pub ts_ms: u64,
    pub format: Format,
}

/// Milliseconds since the Unix epoch, for `Header::ts_ms`.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn header_bytes(h: &Header) -> [u8; HEADER_LEN] {
    let (kind, f0, f1, f2) = match h.format {
        Format::Pcm { rate, channels } => (MediaKind::Audio, channels, 0u16, rate),
        Format::Jpeg {
            width,
            height,
            quality,
        } => (MediaKind::Screen, quality, width, u32::from(height)),
    };
    let mut b = [0u8; HEADER_LEN];
    b[0] = kind as u8;
    b[1] = f0;
    b[2..4].copy_from_slice(&f1.to_be_bytes());
    b[4..8].copy_from_slice(&f2.to_be_bytes());
    b[8..12].copy_from_slice(&h.session.to_be_bytes());
    b[12..20].copy_from_slice(&h.seq.to_be_bytes());
    b[20..28].copy_from_slice(&h.ts_ms.to_be_bytes());
    b
}

fn parse_header(b: &[u8]) -> Option<Header> {
    let b: &[u8; HEADER_LEN] = b.get(..HEADER_LEN)?.try_into().ok()?;
    let u16_at = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let u64_at = |i: usize| {
        let mut x = [0u8; 8];
        x.copy_from_slice(&b[i..i + 8]);
        u64::from_be_bytes(x)
    };
    let format = match b[0] {
        1 => Format::Pcm {
            rate: u32_at(4),
            channels: b[1],
        },
        2 => Format::Jpeg {
            width: u16_at(2),
            height: u16::try_from(u32_at(4)).ok()?,
            quality: b[1],
        },
        _ => return None,
    };
    Some(Header {
        session: u32_at(8),
        seq: u64_at(12),
        ts_ms: u64_at(20),
        format,
    })
}

/// Seal a frame into a record.
pub fn encode(key: &[u8; 32], h: &Header, payload: &[u8]) -> Option<Vec<u8>> {
    let mut plain = Vec::with_capacity(HEADER_LEN + payload.len());
    plain.extend_from_slice(&header_bytes(h));
    plain.extend_from_slice(payload);
    let sealed = seal(key, AAD, &plain)?;
    let mut out = Vec::with_capacity(1 + sealed.len());
    out.push(VERSION);
    out.extend_from_slice(&sealed);
    Some(out)
}

/// Open a record: `None` for another version, another key, or tampering.
pub fn decode(key: &[u8; 32], record: &[u8]) -> Option<(Header, Vec<u8>)> {
    let (&ver, sealed) = record.split_first()?;
    if ver != VERSION {
        return None;
    }
    let mut plain = open(key, AAD, sealed)?;
    let h = parse_header(&plain)?;
    let payload = plain.split_off(HEADER_LEN);
    Some((h, payload))
}

/// How a binary send went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sent {
    /// Handed to the link.
    Yes,
    /// The link is slower than the frames: this one is dropped.
    Dropped,
    /// No binary path to the peer (an older app or server, not QUIC or
    /// relay, no key): send the JSON message instead.
    NotBinary,
}

/// Send a frame to `node` as a binary record, if its link takes them.
pub(crate) fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, h: &Header, payload: &[u8]) -> Sent {
    if !ctx.config.bin_media {
        return Sent::NotBinary;
    }
    let Some(route) = ctx.ws_conns.lock().get(node).map(|(id, _)| *id) else {
        return Sent::NotBinary;
    };
    let quic = ctx.quic.reads_media(node, route);
    // (Checked before sealing: most peers take JSON.)
    if !quic && !super::relay::reads_media(ctx, node, route) {
        return Sent::NotBinary;
    }
    let Some(key) = *ctx.key.lock() else {
        return Sent::NotBinary;
    };
    let Some(record) = encode(&key, h, payload) else {
        return Sent::NotBinary;
    };
    let sent = if quic {
        ctx.quic.send_media(node, route, h.format.kind(), record)
    } else {
        super::relay::send_media(ctx, node, route, &record)
    };
    if sent == Sent::Yes {
        super::note_use(&ctx.p2p_conns, node);
    }
    sent
}

/// A record arrived from `from` (the QUIC link's peer, or the relay's
/// sender): open it and hand
/// the frame to its feature, which checks it as it checks the JSON one.
/// One that doesn't open is dropped quietly (counted, never shown).
pub(crate) fn on_record<P: Platform>(ctx: &NetCtx<P>, from: &str, record: &[u8]) {
    let Some(key) = *ctx.key.lock() else {
        return;
    };
    let Some((h, payload)) = decode(&key, record) else {
        ctx.decrypt_fail_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return;
    };
    super::note_use(&ctx.p2p_conns, from);
    match h.format {
        Format::Pcm { rate, channels } => {
            crate::features::audio_share::deliver(ctx, from, h.seq, rate, channels, &payload)
        }
        Format::Jpeg { .. } => crate::features::screen::deliver(ctx, from, h.seq, &payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [9u8; 32];

    fn audio(seq: u64) -> Header {
        Header {
            session: 0xdead_beef,
            seq,
            ts_ms: 1_700_000_000_123,
            format: Format::Pcm {
                rate: 48_000,
                channels: 2,
            },
        }
    }

    #[test]
    fn a_record_round_trips_with_its_header() {
        let pcm: Vec<u8> = (0..3840u32).map(|i| (i % 251) as u8).collect();
        let rec = encode(&KEY, &audio(42), &pcm).unwrap();
        assert_eq!(rec.len(), pcm.len() + OVERHEAD);
        let (h, back) = decode(&KEY, &rec).unwrap();
        assert_eq!(h, audio(42));
        assert_eq!(back, pcm);

        let jpeg = Header {
            format: Format::Jpeg {
                width: 1920,
                height: 1080,
                quality: 70,
            },
            ..audio(7)
        };
        let (h, back) = decode(&KEY, &encode(&KEY, &jpeg, b"\xff\xd8jpeg").unwrap()).unwrap();
        assert_eq!(h, jpeg);
        assert_eq!(h.format.kind(), MediaKind::Screen);
        assert_eq!(back, b"\xff\xd8jpeg");
    }

    #[test]
    fn foreign_or_damaged_records_are_refused() {
        let rec = encode(&KEY, &audio(1), b"abcd").unwrap();
        assert!(decode(&[1u8; 32], &rec).is_none(), "another key");
        let mut v2 = rec.clone();
        v2[0] = 2;
        assert!(decode(&KEY, &v2).is_none(), "another version");
        let mut bad = rec.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(decode(&KEY, &bad).is_none(), "tampered");
        assert!(decode(&KEY, &[]).is_none());
        assert!(decode(&KEY, &rec[..10]).is_none());
        // A JSON envelope's sealing can't be mistaken for a record's.
        let other = crate::crypto::seal(&KEY, b"something else", b"x").unwrap();
        let mut forged = vec![VERSION];
        forged.extend_from_slice(&other);
        assert!(decode(&KEY, &forged).is_none());
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let mut h = header_bytes(&audio(1));
        h[0] = 9;
        assert!(parse_header(&h).is_none());
        assert!(parse_header(&h[..5]).is_none());
    }
}
