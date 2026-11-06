//! The receiving end of a device's audio: an adaptive jitter buffer with
//! loss concealment and clock-drift compensation. The desktop plays a
//! phone's audio through it (`phone_audio.rs`); the Android app has the
//! same rules in `AudioJitter.kt` (a computer's sound on the phone), with
//! the same constants and tests.
//!
//! - **Adaptive target.** Playing starts once `target_ms` is buffered. Running
//!   dry grows the target by `GROW_UNDERRUN_MS`, a frame that came too late
//!   by `GROW_LATE_MS` (fast), up to `CEIL_MS`; `STABLE_MS` of playing with
//!   neither shrinks it by `SHRINK_MS` (slowly), down to the floor:
//!   `FLOOR_MS`, or `FLOOR_NEARBY_MS` on a nearby low-latency link
//!   (`features::nearby`), which is all nearby mode changes here.
//!   Over Bluetooth (`features::opus_audio`) the floor is `FLOOR_BLE_MS`.
//! - **Concealment.** Running dry doesn't go silent at once: the last
//!   `HISTORY_MS` played repeats, fading out over `CONCEAL_MS`; when audio
//!   comes back it crossfades in over `XFADE_MS` (no click). Only after
//!   that does it wait for the target again.
//! - **Drift.** The two devices' clocks never quite agree, so the buffer
//!   slowly fills or drains. Instead of dropping or inserting whole frames,
//!   it plays a tiny bit faster or slower (at most `DRIFT_MAX`, ±0.5 %,
//!   linear interpolation) to keep the fill at the target. Only a pile-up
//!   far past the ceiling (a burst after a stall) is trimmed.
//!
//! Time is counted in output frames, so the rules are the same whatever
//! the output rate, and tests run without a clock.

use std::collections::VecDeque;

/// The target a stream starts with.
pub const START_MS: u32 = 80;
/// The lowest target over the internet or the relay.
pub const FLOOR_MS: u32 = 60;
/// The lowest target on a nearby low-latency link.
pub const FLOOR_NEARBY_MS: u32 = 30;
/// The lowest target over a Bluetooth link (Opus records come three
/// frames, 60 ms, at a time).
pub const FLOOR_BLE_MS: u32 = 150;
/// The highest target.
pub const CEIL_MS: u32 = 200;
/// Running dry grows the target this much.
pub const GROW_UNDERRUN_MS: u32 = 20;
/// A frame that came too late (after a later one) grows it this much.
pub const GROW_LATE_MS: u32 = 10;
/// This long without trouble shrinks it by `SHRINK_MS`.
pub const STABLE_MS: u32 = 5_000;
pub const SHRINK_MS: u32 = 10;
/// The longest stretch concealed by repeating, fading out.
pub const CONCEAL_MS: u32 = 60;
/// What's repeated: the last this many ms played.
pub const HISTORY_MS: u32 = 10;
/// Crossfade from concealment (or silence) back to the stream.
pub const XFADE_MS: u32 = 5;
/// Most the playback speed is nudged for drift (0.5 %).
pub const DRIFT_MAX: f64 = 0.005;
/// How hard a fill off the target pushes the speed.
const DRIFT_GAIN: f64 = 0.01;
/// How quickly the fill estimate follows (per `fill` call).
const FILL_SMOOTH: f64 = 0.02;

/// The playback speed that steers a buffer at `fill_ms` towards
/// `target_ms`: above 1 plays faster (drains), below 1 slower.
pub fn drift_ratio(fill_ms: f64, target_ms: f64) -> f64 {
    if target_ms <= 0.0 {
        return 1.0;
    }
    let err = (fill_ms - target_ms) / target_ms;
    1.0 + (err * DRIFT_GAIN).clamp(-DRIFT_MAX, DRIFT_MAX)
}

/// The adaptive target after an event (pure, for tests and both apps).
pub fn adapt(target_ms: u32, floor_ms: u32, event: Event) -> u32 {
    match event {
        Event::Underrun => (target_ms + GROW_UNDERRUN_MS).min(CEIL_MS),
        Event::Late => (target_ms + GROW_LATE_MS).min(CEIL_MS),
        Event::Stable => target_ms.saturating_sub(SHRINK_MS).max(floor_ms),
    }
    .max(floor_ms)
}

/// What moves the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Underrun,
    Late,
    Stable,
}

