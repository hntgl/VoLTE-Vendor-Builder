//! Byte-level VoLTE flag patching.
//!
//! Ports `_prop_boundary` (`hd_vendor_build.py:682`), `FLAG_PAIRS` (`:179`),
//! `patch_flags_keep_size` (`:689`) and `read_vendor_flag_bit` (`:735`). Every
//! pair is length-preserving, so the image size never changes.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::config::{CHUNK, OVERLAP};
use crate::error::{Error, ErrorKind, Result};
use crate::logging::Logger;
use crate::path_id;

/// The 65 `=0 → =1` pairs (three of them inverted), byte-for-byte.
pub const FLAG_PAIRS: [(&[u8], &[u8]); 65] = [
    (b"volte_support=0", b"volte_support=1"),
    (b"volte.enable=0", b"volte.enable=1"),
    (b"volte_state=0", b"volte_state=1"),
    (b"ims_support=0", b"ims_support=1"),
    (b"volte_avail_ovr=0", b"volte_avail_ovr=1"),
    (b"ims_volte_enable=0", b"ims_volte_enable=1"),
    (b"vt_avail_ovr=0", b"vt_avail_ovr=1"),
    (b"wfc_avail_ovr=0", b"wfc_avail_ovr=1"),
    (b"calls.on.ims=0", b"calls.on.ims=1"),
    (b"mtk_dynamic_ims_switch=1", b"mtk_dynamic_ims_switch=0"),
    (
        b"persist.dbg.volte_avail_ovr=0",
        b"persist.dbg.volte_avail_ovr=1",
    ),
    (
        b"persist.dbg.ims_volte_enable=0",
        b"persist.dbg.ims_volte_enable=1",
    ),
    (b"persist.dbg.vt_avail_ovr=0", b"persist.dbg.vt_avail_ovr=1"),
    (
        b"persist.dbg.wfc_avail_ovr=0",
        b"persist.dbg.wfc_avail_ovr=1",
    ),
    (
        b"persist.dbg.allow_ims_off=1",
        b"persist.dbg.allow_ims_off=0",
    ),
    (
        b"persist.mtk.volte_support=0",
        b"persist.mtk.volte_support=1",
    ),
    (b"persist.mtk.ims_support=0", b"persist.mtk.ims_support=1"),
    (
        b"persist.vendor.mtk.volte_support=0",
        b"persist.vendor.mtk.volte_support=1",
    ),
    (
        b"persist.vendor.mtk.ims_support=0",
        b"persist.vendor.mtk.ims_support=1",
    ),
    (
        b"persist.vendor.radio.volte_avail_ovr=0",
        b"persist.vendor.radio.volte_avail_ovr=1",
    ),
    (
        b"persist.vendor.radio.vt_avail_ovr=0",
        b"persist.vendor.radio.vt_avail_ovr=1",
    ),
    (
        b"persist.vendor.radio.calls.on.ims=0",
        b"persist.vendor.radio.calls.on.ims=1",
    ),
    (
        b"persist.vendor.ims.enable=0",
        b"persist.vendor.ims.enable=1",
    ),
    (
        b"persist.radio.volte_state=0",
        b"persist.radio.volte_state=1",
    ),
    (
        b"persist.radio.calls.on.ims=0",
        b"persist.radio.calls.on.ims=1",
    ),
    (
        b"persist.radio.volte.enable=0",
        b"persist.radio.volte.enable=1",
    ),
    (
        b"persist.vendor.radio.volte.enable=0",
        b"persist.vendor.radio.volte.enable=1",
    ),
    (
        b"persist.sys.oplus.radio.volte_icon=0",
        b"persist.sys.oplus.radio.volte_icon=1",
    ),
    (
        b"persist.sys.oplus.radio.show_volte=0",
        b"persist.sys.oplus.radio.show_volte=1",
    ),
    (
        b"persist.oplus.radio.volte_icon=0",
        b"persist.oplus.radio.volte_icon=1",
    ),
    (b"persist.radio.volte.icon=0", b"persist.radio.volte.icon=1"),
    (
        b"persist.sys.oplus.radio.volte_switch=0",
        b"persist.sys.oplus.radio.volte_switch=1",
    ),
    (
        b"persist.sys.oppo.radio.show_volte=0",
        b"persist.sys.oppo.radio.show_volte=1",
    ),
    (
        b"persist.sys.oppo.radio.volte_icon=0",
        b"persist.sys.oppo.radio.volte_icon=1",
    ),
    (
        b"persist.sys.oppo.radio.volte_switch=0",
        b"persist.sys.oppo.radio.volte_switch=1",
    ),
    (
        b"persist.sys.oppo.show_volte=0",
        b"persist.sys.oppo.show_volte=1",
    ),
    (
        b"persist.radio.oppo.show_volte=0",
        b"persist.radio.oppo.show_volte=1",
    ),
    (b"ro.oppo.radio.show_volte=0", b"ro.oppo.radio.show_volte=1"),
    (
        b"persist.vendor.mtk_dynamic_ims_switch=1",
        b"persist.vendor.mtk_dynamic_ims_switch=0",
    ),
    (b"show_volte_icon=0", b"show_volte_icon=1"),
    (
        b"mtk_ct_volte_support\",\"0\"",
        b"mtk_ct_volte_support\",\"1\"",
    ),
    (
        b"mtk_ct_vilte_support\",\"0\"",
        b"mtk_ct_vilte_support\",\"1\"",
    ),
    (b"vilte_support\",\"0\"", b"vilte_support\",\"1\""),
    (b"volte_support\",\"0\"", b"volte_support\",\"1\""),
    (b"ro.mtk_volte_support=0", b"ro.mtk_volte_support=1"),
    (b"ro.mtk_ims_support=0", b"ro.mtk_ims_support=1"),
    (b"ro.mtk_vilte_support=0", b"ro.mtk_vilte_support=1"),
    (
        b"ro.oplus.radio.support_volte=0",
        b"ro.oplus.radio.support_volte=1",
    ),
    (
        b"ro.oplus.radio.show_volte_switch=0",
        b"ro.oplus.radio.show_volte_switch=1",
    ),
    (b"mtk_volte_support=0", b"mtk_volte_support=1"),
    (b"mtk_ims_support=0", b"mtk_ims_support=1"),
    (
        b"persist.vendor.volte_support=0",
        b"persist.vendor.volte_support=1",
    ),
    (
        b"persist.vendor.ims_support=0",
        b"persist.vendor.ims_support=1",
    ),
    (
        b"persist.vendor.mtk.volte.enable=0",
        b"persist.vendor.mtk.volte.enable=1",
    ),
    (b"volte_support=false", b"volte_support=true "),
    (b"ims_support=false", b"ims_support=true "),
    (b"show_volte=0", b"show_volte=1"),
    (b"volte_icon=0", b"volte_icon=1"),
    (b"volte_switch=0", b"volte_switch=1"),
    (b"support_volte=0", b"support_volte=1"),
    (b"oppo_volte=0", b"oppo_volte=1"),
    (b"volte_enabled=0", b"volte_enabled=1"),
    (b"carrier_volte=0", b"carrier_volte=1"),
    (b"show_vilte=0", b"show_vilte=1"),
    (b"support_vilte=0", b"support_vilte=1"),
];

