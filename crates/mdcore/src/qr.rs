//! A small QR code encoder: byte mode, error correction level M, versions
//! 1 to 20 (up to 666 bytes), the mask chosen by the standard's penalty
//! rules (ISO/IEC 18004). Enough for a join link (`crate::join`), which is
//! about 170 bytes (version 9 or 10). Rendered by the apps: the desktop as an
//! SVG path ([`QrCode::svg_path`]), Android on a Canvas from [`QrCode::modules`].

/// Error correction codewords per block, level M, versions 1..=20.
const ECC_PER_BLOCK: [usize; 20] = [
    10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26,
];
/// Error correction blocks, level M, versions 1..=20.
const BLOCKS: [usize; 20] = [
    1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16,
];
pub const MAX_VERSION: usize = 20;
/// Format bits for level M are `00`.
const ECL_M_BITS: u32 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrCode {
    /// Modules per side (21 for version 1, +4 per version).
    pub size: usize,
    pub version: usize,
    /// Row-major, `true` = dark. `size * size` entries.
    pub modules: Vec<bool>,
}

impl QrCode {
    /// Encode `data` as bytes. `None` if it doesn't fit version 20.
    pub fn encode(data: &[u8]) -> Option<QrCode> {
        let version = (1..=MAX_VERSION).find(|&v| {
            let count_bits = if v <= 9 { 8 } else { 16 };
            data.len() < (1 << count_bits)
                && 4 + count_bits + data.len() * 8 <= data_codewords(v) * 8
        })?;
        let codewords = add_ecc_and_interleave(version, &data_bits(version, data));
        let mut qr = Builder::new(version);
        qr.draw_function_patterns();
        qr.draw_codewords(&codewords);
        // Try every mask, keep the one with the lowest penalty.
        let mut best: Option<(u32, u8)> = None;
        for mask in 0..8u8 {
            qr.apply_mask(mask);
            qr.draw_format_bits(mask);
            let p = qr.penalty();
            if best.is_none_or(|(bp, _)| p < bp) {
                best = Some((p, mask));
            }
            qr.apply_mask(mask); // XOR again: undo
        }
        let mask = best.map(|(_, m)| m).unwrap_or(0);
        qr.apply_mask(mask);
        qr.draw_format_bits(mask);
        Some(QrCode {
            size: qr.size,
            version,
            modules: qr.modules,
        })
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    /// The dark modules as one SVG path in module units (one `h` run per
    /// row stretch), with a 4-module quiet zone: use it with
    /// `viewBox="0 0 {size+8} {size+8}"`.
    pub fn svg_path(&self) -> String {
        let mut d = String::new();
        for y in 0..self.size {
            let mut x = 0;
            while x < self.size {
                if self.get(x, y) {
                    let start = x;
                    while x < self.size && self.get(x, y) {
                        x += 1;
                    }
                    d.push_str(&format!(
                        "M{} {}h{}v1h-{}z",
                        start + 4,
                        y + 4,
                        x - start,
                        x - start
                    ));
                } else {
                    x += 1;
                }
            }
        }
        d
    }
}

/// Raw data modules of a version (everything but function patterns).
fn raw_data_modules(ver: usize) -> usize {
    let mut result = (16 * ver + 128) * ver + 64;
    if ver >= 2 {
        let align = ver / 7 + 2;
        result -= (25 * align - 10) * align - 55;
        if ver >= 7 {
            result -= 36;
        }
    }
    result
}

fn data_codewords(ver: usize) -> usize {
    raw_data_modules(ver) / 8 - ECC_PER_BLOCK[ver - 1] * BLOCKS[ver - 1]
}

/// Mode, count, data, terminator and padding, as `data_codewords` bytes.
fn data_bits(ver: usize, data: &[u8]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let push = |val: u32, n: usize, bits: &mut Vec<bool>| {
        for i in (0..n).rev() {
            bits.push((val >> i) & 1 == 1);
        }
    };
    push(0b0100, 4, &mut bits);
    push(data.len() as u32, if ver <= 9 { 8 } else { 16 }, &mut bits);
    for &b in data {
        push(u32::from(b), 8, &mut bits);
    }
    let cap = data_codewords(ver) * 8;
    let term = (cap - bits.len()).min(4);
    push(0, term, &mut bits);
    let pad = (8 - bits.len() % 8) % 8;
    push(0, pad, &mut bits);
    let mut out: Vec<u8> = bits
        .chunks(8)
        .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
        .collect();
    let mut fill = [0xEC, 0x11].into_iter().cycle();
    while out.len() < data_codewords(ver) {
        out.push(fill.next().unwrap_or(0xEC));
    }
    out
}

/// GF(256) multiply, polynomial 0x11D.
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((u32::from(y) >> i) & 1) * u32::from(x);
    }
    z as u8
}

