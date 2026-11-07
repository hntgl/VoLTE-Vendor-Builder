//! Post-patch verification: dump the overlay APK and RC back out and hash them.
//!
//! Port of `verify_payload` (`hd_vendor_build.py:1208`). The dump loop and the
//! two thresholds are kept exactly: the loop stops early once the shared host
//! file is bigger than 1000 (APK) / 10 (RC) bytes, but the verdict uses 500
//! bytes for the APK. One deliberate divergence: Python logs the literal
//! `VERIFIED_APK_OK` / `VERIFIED_RC_OK` when nothing was dumped, while this
//! port reports `UNVERIFIED` and returns [`Verdict::NotVerified`].

use std::path::Path;

use crate::config;
use crate::env::forward_slashes;
use crate::error::Result;
use crate::ext4::Ext4Ops;
use crate::logging::Logger;
use crate::sha256;

/// Python's `apk_cands` (`:1214`): six names, independent of the chip branch.
pub const APK_CANDIDATES: [&str; 6] = [
    config::APK_NAME_QC,
    "HD_OPPO_QC_VoLTE_Overlay.apk",
    "HD_OPPO_MTK_VoLTE_Overlay.apk",
    "HA_OPPO_QC_VoLTE_Overlay.apk",
    "HA_OPPO_MTK_VoLTE_Overlay.apk",
    "GAQ_OPPO_MTK_VoLTE_Overlay.apk",
];

/// Python's `rc_cands` (`:1227`), first entry selected by the chip branch.
pub fn rc_candidates(qc: bool) -> [&'static str; 5] {
    [
        if qc { config::RC_NAME_QC } else { config::RC_NAME_MTK },
        config::RC_NAME_QC,
        config::RC_NAME_MTK,
        "ha_oppo_qc_volte.rc",
        "ha_oppo_mtk_volte.rc",
    ]
}

/// Whether the dumped payload really landed in the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    NotVerified,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Verified => "verified",
            Verdict::NotVerified => "not verified",
        }
    }
}

/// What `verify_payload` returns for the report (`apk_h`, `rc_h`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub apk_sha256: String,
    pub rc_sha256: String,
    pub apk: Verdict,
    pub rc: Verdict,
}

fn size_gt(path: &Path, limit: u64) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > limit)
        .unwrap_or(false)
}

fn dump_batch(image: &Path, td: &Path, vpaths: [String; 2], host: &Path, ops: &mut dyn Ext4Ops) {
    let dest = forward_slashes(host);
    let commands = [
        format!("dump -p {} {dest}", vpaths[0]),
        format!("dump -p {} {dest}", vpaths[1]),
    ];
    // Python swallows a failed dump and moves on to the next candidate name.
    let mut quiet = Logger::new();
    let _ = ops.run(image, &commands, td, &mut quiet);
}

