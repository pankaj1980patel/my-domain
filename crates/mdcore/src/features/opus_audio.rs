//! Opus audio for the binary media path (`transport::media`, kind 1): the
//! codec a device uses with another that says `audio_codec: 2` (or more)
//! in its status and `addr_update`. An older app gets ADPCM
//! (`audio_codec: 1`, `features::audio_codec`) or plain PCM, as before.
//!
//! Opus is 20 ms frames at 48 kHz (or 24, 16, 12, 8 kHz: the capture's
//! rate, never resampled here) and about a tenth of ADPCM's bytes:
//!
//! | profile    | stereo     | mono       | in-band FEC        | frames a record | RED |
//! |------------|------------|------------|--------------------|-----------------|-----|
//! | network    | 96 kbit/s  | 64 kbit/s  | on over QUIC       | 1               | QUIC DATAGRAMs |
//! | Bluetooth  | 32 kbit/s  | 24 kbit/s  | off (reliable)     | 3 (60 ms)       | no  |
//!
//! The encoder's expected packet loss follows the loss measured on the
//! link (QUIC's own count, and what the sink says in its `audio_play`
//! renewals, `loss`), so in-band FEC costs bits only when it's needed.
//! (Opus carries FEC in its SILK layer: at music bitrates it's mostly
//! CELT and RED does the recovering; at Bluetooth's both help.)
//!
//! A record's `f1` says [`FLAG_OPUS`]; the payload:
//!
//! ```text
//! [u32 number of the first frame][u8 frames k][u8 red 0|1]
//! k x [u16 len][Opus packet]            frames first, first + 1, ...
//! red: [u16 len][Opus packet]           frame first - 1, again (RED)
//! ```
//!
//! Frame numbers count up by one per 20 ms frame per sender and peer,
//! with no gaps, so the sink knows exactly which frames never came. For
//! each missing frame (up to [`MAX_FILL`] in a row) it plays, in order of
//! preference: the redundant copy, Opus's in-band FEC from the next frame,
//! or Opus's own packet-loss concealment (PLC). That replaces the jitter
//! buffer's faded repeat for lost frames (it still covers a buffer that
//! runs dry). Repeats and late frames are dropped by number.

use opus::{Application, Bitrate, Channels, Decoder, Encoder};

/// `f1` flag: the payload is Opus (`features::audio_codec` has 1 and 2).
pub const FLAG_OPUS: u16 = 4;
/// Rates Opus codes; others go as ADPCM or PCM.
pub const RATES: [u32; 5] = [8_000, 12_000, 16_000, 24_000, 48_000];
/// Milliseconds a frame.
pub const FRAME_MS: u32 = 20;
/// Missing frames in a row filled in; a longer gap is left to the jitter
/// buffer (the stream stalled, or the sender moved away and back).
pub const MAX_FILL: u32 = 5;
/// Largest Opus packet made (bytes).
const MAX_PACKET: usize = 1_275;
/// Frames a record at most (as read).
const MAX_FRAMES: usize = 6;
/// Pending input kept at most (the source's frames are 10 to 100 ms).
const MAX_PENDING_MS: u32 = 200;

/// Whether Opus codes `rate`.
pub fn codes(rate: u32) -> bool {
    RATES.contains(&rate)
}

/// How the frames travel, which sets the bitrate and packing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Wi-Fi, a direct link or the relay. `lossy`: QUIC DATAGRAMs (FEC and
    /// RED help); the relay is a reliable stream.
    Network { lossy: bool },
    /// A Bluetooth LE link: little bandwidth, reliable, slow per packet.
    Bluetooth,
}

