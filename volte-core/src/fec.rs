//! Embedded AVB FEC: Reed-Solomon RS(255,K) parity over data + hashtree.
//!
//! The layout follows `system/extras/libfec`: `K = 255 - roots`, one
//! codeword per logical position of `rounds * 4096` bytes, and the
//! interleaved position of segment `j` at position `p` is `j * rounds * 4096
//! + p`. Parity for codeword `p` is stored at `p * roots`.
//!
//! Only the roots values listed in [`TESTED_ROOTS`] are accepted; every
//! value there has an encode + decode round-trip test.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config::BLOCK;
use crate::error::{Error, Result};

/// Roots values this build publishes. Each one is covered by an encode and a
/// single-symbol-error decode test.
pub const TESTED_ROOTS: &[u32] = &[2];

pub fn check_roots(roots: u32) -> Result<()> {
    if TESTED_ROOTS.contains(&roots) {
        return Ok(());
    }
    let tested = TESTED_ROOTS
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Err(Error::build(format!(
        "FEC roots={roots} is not supported (tested values: {tested})"
    )))
}

fn check_input(input_size: u64) -> Result<()> {
    if input_size == 0 {
        return Err(Error::build("FEC input is empty"));
    }
    if !input_size.is_multiple_of(BLOCK) {
        return Err(Error::build(format!(
            "FEC input size {input_size} is not a multiple of the {BLOCK}-byte block"
        )));
    }
    Ok(())
}

fn rounds(input_size: u64, roots: u32) -> u64 {
    let k = 255 - u64::from(roots);
    (input_size / BLOCK).div_ceil(k)
}

/// Number of parity bytes for `input_size` bytes of covered data:
/// `rounds * roots * 4096`.
pub fn parity_size(input_size: u64, roots: u32) -> Result<u64> {
    check_input(input_size)?;
    check_roots(roots)?;
    rounds(input_size, roots)
        .checked_mul(u64::from(roots))
        .and_then(|n| n.checked_mul(BLOCK))
        .ok_or_else(|| {
            Error::build(format!(
                "FEC parity size for {input_size} input bytes overflows"
            ))
        })
}

/// Bytes per interleaved segment (`rounds * 4096`).
fn segment_len(input_size: u64, roots: u32) -> Result<u64> {
    rounds(input_size, roots)
        .checked_mul(BLOCK)
        .ok_or_else(|| Error::build("FEC segment length overflows"))
}

/// Computes the parity for `[0, input_size)` of `image`. The file must be at
/// least `input_size` bytes long; anything beyond is ignored. Positions past
/// the end of a segment are fed as zero symbols, exactly like libfec.
pub fn generate_parity(image: &Path, input_size: u64, roots: u32) -> Result<Vec<u8>> {
    check_input(input_size)?;
    check_roots(roots)?;
    let mut f = File::open(image)?;
    let len = f.metadata()?.len();
    if len < input_size {
        return Err(Error::build(format!(
            "FEC input needs {input_size} bytes but {} is only {len} bytes",
            image.display()
        )));
    }
    generate_parity_from(&mut f, input_size, roots)
}

pub(crate) fn generate_parity_from(
    f: &mut File,
    input_size: u64,
    roots: u32,
) -> Result<Vec<u8>> {
    let seg = segment_len(input_size, roots)?;
    let roots_usize = usize::try_from(roots)
        .map_err(|_| Error::build(format!("FEC roots {roots} does not fit in memory")))?;
    let seg_usize = usize::try_from(seg)
        .map_err(|_| Error::build(format!("FEC segment length {seg} is too large")))?;
    let parity_len = seg_usize
        .checked_mul(roots_usize)
        .ok_or_else(|| Error::build("FEC parity buffer size overflows"))?;
    let mut parity = vec![0u8; parity_len];
    let rs = Rs::new(roots_usize);
    let k = 255 - u64::from(roots);
    let mut buf = vec![0u8; 256 * 1024];

    for j in 0..k {
        let start = j
            .checked_mul(seg)
            .ok_or_else(|| Error::build("FEC segment offset overflows"))?;
        let mut p = 0usize;
        if start < input_size {
            let take_all = (input_size - start).min(seg);
            f.seek(SeekFrom::Start(start))?;
            let mut remaining = take_all;
            while remaining > 0 {
                let take = remaining.min(buf.len() as u64) as usize;
                f.read_exact(&mut buf[..take])?;
                for &byte in &buf[..take] {
                    let at = p * roots_usize;
                    rs.encode(&mut parity[at..at + roots_usize], byte);
                    p += 1;
                }
                remaining -= take as u64;
            }
        }
        while p < seg_usize {
            let at = p * roots_usize;
            rs.encode(&mut parity[at..at + roots_usize], 0);
            p += 1;
        }
    }
    Ok(parity)
}