/// `verify_payload` (`:1208`). Dump failures are not errors; a hash failure is.
pub fn verify_payload(
    image: &Path,
    td: &Path,
    qc: bool,
    ops: &mut dyn Ext4Ops,
    log: &mut Logger,
) -> Result<VerifyResult> {
    let dump_apk = td.join("verify_overlay.apk");
    let dump_rc = td.join("verify_volte.rc");

    for name in APK_CANDIDATES {
        if size_gt(&dump_apk, 1000) {
            break;
        }
        dump_batch(
            image,
            td,
            [
                format!("/overlay/{name}"),
                format!("/overlay/HDVoLTE/{name}"),
            ],
            &dump_apk,
            ops,
        );
    }
    for name in rc_candidates(qc) {
        if size_gt(&dump_rc, 10) {
            break;
        }
        dump_batch(
            image,
            td,
            [format!("/etc/init/{name}"), format!("/etc/init/hw/{name}")],
            &dump_rc,
            ops,
        );
    }

    let (apk_sha256, apk) = if size_gt(&dump_apk, 500) {
        (sha256::file_hex_upper(&dump_apk)?, Verdict::Verified)
    } else {
        ("UNVERIFIED".to_string(), Verdict::NotVerified)
    };
    let (rc_sha256, rc) = if size_gt(&dump_rc, 10) {
        (sha256::file_hex_upper(&dump_rc)?, Verdict::Verified)
    } else {
        ("UNVERIFIED".to_string(), Verdict::NotVerified)
    };

    log.log(format!("{}{}", config::TOKEN_APK_SHA256, apk_sha256));
    log.log(format!("{}{}", config::TOKEN_RC_SHA256, rc_sha256));
    if apk == Verdict::Verified && rc == Verdict::Verified {
        log.log("[OK] Overlay APK + RC verified inside the image.");
    } else {
        log.log(format!(
            "[WARN] Overlay payload not verified (APK: {}, RC: {}).",
            apk.label(),
            rc.label()
        ));
    }

    Ok(VerifyResult { apk_sha256, rc_sha256, apk, rc })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[derive(Default)]
    struct FakeOps {
        apk_size: Option<usize>,
        rc_size: Option<usize>,
        fail: bool,
        calls: Vec<Vec<String>>,
    }

    impl Ext4Ops for FakeOps {
        fn run(
            &mut self,
            _image: &Path,
            commands: &[String],
            _workdir: &Path,
            _log: &mut Logger,
        ) -> Result<String> {
            self.calls.push(commands.to_vec());
            if self.fail {
                return Err(Error::build("dump failed"));
            }
            for c in commands {
                let Some(rest) = c.strip_prefix("dump -p ") else {
                    continue;
                };
                let Some((vpath, host)) = rest.rsplit_once(' ') else {
                    continue;
                };
                let size = if vpath.starts_with("/overlay") {
                    self.apk_size
                } else {
                    self.rc_size
                };
                if let Some(n) = size {
                    std::fs::write(host, vec![b'A'; n]).unwrap();
                }
            }
            Ok(String::new())
        }

        fn path_exists(&mut self, _image: &Path, _vpath: &str, _workdir: &Path) -> Result<bool> {
            Ok(false)
        }
    }

    fn recording_log() -> (Logger, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        struct Shared(std::rc::Rc<std::cell::RefCell<Vec<String>>>);
        impl crate::logging::Sink for Shared {
            fn emit(&mut self, line: &str) {
                self.0.borrow_mut().push(line.to_owned());
            }
        }
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        (Logger::new().with_sink(Shared(lines.clone())), lines)
    }

    #[test]
    fn candidate_chains_match_python() {
        assert_eq!(APK_CANDIDATES.len(), 6);
        assert_eq!(APK_CANDIDATES[0], "OPPO_VoLTE_Overlay.apk");
        assert_eq!(APK_CANDIDATES[5], "GAQ_OPPO_MTK_VoLTE_Overlay.apk");
        assert_eq!(
            rc_candidates(true),
            [
                "hd_oppo_qc_volte.rc",
                "hd_oppo_qc_volte.rc",
                "hd_oppo_mtk_volte.rc",
                "ha_oppo_qc_volte.rc",
                "ha_oppo_mtk_volte.rc",
            ]
        );
        assert_eq!(
            rc_candidates(false),
            [
                "hd_oppo_mtk_volte.rc",
                "hd_oppo_qc_volte.rc",
                "hd_oppo_mtk_volte.rc",
                "ha_oppo_qc_volte.rc",
                "ha_oppo_mtk_volte.rc",
            ]
        );
    }

    #[test]
    fn nothing_dumped_is_not_verified_and_never_prints_verified_tokens() {
        let dir = tmp("volte_verify_nothing");
        let mut ops = FakeOps::default();
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(got.rc, Verdict::NotVerified);
        assert_eq!(got.apk_sha256, "UNVERIFIED");
        assert_eq!(got.rc_sha256, "UNVERIFIED");
        assert_eq!(ops.calls.len(), 6 + 5, "every candidate is tried");
        assert!(ops.calls.iter().all(|c| c.len() == 2));
        let lines = lines.borrow().clone();
        assert_eq!(lines[0], "APK_SHA256=UNVERIFIED");
        assert_eq!(lines[1], "RC_SHA256=UNVERIFIED");
        assert!(lines[2].starts_with("[WARN]"));
        assert!(!lines.iter().any(|l| l.contains("VERIFIED_")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_dump_breaks_the_loop_and_hashes_the_payload() {
        let dir = tmp("volte_verify_full");
        let mut ops = FakeOps { apk_size: Some(2000), rc_size: Some(20), ..Default::default() };
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::Verified);
        assert_eq!(got.rc, Verdict::Verified);
        assert_eq!(got.apk_sha256, sha256::hex_upper(&sha256::digest(&vec![b'A'; 2000])));
        assert_eq!(got.rc_sha256, sha256::hex_upper(&sha256::digest(&[b'A'; 20])));
        assert_eq!(ops.calls.len(), 2, "one batch per payload, then break");
        let lines = lines.borrow().clone();
        assert!(lines[2].starts_with("[OK]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_loop_threshold_and_the_verdict_threshold_differ_for_the_apk() {
        let dir = tmp("volte_verify_threshold");
        // 600 bytes: above the 500 verdict, below the 1000 early-break.
        let mut ops = FakeOps { apk_size: Some(600), rc_size: Some(20), ..Default::default() };
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::Verified);
        assert_eq!(ops.calls.len(), 6 + 1, "APK tries all six names, RC breaks immediately");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_payload_exactly_at_the_threshold_is_not_verified() {
        let dir = tmp("volte_verify_exact");
        let mut ops = FakeOps { apk_size: Some(500), rc_size: Some(10), ..Default::default() };
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, false, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(got.rc, Verdict::NotVerified);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dump_errors_are_swallowed_and_do_not_hide_a_later_success() {
        let dir = tmp("volte_verify_errors");
        let mut ops = FakeOps { fail: true, ..Default::default() };
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(ops.calls.len(), 6 + 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_two_payloads_are_judged_independently() {
        let dir = tmp("volte_verify_mixed");
        let mut ops = FakeOps { apk_size: Some(2000), rc_size: None, ..Default::default() };
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, false, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::Verified);
        assert_eq!(got.rc, Verdict::NotVerified);
        let lines = lines.borrow().clone();
        assert_eq!(lines[0], format!("APK_SHA256={}", got.apk_sha256));
        assert_eq!(lines[1], "RC_SHA256=UNVERIFIED");
        assert!(lines[2].contains("RC: not verified"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_dump_file_from_an_earlier_step_is_not_hashed_as_fresh() {
        let dir = tmp("volte_verify_stale");
        std::fs::write(dir.join("verify_overlay.apk"), vec![b'B'; 300]).unwrap();
        let mut ops = FakeOps { apk_size: Some(2000), rc_size: Some(20), ..Default::default() };
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk_sha256, sha256::hex_upper(&sha256::digest(&vec![b'A'; 2000])));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
