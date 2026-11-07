//! Report and summary text.
//!
//! Ports `hue_avb_lines` (`hd_vendor_build.py:1374`), `volte_flag_note` (`:764`)
//! and the token block of `build` (`:1717-1745`). Machine-readable tokens keep
//! their Python spelling byte for byte; human-readable lines are English.

use crate::avb::AvbInfo;
use crate::config;

/// `hue_avb_lines` (`:1374`): the nine AVB report lines.
pub fn avb_lines(info: &AvbInfo) -> Vec<String> {
    if !info.has_footer {
        return vec!["avbtool: Given image does not look like a vbmeta image.".to_string()];
    }
    let name = if info.partition_name.is_empty() {
        "vendor"
    } else {
        info.partition_name.as_str()
    };
    let alg = if info.hash_algorithm.is_empty() {
        "sha256"
    } else {
        info.hash_algorithm.as_str()
    };
    vec![
        format!(
            "Footer version:           {}.{}",
            info.footer_major, info.footer_minor
        ),
        format!("Image size:               {} bytes", info.image_size),
        format!(
            "Original image size:      {} bytes",
            info.original_image_size
        ),
        format!("VBMeta offset:            {}", info.vbmeta_offset),
        format!("VBMeta size:              {} bytes", info.vbmeta_size),
        format!("Partition Name:           {name}"),
        format!("Hash Algorithm:           {alg}"),
        "Algorithm:                NONE".to_string(),
        format!("FEC num roots:            {}", info.fec_roots),
    ]
}

/// `volte_flag_note` (`:764`): the flag value plus the two log lines.
pub fn flag_note(nflag: i64) -> (u8, String, String) {
    let n = if nflag >= 1 { 1u8 } else { 0u8 };
    let hit = format!("VoLTE flag: {n}");
    let tip = if n >= 1 {
        "Flag =1 because the original vendor already had =1, or 0->1 was patched in the image.".to_string()
    } else {
        "Flag =0 - the original vendor has no volte_support=1 and no 0->1 patch was applied.".to_string()
    };
    (n, hit, tip)
}

/// Verdict of the mandatory `e2fsck -fn` step, rendered as the `e2fsck=` token.
///
/// Python only ever writes `PASS`; the port distinguishes the three real states
/// so a failed or never-run check can never be rendered as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsckStatus {
    NotRun,
    Pass,
    Fail,
}

impl FsckStatus {
    pub fn token(self) -> &'static str {
        match self {
            FsckStatus::NotRun => "NOTRUN",
            FsckStatus::Pass => "PASS",
            FsckStatus::Fail => "FAIL",
        }
    }
}

/// The token block after the final size check (`:1721-1736`). `File=` and
/// `Size=` are emitted by the pipeline first, before its own size check.
pub struct Summary<'a> {
    pub out_path: &'a str,
    pub out_size: u64,
    pub e2fsck: FsckStatus,
    pub avb_hashtree: &'a str,
    pub flags: u8,
    pub apk_sha256: Option<&'a str>,
    pub rc_sha256: Option<&'a str>,
    pub total_time: &'a str,
}

/// The summary lines in Python's order and spelling of the machine tokens.
/// The flashability lines are derived from the verdicts, never from a constant.
pub fn summary_lines(s: &Summary) -> Vec<String> {
    let mut lines = vec![
        if s.e2fsck == FsckStatus::Pass {
            "[OK] EDL size matches - UnlockTool can flash it.".to_string()
        } else {
            format!(
                "[ERROR] e2fsck={} - the image is not confirmed flashable.",
                s.e2fsck.token()
            )
        },
        format!("{}{}", config::TOKEN_E2FSCK, s.e2fsck.token()),
        format!("{}{}", config::TOKEN_AVB_HASHTREE, s.avb_hashtree),
    ];
    if s.avb_hashtree == "SKIP" {
        lines.push(
            "[NOTE] AVB hashtree skipped - EDL + overlay; the device boots the original image (footer not truncated)."
                .to_string(),
        );
    } else if s.e2fsck == FsckStatus::Pass {
        lines.push(
            "[NOTE] STANDARD AVB hashtree PASS + matching size - safe to boot.".to_string(),
        );
    } else {
        lines.push(
            "[ERROR] AVB hashtree PASS but e2fsck did not pass - do not flash.".to_string(),
        );
    }
    lines.push(format!("{}{}", config::TOKEN_FLAGS, s.flags));
    lines.push(config::TOKEN_AVB_ALGORITHM_NONE.to_string());
    if let Some(h) = s.apk_sha256 {
        lines.push(format!("{}{}", config::TOKEN_APK_SHA256, h.to_uppercase()));
    }
    if let Some(h) = s.rc_sha256 {
        lines.push(format!("{}{}", config::TOKEN_RC_SHA256, h.to_uppercase()));
    }
    lines.push(config::TOKEN_FLASH.to_string());
    lines.push(format!("[DONE] Total time: {}", s.total_time));
    lines.push(format!("{}{}", config::TAG_OUTPUT, s.out_path));
    lines
}