impl Profile {
    /// The target bitrate (bit/s) for `channels`.
    pub fn bitrate(self, channels: u8) -> i32 {
        match (self, channels) {
            (Profile::Network { .. }, 2) => 96_000,
            (Profile::Network { .. }, _) => 64_000,
            (Profile::Bluetooth, 2) => 32_000,
            (Profile::Bluetooth, _) => 24_000,
        }
    }
    /// Frames sent together in one record.
    pub fn frames_per_record(self) -> usize {
        match self {
            Profile::Network { .. } => 1,
            Profile::Bluetooth => 3,
        }
    }
    /// Each record carries the frame before it too.
    pub fn red(self) -> bool {
        matches!(self, Profile::Network { lossy: true })
    }
    /// In-band FEC on.
    pub fn fec(self) -> bool {
        matches!(self, Profile::Network { lossy: true })
    }
}

/// The packet-loss percentage to tell the encoder, from what was measured:
/// at least 2 % on a lossy link (FEC stays ready), at most 30 %.
pub fn loss_for_encoder(profile: Profile, measured_pct: f32) -> i32 {
    if !profile.fec() {
        return 0;
    }
    let pct = if measured_pct.is_finite() {
        measured_pct.ceil() as i32
    } else {
        0
    };
    pct.clamp(2, 30)
}

/// A record's payload, parsed.
#[derive(Debug, PartialEq, Eq)]
pub struct Record<'a> {
    pub first: u32,
    pub frames: Vec<&'a [u8]>,
    pub red: Option<&'a [u8]>,
}