/// What the audio view shows about the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JitterStats {
    pub buffered_ms: u32,
    pub target_ms: u32,
    /// Times it ran dry while playing.
    pub underruns: u32,
    /// Output frames made up by concealment, in ms.
    pub concealed_ms: u32,
    /// Frames that came after a later one (dropped).
    pub late: u32,
    /// Times a pile-up was trimmed.
    pub trimmed: u32,
    /// The drift speed now, in parts per million off 1 (+: faster).
    pub drift_ppm: i32,
}

/// Audio waiting to be played: stereo frames at the stream's rate, f32.
pub struct Jitter {
    rate: u32,
    channels: u8,
    /// Interleaved L, R.
    buf: VecDeque<f32>,
    next_seq: Option<u64>,
    priming: bool,
    /// Where the next output frame falls between the first two frames.
    pos: f64,
    target_ms: u32,
    floor_ms: u32,
    /// Smoothed fill, ms.
    fill_ms: f64,
    ratio: f64,
    /// Output frames played since the last trouble.
    calm: u64,
    /// The last frames played (L, R), for concealment.
    history: VecDeque<(f32, f32)>,
    /// Concealing: output frames made up so far (None: not concealing).
    concealing: Option<u64>,
    /// Crossfading back in: frames left, and the gain concealment had.
    xfade: Option<(u32, f32)>,
    conceal_at: usize,
    pub underruns: u32,
    pub late: u32,
    pub trimmed: u32,
    concealed_frames: u64,
    concealed_rate: u32,
}

impl Default for Jitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Jitter {
    pub fn new() -> Self {
        Jitter {
            rate: 0,
            channels: 0,
            buf: VecDeque::new(),
            next_seq: None,
            priming: true,
            pos: 0.0,
            target_ms: START_MS,
            floor_ms: FLOOR_MS,
            fill_ms: 0.0,
            ratio: 1.0,
            calm: 0,
            history: VecDeque::new(),
            concealing: None,
            xfade: None,
            conceal_at: 0,
            underruns: 0,
            late: 0,
            trimmed: 0,
            concealed_frames: 0,
            concealed_rate: 48_000,
        }
    }

    /// On a nearby low-latency link the target may shrink lower.
    pub fn set_low_latency(&mut self, on: bool) {
        self.floor_ms = if on { FLOOR_NEARBY_MS } else { FLOOR_MS };
        self.target_ms = self.target_ms.max(self.floor_ms);
    }

    pub fn low_latency(&self) -> bool {
        self.floor_ms == FLOOR_NEARBY_MS
    }

    /// Over a Bluetooth link the target stays higher.
    pub fn set_bluetooth(&mut self, on: bool) {
        self.floor_ms = if on { FLOOR_BLE_MS } else { FLOOR_MS };
        self.target_ms = self.target_ms.max(self.floor_ms);
    }

    pub fn bluetooth(&self) -> bool {
        self.floor_ms == FLOOR_BLE_MS
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn playing(&self) -> bool {
        !self.priming
    }

    pub fn target_ms(&self) -> u32 {
        self.target_ms
    }

    fn frames(&self) -> usize {
        self.buf.len() / 2
    }

    fn frames_for(&self, ms: u32) -> usize {
        (u64::from(self.rate) * u64::from(ms) / 1000) as usize
    }

    /// How much is waiting, in milliseconds.
    pub fn buffered_ms(&self) -> u32 {
        if self.rate == 0 {
            return 0;
        }
        u32::try_from(self.frames() as u64 * 1000 / u64::from(self.rate)).unwrap_or(u32::MAX)
    }

    pub fn stats(&self) -> JitterStats {
        JitterStats {
            buffered_ms: self.buffered_ms(),
            target_ms: self.target_ms,
            underruns: self.underruns,
            concealed_ms: u32::try_from(
                self.concealed_frames * 1000 / u64::from(self.concealed_rate.max(1)),
            )
            .unwrap_or(u32::MAX),
            late: self.late,
            trimmed: self.trimmed,
            drift_ppm: ((self.ratio - 1.0) * 1e6).round() as i32,
        }
    }

    fn trouble(&mut self, e: Event) {
        self.target_ms = adapt(self.target_ms, self.floor_ms, e);
        self.calm = 0;
    }

    /// A frame: `pcm` is s16le, `channels` interleaved, at `rate` Hz. A new
    /// format starts over; a late frame is dropped (and grows the target).
    pub fn push(&mut self, seq: u64, rate: u32, channels: u8, pcm: &[u8]) {
        if rate == 0 || !(1..=2).contains(&channels) {
            return;
        }
        if rate != self.rate || channels != self.channels {
            self.rate = rate;
            self.channels = channels;
            self.buf.clear();
            self.next_seq = None;
            self.priming = true;
            self.pos = 0.0;
        }
        if self.next_seq.is_some_and(|n| seq < n) {
            self.late = self.late.saturating_add(1);
            self.trouble(Event::Late);
            return;
        }
        self.next_seq = Some(seq.saturating_add(1));
        let samples = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0);
        if channels == 1 {
            for s in samples {
                self.buf.extend([s, s]);
            }
        } else {
            self.buf.extend(samples);
            if self.buf.len() % 2 == 1 {
                self.buf.pop_back();
            }
        }
        // Far past the ceiling (a burst after a stall): back to the target.
        if self.frames() > self.frames_for(CEIL_MS + self.target_ms) {
            let excess = self.frames() - self.frames_for(self.target_ms);
            self.buf.drain(..excess * 2);
            self.pos = 0.0;
            self.trimmed = self.trimmed.saturating_add(1);
        }
    }

