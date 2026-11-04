//! The call audio of one SCO link as plain samples.
//!
//! The SCO socket carries 16-bit little-endian mono PCM at the codec's rate
//! (CVSD: 8 kHz; the radio does the CVSD coding). The link's pump hands
//! what arrives to [`ScoPcm::from_air`] and writes back [`ScoPcm::to_air`]
//! of the same length, so this side sends exactly as fast as the phone does
//! (the usual way to pace SCO). The app, meanwhile, installs a sink for the
//! caller's voice and pushes its microphone with [`ScoPcm::push_mic`].

use std::collections::VecDeque;
use std::sync::Mutex;

/// Receives the caller's voice: mono samples in -1..1 at [`ScoPcm::rate`].
pub type Sink = Box<dyn FnMut(&[f32]) + Send>;

/// The microphone queue's limit, in milliseconds: more than this waiting
/// means the microphone runs ahead of the link; the oldest goes.
const MIC_MAX_MS: u32 = 120;

pub struct ScoPcm {
    rate: u32,
    sink: Mutex<Option<Sink>>,
    mic: Mutex<VecDeque<f32>>,
}

impl ScoPcm {
    pub fn new(rate: u32) -> Self {
        ScoPcm {
            rate,
            sink: Mutex::new(None),
            mic: Mutex::new(VecDeque::new()),
        }
    }

    /// Samples per second each way.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Where the caller's voice goes (`None`: nowhere).
    pub fn set_sink(&self, sink: Option<Sink>) {
        if let Ok(mut s) = self.sink.lock() {
            *s = sink;
        }
    }

    /// Microphone samples (mono, -1..1, at [`ScoPcm::rate`]) for the phone.
    pub fn push_mic(&self, samples: &[f32]) {
        let cap = (self.rate * MIC_MAX_MS / 1000) as usize;
        if let Ok(mut q) = self.mic.lock() {
            q.extend(samples.iter().copied());
            let extra = q.len().saturating_sub(cap);
            q.drain(..extra);
        }
    }

    /// A packet from the phone (16-bit LE PCM).
    pub fn from_air(&self, bytes: &[u8]) {
        let samples: Vec<f32> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
            .collect();
        if let Ok(mut s) = self.sink.lock() {
            if let Some(sink) = s.as_mut() {
                sink(&samples);
            }
        }
    }

    /// `len` bytes for the phone (16-bit LE PCM): the microphone, silence
    /// where it hasn't caught up.
    pub fn to_air(&self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut q = self.mic.lock().ok();
        for _ in 0..len / 2 {
            let s = q.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0);
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.resize(len, 0);
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn pcm_both_ways() {
        let pcm = ScoPcm::new(8000);
        assert_eq!(pcm.rate(), 8000);
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        // Nowhere to go yet: dropped.
        pcm.from_air(&[0, 0x40]);
        pcm.set_sink(Some(Box::new(move |s| {
            g.lock().unwrap().extend_from_slice(s)
        })));
        pcm.from_air(&[0x00, 0x40, 0x00, 0xC0, 0xFF, 0x7F, 0x01]);
        assert_eq!(*got.lock().unwrap(), vec![0.5, -0.5, 32767.0 / 32768.0]);
        pcm.set_sink(None);
        pcm.from_air(&[0, 0x40]);
        assert_eq!(got.lock().unwrap().len(), 3);

        // Microphone out: as much as asked, silence after, clamped.
        pcm.push_mic(&[1.0, -2.0]);
        assert_eq!(pcm.to_air(6), vec![0xFF, 0x7F, 0x01, 0x80, 0, 0]);
        assert_eq!(pcm.to_air(3), vec![0, 0, 0]);
    }

    #[test]
    fn pcm_mic_queue_is_bounded() {
        let pcm = ScoPcm::new(8000);
        pcm.push_mic(&[0.0; 5000]);
        pcm.push_mic(&[0.5; 960]);
        // 120 ms at 8 kHz = 960 samples: only the newest are left.
        let out = pcm.to_air(960 * 2);
        assert!(out.chunks(2).all(|b| b == [0x00, 0x40]));
        assert_eq!(pcm.to_air(2), vec![0, 0]);
    }
}