/// Pack frames `first`, `first + 1`, ... (and the frame before, `red`).
pub fn pack(first: u32, frames: &[Vec<u8>], red: Option<&[u8]>) -> Vec<u8> {
    let len: usize =
        frames.iter().map(|f| f.len() + 2).sum::<usize>() + red.map_or(0, |r| r.len() + 2);
    let mut out = Vec::with_capacity(6 + len);
    out.extend_from_slice(&first.to_le_bytes());
    out.push(u8::try_from(frames.len()).unwrap_or(u8::MAX));
    out.push(u8::from(red.is_some()));
    for f in frames.iter().map(Vec::as_slice).chain(red) {
        out.extend_from_slice(&u16::try_from(f.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(f);
    }
    out
}

/// Read a record's payload; `None` when it's damaged.
pub fn unpack(data: &[u8]) -> Option<Record<'_>> {
    let first = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    let k = usize::from(*data.get(4)?);
    let red = *data.get(5)?;
    if k == 0 || k > MAX_FRAMES || red > 1 {
        return None;
    }
    let mut rest = &data[6..];
    let mut take = || -> Option<&[u8]> {
        let n = usize::from(u16::from_le_bytes(rest.get(..2)?.try_into().ok()?));
        let f = rest.get(2..2 + n)?;
        if n == 0 || n > MAX_PACKET {
            return None;
        }
        rest = &rest[2 + n..];
        Some(f)
    };
    let mut frames = Vec::with_capacity(k);
    for _ in 0..k {
        frames.push(take()?);
    }
    let red = if red == 1 { Some(take()?) } else { None };
    if !rest.is_empty() {
        return None;
    }
    Some(Record { first, frames, red })
}

fn channels_of(ch: u8) -> Option<Channels> {
    match ch {
        1 => Some(Channels::Mono),
        2 => Some(Channels::Stereo),
        _ => None,
    }
}

/// One sender's encoder for one peer: PCM in (any length), records out.
pub struct Enc {
    rate: u32,
    channels: u8,
    profile: Profile,
    enc: Encoder,
    /// Samples (interleaved) not yet a whole frame.
    pending: Vec<i16>,
    /// The next frame's number.
    next: u32,
    /// Frames coded, not yet sent (Bluetooth sends three together).
    batch: Vec<Vec<u8>>,
    /// The number of `batch[0]`.
    batch_first: u32,
    /// The last frame sent and its number, for the next record's RED.
    last: Option<(u32, Vec<u8>)>,
    loss: i32,
}

impl Enc {
    /// An encoder for `rate` Hz, `channels`, `profile`; frames numbered from
    /// `next`. `None` for a rate or channel count Opus doesn't take.
    pub fn new(rate: u32, channels: u8, profile: Profile, next: u32) -> Option<Self> {
        if !codes(rate) {
            return None;
        }
        let mut enc = Encoder::new(rate, channels_of(channels)?, Application::Audio).ok()?;
        enc.set_bitrate(Bitrate::Bits(profile.bitrate(channels)))
            .ok()?;
        enc.set_vbr(true).ok()?;
        enc.set_complexity(if matches!(profile, Profile::Bluetooth) {
            10
        } else {
            8
        })
        .ok()?;
        enc.set_inband_fec(profile.fec()).ok()?;
        let loss = loss_for_encoder(profile, 0.0);
        enc.set_packet_loss_perc(loss).ok()?;
        Some(Enc {
            rate,
            channels,
            profile,
            enc,
            pending: Vec::new(),
            next,
            batch: Vec::new(),
            batch_first: next,
            last: None,
            loss,
        })
    }

    pub fn format(&self) -> (u32, u8, Profile) {
        (self.rate, self.channels, self.profile)
    }

    /// The next frame's number (to carry on after a new encoder).
    pub fn next(&self) -> u32 {
        self.next
    }

    /// The loss measured on the link, in percent.
    pub fn set_loss(&mut self, measured_pct: f32) {
        let l = loss_for_encoder(self.profile, measured_pct);
        if l != self.loss && self.enc.set_packet_loss_perc(l).is_ok() {
            self.loss = l;
        }
    }

    pub fn loss(&self) -> i32 {
        self.loss
    }

    /// Samples (per channel) a frame.
    fn frame_samples(&self) -> usize {
        (self.rate * FRAME_MS / 1000) as usize
    }

    /// Forget what's waiting (the stream paused or went another way): the
    /// numbers carry on.
    pub fn flush(&mut self) {
        self.pending.clear();
        self.batch.clear();
        self.last = None;
    }

    /// Add `pcm` (s16le, interleaved); returns the payloads ready to send,
    /// each with the number of its first frame.
    pub fn push(&mut self, pcm: &[u8]) -> Vec<(u32, Vec<u8>)> {
        self.pending.extend(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b)),
        );
        let ch = usize::from(self.channels);
        let frame = self.frame_samples() * ch;
        let cap = (self.rate * MAX_PENDING_MS / 1000) as usize * ch;
        if self.pending.len() > cap {
            // Far more than a frame waiting: drop the oldest.
            let cut = self.pending.len() - cap;
            let cut = cut - cut % ch;
            self.pending.drain(..cut);
        }
        let mut out = Vec::new();
        let mut buf = [0u8; MAX_PACKET];
        while self.pending.len() >= frame {
            let n = self.enc.encode(&self.pending[..frame], &mut buf);
            self.pending.drain(..frame);
            let number = self.next;
            self.next = self.next.wrapping_add(1);
            let Ok(n) = n else {
                // A frame that didn't code is a lost frame to the sink.
                self.batch.clear();
                self.last = None;
                continue;
            };
            if self.batch.is_empty() {
                self.batch_first = number;
            }
            self.batch.push(buf[..n].to_vec());
            if self.batch.len() < self.profile.frames_per_record() {
                continue;
            }
            let frames = std::mem::take(&mut self.batch);
            let first = self.batch_first;
            let red = if self.profile.red() {
                self.last
                    .as_ref()
                    .filter(|(n, _)| n.wrapping_add(1) == first)
                    .map(|(_, p)| p.as_slice())
            } else {
                None
            };
            out.push((first, pack(first, &frames, red)));
            let last_n = first.wrapping_add(u32::try_from(frames.len()).unwrap_or(1) - 1);
            self.last = frames.into_iter().last().map(|p| (last_n, p));
        }
        out
    }
}

