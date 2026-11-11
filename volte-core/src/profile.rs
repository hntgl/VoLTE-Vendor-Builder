//! Per-chip constants, ported from `hd_vendor_build.py:81-96`.
//!
//! The payload files themselves are never copied into this project; these are
//! the names and search chains the CLI applies to the payload directories.
//!
//! Divergence of 2026-11-08: the primary `.rc` names (`config::RC_NAME_MTK`
//! / `config::RC_NAME_QC`, and so the strings written to the vendor
//! partition and the `import` lines below) dropped the `hd_` prefix
//! Python used, becoming `oppo_mtk_volte.rc` / `oppo_qc_volte.rc`.
//!
//! Divergence of 2026-11-08 (follow-up): the `HD_`/`HA_`/`GAQ_` OEM fallback
//! candidates (other teams' alternate names for the same payload files) were
//! removed by request. `first_payload` now only ever finds the canonical
//! name, so a payload file under one of those old OEM names alone will fail
//! the build with "missing payload" instead of being auto-detected. The one
//! fallback that remains, in `RC_QC_CANDIDATES` — `config::RC_NAME_MTK` — is
//! not an OEM alias; it is this tool's own QC→MTK substitution for when only
//! the MTK `.rc` is available, and was kept.

use crate::config;

pub const APK_MTK_CANDIDATES: [&str; 1] = [config::APK_NAME];

pub const APK_QC_CANDIDATES: [&str; 1] = [config::APK_NAME];

pub const RC_MTK_CANDIDATES: [&str; 1] = [config::RC_NAME_MTK];

pub const RC_QC_CANDIDATES: [&str; 2] = [config::RC_NAME_QC, config::RC_NAME_MTK];

pub const IMPORT_LINE: &str = "import /vendor/etc/init/oppo_qc_volte.rc";
pub const IMPORT_LINE_HW: &str = "import /vendor/etc/init/hw/oppo_qc_volte.rc";
pub const IMPORT_LINE_MTK: &str = "import /vendor/etc/init/oppo_mtk_volte.rc";
pub const IMPORT_LINE_MTK_HW: &str = "import /vendor/etc/init/hw/oppo_mtk_volte.rc";

/// `link_init_target` (`:965`): MTK first tries project/connectivity/sensor/
/// SoC-specific files, then the shared tail.
pub const INIT_VPATHS_MTK: [&str; 18] = [
    "/etc/init/hw/init.project.rc",
    "/etc/init/init.project.rc",
    "/etc/init/hw/init.connectivity.rc",
    "/etc/init/init.connectivity.rc",
    "/etc/init/hw/init.sensor_1_0.rc",
    "/etc/init/hw/init.mt6771.rc",
    "/etc/init/hw/init.mt6765.rc",
    "/etc/init/hw/init.mt6768.rc",
    "/etc/init/hw/init.mt6785.rc",
    "/etc/init/hw/init.mt6833.rc",
    "/etc/init/hw/init.mt6877.rc",
    "/etc/init/hw/init.mt6893.rc",
    "/etc/init/hw/init.mediatek.rc",
    "/etc/init/init.mediatek.rc",
    "/etc/init/hw/init.vendor.rc",
    "/etc/init/init.vendor.rc",
    "/etc/init/hw/init.modem.rc",
    "/etc/init/init.modem.rc",
];

/// `link_init_target` (`:992`): Qualcomm candidates.
pub const INIT_VPATHS_QC: [&str; 5] = [
    "/etc/init/hw/init.target.rc",
    "/init.target.rc",
    "/etc/init/hw/init.qcom.rc",
    "/etc/init/init.qcom.rc",
    "/etc/init/hw/init.vendor.rc",
];

/// `link_init_target` (`:1000`): always tried last.
pub const INIT_VPATHS_SHARED: [&str; 3] = [
    "/etc/init/hw/init.target.rc",
    "/init.target.rc",
    "/etc/init/hw/init.rc",
];

pub fn apk_candidates(qc: bool) -> &'static [&'static str] {
    if qc {
        &APK_QC_CANDIDATES
    } else {
        &APK_MTK_CANDIDATES
    }
}

pub fn rc_candidates(qc: bool) -> &'static [&'static str] {
    if qc {
        &RC_QC_CANDIDATES
    } else {
        &RC_MTK_CANDIDATES
    }
}

pub fn apk_name(qc: bool) -> &'static str {
    if qc {
        config::APK_NAME_QC
    } else {
        config::APK_NAME_MTK
    }
}

pub fn rc_name(qc: bool) -> &'static str {
    if qc {
        config::RC_NAME_QC
    } else {
        config::RC_NAME_MTK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_chains_are_canonical_names_only() {
        assert_eq!(APK_QC_CANDIDATES, ["OPPO_VoLTE_Overlay.apk"]);
        assert_eq!(APK_MTK_CANDIDATES, ["OPPO_VoLTE_Overlay.apk"]);
        assert_eq!(APK_MTK_CANDIDATES.len(), 1);
        assert_eq!(RC_QC_CANDIDATES.len(), 2);
        assert_eq!(RC_MTK_CANDIDATES.len(), 1);
    }

    #[test]
    fn qc_rc_chain_falls_back_to_mtk_name() {
        assert_eq!(RC_QC_CANDIDATES[0], config::RC_NAME_QC);
        assert_eq!(RC_QC_CANDIDATES[1], config::RC_NAME_MTK);
    }

    #[test]
    fn import_lines_match_the_renamed_rc_files() {
        // Divergence of 2026-11-08: the shipped `.rc` names dropped the
        // `hd_` prefix (`oppo_mtk_volte.rc` / `oppo_qc_volte.rc`), so these
        // import lines no longer match the Python source verbatim.
        assert_eq!(IMPORT_LINE, "import /vendor/etc/init/oppo_qc_volte.rc");
        assert_eq!(
            IMPORT_LINE_HW,
            "import /vendor/etc/init/hw/oppo_qc_volte.rc"
        );
        assert_eq!(IMPORT_LINE_MTK, "import /vendor/etc/init/oppo_mtk_volte.rc");
        assert_eq!(
            IMPORT_LINE_MTK_HW,
            "import /vendor/etc/init/hw/oppo_mtk_volte.rc"
        );
    }

    #[test]
    fn init_vpath_lists_have_measured_lengths() {
        assert_eq!(INIT_VPATHS_MTK.len(), 18);
        assert_eq!(INIT_VPATHS_QC.len(), 5);
        assert_eq!(INIT_VPATHS_SHARED.len(), 3);
        assert_eq!(INIT_VPATHS_MTK.len() + INIT_VPATHS_SHARED.len(), 21);
        assert_eq!(INIT_VPATHS_QC.len() + INIT_VPATHS_SHARED.len(), 8);
    }

    #[test]
    fn selector_helpers_follow_the_branch() {
        assert_eq!(apk_name(true), "OPPO_VoLTE_Overlay.apk");
        assert_eq!(apk_name(false), "OPPO_VoLTE_Overlay.apk");
        assert_eq!(rc_name(true), "oppo_qc_volte.rc");
        assert_eq!(rc_name(false), "oppo_mtk_volte.rc");
        assert_eq!(apk_candidates(true).len(), 1);
        assert_eq!(rc_candidates(false).len(), 1);
    }
}
