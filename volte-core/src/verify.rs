//! Post-patch verification: dump the overlay APK and both RC copies back out
//! and compare them with the payload that was written.
//!
//! Python's `verify_payload` (`hd_vendor_build.py:1208`) probes a chain of
//! legacy names into one shared host file and calls a payload verified when the
//! file is larger than a threshold. That reports a stale file as fresh (E01) and
//! never checks the second copy written by `overlay_commands` (E02). This port
//! keeps the canonical destinations that the injector writes, dumps each copy
//! into its own host file after deleting it, and requires a SHA-256 match with
//! `payload.apk` / `payload.rc` before anything is called verified.
//!
//! Divergence of 2026-11-08: the APK is now checked at a single destination,
//! `/overlay/<apk_name>` — `overlay_commands` no longer writes a second copy
//! under `/overlay/HDVoLTE/`, so there is nothing left to verify there (see
//! `payload.rs`). The RC still has two destinations (`/etc/init/` and
//! `/etc/init/hw/`) and is unaffected.

use std::path::Path;

use crate::config;
use crate::env::forward_slashes;
use crate::error::Result;
use crate::ext4_tools::Ext4Ops;
use crate::logging::Logger;
use crate::profile;
use crate::sha256;

/// Whether a payload is attested inside the image.
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

/// Why a single dumped copy is or is not attested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyState {
    /// Freshly dumped and byte-identical to the source payload.
    Verified,
    /// The dump reported success but produced no host file.
    Missing,
    /// The dump command failed.
    Failed,
    /// A host file from an earlier run could not be removed, so it cannot be
    /// trusted as the result of this dump.
    Stale,
    /// Freshly dumped, but its SHA-256 differs from the source payload.
    Mismatch,
}

impl CopyState {
    pub fn label(self) -> &'static str {
        match self {
            CopyState::Verified => "verified",
            CopyState::Missing => "missing (the dump wrote nothing)",
            CopyState::Failed => "dump failed",
            CopyState::Stale => "stale host file could not be removed",
            CopyState::Mismatch => "content mismatch",
        }
    }
}

/// One dumped copy and its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyCheck {
    pub vpath: String,
    pub state: CopyState,
    /// SHA-256 of the dumped file, or `UNVERIFIED` when it was not hashed.
    pub sha256: String,
}

/// What `verify_payload` returns for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub apk_sha256: String,
    pub rc_sha256: String,
    pub apk: Verdict,
    pub rc: Verdict,
    pub apk_copies: Vec<CopyCheck>,
    pub rc_copies: Vec<CopyCheck>,
}

const UNVERIFIED: &str = "UNVERIFIED";

/// Dumps one virtual path into its own host file and judges the result.
fn check_copy(
    image: &Path,
    td: &Path,
    vpath: &str,
    host: &Path,
    source_sha256: Option<&str>,
    ops: &mut dyn Ext4Ops,
) -> CopyCheck {
    let mut check = CopyCheck {
        vpath: vpath.to_owned(),
        state: CopyState::Failed,
        sha256: UNVERIFIED.to_owned(),
    };

    if host.exists() && std::fs::remove_file(host).is_err() {
        check.state = CopyState::Stale;
        return check;
    }

    let command = format!("dump -p {vpath} {}", forward_slashes(host));
    let mut quiet = Logger::new();
    if ops.run(image, &[command], td, &mut quiet).is_err() {
        return check;
    }
    if !host.is_file() {
        check.state = CopyState::Missing;
        return check;
    }
    let Ok(sha) = sha256::file_hex_upper(host) else {
        return check;
    };
    check.sha256 = sha;
    check.state = match source_sha256 {
        Some(want) if want == check.sha256 => CopyState::Verified,
        _ => CopyState::Mismatch,
    };
    check
}

fn verify_copies(
    image: &Path,
    td: &Path,
    vpaths: &[String],
    hosts: &[std::path::PathBuf],
    source_sha256: Option<&str>,
    ops: &mut dyn Ext4Ops,
) -> Vec<CopyCheck> {
    vpaths
        .iter()
        .zip(hosts.iter())
        .map(|(vpath, host)| check_copy(image, td, vpath, host, source_sha256, ops))
        .collect()
}

