//! MSB-first bit I/O over 64-bit words.

#[derive(Default)]
pub struct BitWriter {
    words: Vec<u64>,
    cur: u64,
    cur_bits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes the low `n` bits of `value` (`n <= 64`).
    pub fn write(&mut self, value: u64, n: u32) {
        debug_assert!(n <= 64);
        if n == 0 {
            return;
        }
        let v = if n == 64 {
            value
        } else {
            value & ((1u64 << n) - 1)
        };
        let free = 64 - self.cur_bits;
        if n < free {
            self.cur |= v << (free - n);
            self.cur_bits += n;
        } else {
            let spill = n - free;
            self.cur |= v >> spill;
            self.words.push(self.cur);
            self.cur = if spill == 0 { 0 } else { v << (64 - spill) };
            self.cur_bits = spill;
        }
    }

    pub fn bit_len(&self) -> usize {
        self.words.len() * 64 + self.cur_bits as usize
    }

    pub fn into_bytes(mut self) -> Vec<u8> {
        let len = self.bit_len().div_ceil(8);
        if self.cur_bits > 0 {
            self.words.push(self.cur);
        }
        let mut out: Vec<u8> = self.words.iter().flat_map(|w| w.to_be_bytes()).collect();
        out.truncate(len);
        out
    }
}

pub struct BitReader {
    words: Vec<u64>,
    pos: usize,
    len: usize,
}

impl BitReader {
    pub fn new(bytes: &[u8]) -> Self {
        let words = bytes
            .chunks(8)
            .map(|c| {
                let mut b = [0u8; 8];
                b[..c.len()].copy_from_slice(c);
                u64::from_be_bytes(b)
            })
            .collect();
        BitReader {
            words,
            pos: 0,
            len: bytes.len() * 8,
        }
    }

    pub fn read(&mut self, n: u32) -> Option<u64> {
        debug_assert!(n <= 64);
        if n == 0 {
            return Some(0);
        }
        if self.pos + n as usize > self.len {
            return None;
        }
        let (idx, off) = (self.pos / 64, (self.pos % 64) as u32);
        self.pos += n as usize;
        let hi = self.words[idx] << off;
        let v = if off + n <= 64 {
            hi >> (64 - n)
        } else {
            let lo = self.words[idx + 1] >> (128 - off - n);
            (hi >> (64 - n)) | lo
        };
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mixed_widths() {
        let items: Vec<(u64, u32)> = (0..500u64)
            .map(|i| (i.wrapping_mul(0x9E37_79B9_7F4A_7C15), (i % 65) as u32))
            .collect();
        let mut w = BitWriter::new();
        for &(v, n) in &items {
            w.write(v, n);
        }
        let mut r = BitReader::new(&w.into_bytes());
        for &(v, n) in &items {
            let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
            assert_eq!(r.read(n).unwrap(), v & mask);
        }
    }
}
