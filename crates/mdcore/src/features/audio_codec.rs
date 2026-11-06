//! Smaller, sturdier audio frames for the binary media path
//! (`transport::media`, kind 1): IMA ADPCM (4 bits a sample, a quarter of
//! s16 PCM) and RED-style redundancy (each packet also carries the one
//! before it, so a single lost DATAGRAM is recovered at the other end).
//!
//! Only to a device that says `audio_codec: 1` in its status and
//! `addr_update`: an older app reads every kind-1 record as PCM. The
//! header's `f1` (zero in records from before) says what the payload is:
//!
//! ```text
//! f1 bit 0  ADPCM: payload = [u16 samples per channel][block per channel]
//!                  block   = [i16 predictor][u8 step index][u8 0][nibbles, low first]
//! f1 bit 1  RED:   payload = [u16 primary len][primary][u8 seq back][redundant]
//!                  (the redundant part is the record `seq back` before, coded the same)
//! ```
//!
//! Each block starts with the coder's state, so a block decodes alone (a
//! lost one costs only itself), while the sender's coder runs on across
//! blocks (no click at block starts). The Android app has the same coder
//! (`ImaAdpcm.kt`), checked against the same vectors (`VECTOR_*`).

/// `f1` flag: the payload is IMA ADPCM.
pub const FLAG_ADPCM: u16 = 1;
/// `f1` flag: the payload carries the record before it too.
pub const FLAG_RED: u16 = 2;
/// Every flag this build reads.
pub const KNOWN_FLAGS: u16 = FLAG_ADPCM | FLAG_RED;

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_STEP: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// One channel's coder state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub predictor: i32,
    pub index: i32,
}

impl State {
    /// The value a nibble stands for, and the state after it.
    fn step(&mut self, nibble: u8) -> i16 {
        let step = STEPS[self.index as usize];
        let mut delta = step >> 3;
        if nibble & 4 != 0 {
            delta += step;
        }
        if nibble & 2 != 0 {
            delta += step >> 1;
        }
        if nibble & 1 != 0 {
            delta += step >> 2;
        }
        self.predictor = if nibble & 8 != 0 {
            self.predictor - delta
        } else {
            self.predictor + delta
        }
        .clamp(-32768, 32767);
        self.index = (self.index + INDEX_STEP[usize::from(nibble & 15)]).clamp(0, 88);
        self.predictor as i16
    }

    /// The nibble for `sample`, moving the state as the decoder will.
    fn encode(&mut self, sample: i16) -> u8 {
        let step = STEPS[self.index as usize];
        let mut diff = i32::from(sample) - self.predictor;
        let mut nibble = 0u8;
        if diff < 0 {
            nibble = 8;
            diff = -diff;
        }
        if diff >= step {
            nibble |= 4;
            diff -= step;
        }
        if diff >= step >> 1 {
            nibble |= 2;
            diff -= step >> 1;
        }
        if diff >= step >> 2 {
            nibble |= 1;
        }
        self.step(nibble);
        nibble
    }
}

/// The sender's coder: one state per channel, running on across frames.
#[derive(Clone, Debug, Default)]
pub struct Encoder {
    states: Vec<State>,
}

impl Encoder {
    /// Code `pcm` (s16le, `channels` interleaved) as one ADPCM payload.
    pub fn encode(&mut self, pcm: &[u8], channels: u8) -> Vec<u8> {
        let ch = usize::from(channels.max(1));
        if self.states.len() != ch {
            self.states = vec![State::default(); ch];
        }
        let samples: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        let n = samples.len() / ch;
        let n16 = u16::try_from(n).unwrap_or(u16::MAX);
        let n = usize::from(n16);
        let mut out = Vec::with_capacity(2 + ch * (4 + n.div_ceil(2)));
        out.extend_from_slice(&n16.to_le_bytes());
        for (c, st) in self.states.iter_mut().enumerate() {
            out.extend_from_slice(&(st.predictor as i16).to_le_bytes());
            out.push(st.index as u8);
            out.push(0);
            let mut byte = 0u8;
            for i in 0..n {
                let nib = st.encode(samples[i * ch + c]);
                if i % 2 == 0 {
                    byte = nib;
                } else {
                    out.push(byte | (nib << 4));
                }
            }
            if n % 2 == 1 {
                out.push(byte);
            }
        }
        out
    }
}

/// Bytes an ADPCM payload of `samples` per channel takes.
pub fn encoded_len(samples: usize, channels: u8) -> usize {
    2 + usize::from(channels.max(1)) * (4 + samples.div_ceil(2))
}

/// Decode an ADPCM payload into s16le PCM (`channels` interleaved);
/// `None` when it's damaged.
pub fn decode(data: &[u8], channels: u8) -> Option<Vec<u8>> {
    let ch = usize::from(channels);
    if ch == 0 || data.len() < 2 {
        return None;
    }
    let n = usize::from(u16::from_le_bytes([data[0], data[1]]));
    let block = 4 + n.div_ceil(2);
    if data.len() != 2 + ch * block {
        return None;
    }
    let mut out = vec![0u8; n * ch * 2];
    for c in 0..ch {
        let b = &data[2 + c * block..2 + (c + 1) * block];
        let mut st = State {
            predictor: i32::from(i16::from_le_bytes([b[0], b[1]])),
            index: i32::from(b[2]),
        };
        if st.index > 88 {
            return None;
        }
        for i in 0..n {
            let byte = b[4 + i / 2];
            let nib = if i % 2 == 0 { byte & 15 } else { byte >> 4 };
            let s = st.step(nib);
            let at = (i * ch + c) * 2;
            out[at..at + 2].copy_from_slice(&s.to_le_bytes());
        }
    }
    Some(out)
}

