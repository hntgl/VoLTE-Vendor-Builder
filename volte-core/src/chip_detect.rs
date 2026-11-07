//! Chip detection.
//!
//! Ports `QC_BIN_MARKERS` / `MTK_BIN_MARKERS` (`hd_vendor_build.py:779`),
//! `QC_KEYWORDS` / `MTK_KEYWORDS` (`:786`), `detect_vendor_chip` (`:811`) and
//! `refine_chip_from_props` (`:841`). The result is a heuristic: the caller
//! still has to state which branch was chosen and why it could be wrong.

use std::io::Read;
use std::path::Path;

pub const QC_BIN_MARKERS: [&[u8]; 6] = [
    b"init.qcom.rc",
    b"ro.hardware=qcom",
    b"ro.hardware.chipname=sm",
    b"ro.soc.model=SM",
    b"ro.soc.model=sdm",
    b"qcom,common",
];

pub const MTK_BIN_MARKERS: [&[u8]; 5] = [
    b"ro.hardware=mt6",
    b"init.mt6",
    b"ro.mediatek.",
    b"mediatek.common",
    b"mediatek.platform",
];

pub const QC_KEYWORDS: [&str; 56] = [
    "qualcomm",
    "snapdragon",
    "qcom",
    "trinket",
    "bengal",
    "lahaina",
    "kona",
    "lito",
    "atoll",
    "holi",
    "taro",
    "kalama",
    "sdm450",
    "sdm660",
    "sdm636",
    "msm8953",
    "msm8937",
    "msm8998",
    "sm6125",
    "sm6115",
    "sm4250",
    "sm6225",
    "sm6375",
    "cph1803",
    "cph1805",
    "cph1853",
    "cph1905",
    "a3s",
    "cph1931",
    "cph1933",
    "cph1935",
    "cph1943",
    "a5 2020",
    "a5_2020",
    "a5-2020",
    "cph1937",
    "cph1939",
    "a9 2020",
    "a9_2020",
    "a9-2020",
    "pchm10",
    "pcht10",
    "a11x",
    "cph2059",
    "cph2061",
    "cph2069",
    "a92",
    "a72",
    "cph2065",
    "cph2067",
    "a52",
    "cph2127",
    "cph2131",
    "cph2139",
    "cph2145",
    "a53",
];

pub const MTK_KEYWORDS: [&str; 37] = [
    "mediatek",
    "dimensity",
    "helio",
    "mt67",
    "mt68",
    "mt65",
    "mt69",
    "oppo67",
    "oppo68",
    "cph1909",
    "cph1920",
    "a5s",
    "a12",
    "cph1923",
    "a1k",
    "cph2015",
    "cph2083",
    "cph2077",
    "a31",
    "cph2185",
    "cph2179",
    "a15",
    "cph2269",
    "cph2349",
    "cph2421",
    "a16",
    "cph2239",
    "a54",
    "cph2387",
    "a57",
    "cph1819",
    "cph1823",
    "cph1825",
    "cph1969",
    "f7",
    "f9",
    "f11",
];

const HEAD_LEN: usize = 16 * 1024 * 1024;

/// `detect_vendor_chip` (`:811`). Returns `(chip, how)` where `how` is
/// `binary` (marker in the first 16 MiB), `path` (keyword in name/dir) or
/// `default` (blind → MediaTek).
pub fn detect_vendor_chip(path: &Path) -> (&'static str, &'static str) {
    if path.is_file() {
        if let Some(head) = read_head(path) {
            if QC_BIN_MARKERS.iter().any(|m| contains(&head, m)) {
                return ("qc", "binary");
            }
            if MTK_BIN_MARKERS.iter().any(|m| contains(&head, m)) {
                return ("mtk", "binary");
            }
        }
    }

    let base = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let dir = path
        .parent()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = format!("{base} {dir}");
    let is_qc = QC_KEYWORDS.iter().any(|k| name.contains(k));
    let is_mtk = MTK_KEYWORDS.iter().any(|k| name.contains(k));
    if is_qc && !is_mtk {
        return ("qc", "path");
    }
    if is_mtk && !is_qc {
        return ("mtk", "path");
    }
    if is_qc && is_mtk {
        if QC_KEYWORDS.iter().any(|k| base.contains(k)) {
            return ("qc", "path");
        }
        if MTK_KEYWORDS.iter().any(|k| base.contains(k)) {
            return ("mtk", "path");
        }
        return ("qc", "path");
    }
    ("mtk", "default")
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD_LEN];
    let mut done = 0;
    while done < HEAD_LEN {
        match f.read(&mut buf[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(_) => return None,
        }
    }
    buf.truncate(done);
    Some(buf)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    let first = needle[0];
    let last = hay.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if hay[i] == first && &hay[i..i + needle.len()] == needle {
            return true;
        }
        i += 1;
    }
    false
}