/// Recomputes the parity from the data on disk and compares it with the bytes
/// stored at `[fec_offset, fec_offset + fec_size)`. `Ok(false)` means the
/// stored parity does not describe the data or hashtree on disk.
pub fn verify_parity(
    image: &Path,
    fec_offset: u64,
    fec_size: u64,
    roots: u32,
) -> Result<bool> {
    let expected = parity_size(fec_offset, roots)?;
    if expected != fec_size {
        return Err(Error::build(format!(
            "FEC size {fec_size} does not match the {expected} bytes implied by offset {fec_offset} and roots={roots}"
        )));
    }
    let generated = generate_parity(image, fec_offset, roots)?;
    let end = fec_offset
        .checked_add(fec_size)
        .ok_or_else(|| Error::build("FEC region end overflows"))?;
    let mut f = File::open(image)?;
    let len = f.metadata()?.len();
    if end > len {
        return Err(Error::build(format!(
            "FEC region ends at {end}, past the {len}-byte image"
        )));
    }
    let mut on_disk = vec![0u8; generated.len()];
    f.seek(SeekFrom::Start(fec_offset))?;
    f.read_exact(&mut on_disk)?;
    Ok(on_disk == generated)
}

/// GF(2^8) with the AOSP field polynomial 0x11d and the generator
/// `(x + 1)(x + alpha)...`, matching libfec's encoder.
struct Rs {
    exp: [u8; 512],
    log: [u8; 256],
    /// Generator polynomial coefficients, constant term first.
    poly: Vec<u8>,
}

impl Rs {
    fn new(roots: usize) -> Self {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11d;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }

        let mut poly = vec![1u8];
        for i in 0..roots {
            let alpha = exp[i % 255];
            let mut next = vec![0u8; poly.len() + 1];
            for (j, &c) in poly.iter().enumerate() {
                next[j] ^= mul(&exp, &log, c, alpha);
                next[j + 1] ^= c;
            }
            poly = next;
        }
        Rs { exp, log, poly }
    }

    fn mul(&self, a: u8, b: u8) -> u8 {
        mul(&self.exp, &self.log, a, b)
    }

    fn encode(&self, parity: &mut [u8], data: u8) {
        let roots = parity.len();
        let feedback = data ^ parity[0];
        if feedback != 0 {
            for j in 1..roots {
                parity[j] ^= self.mul(feedback, self.poly[roots - j]);
            }
        }
        parity.copy_within(1..roots, 0);
        parity[roots - 1] = if feedback != 0 {
            self.mul(feedback, self.poly[0])
        } else {
            0
        };
    }
}

