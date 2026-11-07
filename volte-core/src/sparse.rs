//! Android sparse → raw conversion.
//!
//! Port of `is_sparse` and `unsparse` (`hd_vendor_build.py:452-523`). Header and
//! chunk sizes follow the on-disk format: a 28-byte file header, a 12-byte
//! chunk header, and the declared extra header bytes are skipped when larger.
//!
//! Unlike Python, this parser validates the header before touching `dst`:
//! version, header sizes, block size, declared image size and per-chunk sizes
//! are checked, and every size computation is overflow-checked. A malformed
//! image is rejected instead of producing a truncated or oversized output.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::config::SPARSE_MAGIC;
use crate::error::{Error, Result};

const FILE_HEADER_LEN: u64 = 28;
const CHUNK_HEADER_LEN: u64 = 12;
const SPARSE_MAJOR: u16 = 1;
/// Upper bound for one raw image. Vendor partitions are far below this; the
/// limit only rejects headers that declare an absurd output size.
const MAX_IMAGE_SIZE: u64 = 64 * 1024 * 1024 * 1024;
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

struct Header {
    file_hdr_sz: u64,
    chunk_hdr_sz: u64,
    blk_sz: u64,
    total_blks: u64,
    total_chunks: u32,
    expected: u64,
}

/// Validates the 28-byte file header.
fn parse_header(head: &[u8; FILE_HEADER_LEN as usize]) -> Result<Header> {
    let major = u16::from_le_bytes([head[4], head[5]]);
    let file_hdr_sz = u16::from_le_bytes([head[8], head[9]]) as u64;
    let chunk_hdr_sz = u16::from_le_bytes([head[10], head[11]]) as u64;
    let blk_sz = u32::from_le_bytes([head[12], head[13], head[14], head[15]]) as u64;
    let total_blks = u32::from_le_bytes([head[16], head[17], head[18], head[19]]) as u64;
    let total_chunks = u32::from_le_bytes([head[20], head[21], head[22], head[23]]);

    if major != SPARSE_MAJOR {
        return Err(Error::build(format!(
            "unsupported sparse version {major}.{} (expected major {SPARSE_MAJOR})",
            u16::from_le_bytes([head[6], head[7]])
        )));
    }
    if file_hdr_sz < FILE_HEADER_LEN {
        return Err(Error::build(format!(
            "sparse file header is {file_hdr_sz} bytes, smaller than {FILE_HEADER_LEN}"
        )));
    }
    if chunk_hdr_sz < CHUNK_HEADER_LEN {
        return Err(Error::build(format!(
            "sparse chunk header is {chunk_hdr_sz} bytes, smaller than {CHUNK_HEADER_LEN}"
        )));
    }
    if blk_sz == 0 || !blk_sz.is_multiple_of(4) {
        return Err(Error::build(format!(
            "sparse block size {blk_sz} is not a positive multiple of 4"
        )));
    }
    if total_blks == 0 {
        return Err(Error::build("sparse header declares no blocks"));
    }
    let expected = total_blks
        .checked_mul(blk_sz)
        .ok_or_else(|| Error::build("sparse header declares an overflowing image size"))?;
    if expected > MAX_IMAGE_SIZE {
        return Err(Error::build(format!(
            "sparse header declares {expected} bytes, above the {MAX_IMAGE_SIZE}-byte limit"
        )));
    }
    // Every chunk except a CRC covers at least one block, and a file needs at
    // most one CRC per chunk. More than twice the block count cannot be real.
    let max_chunks = total_blks.saturating_mul(2).saturating_add(1);
    if total_chunks as u64 > max_chunks {
        return Err(Error::build(format!(
            "sparse header declares {total_chunks} chunks for {total_blks} blocks"
        )));
    }
    Ok(Header {
        file_hdr_sz,
        chunk_hdr_sz,
        blk_sz,
        total_blks,
        total_chunks,
        expected,
    })
}