/// `vendor_VolLTE_report.txt` (`:1740-1745`): six lines, LF terminated.
pub fn report_body(
    out_path: &str,
    out_size: u64,
    e2fsck: FsckStatus,
    avb_hashtree: &str,
) -> String {
    format!(
        "{}{}\n{}{}\n{}{}\n{}{}\n{}\n{}\n",
        config::TOKEN_FILE,
        out_path,
        config::TOKEN_SIZE,
        out_size,
        config::TOKEN_E2FSCK,
        e2fsck.token(),
        config::TOKEN_AVB_HASHTREE,
        avb_hashtree,
        config::TOKEN_AVB_ALGORITHM_NONE,
        config::TOKEN_FLASH
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn footer_info() -> AvbInfo {
        let mut info = AvbInfo::empty(2 * 1024 * 1024);
        info.has_footer = true;
        info.original_image_size = 1_000_000;
        info.vbmeta_offset = 2_000_000;
        info.vbmeta_size = 4096;
        info.fec_roots = 0;
        info
    }

    fn summary<'a>(avb: &'a str) -> Summary<'a> {
        Summary {
            out_path: "/out/vendor_VolLTE.img",
            out_size: 2 * 1024 * 1024,
            e2fsck: FsckStatus::Pass,
            avb_hashtree: avb,
            flags: 1,
            apk_sha256: Some("abc"),
            rc_sha256: Some("def"),
            total_time: "01:02",
        }
    }

    #[test]
    fn avb_lines_snapshot_is_byte_for_byte() {
        assert_eq!(
            avb_lines(&AvbInfo::empty(2 * 1024 * 1024)),
            vec!["avbtool: Given image does not look like a vbmeta image.".to_string()]
        );
        assert_eq!(
            avb_lines(&footer_info()),
            vec![
                "Footer version:           0.0",
                "Image size:               2097152 bytes",
                "Original image size:      1000000 bytes",
                "VBMeta offset:            2000000",
                "VBMeta size:              4096 bytes",
                "Partition Name:           vendor",
                "Hash Algorithm:           sha256",
                "Algorithm:                NONE",
                "FEC num roots:            0",
            ]
        );
    }

    #[test]
    fn avb_lines_fall_back_to_vendor_and_sha256() {
        let mut info = footer_info();
        info.partition_name = String::new();
        info.hash_algorithm = String::new();
        let lines = avb_lines(&info);
        assert_eq!(lines[5], "Partition Name:           vendor");
        assert_eq!(lines[6], "Hash Algorithm:           sha256");
    }

    #[test]
    fn flag_note_maps_to_zero_or_one() {
        let (n, hit, tip) = flag_note(0);
        assert_eq!(n, 0);
        assert_eq!(hit, "VoLTE flag: 0");
        assert!(tip.contains("Flag =0"));
        assert_eq!(flag_note(1).0, 1);
        assert_eq!(flag_note(7).0, 1);
        assert_eq!(flag_note(-3).0, 0);
        assert_eq!(flag_note(2).1, "VoLTE flag: 1");
    }

    #[test]
    fn summary_lines_match_python_order_and_tokens() {
        assert_eq!(
            summary_lines(&summary("PASS")),
            vec![
                "[OK] EDL size matches - UnlockTool can flash it.",
                "e2fsck=PASS",
                "AVB_HASHTREE=PASS",
                "[NOTE] STANDARD AVB hashtree PASS + matching size - safe to boot.",
                "VOLTE_FLAGS=1",
                "AVB_ALGORITHM=NONE",
                "APK_SHA256=ABC",
                "RC_SHA256=DEF",
                "FLASH=NO",
                "[DONE] Total time: 01:02",
                "[OUTPUT] /out/vendor_VolLTE.img",
            ]
        );
    }

    #[test]
    fn the_skip_note_replaces_the_pass_note() {
        let lines = summary_lines(&summary("SKIP"));
        assert!(lines[2].starts_with("AVB_HASHTREE=SKIP"));
        assert!(lines[3].starts_with("[NOTE] AVB hashtree skipped"));
        assert!(!lines.iter().any(|l| l.contains("STANDARD")));
    }

    #[test]
    fn a_failed_fsck_is_never_rendered_as_flashable() {
        let mut s = summary("PASS");
        s.e2fsck = FsckStatus::Fail;
        let lines = summary_lines(&s);
        assert!(!lines.iter().any(|l| l.contains("[OK]")), "{lines:?}");
        assert_eq!(
            lines[0],
            "[ERROR] e2fsck=FAIL - the image is not confirmed flashable."
        );
        assert_eq!(lines[1], "e2fsck=FAIL");
        assert!(lines[3].contains("do not flash"), "{lines:?}");
    }

    #[test]
    fn a_never_run_fsck_is_never_rendered_as_flashable() {
        let mut s = summary("PASS");
        s.e2fsck = FsckStatus::NotRun;
        let lines = summary_lines(&s);
        assert_eq!(lines[1], "e2fsck=NOTRUN");
        assert!(!lines.iter().any(|l| l.contains("[OK]")), "{lines:?}");
    }

    #[test]
    fn missing_hashes_omit_the_token_like_python_falsy_values() {
        let mut s = summary("PASS");
        s.apk_sha256 = None;
        s.rc_sha256 = None;
        let lines = summary_lines(&s);
        assert!(!lines.iter().any(|l| l.starts_with("APK_SHA256=")));
        assert!(!lines.iter().any(|l| l.starts_with("RC_SHA256=")));
    }

    #[test]
    fn report_body_is_the_six_line_python_file() {
        assert_eq!(
            report_body("/out/vendor_VolLTE.img", 2 * 1024 * 1024, FsckStatus::Fail, "SKIP"),
            "File=/out/vendor_VolLTE.img\nSize=2097152\ne2fsck=FAIL\n\
             AVB_HASHTREE=SKIP\nAVB_ALGORITHM=NONE\nFLASH=NO\n"
        );
    }
}