fn mul(exp: &[u8; 512], log: &[u8; 256], a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        exp[log[a as usize] as usize + log[b as usize] as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn pattern(len: usize, seed: usize) -> Vec<u8> {
        (0..len)
            .map(|i| ((i.wrapping_mul(37).wrapping_add(seed)) % 251) as u8)
            .collect()
    }

    fn write_image(dir: &std::path::Path, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join("fec_input.bin");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Evaluates the codeword `data || parity` at alpha^power. A valid RS
    /// codeword is zero at alpha^0 .. alpha^(roots-1).
    fn syndrome(rs: &Rs, power: usize, codeword: &[u8]) -> u8 {
        let mut acc = 0u8;
        for (i, &c) in codeword.iter().enumerate() {
            // The first symbol of the (shortened) code is still the
            // coefficient of x^254.
            let degree = 254 - i;
            let e = (power * degree) % 255;
            acc ^= rs.mul(c, rs.exp[e]);
        }
        acc
    }

    /// Corrects one symbol error (the maximum for roots=2) using the two
    /// syndromes: S0 = e and S1 = e * alpha^degree.
    fn repair_one_error(rs: &Rs, codeword: &mut [u8]) -> bool {
        let s0 = syndrome(rs, 0, codeword);
        if s0 == 0 {
            return true;
        }
        let s1 = syndrome(rs, 1, codeword);
        let degree = {
            let ratio = rs.exp
                [(rs.log[s1 as usize] as usize + 255 - rs.log[s0 as usize] as usize) % 255];
            let mut found = None;
            for e in 0..255 {
                if rs.exp[e] == ratio {
                    found = Some(e);
                    break;
                }
            }
            match found {
                Some(e) => e,
                None => return false,
            }
        };
        let index = 254 - degree;
        if index >= codeword.len() {
            return false;
        }
        codeword[index] ^= s0;
        syndrome(rs, 0, codeword) == 0 && syndrome(rs, 1, codeword) == 0
    }

    #[test]
    fn roots_one_parity_is_the_xor_of_the_data() {
        let rs = Rs::new(1);
        let data = [0x11u8, 0x22, 0x33, 0xf0];
        let mut parity = [0u8; 1];
        for &b in &data {
            rs.encode(&mut parity, b);
        }
        assert_eq!(parity[0], data.iter().fold(0, |a, b| a ^ b));
    }

    #[test]
    fn roots_two_codewords_have_zero_syndromes() {
        let roots = 2;
        let rs = Rs::new(roots);
        let data = pattern(253, 7);
        let mut parity = vec![0u8; roots];
        let mut state = vec![0u8; roots];
        for &b in &data {
            rs.encode(&mut state, b);
        }
        parity.copy_from_slice(&state);
        let mut codeword = data.clone();
        codeword.extend_from_slice(&parity);
        for power in 0..roots {
            assert_eq!(syndrome(&rs, power, &codeword), 0, "power {power}");
        }
    }

    #[test]
    fn roots_two_decodes_a_single_symbol_error() {
        let roots = 2;
        let rs = Rs::new(roots);
        for pos in [0usize, 1, 100, 252] {
            let data = pattern(253, pos);
            let mut state = vec![0u8; roots];
            for &b in &data {
                rs.encode(&mut state, b);
            }
            let mut codeword = data.clone();
            codeword.extend_from_slice(&state);
            let good = codeword.clone();
            codeword[pos] ^= 0xa5;
            assert!(repair_one_error(&rs, &mut codeword), "pos {pos}");
            assert_eq!(codeword, good, "pos {pos}");
        }
    }

    #[test]
    fn streaming_parity_matches_the_flat_interleave() {
        let dir = tmp("volte_fec_flat");
        let blocks = 8usize;
        let bytes = pattern(blocks * BLOCK as usize, 3);
        let p = write_image(&dir, &bytes);
        let roots = 2u32;
        let got = generate_parity(&p, bytes.len() as u64, roots).unwrap();

        let k = (255 - roots as usize) as usize;
        let seg = BLOCK as usize; // rounds == 1 for 8 blocks
        let rs = Rs::new(roots as usize);
        let mut want = vec![0u8; seg * roots as usize];
        for position in 0..seg {
            let mut state = vec![0u8; roots as usize];
            for j in 0..k {
                let byte = if j < blocks {
                    bytes[j * seg + position]
                } else {
                    0
                };
                rs.encode(&mut state, byte);
            }
            want[position * roots as usize..(position + 1) * roots as usize]
                .copy_from_slice(&state);
        }
        assert_eq!(got, want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parity_round_trip_on_a_representative_image() {
        let dir = tmp("volte_fec_round_trip");
        let bytes = pattern(16 * 1024 * 1024, 11);
        let p = write_image(&dir, &bytes);
        let size = bytes.len() as u64;
        let roots = 2u32;
        let parity = generate_parity(&p, size, roots).unwrap();
        assert_eq!(parity.len() as u64, parity_size(size, roots).unwrap());

        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&p)
            .unwrap();
        f.seek(SeekFrom::Start(size)).unwrap();
        f.write_all(&parity).unwrap();
        f.sync_all().unwrap();
        drop(f);

        assert!(verify_parity(&p, size, parity.len() as u64, roots).unwrap());

        let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.seek(SeekFrom::Start(5 * 1024 * 1024)).unwrap();
        f.write_all(&[bytes[5 * 1024 * 1024] ^ 0x01]).unwrap();
        f.sync_all().unwrap();
        drop(f);
        assert!(!verify_parity(&p, size, parity.len() as u64, roots).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parity_size_matches_the_a3s_layout() {
        let fec_offset = 1_097_682_944u64;
        let roots = 2u32;
        assert_eq!(parity_size(fec_offset, roots).unwrap(), 8_683_520);
    }

    #[test]
    fn unsupported_roots_are_rejected() {
        for roots in [0u32, 1, 3, 4, 253, 254] {
            let err = parity_size(BLOCK, roots).unwrap_err();
            assert!(err.message().contains("not supported"), "{err}");
        }
    }

    #[test]
    fn unaligned_and_empty_inputs_are_rejected() {
        assert!(parity_size(0, 2).is_err());
        assert!(parity_size(BLOCK + 1, 2).is_err());
    }
}