/// Reed–Solomon generator polynomial of `degree` (leading 1 dropped).
fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root: u8 = 1;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    result
}

/// The error correction codewords of `data`.
fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result[0];
        result.remove(0);
        result.push(0);
        for (r, &d) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(d, factor);
        }
    }
    result
}

fn add_ecc_and_interleave(ver: usize, data: &[u8]) -> Vec<u8> {
    let blocks = BLOCKS[ver - 1];
    let ecc_len = ECC_PER_BLOCK[ver - 1];
    let raw = raw_data_modules(ver) / 8;
    let short_blocks = blocks - raw % blocks;
    let short_len = raw / blocks;
    let divisor = rs_divisor(ecc_len);
    let mut all: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut k = 0;
    for i in 0..blocks {
        let dat_len = short_len - ecc_len + usize::from(i >= short_blocks);
        let dat = &data[k..k + dat_len];
        k += dat_len;
        let ecc = rs_remainder(dat, &divisor);
        let mut block = dat.to_vec();
        if i < short_blocks {
            block.push(0); // placeholder, skipped below
        }
        block.extend_from_slice(&ecc);
        all.push(block);
    }
    let mut out = Vec::with_capacity(raw);
    for i in 0..=short_len {
        for (j, block) in all.iter().enumerate() {
            // The short blocks' placeholder byte sits at `short_len - ecc_len`.
            if i != short_len - ecc_len || j >= short_blocks {
                out.push(block[i]);
            }
        }
    }
    out
}

struct Builder {
    size: usize,
    version: usize,
    modules: Vec<bool>,
    is_function: Vec<bool>,
}

impl Builder {
    fn new(version: usize) -> Self {
        let size = version * 4 + 17;
        Builder {
            size,
            version,
            modules: vec![false; size * size],
            is_function: vec![false; size * size],
        }
    }

    fn get(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    fn set_function(&mut self, x: usize, y: usize, dark: bool) {
        self.modules[y * self.size + x] = dark;
        self.is_function[y * self.size + x] = true;
    }

    fn alignment_positions(&self) -> Vec<usize> {
        let ver = self.version;
        if ver == 1 {
            return vec![];
        }
        let num = ver / 7 + 2;
        let step = (ver * 8 + num * 3 + 5) / (num * 4 - 4) * 2;
        let mut result: Vec<usize> = (0..num - 1).map(|i| self.size - 7 - i * step).collect();
        result.push(6);
        result.reverse();
        result
    }

    fn draw_function_patterns(&mut self) {
        let size = self.size;
        for i in 0..size {
            self.set_function(6, i, i % 2 == 0);
            self.set_function(i, 6, i % 2 == 0);
        }
        self.draw_finder(3, 3);
        self.draw_finder(size as i32 - 4, 3);
        self.draw_finder(3, size as i32 - 4);
        let pos = self.alignment_positions();
        let n = pos.len();
        for i in 0..n {
            for j in 0..n {
                // Not on the three finder corners.
                if !((i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0)) {
                    self.draw_alignment(pos[i], pos[j]);
                }
            }
        }
        // Reserve the format areas (drawn per mask later) and draw the version.
        self.draw_format_bits(0);
        self.draw_version();
    }

    fn draw_finder(&mut self, cx: i32, cy: i32) {
        for dy in -4..=4i32 {
            for dx in -4..=4i32 {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < self.size && (y as usize) < self.size {
                    let dist = dx.abs().max(dy.abs());
                    self.set_function(x as usize, y as usize, dist != 2 && dist != 4);
                }
            }
        }
    }

    fn draw_alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let dist = dx.abs().max(dy.abs());
                self.set_function(
                    (cx as i32 + dx) as usize,
                    (cy as i32 + dy) as usize,
                    dist != 1,
                );
            }
        }
    }

    fn draw_format_bits(&mut self, mask: u8) {
        let bits = format_bits(mask);
        let bit = |i: usize| (bits >> i) & 1 == 1;
        let size = self.size;
        for i in 0..=5 {
            self.set_function(8, i, bit(i));
        }
        self.set_function(8, 7, bit(6));
        self.set_function(8, 8, bit(7));
        self.set_function(7, 8, bit(8));
        for i in 9..15 {
            self.set_function(14 - i, 8, bit(i));
        }
        for i in 0..8 {
            self.set_function(size - 1 - i, 8, bit(i));
        }
        for i in 8..15 {
            self.set_function(8, size - 15 + i, bit(i));
        }
        self.set_function(8, size - 8, true); // the dark module
    }

    fn draw_version(&mut self) {
        if self.version < 7 {
            return;
        }
        let bits = version_bits(self.version);
        for i in 0..18 {
            let dark = (bits >> i) & 1 == 1;
            let a = self.size - 11 + i % 3;
            let b = i / 3;
            self.set_function(a, b, dark);
            self.set_function(b, a, dark);
        }
    }

    fn draw_codewords(&mut self, data: &[u8]) {
        let size = self.size as i32;
        let mut i = 0usize;
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert } as usize;
                    if !self.is_function[y * self.size + x] && i < data.len() * 8 {
                        self.modules[y * self.size + x] = (data[i >> 3] >> (7 - (i & 7))) & 1 == 1;
                        i += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                let invert = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                let k = y * self.size + x;
                if invert && !self.is_function[k] {
                    self.modules[k] = !self.modules[k];
                }
            }
        }
    }

    /// The standard's penalty score (lower is better).
    fn penalty(&self) -> u32 {
        let size = self.size;
        let mut result = 0u32;
        // Rules 1 and 3 on rows, then on columns.
        for horizontal in [true, false] {
            for a in 0..size {
                let at = |b: usize| {
                    if horizontal {
                        self.get(b, a)
                    } else {
                        self.get(a, b)
                    }
                };
                let line: Vec<bool> = (0..size).map(at).collect();
                // Rule 1: runs of 5+ same-colored modules.
                let mut run = 1;
                for b in 1..size {
                    if line[b] == line[b - 1] {
                        run += 1;
                        if run == 5 {
                            result += 3;
                        } else if run > 5 {
                            result += 1;
                        }
                    } else {
                        run = 1;
                    }
                }
                // Rule 3: finder-like 1:1:3:1:1 with 4 light on a side.
                let light = |i: isize| i < 0 || i >= size as isize || !line[i as usize];
                for b in 0..size as isize {
                    let core = [true, false, true, true, true, false, true];
                    if b + 7 > size as isize {
                        break;
                    }
                    if (0..7).all(|k| line[(b + k) as usize] == core[k as usize])
                        && ((1..=4).all(|k| light(b - k)) || (1..=4).all(|k| light(b + 6 + k)))
                    {
                        result += 40;
                    }
                }
            }
        }
        // Rule 2: 2x2 blocks of one color.
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1)
                {
                    result += 3;
                }
            }
        }
        // Rule 4: balance of dark and light.
        let dark = self.modules.iter().filter(|&&m| m).count();
        let total = size * size;
        let k = (dark * 20)
            .abs_diff(total * 10)
            .div_ceil(total)
            .saturating_sub(1);
        result + k as u32 * 10
    }
}

