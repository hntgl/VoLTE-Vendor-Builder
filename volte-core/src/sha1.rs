//! SHA-1 (FIPS 180-4), written here instead of pulling a crate so V1.0 stays
//! dependency-free. Only used to rebuild AVB hashtrees on images that were
//! signed with `sha1`; it is not exposed for new signatures.

const H0: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];

/// Streaming SHA-1 state.
pub struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Default for Sha1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha1 {
    pub fn new() -> Self {
        Sha1 {
            h: H0,
            buf: [0u8; 64],
            buf_len: 0,
            total: 0,
        }
    }

    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.total.wrapping_mul(8);
        let pad_len = if self.buf_len < 56 {
            56 - self.buf_len
        } else {
            120 - self.buf_len
        };
        let mut pad = [0u8; 64];
        pad[0] = 0x80;
        self.update(&pad[..pad_len]);
        self.update(&bits.to_be_bytes());
        debug_assert_eq!(self.buf_len, 0);
        let mut out = [0u8; 20];
        for (i, word) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        for (dst, add) in self.h.iter_mut().zip([a, b, c, d, e]) {
            *dst = dst.wrapping_add(add);
        }
    }
}

/// One-shot digest.
pub fn digest(data: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(data);
    h.finish()
}

/// Upper-case hex of a byte slice.
pub fn hex_upper(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0F) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: &str = "DA39A3EE5E6B4B0D3255BFEF95601890AFD80709";
    const ABC: &str = "A9993E364706816ABA3E25717850C26C9CD0D89D";
    const MSG448: &str = "84983E441C3BD26EBAAE4AA1F95129E5E54670F1";
    const MSG896: &str = "A49B2446A02C645BF419F995B67091253A04A259";
    const MILLION_A: &str = "34AA973CD4C4DAA4F61EEB2BDBAD27316534016F";
    const FOX: &str = "2FD4E1C67A2D28FCED849EE1BB76E7391B93EB12";

    fn hex_of(data: &[u8]) -> String {
        hex_upper(&digest(data))
    }

    #[test]
    fn nist_empty_message() {
        assert_eq!(hex_of(b""), EMPTY);
    }

    #[test]
    fn nist_abc() {
        assert_eq!(hex_of(b"abc"), ABC);
    }

    #[test]
    fn nist_448_bit_message() {
        let m = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(m.len(), 56);
        assert_eq!(hex_of(m), MSG448);
    }

    #[test]
    fn nist_896_bit_message() {
        let m = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
        assert_eq!(m.len(), 112);
        assert_eq!(hex_of(m), MSG896);
    }

    #[test]
    fn nist_one_million_a_repeating_block() {
        let block = [b'a'; 1000];
        let mut h = Sha1::new();
        for _ in 0..1000 {
            h.update(&block);
        }
        assert_eq!(hex_upper(&h.finish()), MILLION_A);
    }

    #[test]
    fn known_fox_vector() {
        assert_eq!(hex_of(b"The quick brown fox jumps over the lazy dog"), FOX);
    }

    #[test]
    fn padding_boundaries_round_trip_through_streaming() {
        for len in [
            0usize, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 129, 1000,
        ] {
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let one_shot = digest(&data);
            let mut streamed = Sha1::new();
            for byte in data.chunks(7) {
                streamed.update(byte);
            }
            assert_eq!(streamed.finish(), one_shot, "length {len}");
        }
    }

    #[test]
    fn hex_is_upper_case() {
        assert_eq!(hex_upper(&[0xde, 0xad, 0xbe, 0xef]), "DEADBEEF");
    }
}