/// Raw size declared by a sparse header, or `None` when the file is not sparse
/// (or shorter than a header). Used to size disk-space probes; the same
/// validation as [`unsparse`] applies, so a malformed header fails early.
pub fn output_size(src: &Path) -> Result<Option<u64>> {
    let mut f = File::open(src)?;
    let mut head = [0u8; FILE_HEADER_LEN as usize];
    if f.read(&mut head)? < head.len() {
        return Ok(None);
    }
    if u32::from_le_bytes([head[0], head[1], head[2], head[3]]) != SPARSE_MAGIC {
        return Ok(None);
    }
    Ok(Some(parse_header(&head)?.expected))
}

/// Rejects a chunk whose declared payload size disagrees with its block count.
fn check_chunk_size(ctype: u16, csize: u64, blk_sz: u64, tsize: u64) -> Result<u64> {
    if ctype == CRC32 {
        if csize != 0 || tsize != 4 {
            return Err(Error::build(format!(
                "sparse CRC chunk declares {csize} blocks and {tsize} bytes (expected 0 and 4)"
            )));
        }
        return Ok(0);
    }
    let data_len = csize
        .checked_mul(blk_sz)
        .ok_or_else(|| Error::build("sparse chunk size overflows"))?;
    if csize == 0 {
        return Err(Error::build(format!(
            "sparse chunk 0x{ctype:X} declares no blocks"
        )));
    }
    if tsize != data_len {
        return Err(Error::build(format!(
            "sparse chunk 0x{ctype:X} declares {tsize} bytes for {csize} blocks x {blk_sz}"
        )));
    }
    Ok(data_len)
}

/// Port of `unsparse`. Returns `Unsparse::NotSparse` for a non-sparse or
/// shorter-than-header file (Python returns `False` without touching `dst`).
/// A malformed image leaves no partial output behind.
pub fn unsparse(src: &Path, dst: &Path) -> Result<Unsparse> {
    let mut f = BufReader::new(File::open(src)?);

    let mut head = [0u8; FILE_HEADER_LEN as usize];
    if f.read(&mut head)? < head.len() {
        return Ok(Unsparse::NotSparse);
    }
    let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    if magic != SPARSE_MAGIC {
        return Ok(Unsparse::NotSparse);
    }
    let hdr = parse_header(&head)?;
    if hdr.file_hdr_sz > FILE_HEADER_LEN {
        f.seek(SeekFrom::Current(
            (hdr.file_hdr_sz - FILE_HEADER_LEN) as i64,
        ))?;
    }

    let out = BufWriter::new(File::create(dst)?);
    let written = match write_image(&mut f, dst, out, &hdr) {
        Ok(written) => written,
        Err(e) => {
            let _ = std::fs::remove_file(dst);
            return Err(e);
        }
    };
    Ok(Unsparse::Converted { written })
}