/// Put a record's payload and the one `back` records before it together.
pub fn pack_red(primary: &[u8], back: u8, redundant: &[u8]) -> Vec<u8> {
    let len = u16::try_from(primary.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(3 + primary.len() + redundant.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&primary[..usize::from(len)]);
    out.push(back);
    out.extend_from_slice(redundant);
    out
}

/// Split a RED payload: (primary, how many records back, redundant).
pub fn unpack_red(data: &[u8]) -> Option<(&[u8], u8, &[u8])> {
    let len = usize::from(u16::from_le_bytes([*data.first()?, *data.get(1)?]));
    let primary = data.get(2..2 + len)?;
    let back = *data.get(2 + len)?;
    let redundant = data.get(3 + len..)?;
    if back == 0 {
        return None;
    }
    Some((primary, back, redundant))
}

/// Test vectors shared with the Android app's `ImaAdpcmTest`: a sample
/// sequence and the exact bytes its first (mono) block codes to.
pub const VECTOR_PCM: [i16; 16] = [
    0, 1000, 3000, 6000, 8000, 7000, 3000, -2000, -8000, -12000, -9000, -3000, 0, 32767, -32768,
    100,
];
/// `Encoder::default().encode(VECTOR_PCM, 1)`.
pub const VECTOR_ADPCM: [u8; 14] = [16, 0, 0, 0, 0, 0, 112, 119, 119, 247, 255, 48, 113, 31];
/// `decode(VECTOR_ADPCM, 1)`, as s16.
pub const VECTOR_DECODED: [i16; 16] = [
    0, 11, 41, 104, 240, 533, 1164, -193, -3103, -9339, -8448, -2775, -566, 9479, -12057, -2825,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn le(s: &[i16]) -> Vec<u8> {
        s.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn samples(b: &[u8]) -> Vec<i16> {
        b.as_chunks::<2>()
            .0
            .iter()
            .map(|x| i16::from_le_bytes(*x))
            .collect()
    }

    #[test]
    fn the_shared_vector_codes_the_same_as_on_android() {
        let got = Encoder::default().encode(&le(&VECTOR_PCM), 1);
        assert_eq!(got, VECTOR_ADPCM);
        // And decodes to the agreed samples.
        let back = samples(&decode(&got, 1).unwrap());
        assert_eq!(back, VECTOR_DECODED);
    }

    #[test]
    fn a_sine_round_trips_closely_and_is_a_quarter_the_size() {
        let mut enc = Encoder::default();
        let mut worst = 0i32;
        let mut t = 0usize;
        for _frame in 0..20 {
            let s: Vec<i16> = (0..960)
                .flat_map(|_| {
                    let v = ((t as f64 * 440.0 * 2.0 * std::f64::consts::PI / 48_000.0).sin()
                        * 12_000.0) as i16;
                    t += 1;
                    [v, v / 2]
                })
                .collect();
            let pcm = le(&s);
            let a = enc.encode(&pcm, 2);
            assert_eq!(a.len(), encoded_len(960, 2));
            assert!(a.len() * 4 <= pcm.len() + 40);
            let back = samples(&decode(&a, 2).unwrap());
            if _frame > 0 {
                for (x, y) in s.iter().zip(&back) {
                    worst = worst.max((i32::from(*x) - i32::from(*y)).abs());
                }
            }
        }
        assert!(worst < 1200, "error {worst}");
    }

    #[test]
    fn each_block_decodes_alone() {
        let mut enc = Encoder::default();
        let a = enc.encode(&le(&[100; 64]), 1);
        let b = enc.encode(&le(&[5000; 64]), 1);
        let only_b = samples(&decode(&b, 1).unwrap());
        // Decoded without `a`: starts where the coder was, not from zero.
        assert!(only_b[0] > 100, "{}", only_b[0]);
        assert!(decode(&a, 1).is_some());
    }

    #[test]
    fn damaged_payloads_are_refused() {
        let a = Encoder::default().encode(&le(&[1, 2, 3, 4]), 2);
        assert!(decode(&a[..a.len() - 1], 2).is_none());
        assert!(decode(&a, 1).is_none());
        assert!(decode(&[], 1).is_none());
        let mut bad = a.clone();
        bad[4] = 200; // step index out of range
        assert!(decode(&bad, 2).is_none());
        // Odd sample counts.
        let odd = Encoder::default().encode(&le(&[1, 2, 3]), 1);
        assert_eq!(samples(&decode(&odd, 1).unwrap()).len(), 3);
    }

    #[test]
    fn red_packs_and_unpacks() {
        let p = pack_red(b"now", 1, b"before");
        assert_eq!(unpack_red(&p), Some((&b"now"[..], 1, &b"before"[..])));
        assert!(unpack_red(&p[..3]).is_none());
        assert!(unpack_red(&pack_red(b"x", 0, b"y")).is_none());
        assert!(unpack_red(&[]).is_none());
    }
}