    /// Fill a device buffer (`out_channels` interleaved at `out_rate`) at
    /// `volume` (0 to 1), resampling linearly (with the drift nudge).
    pub fn fill(&mut self, out: &mut [f32], out_channels: usize, out_rate: u32, volume: f32) {
        let out_channels = out_channels.max(1);
        if self.rate == 0 || out_rate == 0 {
            out.iter_mut().for_each(|o| *o = 0.0);
            return;
        }
        self.concealed_rate = out_rate;
        // Drift: steer the smoothed fill to the target.
        let now_ms = self.frames() as f64 * 1000.0 / f64::from(self.rate);
        self.fill_ms += (now_ms - self.fill_ms) * FILL_SMOOTH;
        self.ratio = if self.priming {
            1.0
        } else {
            drift_ratio(self.fill_ms, f64::from(self.target_ms))
        };
        let step = f64::from(self.rate) / f64::from(out_rate) * self.ratio;
        let conceal_max = u64::from(out_rate) * u64::from(CONCEAL_MS) / 1000;
        let xfade_frames = (out_rate * XFADE_MS / 1000).max(1);
        let history_max = (out_rate * HISTORY_MS / 1000).max(1) as usize;
        let stable = u64::from(out_rate) * u64::from(STABLE_MS) / 1000;
        for frame in out.chunks_mut(out_channels) {
            let (l, r) = self.next(step, conceal_max, xfade_frames, history_max);
            if out_channels == 1 {
                frame[0] = (l + r) * 0.5 * volume;
            } else {
                frame[0] = l * volume;
                frame[1] = r * volume;
                frame[2..].iter_mut().for_each(|o| *o = 0.0);
            }
            if !self.priming && self.concealing.is_none() {
                self.calm += 1;
                if self.calm >= stable {
                    self.trouble(Event::Stable);
                }
            }
        }
    }

    /// One output frame.
    fn next(
        &mut self,
        step: f64,
        conceal_max: u64,
        xfade_frames: u32,
        history_max: usize,
    ) -> (f32, f32) {
        if self.priming {
            if self.frames() >= self.frames_for(self.target_ms).max(2) {
                self.priming = false;
                // Fade in from silence.
                self.xfade = Some((xfade_frames, 0.0));
            } else {
                return (0.0, 0.0);
            }
        }
        let i = self.pos as usize;
        if i + 1 < self.frames() {
            if self.concealing.take().is_some() {
                let gain = self.conceal_gain(conceal_max);
                self.xfade = Some((xfade_frames, gain));
            }
            let f = (self.pos - i as f64) as f32;
            let at = |k: usize| self.buf[k];
            let mut l = at(2 * i) + (at(2 * i + 2) - at(2 * i)) * f;
            let mut r = at(2 * i + 1) + (at(2 * i + 3) - at(2 * i + 1)) * f;
            if let Some((left, from_gain)) = self.xfade {
                let t = 1.0 - left as f32 / xfade_frames as f32;
                let (cl, cr) = self.repeat();
                l = l * t + cl * from_gain * (1.0 - t);
                r = r * t + cr * from_gain * (1.0 - t);
                self.xfade = (left > 1).then_some((left - 1, from_gain));
            }
            self.pos += step;
            let whole = self.pos as usize;
            if whole > 0 {
                self.buf.drain(..(whole * 2).min(self.buf.len()));
                self.pos -= whole as f64;
            }
            self.history.push_back((l, r));
            while self.history.len() > history_max {
                self.history.pop_front();
            }
            return (l, r);
        }
        // Dry.
        let n = match self.concealing {
            Some(n) => n,
            None => {
                self.underruns = self.underruns.saturating_add(1);
                self.trouble(Event::Underrun);
                self.conceal_at = 0;
                0
            }
        };
        if n < conceal_max && !self.history.is_empty() {
            self.concealing = Some(n + 1);
            self.concealed_frames += 1;
            let g = self.conceal_gain(conceal_max);
            let (l, r) = self.repeat();
            return (l * g, r * g);
        }
        self.concealing = None;
        self.xfade = None;
        self.priming = true;
        (0.0, 0.0)
    }