fn write_image(
    f: &mut (impl Read + Seek),
    dst: &Path,
    mut out: BufWriter<File>,
    hdr: &Header,
) -> Result<u64> {
    let mut written: u64 = 0;

    for _ in 0..hdr.total_chunks {
        let mut ch = [0u8; CHUNK_HEADER_LEN as usize];
        read_exact(f, &mut ch, "chunk header")?;
        let ctype = u16::from_le_bytes([ch[0], ch[1]]);
        let csize = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]) as u64;
        let tsize = u32::from_le_bytes([ch[8], ch[9], ch[10], ch[11]]) as u64;
        if hdr.chunk_hdr_sz > CHUNK_HEADER_LEN {
            f.seek(SeekFrom::Current(
                (hdr.chunk_hdr_sz - CHUNK_HEADER_LEN) as i64,
            ))?;
        }

        let data_len = match ctype {
            RAW | FILL | DONT_CARE | CRC32 => check_chunk_size(ctype, csize, hdr.blk_sz, tsize)?,
            other => return Err(Error::build(format!("bad sparse chunk type: 0x{other:X}"))),
        };
        let next = written
            .checked_add(data_len)
            .ok_or_else(|| Error::build("sparse chunk sizes overflow"))?;
        if next > hdr.expected {
            return Err(Error::build(format!(
                "sparse chunks cover at least {next} bytes but the header declares {}",
                hdr.expected
            )));
        }

        match ctype {
            RAW => {
                let mut left = data_len;
                let mut buf = vec![0u8; 8 * 1024 * 1024];
                while left > 0 {
                    let n = left.min(buf.len() as u64) as usize;
                    read_exact(f, &mut buf[..n], "raw chunk")?;
                    out.write_all(&buf[..n])?;
                    left -= n as u64;
                }
            }
            FILL => {
                let mut pat = [0u8; 4];
                read_exact(f, &mut pat, "fill chunk")?;
                let step = 1024 * 1024usize;
                let mut chunk = vec![0u8; step];
                for i in (0..step).step_by(4) {
                    chunk[i..i + 4].copy_from_slice(&pat);
                }
                let mut left = data_len;
                while left > 0 {
                    let k = left.min(step as u64) as usize;
                    out.write_all(&chunk[..k])?;
                    left -= k as u64;
                }
            }
            DONT_CARE => {
                out.seek(SeekFrom::Current(data_len as i64))?;
            }
            CRC32 => {
                let mut crc = [0u8; 4];
                read_exact(f, &mut crc, "crc chunk")?;
            }
            other => return Err(Error::build(format!("bad sparse chunk type: 0x{other:X}"))),
        }
        written = next;
    }

    if written != hdr.expected {
        return Err(Error::build(format!(
            "sparse chunks cover {written} bytes but the header declares {} \
             ({} blocks x {})",
            hdr.expected, hdr.total_blks, hdr.blk_sz
        )));
    }
    out.flush()?;
    let file = out.get_ref();
    file.set_len(written)?;
    let actual = file.metadata()?.len();
    if actual != written {
        let _ = std::fs::remove_file(dst);
        return Err(Error::build(format!(
            "sparse output is {actual} bytes after conversion, expected {written}"
        )));
    }
    Ok(written)
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

        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted {
                written: 5 * BLK as u64
            }
        );
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
            raw[3 * BLK as usize..4 * BLK as usize]
                .iter()
                .all(|&b| b == 0),
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
            Unsparse::Converted {
                written: 4 * BLK as u64
            }
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
        std::fs::write(
            &src,
            build_sparse(&[Chunk::DontCare(2), Chunk::DontCare(3)]),
        )
        .unwrap();

        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted {
                written: 5 * BLK as u64
            }
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
            Unsparse::Converted {
                written: 3 * BLK as u64
            }
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
        assert_eq!(
            unsparse(&src, &dst).unwrap(),
            Unsparse::Converted {
                written: BLK as u64
            }
        );
        let raw = std::fs::read(&dst).unwrap();
        assert!(raw.iter().all(|&b| b == 9));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn header_error(name: &str, patch: impl FnOnce(&mut Vec<u8>), needle: &str) {
        let dir = tmp(name);
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::DontCare(1)]);
        patch(&mut sp);
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains(needle), "{}", err);
        assert!(!dst.exists(), "a bad header must not create the output");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unsupported_major_version_is_rejected() {
        header_error(
            "volte_sparse_version",
            |sp| sp[4..6].copy_from_slice(&2u16.to_le_bytes()),
            "unsupported sparse version 2.0",
        );
    }

    #[test]
    fn a_short_file_header_is_rejected() {
        header_error(
            "volte_sparse_short_file_hdr",
            |sp| sp[8..10].copy_from_slice(&20u16.to_le_bytes()),
            "smaller than 28",
        );
    }

    #[test]
    fn a_short_chunk_header_is_rejected() {
        header_error(
            "volte_sparse_short_chunk_hdr",
            |sp| sp[10..12].copy_from_slice(&8u16.to_le_bytes()),
            "smaller than 12",
        );
    }

    #[test]
    fn a_zero_block_size_is_rejected() {
        header_error(
            "volte_sparse_zero_blk",
            |sp| sp[12..16].copy_from_slice(&0u32.to_le_bytes()),
            "not a positive multiple of 4",
        );
    }

    #[test]
    fn a_block_size_not_multiple_of_four_is_rejected() {
        header_error(
            "volte_sparse_odd_blk",
            |sp| sp[12..16].copy_from_slice(&4093u32.to_le_bytes()),
            "not a positive multiple of 4",
        );
    }

    #[test]
    fn an_image_declaring_no_blocks_is_rejected() {
        header_error(
            "volte_sparse_no_blocks",
            |sp| sp[16..20].copy_from_slice(&0u32.to_le_bytes()),
            "declares no blocks",
        );
    }

    #[test]
    fn an_absurd_declared_size_is_rejected() {
        header_error(
            "volte_sparse_absurd_size",
            |sp| sp[16..20].copy_from_slice(&u32::MAX.to_le_bytes()),
            "above the",
        );
    }

    #[test]
    fn a_chunk_count_that_cannot_fit_the_file_is_rejected() {
        header_error(
            "volte_sparse_chunk_count",
            |sp| sp[20..24].copy_from_slice(&u32::MAX.to_le_bytes()),
            "declares 4294967295 chunks",
        );
    }

    #[test]
    fn a_chunk_payload_size_that_disagrees_with_its_blocks_is_rejected() {
        header_error(
            "volte_sparse_tsize_mismatch",
            |sp| {
                let tsize_at = 28 + 8;
                sp[tsize_at..tsize_at + 4].copy_from_slice(&123u32.to_le_bytes());
            },
            "declares 123 bytes",
        );
    }

    #[test]
    fn a_crc_chunk_that_declares_blocks_is_rejected() {
        let dir = tmp("volte_sparse_crc_blocks");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[Chunk::DontCare(1), Chunk::Crc(0xDECAFBAD)]);
        // The CRC chunk follows the 12-byte header of the DONT_CARE chunk.
        let crc_at = 28 + 12;
        sp[crc_at + 4..crc_at + 8].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("CRC chunk"), "{}", err);
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunks_that_cover_more_than_the_header_declares_are_rejected_early() {
        let dir = tmp("volte_sparse_over_cover");
        let src = dir.join("bad.sparse");
        let dst = dir.join("out.raw");
        let mut sp = build_sparse(&[
            Chunk::Raw(vec![1u8; BLK as usize]),
            Chunk::Raw(vec![2u8; BLK as usize]),
        ]);
        sp[16..20].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&src, &sp).unwrap();
        let err = unsparse(&src, &dst).unwrap_err();
        assert!(err.message().contains("cover at least"), "{}", err);
        assert!(!dst.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_size_reads_the_declared_raw_size() {
        let dir = tmp("volte_sparse_output_size");
        let sp = dir.join("image.sparse");
        let raw = dir.join("image.raw");
        std::fs::write(
            &sp,
            build_sparse(&[Chunk::DontCare(2), Chunk::Raw(vec![0; BLK as usize])]),
        )
        .unwrap();
        std::fs::write(&raw, vec![0u8; 64]).unwrap();

        assert_eq!(output_size(&sp).unwrap(), Some(3 * BLK as u64));
        assert_eq!(output_size(&raw).unwrap(), None);

        let mut bad = build_sparse(&[Chunk::DontCare(1)]);
        bad[4..6].copy_from_slice(&7u16.to_le_bytes());
        let bad_path = dir.join("bad.sparse");
        std::fs::write(&bad_path, &bad).unwrap();
        assert!(output_size(&bad_path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