/// How a frame played came about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// It came.
    Received,
    /// It was lost; its redundant copy came in the next record.
    Redundant,
    /// It was lost; rebuilt from the in-band FEC in the next frame.
    Fec,
    /// It was lost; Opus made something up (PLC).
    Concealed,
}

/// What a sink counted from one source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecStats {
    pub frames: u32,
    pub redundant: u32,
    pub fec: u32,
    pub concealed: u32,
    pub duplicates: u32,
    /// Frames that never came (whatever filled them), and all expected,
    /// since the last `take_loss`.
    lost: u32,
    expected: u32,
}

/// One sink's decoder for one source.
pub struct Dec {
    rate: u32,
    channels: u8,
    dec: Decoder,
    /// The newest frame number played.
    last: Option<u32>,
    pub stats: DecStats,
}

impl Dec {
    pub fn new(rate: u32, channels: u8) -> Option<Self> {
        if !codes(rate) {
            return None;
        }
        Some(Dec {
            rate,
            channels,
            dec: Decoder::new(rate, channels_of(channels)?).ok()?,
            last: None,
            stats: DecStats::default(),
        })
    }

    pub fn format(&self) -> (u32, u8) {
        (self.rate, self.channels)
    }

    /// The share of frames lost (percent) since it was last asked; starts
    /// counting again.
    pub fn take_loss(&mut self) -> Option<f32> {
        let s = &mut self.stats;
        let out = (s.expected > 0).then(|| s.lost as f32 * 100.0 / s.expected as f32);
        s.lost = 0;
        s.expected = 0;
        out
    }

    fn samples(&self) -> usize {
        (self.rate * FRAME_MS / 1000) as usize * usize::from(self.channels)
    }

    fn run(&mut self, packet: &[u8], fec: bool) -> Option<Vec<u8>> {
        // Up to 120 ms, the longest an Opus packet holds.
        let mut out = vec![0i16; self.samples() * 6];
        let want = if packet.is_empty() || fec {
            // PLC and FEC give exactly one frame.
            self.samples()
        } else {
            out.len()
        };
        let n = self.dec.decode(packet, &mut out[..want], fec).ok()?;
        let n = n * usize::from(self.channels);
        if n == 0 {
            return None;
        }
        Some(out[..n].iter().flat_map(|s| s.to_le_bytes()).collect())
    }

