//! Linking the VoLTE RC into an init file.
//!
//! Port of `link_init_target` (`hd_vendor_build.py:965`): dump the first
//! candidate init file that exists, append the missing `import` lines, write it
//! back with mode `0x81A4`, uid/gid 0 and the SELinux label from `config.bin`.
//! A missing or undumpable init file is not an error — Android 9+ loads
//! `/vendor/etc/init` on its own and the build continues.

use std::path::Path;

use crate::env::forward_slashes;
use crate::error::Result;
use crate::ext4_tools::Ext4Ops;
use crate::logging::Logger;
use crate::profile;

/// The 21 MTK / 8 QC candidates in Python's order (shared tail included).
pub fn candidate_vpaths(needles: &[&str]) -> Vec<&'static str> {
    let mut vpaths: Vec<&'static str> = Vec::new();
    if needles.iter().any(|n| n.contains("hd_oppo_mtk_volte.rc")) {
        vpaths.extend(profile::INIT_VPATHS_MTK);
    }
    if needles.iter().any(|n| n.contains("hd_oppo_qc_volte.rc")) {
        vpaths.extend(profile::INIT_VPATHS_QC);
    }
    vpaths.extend(profile::INIT_VPATHS_SHARED);
    vpaths
}

/// Splits a virtual path into the directory to `cd` into and the file name.
/// `/name` belongs to the root and `a/b/c` to `a/b`; an empty path or a
/// trailing slash has no writable target and returns `None`. There is no
/// silent `/etc/init/hw` fallback: the write must land where the dump came
/// from.
fn split_vpath(vpath: &str) -> Option<(String, String)> {
    if vpath.is_empty() || vpath.ends_with('/') {
        return None;
    }
    if let Some(rest) = vpath.strip_prefix('/') {
        if rest.is_empty() {
            return None;
        }
        return Some(match rest.rsplit_once('/') {
            Some((dir, name)) if !dir.is_empty() => (format!("/{dir}"), name.to_owned()),
            Some((_, name)) => ("/".to_owned(), name.to_owned()),
            None => ("/".to_owned(), rest.to_owned()),
        });
    }
    Some(match vpath.rsplit_once('/') {
        Some((dir, name)) if !dir.is_empty() => (dir.to_owned(), name.to_owned()),
        _ => (".".to_owned(), vpath.to_owned()),
    })
}

