//! Payload lookup and overlay command generation.
//!
//! Ports `payload_dirs` / `payload_file` / `payload_required` /
//! `first_payload` (`hd_vendor_build.py:285-313`) and `overlay_cmds` (`:857`).
//! The write list keeps the Python command order. The cleanup list is
//! intentionally different: it removes each candidate by absolute path instead
//! of `cd`-ing into a directory first, so a missing directory cannot leave the
//! shell at `/` and turn the following `rm` into a no-op.

use std::path::{Path, PathBuf};

use crate::config;
use crate::env::forward_slashes;
use crate::error::{Error, Result};
use crate::profile;

/// `payload_dirs`: `payload\` → `tool\payload\` → root → `vendor_fix_payload\`,
/// existing directories only, de-duplicated in that order.
pub fn payload_dirs(root: &Path) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for cand in [
        root.join("payload"),
        root.join("tool").join("payload"),
        root.to_path_buf(),
        root.join("vendor_fix_payload"),
    ] {
        if cand.is_dir() && !seen.contains(&cand) {
            seen.push(cand);
        }
    }
    seen
}

/// `payload_file`: first directory that holds `name`.
pub fn payload_file(root: &Path, name: &str) -> Option<PathBuf> {
    for dir in payload_dirs(root) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// `payload_required`: same lookup, but a missing file is a build error.
pub fn payload_required(root: &Path, name: &str) -> Result<PathBuf> {
    payload_file(root, name).ok_or_else(|| {
        Error::new(
            crate::error::ErrorKind::Missing,
            format!("missing payload: {name} (place it in the tool directory)"),
        )
    })
}

/// `first_payload`: first candidate that exists; the error lists the chain.
pub fn first_payload(root: &Path, candidates: &[&str]) -> Result<PathBuf> {
    for name in candidates {
        if let Some(p) = payload_file(root, name) {
            return Ok(p);
        }
    }
    Err(Error::new(
        crate::error::ErrorKind::Missing,
        format!("missing payload: {}", candidates.join(" / ")),
    ))
}

/// The two `debugfs` command lists produced by `overlay_cmds`.
pub struct OverlayPlan {
    pub clean: Vec<String>,
    pub write: Vec<String>,
}

/// Copies the selected payloads into `td` and returns the clean/write command
/// lists. This is `overlay_cmds(td, qc)` with the script root passed in.
pub fn overlay_commands(root: &Path, td: &Path, qc: bool) -> Result<OverlayPlan> {
    let apk_src = first_payload(root, profile::apk_candidates(qc))?;
    let rc_src = first_payload(root, profile::rc_candidates(qc))?;
    let apk_name = profile::apk_name(qc);
    let rc_name = profile::rc_name(qc);

    let payload_apk = td.join("payload.apk");
    let payload_rc = td.join("payload.rc");
    std::fs::copy(&apk_src, &payload_apk)?;
    std::fs::copy(&rc_src, &payload_rc)?;
    std::fs::copy(
        payload_required(root, "overlay.bin")?,
        td.join("overlay.bin"),
    )?;
    std::fs::copy(payload_required(root, "config.bin")?, td.join("config.bin"))?;

    let clean: Vec<String> = [
        "rm /overlay/HDVoLTE/OPPO_VoLTE_Overlay.apk",
        "rm /overlay/HDVoLTE/HD_OPPO_MTK_VoLTE_Overlay.apk",
        "rm /overlay/HDVoLTE/HD_OPPO_QC_VoLTE_Overlay.apk",
        "rm /overlay/OPPO_VoLTE_Overlay.apk",
        "rm /overlay/HD_OPPO_MTK_VoLTE_Overlay.apk",
        "rm /overlay/HD_OPPO_QC_VoLTE_Overlay.apk",
        "rmdir /overlay/HDVoLTE",
        "rm /etc/init/hd_oppo_mtk_volte.rc",
        "rm /etc/init/hd_oppo_qc_volte.rc",
        "rm /etc/init/hw/hd_oppo_qc_volte.rc",
        "rm /etc/init/hw/hd_oppo_mtk_volte.rc",
        "mkdir /overlay",
        "mkdir /etc",
        "mkdir /etc/init",
        "mkdir /etc/init/hw",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let ovdir = "HDVoLTE";
    let mut write = vec![
        format!("mkdir /overlay/{ovdir}"),
        format!("cd /overlay/{ovdir}"),
        format!("write {} {}", forward_slashes(&payload_apk), apk_name),
        format!("set_inode_field {apk_name} mode 0x81A4"),
        format!("set_inode_field {apk_name} uid 0"),
        format!("set_inode_field {apk_name} gid 0"),
        format!("ea_set -f overlay.bin {apk_name} security.selinux"),
        "cd /overlay".to_string(),
        format!("set_inode_field {ovdir} mode 0x41ED"),
        format!("set_inode_field {ovdir} uid 0"),
        format!("set_inode_field {ovdir} gid 2000"),
        format!("ea_set -f overlay.bin {ovdir} security.selinux"),
        "cd /overlay".to_string(),
        format!("write {} {}", forward_slashes(&payload_apk), apk_name),
        format!("set_inode_field {apk_name} mode 0x81A4"),
        format!("set_inode_field {apk_name} uid 0"),
        format!("set_inode_field {apk_name} gid 0"),
        format!("ea_set -f overlay.bin {apk_name} security.selinux"),
        "cd /etc/init".to_string(),
        format!("write {} {}", forward_slashes(&payload_rc), rc_name),
        format!("set_inode_field {rc_name} mode 0x81A4"),
        format!("set_inode_field {rc_name} uid 0"),
        format!("set_inode_field {rc_name} gid 0"),
        format!("ea_set -f config.bin {rc_name} security.selinux"),
        "cd /etc/init/hw".to_string(),
        format!("write {} {}", forward_slashes(&payload_rc), rc_name),
        format!("set_inode_field {rc_name} mode 0x81A4"),
        format!("set_inode_field {rc_name} uid 0"),
        format!("set_inode_field {rc_name} gid 0"),
        format!("ea_set -f config.bin {rc_name} security.selinux"),
    ];

    if let Some(feat) = payload_file(root, config::FEATURES_XML) {
        let feat_host = td.join(config::FEATURES_XML);
        std::fs::copy(&feat, &feat_host)?;
        write.push("mkdir /etc/permissions".to_string());
        write.push("cd /etc/permissions".to_string());
        write.push(format!("rm {}", config::FEATURES_XML));
        write.push(format!(
            "write {} {}",
            forward_slashes(&feat_host),
            config::FEATURES_XML
        ));
        write.push(format!(
            "set_inode_field {} mode 0x81A4",
            config::FEATURES_XML
        ));
        write.push(format!("set_inode_field {} uid 0", config::FEATURES_XML));
        write.push(format!("set_inode_field {} gid 0", config::FEATURES_XML));
        write.push(format!(
            "ea_set -f config.bin {} security.selinux",
            config::FEATURES_XML
        ));
    }

    // MTK devices may still need the QC RC; the Python tool writes it too.
    if !qc {
        if let Some(qc_rc) = payload_file(root, config::RC_NAME_QC) {
            let qc_host = td.join("payload_qc.rc");
            std::fs::copy(qc_rc, &qc_host)?;
            write.push("cd /etc/init".to_string());
            write.push(format!(
                "write {} {}",
                forward_slashes(&qc_host),
                config::RC_NAME_QC
            ));
            write.push(format!(
                "set_inode_field {} mode 0x81A4",
                config::RC_NAME_QC
            ));
            write.push(format!("set_inode_field {} uid 0", config::RC_NAME_QC));
            write.push(format!("set_inode_field {} gid 0", config::RC_NAME_QC));
            write.push(format!(
                "ea_set -f config.bin {} security.selinux",
                config::RC_NAME_QC
            ));
        }
    }

    Ok(OverlayPlan { clean, write })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn seed_payloads(root: &Path, features: bool, qc_rc: bool) {
        let dir = root.join("payload");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(config::APK_NAME), b"APKDATA").unwrap();
        std::fs::write(dir.join(config::RC_NAME_MTK), b"mtk rc").unwrap();
        std::fs::write(dir.join(config::RC_NAME_QC), b"qc rc").unwrap();
        std::fs::write(dir.join("overlay.bin"), b"overlay label\0").unwrap();
        std::fs::write(dir.join("config.bin"), b"config label\0").unwrap();
        if features {
            std::fs::write(dir.join(config::FEATURES_XML), b"<permissions/>").unwrap();
        }
        if !qc_rc {
            std::fs::remove_file(dir.join(config::RC_NAME_QC)).unwrap();
        }
    }

    #[test]
    fn payload_dirs_order_and_dedupe() {
        let root = tmp("volte_payload_dirs");
        std::fs::create_dir_all(root.join("payload")).unwrap();
        std::fs::create_dir_all(root.join("tool").join("payload")).unwrap();
        std::fs::create_dir_all(root.join("vendor_fix_payload")).unwrap();
        let dirs = payload_dirs(&root);
        assert_eq!(dirs.len(), 4);
        assert_eq!(dirs[0], root.join("payload"));
        assert_eq!(dirs[1], root.join("tool").join("payload"));
        assert_eq!(dirs[2], root);
        assert_eq!(dirs[3], root.join("vendor_fix_payload"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn payload_dirs_skips_missing_and_duplicates() {
        let root = tmp("volte_payload_dirs_dup");
        std::fs::create_dir_all(root.join("payload")).unwrap();
        let dirs = payload_dirs(&root);
        assert_eq!(dirs, vec![root.join("payload"), root.clone()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn first_directory_wins() {
        let root = tmp("volte_payload_first");
        std::fs::create_dir_all(root.join("payload")).unwrap();
        std::fs::write(root.join("payload").join("same.bin"), b"first").unwrap();
        std::fs::write(root.join("same.bin"), b"second").unwrap();
        let got = payload_file(&root, "same.bin").unwrap();
        assert_eq!(got, root.join("payload").join("same.bin"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_payload_is_a_typed_error() {
        let root = tmp("volte_payload_missing");
        let err = payload_required(&root, "overlay.bin").unwrap_err();
        assert_eq!(err.kind(), crate::error::ErrorKind::Missing);
        assert!(err.message().contains("overlay.bin"));
        let err = first_payload(&root, &["a.apk", "b.apk"]).unwrap_err();
        assert!(err.message().contains("a.apk / b.apk"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn qc_command_counts_match_python() {
        let root = tmp("volte_overlay_qc");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, true, false);
        let plan = overlay_commands(&root, &td, true).unwrap();
        assert_eq!(plan.clean.len(), 15, "clean commands");
        assert_eq!(plan.write.len(), 38, "write commands");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mtk_command_counts_match_python() {
        let root = tmp("volte_overlay_mtk");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, true, true);
        let plan = overlay_commands(&root, &td, false).unwrap();
        assert_eq!(plan.clean.len(), 15, "clean commands");
        assert_eq!(plan.write.len(), 44, "write commands");
        assert!(
            plan.write
                .iter()
                .any(|c| c.contains("write") && c.ends_with("hd_oppo_qc_volte.rc")),
            "MTK must also write the QC RC"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cleanup_uses_absolute_paths_and_never_changes_directory() {
        let root = tmp("volte_overlay_clean_abs");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, true, false);
        let plan = overlay_commands(&root, &td, true).unwrap();
        assert!(
            !plan.clean.iter().any(|c| c.starts_with("cd ")),
            "{:?}",
            plan.clean
        );
        for cmd in &plan.clean {
            let path = cmd
                .strip_prefix("rm ")
                .or_else(|| cmd.strip_prefix("rmdir "))
                .or_else(|| cmd.strip_prefix("mkdir "))
                .unwrap_or_else(|| panic!("unexpected clean command: {cmd}"));
            assert!(path.starts_with('/'), "{cmd} must use an absolute path");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn command_details_match_python_values() {
        let root = tmp("volte_overlay_details");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, true, false);
        let plan = overlay_commands(&root, &td, true).unwrap();
        let all = plan.clean.join("\n") + "\n" + &plan.write.join("\n");
        assert!(all.contains("set_inode_field OPPO_VoLTE_Overlay.apk mode 0x81A4"));
        assert!(all.contains("set_inode_field HDVoLTE mode 0x41ED"));
        assert!(all.contains("set_inode_field HDVoLTE gid 2000"));
        assert!(all.contains("ea_set -f overlay.bin OPPO_VoLTE_Overlay.apk security.selinux"));
        assert!(all.contains("ea_set -f config.bin hd_oppo_qc_volte.rc security.selinux"));
        assert!(all.contains("mkdir /etc/permissions"));
        assert_eq!(
            plan.write
                .iter()
                .filter(|c| c.starts_with("ea_set "))
                .count(),
            6
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_features_xml_drops_only_its_block() {
        let root = tmp("volte_overlay_no_features");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, false, false);
        let qc = overlay_commands(&root, &td, true).unwrap();
        assert_eq!(qc.write.len(), 30);
        assert!(!qc.write.iter().any(|c| c.contains("hd_volte_features.xml")));
        std::fs::write(root.join("payload").join(config::RC_NAME_QC), b"qc rc").unwrap();
        let mtk = overlay_commands(&root, &td, false).unwrap();
        assert_eq!(mtk.write.len(), 36);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copies_land_in_the_work_directory() {
        let root = tmp("volte_overlay_copies");
        let td = root.join("work");
        std::fs::create_dir_all(&td).unwrap();
        seed_payloads(&root, true, true);
        let _ = overlay_commands(&root, &td, false).unwrap();
        for name in [
            "payload.apk",
            "payload.rc",
            "payload_qc.rc",
            "overlay.bin",
            "config.bin",
            config::FEATURES_XML,
        ] {
            assert!(td.join(name).is_file(), "{name} must be copied");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
