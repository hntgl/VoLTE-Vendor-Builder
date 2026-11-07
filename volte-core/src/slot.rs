//! Slot size enforcement.
//!
//! Port of `force_slot_size` (`hd_vendor_build.py:637`): the EDL loader writes
//! into a fixed partition slot, so the image size must equal the slot exactly.
//! The tail is never cut — a too-large file is an error in hard mode and a warn
//! in soft mode.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Error, Result};
use crate::logging::Logger;

/// Returns the resulting size, or `0` when the arguments are unusable (missing
/// file, `slot <= 0`), exactly like Python's early return.
pub fn force_slot_size(path: &Path, slot: u64, hard: bool, tag: &str, log: &mut Logger) -> Result<u64> {
    if slot == 0 || !path.is_file() {
        return Ok(0);
    }
    let cur = std::fs::metadata(path)?.len();
    if cur == slot {
        log.log(format!("[OK] Size {tag} matches slot: {slot} (0x{slot:X})"));
        return Ok(slot);
    }
    log.log(format!(
        "[STEP] Resize {tag} {cur} (0x{cur:X}) -> {slot} (0x{slot:X})"
    ));
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    if cur > slot {
        let msg = format!("file is larger than slot {tag} ({cur} > {slot}) - not truncating (would bootloop)");
        if hard {
            return Err(Error::build(msg));
        }
        log.log(format!("[WARN] {msg}"));
        return Ok(cur);
    }
    file.seek(SeekFrom::End(0))?;
    let mut left = slot - cur;
    const STEP: usize = 8 * 1024 * 1024;
    let zero = vec![0u8; STEP];
    while left > 0 {
        let k = (left as usize).min(STEP);
        file.write_all(&zero[..k])?;
        left -= k as u64;
    }
    file.flush()?;
    let cur2 = std::fs::metadata(path)?.len();
    if cur2 != slot {
        let msg = format!("size mismatch {tag} after resize ({cur2} != {slot} / 0x{slot:X})");
        if hard {
            return Err(Error::build(msg));
        }
        log.log(format!("[WARN] {msg}"));
        return Ok(cur2);
    }
    log.log(format!("[OK] Size {tag} matches slot: {slot} (0x{slot:X})"));
    Ok(slot)
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
    fn equal_size_is_a_noop() {
        let dir = tmp("volte_slot_equal");
        let p = dir.join("img.bin");
        std::fs::write(&p, vec![5u8; 4096]).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(force_slot_size(&p, 4096, true, "EDL", &mut log).unwrap(), 4096);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 4096);
        assert_eq!(std::fs::read(&p).unwrap(), vec![5u8; 4096]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn smaller_file_is_padded_with_zeros_and_content_kept() {
        let dir = tmp("volte_slot_pad");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"HEAD").unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let slot = 8 * 1024 * 1024 + 7;
        assert_eq!(force_slot_size(&p, slot, true, "EDL", &mut log).unwrap(), slot);
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes.len() as u64, slot);
        assert_eq!(&bytes[..4], b"HEAD");
        assert!(bytes[4..].iter().all(|&b| b == 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn larger_file_is_a_hard_error_and_not_truncated() {
        let dir = tmp("volte_slot_hard");
        let p = dir.join("img.bin");
        std::fs::write(&p, vec![7u8; 8192]).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = force_slot_size(&p, 4096, true, "EDL", &mut log).unwrap_err();
        assert!(err.message().contains("not truncating"), "{}", err);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 8192);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn larger_file_in_soft_mode_is_kept_with_a_warning() {
        let dir = tmp("volte_slot_soft");
        let p = dir.join("img.bin");
        std::fs::write(&p, vec![7u8; 8192]).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(force_slot_size(&p, 4096, false, "AVB-PAD", &mut log).unwrap(), 8192);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 8192);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_or_zero_slot_returns_zero() {
        let dir = tmp("volte_slot_invalid");
        let mut log = Logger::new();
        assert_eq!(force_slot_size(&dir.join("absent.bin"), 4096, true, "EDL", &mut log).unwrap(), 0);
        assert_eq!(force_slot_size(&dir.join("absent.bin"), 0, true, "EDL", &mut log).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