/// Returns `true` when an init file was dumped and now imports every needle,
/// `false` when no candidate could be dumped. Errors from the final write are
/// propagated, like the Python call inside its `try` block.
pub fn link(
    image: &Path,
    td: &Path,
    cfgbin: Option<&Path>,
    needles: &[&str],
    ops: &mut dyn Ext4Ops,
    log: &mut Logger,
) -> Result<bool> {
    let host = td.join("init.target.rc");
    let mut found = "/etc/init/hw/init.target.rc".to_owned();
    let mut dumped = false;
    for vpath in candidate_vpaths(needles) {
        if host.is_file() {
            let _ = std::fs::remove_file(&host);
        }
        let command = format!("dump {vpath} {}", forward_slashes(&host));
        let mut quiet = Logger::new();
        if ops
            .run_best_effort(image, &[command], td, &mut quiet)
            .is_err()
        {
            continue;
        }
        let big_enough = std::fs::metadata(&host)
            .map(|m| m.is_file() && m.len() > 8)
            .unwrap_or(false);
        if big_enough {
            dumped = true;
            found = vpath.to_owned();
            break;
        }
    }
    if !dumped {
        log.log("[INFO] Could not dump init.target.rc - Android 9+ loads /vendor/etc/init itself.");
        return Ok(false);
    }

    let raw = std::fs::read(&host)?;
    let mut text = String::from_utf8_lossy(&raw).into_owned();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let missing: Vec<&str> = needles
        .iter()
        .copied()
        .filter(|n| !n.is_empty() && !text.contains(n))
        .collect();
    if missing.is_empty() {
        log.log(format!("[OK] {found} already imports HD VoLTE."));
        return Ok(true);
    }
    for needle in &missing {
        text.push('\n');
        text.push_str(needle);
        text.push('\n');
    }
    std::fs::write(&host, text.as_bytes())?;

    let (dest_dir, dest_name) = split_vpath(&found)
        .ok_or_else(|| crate::error::Error::build(format!("invalid init path: {found}")))?;
    let mut cmds = vec![
        "mkdir /etc".to_owned(),
        "mkdir /etc/init".to_owned(),
        "mkdir /etc/init/hw".to_owned(),
        format!("rm {found}"),
        format!("cd {dest_dir}"),
        format!("write {} {dest_name}", forward_slashes(&host)),
        format!("set_inode_field {dest_name} uid 0"),
        format!("set_inode_field {dest_name} gid 0"),
        format!("set_inode_field {dest_name} mode 0x81A4"),
    ];
    if let Some(cfg) = cfgbin {
        if cfg.is_file() {
            cmds.push(format!(
                "ea_set -f {} {dest_name} security.selinux",
                forward_slashes(cfg)
            ));
        }
    }
    // debugfs 1.44.5 allocates an inode before failing on an existing mkdir.
    let mut filtered = Vec::with_capacity(cmds.len());
    for c in cmds {
        if let Some(vpath) = c.strip_prefix("mkdir ") {
            if ops.path_exists(image, vpath.trim(), td)? {
                continue;
            }
        }
        filtered.push(c);
    }
    ops.run(image, &filtered, td, log)?;
    log.log(format!("[OK] Linked the init script into {found}"));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    struct FakeOps {
        dumps: HashMap<String, Vec<u8>>,
        existing: Vec<String>,
        fail_dump: bool,
        written: Vec<(String, Vec<u8>)>,
        commands: Vec<String>,
    }

    impl FakeOps {
        fn new(dumps: &[(&str, &[u8])]) -> Self {
            FakeOps {
                dumps: dumps
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
                    .collect(),
                existing: vec!["/etc".to_owned(), "/etc/init".to_owned()],
                fail_dump: false,
                written: Vec::new(),
                commands: Vec::new(),
            }
        }
    }

    impl Ext4Ops for FakeOps {
        fn run(
            &mut self,
            _image: &Path,
            commands: &[String],
            _workdir: &Path,
            _log: &mut Logger,
        ) -> Result<String> {
            for c in commands {
                self.commands.push(c.clone());
                if let Some(rest) = c.strip_prefix("dump ") {
                    if self.fail_dump {
                        return Err(crate::error::Error::build("dump failed"));
                    }
                    let mut parts = rest.splitn(2, ' ');
                    let vpath = parts.next().unwrap_or("");
                    let host = parts.next().unwrap_or("");
                    if let Some(data) = self.dumps.get(vpath) {
                        std::fs::write(host, data).unwrap();
                    }
                } else if let Some(rest) = c.strip_prefix("write ") {
                    let mut parts = rest.splitn(2, ' ');
                    let host = parts.next().unwrap_or("");
                    let name = parts.next().unwrap_or("");
                    self.written
                        .push((name.to_owned(), std::fs::read(host).unwrap_or_default()));
                }
            }
            Ok(String::new())
        }

        fn path_exists(&mut self, _image: &Path, vpath: &str, _workdir: &Path) -> Result<bool> {
            Ok(self.existing.iter().any(|e| e == vpath))
        }
    }

    fn qc_needles() -> Vec<&'static str> {
        vec![profile::IMPORT_LINE, profile::IMPORT_LINE_HW]
    }

    #[test]
    fn candidate_counts_are_21_for_mtk_and_8_for_qc() {
        let mtk = candidate_vpaths(&[profile::IMPORT_LINE_MTK]);
        assert_eq!(mtk.len(), 21);
        assert_eq!(mtk[0], "/etc/init/hw/init.project.rc");
        assert_eq!(&mtk[18..], &profile::INIT_VPATHS_SHARED);
        let qc = candidate_vpaths(&qc_needles());
        assert_eq!(qc.len(), 8);
        assert_eq!(qc[0], "/etc/init/hw/init.target.rc");
        assert_eq!(&qc[5..], &profile::INIT_VPATHS_SHARED);
    }

    #[test]
    fn inserts_the_missing_imports_and_keeps_the_file_lf_terminated() {
        let dir = tmp("volte_initrc_insert");
        std::fs::write(
            dir.join("config.bin"),
            b"u:object_r:vendor_configs_file:s0\0",
        )
        .unwrap();
        let mut ops = FakeOps::new(&[("/etc/init/hw/init.target.rc", b"on boot\n    exec x")]);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let cfg = dir.join("config.bin");
        let ok = link(
            Path::new("img"),
            &dir,
            Some(&cfg),
            &qc_needles(),
            &mut ops,
            &mut log,
        )
        .unwrap();
        assert!(ok);
        assert_eq!(ops.written.len(), 1);
        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
        assert!(text.starts_with("on boot\n    exec x\n"));
        assert!(text.contains("\nimport /vendor/etc/init/hd_oppo_qc_volte.rc\n"));
        assert!(text.contains("\nimport /vendor/etc/init/hw/hd_oppo_qc_volte.rc\n"));
        assert_eq!(text.matches(profile::IMPORT_LINE).count(), 1);
        assert!(text.ends_with('\n'));
        assert!(!text.contains('\r'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_imports_are_idempotent_and_skip_the_write_back() {
        let dir = tmp("volte_initrc_idempotent");
        let body = format!(
            "on boot\n{}\n{}\n",
            profile::IMPORT_LINE,
            profile::IMPORT_LINE_HW
        );
        let mut ops = FakeOps::new(&[("/etc/init/hw/init.target.rc", body.as_bytes())]);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert!(link(
            Path::new("img"),
            &dir,
            None,
            &qc_needles(),
            &mut ops,
            &mut log
        )
        .unwrap());
        assert!(ops.written.is_empty());
        assert_eq!(ops.existing.len(), 2, "no mkdir probe on the early return");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undumpable_init_file_returns_false_without_error() {
        let dir = tmp("volte_initrc_undumpable");
        let mut ops = FakeOps::new(&[]);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert!(!link(
            Path::new("img"),
            &dir,
            None,
            &qc_needles(),
            &mut ops,
            &mut log
        )
        .unwrap());
        assert!(ops.written.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dump_error_skips_the_candidate_instead_of_aborting() {
        let dir = tmp("volte_initrc_dump_error");
        let mut ops = FakeOps::new(&[]);
        ops.fail_dump = true;
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert!(!link(
            Path::new("img"),
            &dir,
            None,
            &qc_needles(),
            &mut ops,
            &mut log
        )
        .unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_directories_drop_their_mkdir_commands() {
        let dir = tmp("volte_initrc_mkdir_filter");
        std::fs::write(dir.join("config.bin"), b"label\0").unwrap();
        let mut ops = FakeOps::new(&[("/etc/init/hw/init.target.rc", b"on boot\n    exec x\n")]);
        ops.existing = vec![
            "/etc".to_owned(),
            "/etc/init".to_owned(),
            "/etc/init/hw".to_owned(),
        ];
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let cfg = dir.join("config.bin");
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<String>>>, FakeOps);
        impl Ext4Ops for Recorder {
            fn run(
                &mut self,
                image: &Path,
                commands: &[String],
                workdir: &Path,
                log: &mut Logger,
            ) -> Result<String> {
                if commands.iter().all(|c| !c.starts_with("dump ")) {
                    self.0.lock().unwrap().extend(commands.iter().cloned());
                }
                self.1.run(image, commands, workdir, log)
            }
            fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
                self.1.path_exists(image, vpath, workdir)
            }
        }
        let mut rec = Recorder(recorded.clone(), ops);
        assert!(link(
            Path::new("img"),
            &dir,
            Some(&cfg),
            &qc_needles(),
            &mut rec,
            &mut log
        )
        .unwrap());
        let cmds = recorded.lock().unwrap().clone();
        assert!(!cmds.iter().any(|c| c.starts_with("mkdir ")), "{cmds:?}");
        assert_eq!(cmds[0], "rm /etc/init/hw/init.target.rc");
        assert!(cmds.iter().any(|c| c.starts_with("ea_set -f ")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_vpath_keeps_the_directory_the_file_came_from() {
        assert_eq!(
            split_vpath("/init.target.rc"),
            Some(("/".to_owned(), "init.target.rc".to_owned()))
        );
        assert_eq!(
            split_vpath("/etc/init/hw/init.target.rc"),
            Some(("/etc/init/hw".to_owned(), "init.target.rc".to_owned()))
        );
        assert_eq!(
            split_vpath("etc/init/init.vendor.rc"),
            Some(("etc/init".to_owned(), "init.vendor.rc".to_owned()))
        );
        assert_eq!(
            split_vpath("init.rc"),
            Some((".".to_owned(), "init.rc".to_owned()))
        );
        assert_eq!(split_vpath(""), None);
        assert_eq!(split_vpath("/"), None);
        assert_eq!(split_vpath("/etc/"), None, "a trailing slash names no file");
    }

    #[test]
    fn a_root_init_file_is_written_back_to_the_root() {
        let dir = tmp("volte_initrc_root");
        let mut ops = FakeOps::new(&[("/init.target.rc", b"on boot\n    exec x\n")]);
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        struct Collect(std::rc::Rc<std::cell::RefCell<Vec<String>>>);
        impl crate::logging::Sink for Collect {
            fn emit(&mut self, line: &str) {
                self.0.borrow_mut().push(line.to_owned());
            }
        }
        let mut log = Logger::new().with_sink(Collect(lines.clone()));
        assert!(link(
            Path::new("img"),
            &dir,
            None,
            &qc_needles(),
            &mut ops,
            &mut log
        )
        .unwrap());
        assert_eq!(ops.written.len(), 1);
        assert_eq!(ops.written[0].0, "init.target.rc");
        assert!(ops.commands.iter().any(|c| c == "rm /init.target.rc"));
        assert!(
            ops.commands.iter().any(|c| c == "cd /"),
            "{:?}",
            ops.commands
        );
        assert!(
            !ops.commands.iter().any(|c| c == "cd /etc/init/hw"),
            "the root file must not be redirected: {:?}",
            ops.commands
        );
        let text = lines.borrow().join("\n");
        assert!(text.contains("Linked the init script into /init.target.rc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_nested_init_file_is_written_back_to_its_own_directory() {
        let dir = tmp("volte_initrc_nested");
        let snap = "/etc/init/hw/init.target.rc";
        let mut ops = FakeOps::new(&[(snap, b"on boot\n    exec x\n")]);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert!(link(
            Path::new("img"),
            &dir,
            None,
            &qc_needles(),
            &mut ops,
            &mut log
        )
        .unwrap());
        assert_eq!(ops.written[0].0, "init.target.rc");
        assert!(ops.commands.iter().any(|c| c == "cd /etc/init/hw"));
        assert!(!ops.commands.iter().any(|c| c == "cd /"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
