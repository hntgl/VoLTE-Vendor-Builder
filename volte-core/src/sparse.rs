//! Android sparse → raw conversion.
//!
//! Port of `is_sparse` and `unsparse` (`hd_vendor_build.py:452-523`). Header and
//! chunk sizes follow the on-disk format: a 28-byte file header, a 12-byte
//! chunk header, and the declared extra header bytes are skipped when larger.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::config::SPARSE_MAGIC;
use crate::error::{Error, Result};

const FILE_HEADER_LEN: u64 = 28;
const CHUNK_HEADER_LEN: u64 = 12;
const RAW: u16 = 0xCAC1;
const FILL: u16 = 0xCAC2;
const DONT_CARE: u16 = 0xCAC3;
const CRC32: u16 = 0xCAC4;

/// Outcome of a conversion attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Unsparse {
    /// A sparse image was converted; `written` counts bytes the header declared
    /// (DONT_CARE regions included, matching Python's counter).
    Converted { written: u64 },
    /// The file is too short or does not carry the sparse magic; `dst` is not
    /// created.
    NotSparse,
}

/// Port of `is_sparse`: only the first four bytes matter.
pub fn is_sparse(path: &Path) -> bool {
    let mut head = [0u8; 4];
    match File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) => u32::from_le_bytes(head) == SPARSE_MAGIC,
        Err(_) => false,
    }
}

fn read_exact(f: &mut impl Read, buf: &mut [u8], what: &str) -> Result<()> {
    f.read_exact(buf)
        .map_err(|_| Error::build(format!("sparse image is truncated ({what})")))
}