/// `verify_payload` (`:1208`), strengthened for E01/E02. Dump failures are not
/// errors; the result distinguishes Missing/Failed/Stale/Mismatch/Verified.
pub fn verify_payload(
    image: &Path,
    td: &Path,
    qc: bool,
    ops: &mut dyn Ext4Ops,
    log: &mut Logger,
) -> Result<VerifyResult> {
    let apk_source = sha256::file_hex_upper(&td.join("payload.apk")).ok();
    let rc_source = sha256::file_hex_upper(&td.join("payload.rc")).ok();

    let apk_name = profile::apk_name(qc);
    let rc_name = profile::rc_name(qc);
    // Exactly the destinations `overlay_commands` writes for each payload.
    let apk_copies = verify_copies(
        image,
        td,
        &[format!("/overlay/{apk_name}")],
        &[td.join("verify_overlay_root.apk")],
        apk_source.as_deref(),
        ops,
    );
    let rc_copies = verify_copies(
        image,
        td,
        &[
            format!("/etc/init/{rc_name}"),
            format!("/etc/init/hw/{rc_name}"),
        ],
        &[td.join("verify_volte.rc"), td.join("verify_volte_hw.rc")],
        rc_source.as_deref(),
        ops,
    );

    let apk = if apk_copies.iter().all(|c| c.state == CopyState::Verified) {
        Verdict::Verified
    } else {
        Verdict::NotVerified
    };
    let rc = if rc_copies.iter().all(|c| c.state == CopyState::Verified) {
        Verdict::Verified
    } else {
        Verdict::NotVerified
    };
    let apk_sha256 = match apk {
        Verdict::Verified => apk_source.unwrap_or_else(|| UNVERIFIED.to_owned()),
        Verdict::NotVerified => UNVERIFIED.to_owned(),
    };
    let rc_sha256 = match rc {
        Verdict::Verified => rc_source.unwrap_or_else(|| UNVERIFIED.to_owned()),
        Verdict::NotVerified => UNVERIFIED.to_owned(),
    };

    log.log(format!("{}{}", config::TOKEN_APK_SHA256, apk_sha256));
    log.log(format!("{}{}", config::TOKEN_RC_SHA256, rc_sha256));
    for copy in apk_copies.iter().chain(rc_copies.iter()) {
        if copy.state != CopyState::Verified {
            log.log(format!(
                "[WARN] Payload copy {}: {}",
                copy.vpath,
                copy.state.label()
            ));
        }
    }
    if apk == Verdict::Verified && rc == Verdict::Verified {
        log.log("[OK] Overlay APK + RC verified inside the image.");
    } else {
        log.log(format!(
            "[WARN] Overlay payload not verified (APK: {}, RC: {}).",
            apk.label(),
            rc.label()
        ));
    }

    Ok(VerifyResult {
        apk_sha256,
        rc_sha256,
        apk,
        rc,
        apk_copies,
        rc_copies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use std::collections::HashMap;
    use std::path::PathBuf;

    const APK: &[u8] = b"PK\x03\x04overlay-apk-payload";
    const RC: &[u8] = b"on boot\n    setprop persist.volte 1\n";

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    struct FakeFs {
        files: HashMap<String, Vec<u8>>,
        silent: bool,
        fail: bool,
        calls: Vec<(String, String)>,
    }

    impl FakeFs {
        fn new(files: &[(&str, &[u8])]) -> Self {
            FakeFs {
                files: files
                    .iter()
                    .map(|(path, data)| ((*path).to_owned(), data.to_vec()))
                    .collect(),
                silent: false,
                fail: false,
                calls: Vec::new(),
            }
        }
    }

    impl Ext4Ops for FakeFs {
        fn run(
            &mut self,
            _image: &Path,
            commands: &[String],
            _workdir: &Path,
            _log: &mut Logger,
        ) -> Result<String> {
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
                self.calls.push((vpath.to_owned(), host.to_owned()));
                if self.silent {
                    continue;
                }
                match self.files.get(vpath) {
                    Some(data) => std::fs::write(host, data).unwrap(),
                    None => return Err(Error::build(format!("no such file: {vpath}"))),
                }
            }
            Ok(String::new())
        }

        fn path_exists(&mut self, _image: &Path, _vpath: &str, _workdir: &Path) -> Result<bool> {
            Ok(false)
        }
    }

    fn seed_sources(td: &Path) {
        std::fs::write(td.join("payload.apk"), APK).unwrap();
        std::fs::write(td.join("payload.rc"), RC).unwrap();
    }

    fn qc_files() -> Vec<(&'static str, &'static [u8])> {
        vec![
            ("/overlay/OPPO_VoLTE_Overlay.apk", APK),
            ("/etc/init/oppo_qc_volte.rc", RC),
            ("/etc/init/hw/oppo_qc_volte.rc", RC),
        ]
    }

    fn sha(data: &[u8]) -> String {
        sha256::hex_upper(&sha256::digest(data))
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
    fn both_canonical_copies_must_match_the_payload() {
        let dir = tmp("volte_verify_full");
        seed_sources(&dir);
        let mut ops = FakeFs::new(&qc_files());
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();

        assert_eq!(got.apk, Verdict::Verified);
        assert_eq!(got.rc, Verdict::Verified);
        assert_eq!(got.apk_sha256, sha(APK));
        assert_eq!(got.rc_sha256, sha(RC));
        assert!(got
            .apk_copies
            .iter()
            .all(|c| c.state == CopyState::Verified));
        assert!(got.rc_copies.iter().all(|c| c.state == CopyState::Verified));
        assert_eq!(
            ops.calls.len(),
            3,
            "one dump per copy, no candidate retries"
        );
        let hosts: Vec<&str> = ops.calls.iter().map(|(_, h)| h.as_str()).collect();
        let mut unique = hosts.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            3,
            "each copy has its own host file: {hosts:?}"
        );

        let lines = lines.borrow().clone();
        assert_eq!(lines[0], format!("APK_SHA256={}", sha(APK)));
        assert_eq!(lines[1], format!("RC_SHA256={}", sha(RC)));
        assert!(
            lines[2].contains("[OK] Overlay APK + RC verified"),
            "{lines:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mtk_rc_name_is_verified_in_both_init_directories() {
        let dir = tmp("volte_verify_mtk");
        seed_sources(&dir);
        let mut ops = FakeFs::new(&[
            ("/overlay/OPPO_VoLTE_Overlay.apk", APK),
            ("/etc/init/oppo_mtk_volte.rc", RC),
            ("/etc/init/hw/oppo_mtk_volte.rc", RC),
        ]);
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, false, &mut ops, &mut log).unwrap();
        assert_eq!(got.rc, Verdict::Verified);
        assert!(got
            .rc_copies
            .iter()
            .any(|c| c.vpath == "/etc/init/oppo_mtk_volte.rc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_host_file_is_deleted_before_the_dump_and_never_hashed() {
        let dir = tmp("volte_verify_stale");
        seed_sources(&dir);
        std::fs::write(dir.join("verify_overlay_root.apk"), vec![b'S'; 6000]).unwrap();
        let mut ops = FakeFs::new(&qc_files());
        ops.silent = true;
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk_copies[0].state, CopyState::Missing);
        assert_eq!(got.apk_copies[0].sha256, UNVERIFIED);
        assert_eq!(got.apk, Verdict::NotVerified);
        assert!(
            !dir.join("verify_overlay_root.apk").exists(),
            "the stale file is gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_file_that_cannot_be_replaced_is_stale() {
        let dir = tmp("volte_verify_locked");
        seed_sources(&dir);
        // A directory at the host path makes the pre-dump removal fail.
        std::fs::create_dir(dir.join("verify_volte.rc")).unwrap();
        let mut ops = FakeFs::new(&qc_files());
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.rc_copies[0].state, CopyState::Stale);
        assert_eq!(got.rc, Verdict::NotVerified);
        assert_eq!(got.apk, Verdict::Verified);
        let text = lines.borrow().join("\n");
        assert!(
            text.contains("stale host file could not be removed"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_dump_is_reported_as_failed() {
        let dir = tmp("volte_verify_dump_fail");
        seed_sources(&dir);
        let mut ops = FakeFs::new(&qc_files());
        ops.fail = true;
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert!(got.apk_copies.iter().all(|c| c.state == CopyState::Failed));
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(got.rc, Verdict::NotVerified);
        assert_eq!(got.apk_sha256, UNVERIFIED);
        let text = lines.borrow().join("\n");
        assert!(!text.contains("[OK] Overlay APK + RC verified"), "{text}");
        assert!(text.contains("dump failed"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_with_different_content_is_a_mismatch() {
        let dir = tmp("volte_verify_mismatch");
        seed_sources(&dir);
        let mut ops = FakeFs::new(&[
            ("/overlay/OPPO_VoLTE_Overlay.apk", b"PK\x03\x04truncated"),
            ("/etc/init/oppo_qc_volte.rc", RC),
            ("/etc/init/hw/oppo_qc_volte.rc", RC),
        ]);
        let (mut log, lines) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk_copies[0].state, CopyState::Mismatch);
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(got.apk_sha256, UNVERIFIED);
        assert_eq!(got.rc, Verdict::Verified);
        let text = lines.borrow().join("\n");
        assert!(text.contains("content mismatch"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_copy_is_not_ignored() {
        let dir = tmp("volte_verify_missing_copy");
        seed_sources(&dir);
        let mut ops = FakeFs::new(&[
            ("/overlay/OPPO_VoLTE_Overlay.apk", APK),
            ("/etc/init/oppo_qc_volte.rc", RC),
        ]);
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.rc_copies[1].state, CopyState::Failed);
        assert_eq!(got.rc, Verdict::NotVerified);
        assert_eq!(got.apk, Verdict::Verified);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_source_payload_nothing_is_verified() {
        // Mirrors the audit harness: stale host files, no `payload.apk`.
        let dir = tmp("volte_verify_no_source");
        std::fs::write(dir.join("verify_overlay_root.apk"), vec![b'S'; 600]).unwrap();
        std::fs::write(dir.join("verify_volte.rc"), vec![b'R'; 20]).unwrap();
        let mut ops = FakeFs::new(&qc_files());
        ops.silent = true;
        let (mut log, _) = recording_log();
        let got = verify_payload(Path::new("img"), &dir, true, &mut ops, &mut log).unwrap();
        assert_eq!(got.apk, Verdict::NotVerified);
        assert_eq!(got.rc, Verdict::NotVerified);
        assert_eq!(got.apk_sha256, UNVERIFIED);
        assert_eq!(got.rc_sha256, UNVERIFIED);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