    /// A record's payload arrived: the PCM to play (s16le), in order, each
    /// with its frame number and how it came about. `None`: damaged.
    pub fn receive(&mut self, payload: &[u8]) -> Option<Vec<(u32, Fill, Vec<u8>)>> {
        let rec = unpack(payload)?;
        let count = u32::try_from(rec.frames.len()).ok()?;
        let newest = rec.first.wrapping_add(count - 1);
        let mut out = Vec::new();
        if let Some(last) = self.last {
            // Wrapping compare: "newest is after last".
            let ahead = newest.wrapping_sub(last);
            if ahead == 0 || ahead > u32::MAX / 2 {
                self.stats.duplicates = self.stats.duplicates.saturating_add(1);
                return Some(out);
            }
            let gap = rec.first.wrapping_sub(last).wrapping_sub(1);
            if gap > 0 && gap <= MAX_FILL {
                self.stats.lost = self.stats.lost.saturating_add(gap);
                self.stats.expected = self.stats.expected.saturating_add(gap);
                for i in 0..gap {
                    let n = last.wrapping_add(1 + i);
                    let before_first = i + 1 == gap;
                    let (pcm, how) = match (before_first, rec.red) {
                        (true, Some(r)) => (self.run(r, false), Fill::Redundant),
                        (true, None) if !rec.frames.is_empty() => {
                            (self.run(rec.frames[0], true), Fill::Fec)
                        }
                        _ => (self.run(&[], false), Fill::Concealed),
                    };
                    if let Some(p) = pcm {
                        match how {
                            Fill::Redundant => self.stats.redundant += 1,
                            Fill::Fec => self.stats.fec += 1,
                            _ => self.stats.concealed += 1,
                        }
                        out.push((n, how, p));
                    }
                }
            }
        }
        for (i, f) in rec.frames.iter().enumerate() {
            let n = rec.first.wrapping_add(u32::try_from(i).ok()?);
            if self
                .last
                .is_some_and(|l| n.wrapping_sub(l) == 0 || n.wrapping_sub(l) > u32::MAX / 2)
            {
                continue; // played already
            }
            let Some(p) = self.run(f, false) else {
                continue;
            };
            self.stats.frames = self.stats.frames.saturating_add(1);
            self.stats.expected = self.stats.expected.saturating_add(1);
            out.push((n, Fill::Received, p));
        }
        self.last = Some(newest);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// `ms` of a stereo tone (440 Hz left, 660 right), from sample `at`.
    fn tone(at: usize, ms: usize) -> Vec<u8> {
        let n = RATE as usize * ms / 1000;
        (at..at + n)
            .flat_map(|i| {
                let t = i as f64 / f64::from(RATE);
                let l = (t * 440.0 * std::f64::consts::TAU).sin() * 12_000.0;
                let r = (t * 660.0 * std::f64::consts::TAU).sin() * 12_000.0;
                [l as i16, r as i16]
            })
            .flat_map(i16::to_le_bytes)
            .collect()
    }

    fn rms(pcm: &[u8]) -> f64 {
        let s: Vec<f64> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f64::from(i16::from_le_bytes(*b)))
            .collect();
        (s.iter().map(|x| x * x).sum::<f64>() / s.len().max(1) as f64).sqrt()
    }

    fn frame_bytes() -> usize {
        RATE as usize * 20 / 1000 * 4
    }

    #[test]
    fn records_pack_and_unpack() {
        let frames = vec![vec![1u8, 2, 3], vec![4u8; 70]];
        let p = pack(41, &frames, Some(&[9, 9]));
        let r = unpack(&p).unwrap();
        assert_eq!(r.first, 41);
        assert_eq!(r.frames, vec![&[1u8, 2, 3][..], &[4u8; 70][..]]);
        assert_eq!(r.red, Some(&[9u8, 9][..]));
        let p = pack(7, &frames[..1], None);
        assert_eq!(unpack(&p).unwrap().red, None);
        // Damaged: cut short, trailing bytes, no frames, bad flag.
        assert!(unpack(&p[..p.len() - 1]).is_none());
        assert!(unpack(&[p.clone(), vec![0]].concat()).is_none());
        assert!(unpack(&[0, 0, 0, 0, 0, 0]).is_none());
        let mut bad = p.clone();
        bad[5] = 2;
        assert!(unpack(&bad).is_none());
        assert!(unpack(&[]).is_none());
    }

    #[test]
    fn a_tone_round_trips_at_a_tenth_of_the_bytes() {
        let mut enc = Enc::new(RATE, 2, Profile::Network { lossy: false }, 0).unwrap();
        let mut dec = Dec::new(RATE, 2).unwrap();
        let mut sent = 0usize;
        let mut pcm_in = 0usize;
        let mut played = Vec::new();
        // Odd-sized input (the capture's chunks aren't 20 ms).
        for i in 0..50 {
            let chunk = tone(i * 720, 15);
            pcm_in += chunk.len();
            for (_, rec) in enc.push(&chunk) {
                sent += rec.len();
                for (_, how, p) in dec.receive(&rec).unwrap() {
                    assert_eq!(how, Fill::Received);
                    played.extend(p);
                }
            }
        }
        // 750 ms in: 37 whole frames out, each exactly 20 ms.
        assert_eq!(played.len(), 37 * frame_bytes());
        assert!(sent * 8 < pcm_in, "{sent} B for {pcm_in} B of PCM");
        // About 96 kbit/s: 37 frames of 20 ms.
        let kbps = sent as f64 * 8.0 / 0.74 / 1000.0;
        assert!((60.0..140.0).contains(&kbps), "{kbps} kbit/s");
        // The tone is still there, about as loud (after the codec's start).
        let a = rms(&tone(0, 400));
        let b = rms(&played[frame_bytes() * 5..frame_bytes() * 25]);
        assert!((b / a - 1.0).abs() < 0.2, "level {a} -> {b}");
        assert_eq!(dec.stats.frames, 37);
        assert_eq!(dec.stats.concealed, 0);
    }