/// `_prop_boundary`: a match is accepted only when the preceding byte is not
/// part of a property name. Note `i == 0` returns true — including at the start
/// of every 16 MiB window, which is how Python behaves.
pub fn prop_boundary(buf: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = buf[i - 1];
    !(prev.is_ascii_alphanumeric() || prev == b'_')
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    let n = needle.len();
    if n == 0 || from >= hay.len() || hay.len() - from < n {
        return None;
    }
    let last = hay.len() - n;
    let first = needle[0];
    let mut i = from;
    while i <= last {
        if hay[i] == first && &hay[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn read_chunk(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        let n = f.read(&mut buf[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(done)
}

/// A failed read must stay a read failure: the pipeline uses it to tell an
/// unreadable image from a scan that legitimately found zero flags.
fn io_read(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Read, e.to_string())
}

/// A failed write must stay a write failure, never "0 flags patched".
fn io_write(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Write, e.to_string())
}

/// `patch_flags_keep_size` (`:689`): copy `src` to `dst` when they differ, then
/// apply every flag pair inside `limit` bytes (0 = whole file). Windows advance
/// by `CHUNK - OVERLAP` so a pair straddling a boundary is still seen.
pub fn patch_flags_keep_size(src: &Path, dst: &Path, limit: u64, _log: &mut Logger) -> Result<u64> {
    if !path_id::same_file(src, dst) {
        std::fs::copy(src, dst).map_err(io_write)?;
    }
    let mut hits = 0u64;
    let size = std::fs::metadata(dst).map_err(io_read)?.len();
    if size == 0 {
        return Ok(0);
    }
    let span = if limit == 0 { size } else { size.min(limit) };
    let pairs: Vec<(&[u8], &[u8])> = FLAG_PAIRS
        .iter()
        .filter(|(a, b)| !a.is_empty() && a.len() == b.len())
        .map(|(a, b)| (*a, *b))
        .collect();
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dst)
        .map_err(io_write)?;
    let mut buf = vec![0u8; CHUNK];
    let mut pos = 0u64;
    while pos < span {
        let n = (span - pos).min(CHUNK as u64) as usize;
        f.seek(SeekFrom::Start(pos)).map_err(io_read)?;
        let got = read_chunk(&mut f, &mut buf[..n]).map_err(io_read)?;
        if got == 0 {
            break;
        }
        for (a, b) in &pairs {
            let mut start = 0usize;
            while let Some(i) = find_from(&buf[..got], a, start) {
                if !prop_boundary(&buf[..got], i) {
                    start = i + 1;
                    continue;
                }
                buf[i..i + b.len()].copy_from_slice(b);
                hits += 1;
                start = i + b.len();
            }
        }
        f.seek(SeekFrom::Start(pos)).map_err(io_write)?;
        f.write_all(&buf[..got]).map_err(io_write)?;
        if n < CHUNK {
            break;
        }
        pos += (n - OVERLAP) as u64;
    }
    f.flush().map_err(io_write)?;
    Ok(hits)
}

const ON_MARKERS: [&[u8]; 9] = [
    b"persist.mtk.volte_support=1",
    b"persist.vendor.mtk.volte_support=1",
    b"persist.dbg.volte_avail_ovr=1",
    b"persist.vendor.volte_support=1",
    b"persist.dbg.ims_volte_enable=1",
    b"persist.sys.oplus.radio.show_volte=1",
    b"persist.sys.oplus.radio.volte_icon=1",
    b"persist.vendor.radio.volte_avail_ovr=1",
    b"volte_support=1",
];

/// The scan half of `read_vendor_flag_bit` (`:735`): reads the two prop dumps
/// (when present and larger than 4 bytes), joins them with a newline and
/// reports 1 when any "already on" marker is present.
pub fn flag_bit_from_dumps(workdir: &Path) -> u8 {
    let mut blob: Vec<u8> = Vec::new();
    for name in ["flg_def.prop", "flg_build.prop"] {
        let p = workdir.join(name);
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.is_file() && meta.len() > 4 {
                if let Ok(mut f) = std::fs::File::open(&p) {
                    let mut data = Vec::new();
                    if f.read_to_end(&mut data).is_ok() {
                        blob.push(b'\n');
                        blob.extend_from_slice(&data);
                    }
                }
            }
        }
    }
    if blob.is_empty() {
        return 0;
    }
    if ON_MARKERS.iter().any(|m| find_from(&blob, m, 0).is_some()) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn flag_pairs_are_65_length_equal_and_non_empty() {
        assert_eq!(FLAG_PAIRS.len(), 65);
        for (a, b) in FLAG_PAIRS.iter() {
            assert!(!a.is_empty());
            assert_eq!(a.len(), b.len(), "pair {:?} changes size", a);
            assert_ne!(a, b);
        }
        assert!(FLAG_PAIRS.contains(&(
            &b"mtk_dynamic_ims_switch=1"[..],
            &b"mtk_dynamic_ims_switch=0"[..]
        )));
        assert_eq!(b"volte_support=true ".len(), b"volte_support=false".len());
        let unique: std::collections::HashSet<&[u8]> = FLAG_PAIRS.iter().map(|(a, _)| *a).collect();
        assert_eq!(unique.len(), 65, "duplicate left-hand side");
    }

    #[test]
    fn boundary_uses_ascii_names_only() {
        assert!(prop_boundary(b"volte_support=0", 0));
        assert!(prop_boundary(b".volte_support=0", 1));
        assert!(!prop_boundary(b"xvolte_support=0", 1));
        assert!(!prop_boundary(b"_volte_support=0", 1));
        assert!(!prop_boundary(b"9volte_support=0", 1));
        assert!(prop_boundary(b"/volte_support=0", 1));
        assert!(prop_boundary(b"=volte_support=0", 1));
        assert!(prop_boundary(b"\nvolte_support=0", 1));
        assert!(prop_boundary(b"-volte_support=0", 1));
    }

    #[test]
    fn patches_only_whole_property_names() {
        let dir = tmp("volte_flags_boundary");
        let src = dir.join("in.bin");
        std::fs::write(&src, b"xvolte_support=0\nvolte_support=0\n").unwrap();
        let dst = dir.join("out.bin");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let hits = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap();
        assert_eq!(hits, 1);
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"xvolte_support=0\nvolte_support=1\n"
        );
        assert_eq!(
            std::fs::read(&src).unwrap(),
            b"xvolte_support=0\nvolte_support=0\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_pair_is_applied_once_and_size_is_kept() {
        let dir = tmp("volte_flags_all_pairs");
        let src = dir.join("in.bin");
        let mut data = Vec::new();
        for (a, _) in FLAG_PAIRS.iter() {
            data.extend_from_slice(b"\n");
            data.extend_from_slice(a);
        }
        data.extend_from_slice(b"\n");
        std::fs::write(&src, &data).unwrap();
        let dst = dir.join("out.bin");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let hits = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap();
        assert_eq!(hits, 65);
        let out = std::fs::read(&dst).unwrap();
        assert_eq!(out.len(), data.len());
        for (a, b) in FLAG_PAIRS.iter() {
            assert!(
                find_from(&out, a, 0).is_none(),
                "left side survived: {:?}",
                a
            );
            assert!(
                find_from(&out, b, 0).is_some(),
                "right side missing: {:?}",
                b
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pair_straddling_a_chunk_boundary_is_still_patched() {
        let dir = tmp("volte_flags_overlap");
        let src = dir.join("in.bin");
        let mut data = vec![b'.'; CHUNK + 4096];
        let at = CHUNK - 5;
        data[at..at + 15].copy_from_slice(b"volte_support=0");
        std::fs::write(&src, &data).unwrap();
        let dst = dir.join("out.bin");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let hits = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap();
        assert_eq!(hits, 1);
        let out = std::fs::read(&dst).unwrap();
        assert_eq!(&out[at..at + 15], b"volte_support=1");
        assert_eq!(out.len(), data.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_start_accepts_a_match_python_also_accepts() {
        // Python's `_prop_boundary` returns true at index 0 of every window, so
        // a name glued to the previous byte is patched once the window starts
        // exactly on it. This test pins that quirk.
        let dir = tmp("volte_flags_window_zero");
        let src = dir.join("in.bin");
        let mut data = vec![b'A'; CHUNK + 4096];
        let at = CHUNK - OVERLAP;
        data[at..at + 15].copy_from_slice(b"volte_support=0");
        std::fs::write(&src, &data).unwrap();
        let dst = dir.join("out.bin");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let hits = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap();
        assert_eq!(hits, 1);
        let out = std::fs::read(&dst).unwrap();
        assert_eq!(&out[at..at + 15], b"volte_support=1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn limit_restricts_the_scanned_span() {
        let dir = tmp("volte_flags_limit");
        let src = dir.join("in.bin");
        std::fs::write(&src, b"volte_support=0....volte_support=0").unwrap();
        let dst = dir.join("out.bin");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let hits = patch_flags_keep_size(&src, &dst, 15, &mut log).unwrap();
        assert_eq!(hits, 1);
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"volte_support=1....volte_support=0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_path_patches_in_place_and_empty_file_returns_zero() {
        let dir = tmp("volte_flags_inplace");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"volte_support=0").unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(patch_flags_keep_size(&p, &p, 0, &mut log).unwrap(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"volte_support=1");
        let empty = dir.join("empty.bin");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(
            patch_flags_keep_size(&empty, &empty, 0, &mut log).unwrap(),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn case_alias_patches_in_place_instead_of_copying() {
        let dir = tmp("volte_flags_case_alias");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"volte_support=0").unwrap();
        let alias = dir.join("IMG.BIN");
        assert!(
            alias.is_file(),
            "the test needs a case-insensitive filesystem"
        );
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(patch_flags_keep_size(&p, &alias, 0, &mut log).unwrap(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"volte_support=1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_link_alias_patches_in_place_when_the_platform_allows_it() {
        let dir = tmp("volte_flags_hard_link");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"volte_support=0").unwrap();
        let alias = dir.join("alias.bin");
        if std::fs::hard_link(&p, &alias).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(patch_flags_keep_size(&p, &alias, 0, &mut log).unwrap(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"volte_support=1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flag_bit_reads_both_dumps_and_separates_them() {
        let dir = tmp("volte_flag_bit");
        assert_eq!(flag_bit_from_dumps(&dir), 0);
        std::fs::write(dir.join("flg_def.prop"), b"volte_support=").unwrap();
        std::fs::write(dir.join("flg_build.prop"), b"1xxxx").unwrap();
        assert_eq!(flag_bit_from_dumps(&dir), 0, "newline must split the files");
        std::fs::write(dir.join("flg_build.prop"), b"abc\nvolte_support=1\n").unwrap();
        assert_eq!(flag_bit_from_dumps(&dir), 1);
        std::fs::write(dir.join("flg_def.prop"), b"abcd").unwrap();
        std::fs::write(dir.join("flg_build.prop"), b"\nvolte_support=1").unwrap();
        assert_eq!(
            flag_bit_from_dumps(&dir),
            1,
            "4-byte file is skipped, other one is read"
        );
        std::fs::write(dir.join("flg_build.prop"), b"1").unwrap();
        assert_eq!(
            flag_bit_from_dumps(&dir),
            0,
            "both files <= 4 bytes -> empty blob"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a6_a_missing_image_is_a_read_error_not_a_zero_hit() {
        let dir = tmp("volte_flags_a6_read");
        let missing = dir.join("missing.img");
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = patch_flags_keep_size(&missing, &missing, 0, &mut log).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::Read,
            "an unreadable image must be reported as a read error, never as 0 hits: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a6_a_copy_failure_is_a_write_error_not_a_zero_hit() {
        let dir = tmp("volte_flags_a6_write");
        let src = dir.join("in.bin");
        std::fs::write(&src, b"volte_support=0").unwrap();
        let dst = dir.join("dest");
        std::fs::create_dir(&dst).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::Write,
            "a failed write must be reported as a write error, never as 0 hits: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a6_no_pair_found_is_still_ok_zero() {
        let dir = tmp("volte_flags_a6_not_found");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"ro.keep=1\n").unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(
            patch_flags_keep_size(&p, &p, 0, &mut log).unwrap(),
            0,
            "a complete scan with no matching pair is a valid zero, not an I/O error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