/// Port of `unsparse`. Returns `Unsparse::NotSparse` for a non-sparse or
/// shorter-than-header file (Python returns `False` without touching `dst`).
pub fn unsparse(src: &Path, dst: &Path) -> Result<Unsparse> {
    let file = File::open(src)?;
    let mut f = BufReader::new(file);

    let mut head = [0u8; FILE_HEADER_LEN as usize];
    if f.read(&mut head)? < head.len() {
        return Ok(Unsparse::NotSparse);
    }
    let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let file_hdr_sz = u16::from_le_bytes([head[8], head[9]]) as u64;
    let chunk_hdr_sz = u16::from_le_bytes([head[10], head[11]]) as u64;
    let blk_sz = u32::from_le_bytes([head[12], head[13], head[14], head[15]]) as u64;
    let total_blks = u32::from_le_bytes([head[16], head[17], head[18], head[19]]) as u64;
    let total_chunks = u32::from_le_bytes([head[20], head[21], head[22], head[23]]);
    if magic != SPARSE_MAGIC {
        return Ok(Unsparse::NotSparse);
    }
    if file_hdr_sz > FILE_HEADER_LEN {
        f.seek(SeekFrom::Current((file_hdr_sz - FILE_HEADER_LEN) as i64))?;
    }

    let out_file = File::create(dst)?;
    let mut out = BufWriter::new(out_file);
    let mut written: u64 = 0;

    for _ in 0..total_chunks {
        let mut ch = [0u8; CHUNK_HEADER_LEN as usize];
        read_exact(&mut f, &mut ch, "chunk header")?;
        let ctype = u16::from_le_bytes([ch[0], ch[1]]);
        let csize = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        if chunk_hdr_sz > CHUNK_HEADER_LEN {
            f.seek(SeekFrom::Current((chunk_hdr_sz - CHUNK_HEADER_LEN) as i64))?;
        }

        match ctype {
            RAW => {
                let mut left = csize * blk_sz;
                let mut buf = vec![0u8; 8 * 1024 * 1024];
                while left > 0 {
                    let n = (left as usize).min(buf.len());
                    read_exact(&mut f, &mut buf[..n], "raw chunk")?;
                    out.write_all(&buf[..n])?;
                    left -= n as u64;
                    written += n as u64;
                }
            }
            FILL => {
                let mut pat = [0u8; 4];
                read_exact(&mut f, &mut pat, "fill chunk")?;
                let step = 1024 * 1024usize;
                let mut chunk = vec![0u8; step];
                for i in (0..step).step_by(4) {
                    chunk[i..i + 4].copy_from_slice(&pat);
                }
                let mut left = csize * blk_sz;
                while left > 0 {
                    let k = (left as usize).min(step);
                    out.write_all(&chunk[..k])?;
                    left -= k as u64;
                    written += k as u64;
                }
            }
            DONT_CARE => {
                let n = csize * blk_sz;
                out.seek(SeekFrom::Current(n as i64))?;
                written += n;
            }
            CRC32 => {
                let mut crc = [0u8; 4];
                read_exact(&mut f, &mut crc, "crc chunk")?;
            }
            other => {
                return Err(Error::build(format!("bad sparse chunk type: 0x{other:X}")));
            }
        }
    }

    let expected = total_blks
        .checked_mul(blk_sz)
        .ok_or_else(|| Error::build("sparse header declares an overflowing image size"))?;
    if written != expected {
        return Err(Error::build(format!(
            "sparse chunks cover {written} bytes but the header declares {expected} \
             ({total_blks} blocks x {blk_sz})"
        )));
    }
    out.flush()?;
    let file = out.get_ref();
    file.set_len(written)?;
    let actual = file.metadata()?.len();
    if actual != written {
        return Err(Error::build(format!(
            "sparse output is {actual} bytes after conversion, expected {written}"
        )));
    }
    Ok(Unsparse::Converted { written })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BLK: u32 = 4096;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    enum Chunk {
        Raw(Vec<u8>),
        Fill([u8; 4], u32),
        DontCare(u32),
        Crc(u32),
    }

    fn build_sparse(chunks: &[Chunk]) -> Vec<u8> {
        let mut out = Vec::new();
        let total_blks: u32 = chunks
            .iter()
            .map(|c| match c {
                Chunk::Raw(d) => (d.len() as u32) / BLK,
                Chunk::Fill(_, n) => *n,
                Chunk::DontCare(n) => *n,
                Chunk::Crc(_) => 0,
            })
            .sum();
        out.extend_from_slice(&SPARSE_MAGIC.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&28u16.to_le_bytes());
        out.extend_from_slice(&12u16.to_le_bytes());
        out.extend_from_slice(&BLK.to_le_bytes());
        out.extend_from_slice(&total_blks.to_le_bytes());
        out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for c in chunks {
            match c {
                Chunk::Raw(d) => {
                    out.extend_from_slice(&RAW.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&((d.len() as u32) / BLK).to_le_bytes());
                    out.extend_from_slice(&(d.len() as u32).to_le_bytes());
                    out.extend_from_slice(d);
                }
                Chunk::Fill(pat, n) => {
                    out.extend_from_slice(&FILL.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&n.to_le_bytes());
                    out.extend_from_slice(&(n * BLK).to_le_bytes());
                    out.extend_from_slice(pat);
                }
                Chunk::DontCare(n) => {
                    out.extend_from_slice(&DONT_CARE.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&n.to_le_bytes());
                    out.extend_from_slice(&(n * BLK).to_le_bytes());
                }
                Chunk::Crc(v) => {
                    out.extend_from_slice(&CRC32.to_le_bytes());
                    out.extend_from_slice(&0u16.to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&4u32.to_le_bytes());
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        out
    }

    #[test]
    fn magic_detection() {
        let dir = tmp("volte_sparse_magic");
        let raw = dir.join("raw.bin");
        std::fs::write(&raw, b"ANDROID!").unwrap();
        assert!(!is_sparse(&raw));

        let sp = dir.join("sparse.bin");
        std::fs::write(&sp, SPARSE_MAGIC.to_le_bytes()).unwrap();
        assert!(is_sparse(&sp));

        let short = dir.join("short.bin");
        std::fs::write(&short, b"\x3A").unwrap();
        assert!(!is_sparse(&short));
        assert!(!is_sparse(&dir.join("absent.bin")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn raw_fill_dont_care_crc_round_trip() {
        let dir = tmp("volte_sparse_roundtrip");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");

        let mut raw_block = vec![0u8; BLK as usize];
        for (i, b) in raw_block.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let mut tail_block = vec![0u8; BLK as usize];
        tail_block[0] = 0x5A;
        let sp = build_sparse(&[
            Chunk::Raw(raw_block.clone()),
            Chunk::Fill([0xAB, 0xCD, 0xEF, 0x01], 2),
            Chunk::DontCare(1),
            Chunk::Raw(tail_block.clone()),
            Chunk::Crc(0xDEADBEEF),
        ]);
        std::fs::write(&src, &sp).unwrap();

        assert_eq!(unsparse(&src, &dst).unwrap(), Unsparse::Converted { written: 5 * BLK as u64 });
        let raw = std::fs::read(&dst).unwrap();
        assert_eq!(raw.len(), 5 * BLK as usize);
        assert_eq!(&raw[..BLK as usize], &raw_block[..]);
        let pattern = [0xAB, 0xCD, 0xEF, 0x01];
        for i in 0..2 {
            let off = BLK as usize * (1 + i);
            let block = &raw[off..off + BLK as usize];
            assert!(
                block.iter().enumerate().all(|(j, &b)| b == pattern[j % 4]),
                "fill block {i}"
            );
        }
        assert!(
            raw[3 * BLK as usize..4 * BLK as usize].iter().all(|&b| b == 0),
            "DONT_CARE must stay a hole"
        );
        assert_eq!(&raw[4 * BLK as usize..], &tail_block[..]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_trailing_dont_care_chunk_extends_the_output() {
        let dir = tmp("volte_sparse_trailing_hole");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");
        std::fs::write(&src, build_sparse(&[Chunk::DontCare(4)])).unwrap();

        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted { written: 4 * BLK as u64 }
        );
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), 4 * BLK as u64);
        assert!(std::fs::read(&dst).unwrap().iter().all(|&b| b == 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_all_dont_care_image_is_zero_filled_to_the_declared_size() {
        let dir = tmp("volte_sparse_all_holes");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");
        std::fs::write(&src, build_sparse(&[Chunk::DontCare(2), Chunk::DontCare(3)])).unwrap();

        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted { written: 5 * BLK as u64 }
        );
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), 5 * BLK as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_without_dont_care_keeps_the_exact_length() {
        let dir = tmp("volte_sparse_no_holes");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");
        let sp = build_sparse(&[
            Chunk::Raw(vec![1u8; BLK as usize]),
            Chunk::Fill([2, 3, 4, 5], 2),
        ]);
        std::fs::write(&src, &sp).unwrap();

        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted { written: 3 * BLK as u64 }
        );
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), 3 * BLK as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_header_size_that_the_chunks_do_not_cover_is_an_error() {
        let dir = tmp("volte_sparse_size_mismatch");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");
        let mut sp = build_sparse(&[Chunk::Raw(vec![0u8; BLK as usize]), Chunk::DontCare(1)]);
        sp[16..20].copy_from_slice(&5u32.to_le_bytes());
        std::fs::write(&src, &sp).unwrap();

        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("header declares"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_sparse_file_leaves_destination_absent() {
        let dir = tmp("volte_sparse_not_sparse");
        let src = dir.join("raw.bin");
        let dst = dir.join("out.raw");
        std::fs::write(&src, vec![0u8; 4096]).unwrap();
        assert_eq!(unsparse(&src, &dst).unwrap(), Unsparse::NotSparse);
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_chunk_header_is_an_error() {
        let dir = tmp("volte_sparse_trunc_ch");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::DontCare(1)]);
        sp.truncate(28 + 6);
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("chunk header"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_raw_chunk_is_an_error() {
        let dir = tmp("volte_sparse_trunc_raw");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::Raw(vec![7u8; (2 * BLK) as usize])]);
        sp.truncate(28 + 12 + BLK as usize);
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("raw chunk"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_fill_pattern_is_an_error() {
        let dir = tmp("volte_sparse_trunc_fill");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::Fill([1, 2, 3, 4], 1)]);
        sp.truncate(28 + 12 + 2);
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("fill chunk"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_chunk_type_is_an_error() {
        let dir = tmp("volte_sparse_unknown");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::DontCare(1)]);
        sp[28] = 0xC5;
        sp[29] = 0xCA;
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("0xCAC5"), "{}", err);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_headers_are_skipped() {
        let dir = tmp("volte_sparse_extra_headers");
        let src = dir.join("image.sparse");
        let dst = dir.join("image.raw");
        let mut sp = build_sparse(&[Chunk::Fill([9, 9, 9, 9], 1)]);
        // Grow the file header from 28 to 32 bytes.
        sp[8..10].copy_from_slice(&32u16.to_le_bytes());
        sp.splice(28..28, [0u8; 4]);
        // Grow the chunk header from 12 to 16 bytes.
        sp[10..12].copy_from_slice(&16u16.to_le_bytes());
        sp.splice(32 + 12..32 + 12, [0u8; 4]);
        std::fs::write(&src, &sp).unwrap();
        assert_eq!(unsparse(&src, &dst).unwrap(), Unsparse::Converted { written: BLK as u64 });
        let raw = std::fs::read(&dst).unwrap();
        assert!(raw.iter().all(|&b| b == 9));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
