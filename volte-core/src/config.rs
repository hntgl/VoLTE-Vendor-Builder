//! Constants ported verbatim from `vendor_patcher\hd_vendor_build.py` (lines 60-96)
//! plus the machine-readable report tokens.

/// `VERSION` at `hd_vendor_build.py:61`.
pub const VERSION: &str = "2.9.2 / V298 (standalone)";

/// `SPARSE_MAGIC` at `:65`.
pub const SPARSE_MAGIC: u32 = 0xED26_FF3A;
/// `AVB_MAGIC` at `:66`.
pub const AVB_MAGIC: [u8; 4] = *b"AVBf";
/// `BLOCK` at `:67`.
pub const BLOCK: u64 = 4096;
/// `CHUNK` at `:69`.
pub const CHUNK: usize = 16 * 1024 * 1024;
/// `OVERLAP` at `:70`.
pub const OVERLAP: usize = 256;

pub const APK_NAME: &str = "OPPO_VoLTE_Overlay.apk";
pub const APK_NAME_MTK: &str = APK_NAME;
pub const APK_NAME_QC: &str = APK_NAME;
pub const RC_NAME_MTK: &str = "oppo_mtk_volte.rc";
pub const RC_NAME_QC: &str = "oppo_qc_volte.rc";
pub const FEATURES_XML: &str = "volte_features.xml";

/// Report/summary tokens. Their spelling is part of the product output and must
/// stay byte-identical to the Python tool.
pub const TOKEN_FILE: &str = "File=";
pub const TOKEN_SIZE: &str = "Size=";
pub const TOKEN_E2FSCK: &str = "e2fsck=";
pub const TOKEN_AVB_HASHTREE: &str = "AVB_HASHTREE=";
pub const TOKEN_AVB_ALGORITHM_NONE: &str = "AVB_ALGORITHM=NONE";
pub const TOKEN_AVB_SIGNATURE: &str = "AVB_SIGNATURE=";
pub const TOKEN_FLAGS: &str = "VOLTE_FLAGS=";
pub const TOKEN_APK_SHA256: &str = "APK_SHA256=";
pub const TOKEN_RC_SHA256: &str = "RC_SHA256=";
pub const TOKEN_FLASH: &str = "FLASH=NO";

pub const TAG_OK: &str = "[OK] ";
pub const TAG_WARN: &str = "[WARN] ";
pub const TAG_ERROR: &str = "[ERROR] ";
pub const TAG_INFO: &str = "[INFO] ";
pub const TAG_STEP: &str = "[STEP] ";
pub const TAG_STATUS: &str = "[STATUS] ";
pub const TAG_AVB: &str = "[AVB] ";
pub const TAG_OUTPUT: &str = "[OUTPUT] ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_python_source() {
        assert_eq!(VERSION, "2.9.2 / V298 (standalone)");
    }

    #[test]
    fn sparse_and_avb_magics() {
        assert_eq!(SPARSE_MAGIC, 0xED26_FF3A);
        assert_eq!(AVB_MAGIC, [b'A', b'V', b'B', b'f']);
    }

    #[test]
    fn block_and_window_sizes() {
        assert_eq!(BLOCK, 4096);
        assert_eq!(CHUNK, 16_777_216);
        assert_eq!(OVERLAP, 256);
    }

    #[test]
    fn one_apk_serves_both_branches() {
        assert_eq!(APK_NAME_MTK, APK_NAME_QC);
        assert_eq!(APK_NAME_MTK, "OPPO_VoLTE_Overlay.apk");
    }

    #[test]
    fn machine_tokens_spelling() {
        assert_eq!(TOKEN_E2FSCK, "e2fsck=");
        assert_eq!(TOKEN_AVB_HASHTREE, "AVB_HASHTREE=");
        assert_eq!(TOKEN_AVB_ALGORITHM_NONE, "AVB_ALGORITHM=NONE");
        assert_eq!(TOKEN_FLASH, "FLASH=NO");
    }
}