    fn conceal_gain(&self, conceal_max: u64) -> f32 {
        let n = self.concealing.unwrap_or(0);
        1.0 - (n as f32 / conceal_max.max(1) as f32).min(1.0)
    }

    /// The next frame of the repeated history.
    fn repeat(&mut self) -> (f32, f32) {
        if self.history.is_empty() {
            return (0.0, 0.0);
        }
        let k = self.conceal_at % self.history.len();
        self.conceal_at = self.conceal_at.wrapping_add(1);
        self.history[k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn frame(seq: u64, ms: u32, value: i16) -> (u64, Vec<u8>) {
        let n = (RATE * ms / 1000) as usize;
        let pcm = (0..n * 2).flat_map(|_| value.to_le_bytes()).collect();
        (seq, pcm)
    }

    fn play(j: &mut Jitter, ms: u32) -> Vec<f32> {
        let mut out = vec![0f32; (RATE * ms / 1000) as usize * 2];
        j.fill(&mut out, 2, RATE, 1.0);
        out
    }

    #[test]
    fn the_target_grows_fast_and_shrinks_slowly() {
        assert_eq!(adapt(80, FLOOR_MS, Event::Underrun), 100);
        assert_eq!(adapt(80, FLOOR_MS, Event::Late), 90);
        assert_eq!(adapt(195, FLOOR_MS, Event::Underrun), CEIL_MS);
        assert_eq!(adapt(80, FLOOR_MS, Event::Stable), 70);
        assert_eq!(adapt(65, FLOOR_MS, Event::Stable), FLOOR_MS);
        // Nearby mode only lowers the floor.
        assert_eq!(adapt(35, FLOOR_NEARBY_MS, Event::Stable), FLOOR_NEARBY_MS);
        assert_eq!(adapt(30, FLOOR_NEARBY_MS, Event::Underrun), 50);
    }

    #[test]
    fn bluetooth_keeps_the_target_higher() {
        let mut j = Jitter::new();
        j.set_bluetooth(true);
        assert!(j.bluetooth() && !j.low_latency());
        assert_eq!(j.target_ms(), FLOOR_BLE_MS);
        assert_eq!(
            adapt(FLOOR_BLE_MS, FLOOR_BLE_MS, Event::Stable),
            FLOOR_BLE_MS
        );
        j.set_bluetooth(false);
        assert!(!j.bluetooth());
        assert_eq!(j.target_ms(), FLOOR_BLE_MS, "shrinks slowly, not at once");
    }

    #[test]
    fn steady_audio_shrinks_the_target_to_the_floor() {
        let mut j = Jitter::new();
        j.set_low_latency(true);
        let mut seq = 0;
        // 40 s of perfectly paced 10 ms frames, played 10 ms at a time.
        for _ in 0..12 {
            let (s, p) = frame(seq, 10, 1000);
            j.push(s, RATE, 2, &p);
            seq += 1;
        }
        for _ in 0..4000 {
            let (s, p) = frame(seq, 10, 1000);
            j.push(s, RATE, 2, &p);
            seq += 1;
            play(&mut j, 10);
        }
        assert_eq!(j.underruns, 0);
        assert_eq!(j.target_ms(), FLOOR_NEARBY_MS);
    }

    #[test]
    fn running_dry_conceals_then_grows_the_target() {
        let mut j = Jitter::new();
        for seq in 0..10 {
            let (s, p) = frame(seq, 10, 8000);
            j.push(s, RATE, 2, &p);
        }
        let out = play(&mut j, 100); // everything, then dry
        assert_eq!(j.underruns, 1);
        assert_eq!(j.target_ms(), START_MS + GROW_UNDERRUN_MS);
        // Just after the stream ran out: the repeat, not silence.
        let dry_at = out.len() - 2 * (RATE as usize * 20 / 1000);
        assert!(out[dry_at].abs() > 0.05, "{}", out[dry_at]);
        // A whole concealment later: faded to silence, waiting again.
        let out = play(&mut j, 100);
        assert!(out[out.len() - 2].abs() < 1e-6);
        assert!(!j.playing());
        let st = j.stats();
        assert!(
            (CONCEAL_MS - 2..=CONCEAL_MS + 2).contains(&st.concealed_ms),
            "{st:?}"
        );
    }

    #[test]
    fn resuming_after_a_gap_crossfades() {
        let mut j = Jitter::new();
        for seq in 0..9 {
            let (s, p) = frame(seq, 10, 8000);
            j.push(s, RATE, 2, &p);
        }
        play(&mut j, 95); // nearly empty
        play(&mut j, 10); // dry: concealing
        assert!(j.concealing.is_some());
        let (s, p) = frame(9, 10, -8000);
        j.push(s, RATE, 2, &p);
        let out = play(&mut j, 1);
        // First frames back mix the concealment in: no jump to -8000.
        assert!(out[0] > -0.2, "{}", out[0]);
    }

    #[test]
    fn a_late_frame_is_dropped_and_grows_the_target() {
        let mut j = Jitter::new();
        let (_, p) = frame(0, 10, 1);
        j.push(5, RATE, 2, &p);
        j.push(4, RATE, 2, &p);
        assert_eq!(j.late, 1);
        assert_eq!(j.target_ms(), START_MS + GROW_LATE_MS);
    }

    #[test]
    fn drift_is_steered_by_a_tiny_speed_change() {
        assert_eq!(drift_ratio(80.0, 80.0), 1.0);
        assert!(drift_ratio(120.0, 80.0) > 1.0);
        assert!(drift_ratio(40.0, 80.0) < 1.0);
        assert_eq!(drift_ratio(10_000.0, 80.0), 1.0 + DRIFT_MAX);
        assert_eq!(drift_ratio(0.0, 80.0), 1.0 - DRIFT_MAX);
        // A sender 0.3 % fast: the buffer settles instead of piling up,
        // and nothing is trimmed or dropped.
        let mut j = Jitter::new();
        let mut acc = 0f64;
        for seq in 0..20_000u64 {
            // 10 ms of output; the sender delivers 10.03 ms of audio.
            acc += 480.0 * 1.003;
            let n = acc as usize;
            acc -= n as f64;
            let pcm: Vec<u8> = (0..n * 2).flat_map(|_| 100i16.to_le_bytes()).collect();
            j.push(seq, RATE, 2, &pcm);
            play(&mut j, 10);
        }
        assert_eq!(j.trimmed, 0);
        assert!(j.buffered_ms() < CEIL_MS, "{}", j.buffered_ms());
        assert!(j.stats().drift_ppm > 1000, "{:?}", j.stats());
    }

    #[test]
    fn it_resamples_to_the_device_rate_and_mixes_down() {
        // 24 kHz in, 48 kHz out: a ramp stays a ramp (after the fade-in).
        let mut j = Jitter::new();
        let pcm: Vec<u8> = (0..2400i16)
            .flat_map(|i| {
                let v = i * 10;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect();
        j.push(0, 24_000, 2, &pcm);
        let mut out = vec![0f32; 2000 * 2];
        j.fill(&mut out, 2, 48_000, 1.0);
        let left: Vec<f32> = out.iter().step_by(2).skip(240).copied().collect();
        assert!(left.windows(2).all(|w| w[1] >= w[0]), "not monotonic");
        // Mono out, at half volume; six channels: the rest silent.
        let mut j = Jitter::new();
        let pcm: Vec<u8> = (0..4800).flat_map(|_| 8192i16.to_le_bytes()).collect();
        j.push(0, 48_000, 1, &pcm);
        let mut out = vec![1.0; 1000];
        j.fill(&mut out, 1, 48_000, 0.5);
        assert!(out[500..].iter().all(|s| (*s - 0.125).abs() < 1e-3));
        let mut out = vec![1.0; 600];
        j.fill(&mut out, 6, 48_000, 1.0);
        for f in out.chunks(6) {
            assert!((f[0] - 0.25).abs() < 1e-3 && (f[1] - 0.25).abs() < 1e-3);
            assert!(f[2..].iter().all(|s| *s == 0.0));
        }
    }

    #[test]
    fn a_pile_up_far_past_the_ceiling_is_trimmed() {
        let mut j = Jitter::new();
        let (_, p) = frame(0, 100, 1);
        for seq in 0..4 {
            j.push(seq, RATE, 2, &p);
        }
        assert_eq!(j.trimmed, 1);
        assert!(j.buffered_ms() <= START_MS + 100);
    }
}