    #[test]
    fn lost_frames_are_filled_by_red_fec_or_plc() {
        let mut enc = Enc::new(RATE, 2, Profile::Network { lossy: true }, 100).unwrap();
        enc.set_loss(10.0);
        assert_eq!(enc.loss(), 10);
        let recs: Vec<(u32, Vec<u8>)> =
            (0..30).flat_map(|i| enc.push(&tone(i * 960, 20))).collect();
        assert_eq!(recs.len(), 30);
        assert_eq!(recs[0].0, 100);
        assert!(unpack(&recs[0].1).unwrap().red.is_none(), "nothing before");
        assert!(unpack(&recs[1].1).unwrap().red.is_some());

        let mut dec = Dec::new(RATE, 2).unwrap();
        let mut got: Vec<(u32, Fill)> = Vec::new();
        let mut lens = Vec::new();
        for (i, (_, rec)) in recs.iter().enumerate() {
            // Lose 5 (RED brings it back); lose 10-12 (PLC, then RED for 12).
            if [5, 10, 11, 12].contains(&i) {
                continue;
            }
            for (n, how, p) in dec.receive(rec).unwrap() {
                got.push((n, how));
                lens.push(p.len());
            }
        }
        // Every frame played once, in order.
        let numbers: Vec<u32> = got.iter().map(|g| g.0).collect();
        assert_eq!(numbers, (100..130).collect::<Vec<u32>>());
        assert!(lens.iter().all(|l| *l == frame_bytes()));
        assert_eq!(got[5].1, Fill::Redundant);
        assert_eq!(got[10].1, Fill::Concealed);
        assert_eq!(got[11].1, Fill::Concealed);
        assert_eq!(got[12].1, Fill::Redundant);
        assert_eq!(dec.stats.redundant, 2);
        assert_eq!(dec.stats.concealed, 2);
        assert_eq!(dec.stats.frames, 26);
        // 4 of 30 lost.
        let loss = dec.take_loss().unwrap();
        assert!((loss - 400.0 / 30.0).abs() < 0.01, "{loss}");
        assert_eq!(dec.take_loss(), None, "counting starts again");

        // Repeats and late frames are dropped.
        assert!(dec.receive(&recs[29].1).unwrap().is_empty());
        assert!(dec.receive(&recs[3].1).unwrap().is_empty());
        assert_eq!(dec.stats.duplicates, 2);
    }