/// `refine_chip_from_props` (`:841`): vendor properties win over the heuristic.
pub fn refine_chip_from_props(chip: &str, props_text: &str) -> String {
    let low = props_text.to_lowercase();
    if low.is_empty() {
        return chip.to_string();
    }
    for k in [
        "ro.hardware=qcom",
        "ro.board.platform=sm",
        "ro.board.platform=msm",
        "ro.soc.manufacturer=qcom",
        "ro.vendor.qti",
    ] {
        if low.contains(k) {
            return "qc".to_string();
        }
    }
    for k in [
        "ro.hardware=mt",
        "ro.board.platform=mt",
        "ro.mediatek",
        "ro.vendor.mediatek",
    ] {
        if low.contains(k) {
            return "mtk".to_string();
        }
    }
    chip.to_string()
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
    fn tables_match_the_python_lengths() {
        assert_eq!(QC_BIN_MARKERS.len(), 6);
        assert_eq!(MTK_BIN_MARKERS.len(), 5);
        assert_eq!(QC_KEYWORDS.len(), 56);
        assert_eq!(MTK_KEYWORDS.len(), 37);
        assert!(QC_BIN_MARKERS.contains(&b"ro.soc.model=SM".as_slice()));
        assert!(MTK_BIN_MARKERS.contains(&b"mediatek.platform".as_slice()));
        assert!(QC_KEYWORDS.contains(&"cph1803"));
        assert!(QC_KEYWORDS.contains(&"a9-2020"));
        assert!(MTK_KEYWORDS.contains(&"f11"));
        assert!(MTK_KEYWORDS.contains(&"a57"));
        let unique: std::collections::HashSet<&str> = QC_KEYWORDS.iter().copied().collect();
        assert_eq!(unique.len(), 56);
        let unique: std::collections::HashSet<&str> = MTK_KEYWORDS.iter().copied().collect();
        assert_eq!(unique.len(), 37);
    }

    #[test]
    fn binary_markers_win_and_qc_is_checked_first() {
        let dir = tmp("volte_chip_binary");
        let qc = dir.join("image.bin");
        std::fs::write(&qc, b"\x00\x01init.qcom.rc\x00tail").unwrap();
        assert_eq!(detect_vendor_chip(&qc), ("qc", "binary"));
        let mtk = dir.join("image2.bin");
        std::fs::write(&mtk, b"\x00ro.hardware=mt6765\x00").unwrap();
        assert_eq!(detect_vendor_chip(&mtk), ("mtk", "binary"));
        let both = dir.join("image3.bin");
        std::fs::write(&both, b"init.mt6 ro.hardware=qcom").unwrap();
        assert_eq!(detect_vendor_chip(&both), ("qc", "binary"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn markers_beyond_16_mib_are_not_read() {
        let dir = tmp("volte_chip_head");
        let p = dir.join("vendor.img");
        let mut data = vec![0u8; HEAD_LEN];
        data.extend_from_slice(b"init.qcom.rc");
        std::fs::write(&p, &data).unwrap();
        assert_eq!(detect_vendor_chip(&p), ("mtk", "default"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_keywords_follow_the_python_order() {
        let dir = tmp("volte_chip_path");
        let qc = dir.join("A3s").join("vendor.img");
        assert_eq!(detect_vendor_chip(&qc), ("qc", "path"));
        let mtk = dir.join("f11").join("vendor.img");
        assert_eq!(detect_vendor_chip(&mtk), ("mtk", "path"));
        let blind = dir.join("rom").join("vendor.img");
        assert_eq!(detect_vendor_chip(&blind), ("mtk", "default"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clash_prefers_the_basename_then_qc() {
        let dir = tmp("volte_chip_clash");
        let mtk_base = dir.join("a3s").join("f11_vendor.img");
        assert_eq!(detect_vendor_chip(&mtk_base), ("mtk", "path"));
        let qc_base = dir.join("f11").join("a3s_vendor.img");
        assert_eq!(detect_vendor_chip(&qc_base), ("qc", "path"));
        let neither = dir.join("a3s_f11").join("vendor.img");
        assert_eq!(detect_vendor_chip(&neither), ("qc", "path"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refine_reads_the_vendor_keys_case_insensitively() {
        assert_eq!(refine_chip_from_props("mtk", "ro.hardware=qcom\n"), "qc");
        assert_eq!(refine_chip_from_props("qc", "ro.hardware=qcom"), "qc");
        assert_eq!(refine_chip_from_props("qc", "RO.BOARD.PLATFORM=SM6125"), "qc");
        assert_eq!(refine_chip_from_props("qc", "ro.vendor.qti.ims=1"), "qc");
        assert_eq!(refine_chip_from_props("qc", "ro.hardware=mt6765"), "mtk");
        assert_eq!(refine_chip_from_props("qc", "ro.mediatek.platform=MT6765"), "mtk");
        assert_eq!(refine_chip_from_props("qc", "ro.vendor.mediatek.x=1"), "mtk");
        assert_eq!(refine_chip_from_props("mtk", "ro.product.model=CPH1803"), "mtk");
        assert_eq!(refine_chip_from_props("qc", ""), "qc");
        assert_eq!(
            refine_chip_from_props("mtk", "ro.hardware=mt6\nro.vendor.qti=1"),
            "qc",
            "QC keys are checked first"
        );
    }
}