/// 15 format bits: level and mask, BCH(15,5), XOR 0x5412.
fn format_bits(mask: u8) -> u32 {
    let data = (ECL_M_BITS << 3) | u32::from(mask);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

/// 18 version bits (versions 7+): BCH(18,6).
fn version_bits(ver: usize) -> u32 {
    let mut rem = ver as u32;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    ((ver as u32) << 12) | rem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_matches_the_standard() {
        // Total codewords per version (ISO/IEC 18004 table 1).
        let totals = [
            26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901,
            991, 1085,
        ];
        for v in 1..=MAX_VERSION {
            assert_eq!(raw_data_modules(v) / 8, totals[v - 1], "version {v}");
        }
        // Data codewords at level M (table 7), e.g. 16 for 1-M, 216 for 10-M.
        assert_eq!(data_codewords(1), 16);
        assert_eq!(data_codewords(7), 124);
        assert_eq!(data_codewords(10), 216);
        assert_eq!(data_codewords(20), 669);
    }

    #[test]
    fn reed_solomon_matches_the_worked_example() {
        // "HELLO WORLD" as 1-M (the well-known worked example).
        let data = [
            32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17,
        ];
        assert_eq!(
            rs_remainder(&data, &rs_divisor(10)),
            vec![196, 35, 39, 119, 235, 215, 231, 226, 93, 23]
        );
    }

    #[test]
    fn format_and_version_bits_match_the_tables() {
        assert_eq!(format_bits(0), 0b101010000010010);
        assert_eq!(format_bits(5), 0b100000011001110);
        assert_eq!(format_bits(7), 0b100101010100000);
        assert_eq!(version_bits(7), 0x07C94);
        assert_eq!(version_bits(10), 0x0A4D3);
    }

    #[test]
    fn alignment_patterns_sit_where_the_table_says() {
        assert_eq!(Builder::new(2).alignment_positions(), vec![6, 18]);
        assert_eq!(Builder::new(7).alignment_positions(), vec![6, 22, 38]);
        assert_eq!(Builder::new(10).alignment_positions(), vec![6, 28, 50]);
        assert_eq!(Builder::new(16).alignment_positions(), vec![6, 26, 50, 74]);
        assert_eq!(Builder::new(20).alignment_positions(), vec![6, 34, 62, 90]);
    }

    #[test]
    fn padding_follows_the_rules() {
        let bits = data_bits(1, b"A");
        // 0100 00000001 01000001 0000 then pad bytes.
        assert_eq!(&bits[..3], &[0x40, 0x14, 0x10]);
        assert_eq!(&bits[3..7], &[0xEC, 0x11, 0xEC, 0x11]);
        assert_eq!(bits.len(), 16);
    }

    /// Read the data back out of a symbol (unmask, unzigzag, de-interleave)
    /// and check the codewords and their error correction.
    fn decode(qr: &QrCode) -> Vec<u8> {
        let ver = qr.version;
        let mut b = Builder::new(ver);
        b.draw_function_patterns();
        // The mask from the format bits (top-left copy, bits 10..12 xor'd).
        let mut fmt = 0u32;
        for i in 0..=5 {
            fmt |= u32::from(qr.get(8, i)) << i;
        }
        fmt |= (u32::from(qr.get(8, 7)) << 6)
            | (u32::from(qr.get(8, 8)) << 7)
            | (u32::from(qr.get(7, 8)) << 8);
        for i in 9..15 {
            fmt |= u32::from(qr.get(14 - i, 8)) << i;
        }
        let mask = (((fmt ^ 0x5412) >> 10) & 7) as u8;
        assert_eq!(format_bits(mask), fmt, "format bits intact");
        b.modules = qr.modules.clone();
        b.apply_mask(mask);
        let mut bits = Vec::new();
        let size = b.size as i32;
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert } as usize;
                    if !b.is_function[y * b.size + x] {
                        bits.push(b.get(x, y));
                    }
                }
            }
            right -= 2;
        }
        let raw = raw_data_modules(ver) / 8;
        let bytes: Vec<u8> = bits[..raw * 8]
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |a, &x| (a << 1) | u8::from(x)))
            .collect();
        // De-interleave.
        let blocks = BLOCKS[ver - 1];
        let ecc_len = ECC_PER_BLOCK[ver - 1];
        let short_blocks = blocks - raw % blocks;
        let short_len = raw / blocks;
        let mut per: Vec<Vec<u8>> = vec![Vec::new(); blocks];
        let mut k = 0;
        for i in 0..=short_len {
            for (j, blk) in per.iter_mut().enumerate() {
                if i != short_len - ecc_len || j >= short_blocks {
                    blk.push(bytes[k]);
                    k += 1;
                }
            }
        }
        let mut data = Vec::new();
        for blk in per {
            let (d, e) = blk.split_at(blk.len() - ecc_len);
            assert_eq!(rs_remainder(d, &rs_divisor(ecc_len)), e, "error correction");
            data.extend_from_slice(d);
        }
        // Byte mode, then the count, then the bytes.
        let bit = |i: usize| (data[i / 8] >> (7 - i % 8)) & 1;
        let read = |from: usize, n: usize| {
            (from..from + n).fold(0usize, |a, i| (a << 1) | usize::from(bit(i)))
        };
        assert_eq!(read(0, 4), 0b0100);
        let cb = if ver <= 9 { 8 } else { 16 };
        let len = read(4, cb);
        (0..len).map(|i| read(4 + cb + i * 8, 8) as u8).collect()
    }

    #[test]
    fn encodes_and_reads_back_at_every_size() {
        for len in [0usize, 1, 14, 15, 60, 120, 170, 213, 300, 500, 666] {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 % 251) as u8).collect();
            let qr = QrCode::encode(&data).unwrap();
            assert_eq!(qr.size, qr.version * 4 + 17);
            assert_eq!(decode(&qr), data, "len {len}");
        }
        assert!(QrCode::encode(&[0u8; 667]).is_none());
        // Smallest version that fits: 14 bytes fit 1-M, 15 don't.
        assert_eq!(QrCode::encode(&[1u8; 14]).unwrap().version, 1);
        assert_eq!(QrCode::encode(&[1u8; 15]).unwrap().version, 2);
    }

    #[test]
    fn finder_patterns_and_timing_are_drawn() {
        let qr = QrCode::encode(b"mydomain://join?x=1").unwrap();
        let n = qr.size;
        for (cx, cy) in [(3, 3), (n - 4, 3), (3, n - 4)] {
            assert!(qr.get(cx, cy), "finder center");
            assert!(!qr.get(cx + 2, cy), "finder ring");
            assert!(qr.get(cx + 3, cy), "finder edge");
        }
        for i in 8..n - 8 {
            assert_eq!(qr.get(i, 6), i % 2 == 0);
            assert_eq!(qr.get(6, i), i % 2 == 0);
        }
        assert!(qr.get(8, n - 8), "dark module");
        let path = qr.svg_path();
        assert!(path.starts_with("M4 4h7v1h-7z"), "{path}");
    }
}