    #[test]
    fn with_no_red_the_next_frames_fec_or_plc_fills_the_gap() {
        // A record without the redundant copy (the first after a pause):
        // the lost frame is rebuilt from the next frame's in-band FEC (Opus
        // conceals when that frame carries none).
        let mut enc = Enc::new(RATE, 1, Profile::Network { lossy: true }, 0).unwrap();
        enc.set_loss(20.0);
        let mono: Vec<u8> = tone(0, 600)
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1]])
            .collect();
        let recs: Vec<Vec<u8>> = mono
            .chunks(RATE as usize / 50 * 2)
            .flat_map(|c| enc.push(c))
            .map(|(_, r)| r)
            .collect();
        let bare = |r: &Vec<u8>| {
            let rec = unpack(r).unwrap();
            let frames: Vec<Vec<u8>> = rec.frames.iter().map(|f| f.to_vec()).collect();
            pack(rec.first, &frames, None)
        };
        let mut dec = Dec::new(RATE, 1).unwrap();
        let mut hows = Vec::new();
        for (i, rec) in recs.iter().enumerate() {
            let rec = match i {
                8 => continue,
                9 => bare(rec),
                _ => rec.clone(),
            };
            hows.extend(dec.receive(&rec).unwrap().into_iter().map(|(n, h, p)| {
                assert_eq!(p.len(), RATE as usize / 50 * 2);
                (n, h)
            }));
        }
        assert_eq!(hows.len(), recs.len());
        assert_eq!(hows[8], (8, Fill::Fec));
        assert_eq!(dec.stats.fec, 1);
        // PLC keeps the sound going rather than silence.
        let mut dec = Dec::new(RATE, 1).unwrap();
        dec.receive(&recs[10]).unwrap();
        let out = dec.receive(&bare(&recs[13])).unwrap();
        let hows: Vec<Fill> = out.iter().map(|o| o.1).collect();
        assert_eq!(hows, [Fill::Concealed, Fill::Fec, Fill::Received]);
        assert!(rms(&out[0].2) > 100.0, "PLC is not silence");
    }

    #[test]
    fn a_long_gap_is_not_filled() {
        let mut enc = Enc::new(RATE, 2, Profile::Network { lossy: true }, 0).unwrap();
        let recs: Vec<Vec<u8>> = (0..20)
            .flat_map(|i| enc.push(&tone(i * 960, 20)))
            .map(|(_, r)| r)
            .collect();
        let mut dec = Dec::new(RATE, 2).unwrap();
        dec.receive(&recs[0]).unwrap();
        let out = dec.receive(&recs[15]).unwrap();
        assert_eq!(out.len(), 1, "only the frame that came");
        assert_eq!(out[0].0, 15);
    }

    #[test]
    fn bluetooth_sends_three_frames_a_record_at_a_low_rate() {
        let mut enc = Enc::new(RATE, 2, Profile::Bluetooth, 0).unwrap();
        let recs: Vec<(u32, Vec<u8>)> =
            (0..30).flat_map(|i| enc.push(&tone(i * 960, 20))).collect();
        assert_eq!(recs.len(), 10);
        assert_eq!(
            recs.iter().map(|r| r.0).collect::<Vec<_>>(),
            (0..30).step_by(3).collect::<Vec<u32>>()
        );
        let bytes: usize = recs.iter().map(|r| r.1.len()).sum();
        let kbps = bytes as f64 * 8.0 / 0.6 / 1000.0;
        assert!(kbps < 45.0, "{kbps} kbit/s over Bluetooth");
        let mut dec = Dec::new(RATE, 2).unwrap();
        let mut n = 0;
        for (_, r) in &recs {
            let rec = unpack(r).unwrap();
            assert_eq!(rec.frames.len(), 3);
            assert!(rec.red.is_none());
            n += dec.receive(r).unwrap().len();
        }
        assert_eq!(n, 30);
        assert_eq!(enc.next(), 30);
    }

    #[test]
    fn the_encoders_loss_follows_the_link() {
        let lossy = Profile::Network { lossy: true };
        assert_eq!(loss_for_encoder(lossy, 0.0), 2);
        assert_eq!(loss_for_encoder(lossy, 7.2), 8);
        assert_eq!(loss_for_encoder(lossy, 80.0), 30);
        assert_eq!(loss_for_encoder(lossy, f32::NAN), 2);
        assert_eq!(loss_for_encoder(Profile::Network { lossy: false }, 9.0), 0);
        assert_eq!(loss_for_encoder(Profile::Bluetooth, 9.0), 0);
        assert!(Enc::new(44_100, 2, lossy, 0).is_none(), "not an Opus rate");
        assert!(Enc::new(RATE, 3, lossy, 0).is_none());
        assert!(codes(24_000) && !codes(22_050));
    }
}
