//! VoLTE property injection and device identification.
//!
//! Port of `inject_volte_show_props` (`hd_vendor_build.py:1055`) and
//! `dump_device_props` (`:1167`). `HD_VOLTE_SHOW_PROPS` is byte-identical to the
//! Python tuple: 75 lines, 75 distinct keys, values copied verbatim.

use std::path::Path;

use crate::env::forward_slashes;
use crate::error::Result;
use crate::ext4::Ext4Ops;
use crate::logging::Logger;

/// `HD_VOLTE_SHOW_PROPS` (`:99-175`).
pub const HD_VOLTE_SHOW_PROPS: [&str; 75] = [
    "persist.dbg.ims_volte_enable=1",
    "persist.dbg.volte_avail_ovr=1",
    "persist.dbg.vt_avail_ovr=1",
    "persist.dbg.wfc_avail_ovr=1",
    "persist.dbg.allow_ims_off=0",
    "persist.radio.calls.on.ims=1",
    "persist.radio.volte_state=1",
    "persist.radio.volte.dan_support=true",
    "persist.data.iwlan.enable=true",
    "persist.data.iwlan=1",
    "persist.data.iwlan.ipsec.ap=1",
    "persist.radio.rat_on=combine",
    "persist.radio.data_ltd_sys_ind=1",
    "persist.radio.data_con_rprt=1",
    "persist.sys.cust.lte_config=true",
    "persist.rcs.supported=1",
    "persist.vendor.radio.volte_avail_ovr=1",
    "persist.vendor.radio.vt_avail_ovr=1",
    "persist.vendor.radio.calls.on.ims=1",
    "persist.vendor.ims.enable=1",
    "persist.vendor.radio.volte.enable=3",
    "persist.sys.oplus.radio.volte_icon=1",
    "persist.sys.oplus.radio.show_volte=1",
    "persist.oplus.radio.volte_icon=1",
    "persist.radio.volte.icon=1",
    "ro.oplus.radio.support_volte=1",
    "ro.oplus.radio.show_volte_switch=1",
    "persist.sys.oplus.radio.volte_switch=1",
    "persist.sys.oppo.radio.show_volte=1",
    "persist.sys.oppo.radio.volte_icon=1",
    "persist.sys.oppo.radio.volte_switch=1",
    "persist.sys.oppo.show_volte=1",
    "persist.radio.oppo.show_volte=1",
    "ro.oppo.radio.show_volte=1",
    "ro.oppo.radio.support_volte=1",
    "persist.sys.oppo.ims.enable=1",
    "persist.sys.oem_vt_enabled=1",
    "persist.sys.oppo.region=VN",
    "persist.sys.oppo.multicc_volte=1",
    "persist.sys.oppo.multisim_volte=1",
    "persist.radio.hvolte.enable=1",
    "persist.show.cu.volte=1",
    "persist.mtk.volte_support=1",
    "persist.mtk.ims_support=1",
    "persist.mtk.volte.enable=1",
    "persist.mtk.vilte_support=1",
    "persist.mtk.viwifi_support=1",
    "persist.vendor.mtk.volte_support=1",
    "persist.vendor.mtk.ims_support=1",
    "persist.vendor.mtk.vilte_support=1",
    "persist.vendor.mtk.viwifi_support=1",
    "persist.vendor.mtk.volte.enable=3",
    "persist.vendor.radio.volte_state=3",
    "persist.vendor.mtk_ct_volte_support=1",
    "persist.vendor.mtk_wfc_support=1",
    "persist.vendor.volte_support=1",
    "persist.vendor.ims_support=1",
    "persist.vendor.mtk_dynamic_ims_switch=0",
    "ro.vendor.mtk_ims_support=1",
    "ro.vendor.mtk_volte_support=1",
    "ro.vendor.mtk_vilte_support=1",
    "ro.vendor.mtk_wfc_support=1",
    "ro.mtk_ims_support=1",
    "ro.mtk_volte_support=1",
    "ro.mtk_vilte_support=1",
    "persist.sys.oplus.radio.show_vilte=1",
    "ro.oplus.radio.support_vilte=1",
    "persist.radio.vise.volte=1",
    "persist.radio.VT_ENABLE=1",
    "persist.sys.volte_show_switch=1",
    "persist.sys.volte_status=1",
    "persist.vendor.volte_show_switch=1",
    "persist.vivo.volte_support=1",
    "persist.sys.vivo.volte=1",
    "ro.vivo.volte.support=1",
];

/// The six property files, in Python's order (`:1067-1074`).
pub const PROP_FILES: [(&str, &str); 6] = [
    ("/build.prop", "hd_volte_build.prop"),
    ("/default.prop", "hd_volte_default.prop"),
    ("/etc/build.prop", "hd_volte_etc_build.prop"),
    ("/odm/etc/build.prop", "hd_volte_odm_build.prop"),
    ("/euclid_build.prop", "hd_volte_euclid.prop"),
    ("/vendor_build.prop", "hd_volte_vendor_build.prop"),
];

/// Keys whose device identity may be logged (`:1189-1192`), never widened.
pub const IDENTITY_KEYS: [&str; 10] = [
    "ro.product.model",
    "ro.product.device",
    "ro.product.name",
    "ro.build.product",
    "ro.oppo.market.name",
    "ro.build.version.release",
    "ro.build.version.oplusrom",
    "ro.build.version.opporom",
    "ro.vendor.oplus.market.name",
    "ro.product.oppo.model",
];

fn key_of(line: &str) -> String {
    line.split_once('=')
        .map(|(k, _)| k.trim().to_lowercase())
        .unwrap_or_default()
}

fn wanted(line: &str) -> Option<&'static str> {
    HD_VOLTE_SHOW_PROPS
        .iter()
        .rev()
        .find(|w| key_of(w) == key_of(line))
        .copied()
}

fn write_back(image: &Path, td: &Path, vpath: &str, host: &Path, ops: &mut dyn Ext4Ops) -> Result<()> {
    let (dir, name) = match vpath.rsplit_once('/') {
        Some((d, n)) if !d.is_empty() => (d, n),
        Some((_, n)) => ("/", n),
        None => ("/", vpath),
    };
    let cmds = vec![
        format!("rm {vpath}"),
        format!("cd {dir}"),
        format!("write {} {name}", forward_slashes(host)),
        format!("set_inode_field {name} uid 0"),
        format!("set_inode_field {name} gid 0"),
        format!("set_inode_field {name} mode 0x81A4"),
    ];
    let mut quiet = Logger::new();
    ops.run(image, &cmds, td, &mut quiet).map(|_| ())
}

/// Upserts the 75 properties into every property file that exists and returns
/// Python's `changed` count (added + fixed lines). Per-file failures are logged
/// and skipped, never fatal.
pub fn upsert(image: &Path, td: &Path, ops: &mut dyn Ext4Ops, log: &mut Logger) -> Result<usize> {
    let mut changed = 0usize;
    for (vpath, host_name) in PROP_FILES {
        let host = td.join(host_name);
        if host.is_file() {
            let _ = std::fs::remove_file(&host);
        }
        let dump = format!("dump {vpath} {}", forward_slashes(&host));
        let mut quiet = Logger::new();
        if ops.run_best_effort(image, &[dump], td, &mut quiet).is_err() {
            continue;
        }
        let big_enough = std::fs::metadata(&host)
            .map(|m| m.is_file() && m.len() >= 8)
            .unwrap_or(false);
        if !big_enough {
            continue;
        }
        let Ok(raw) = std::fs::read(&host) else {
            continue;
        };
        let text = String::from_utf8_lossy(&raw);

        let mut lines_out: Vec<&str> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut dirty = 0usize;
        for ln in text.lines() {
            let s = ln.trim();
            if s.is_empty() || s.starts_with('#') || !s.contains('=') {
                lines_out.push(ln);
                continue;
            }
            let key = key_of(s);
            if let Some(want) = wanted(s) {
                if s != want {
                    lines_out.push(want);
                    dirty += 1;
                } else {
                    lines_out.push(ln);
                }
                seen.push(key);
                continue;
            }
            lines_out.push(ln);
        }

        let mut add: Vec<&str> = Vec::new();
        for line in HD_VOLTE_SHOW_PROPS {
            let key = key_of(line);
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            add.push(line);
            seen.push(key);
        }
        if add.is_empty() && dirty == 0 {
            continue;
        }

        let mut out = lines_out.join("\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if !add.is_empty() {
            out.push_str("\n# HD VoLTE show switch\n");
            out.push_str(&add.join("\n"));
            out.push('\n');
        }
        std::fs::write(&host, out.as_bytes())?;

        match write_back(image, td, vpath, &host, ops) {
            Ok(()) => {
                let n = add.len() + dirty;
                changed += n;
                log.log(format!(
                    "[OK] Upserted {n} VoLTE props into {vpath} (+{} new, fixed {dirty})",
                    add.len()
                ));
            }
            Err(e) => {
                log.log(format!("[WARN] Write {vpath}: {}", &e.message()[..e.message().len().min(120)]));
                continue;
            }
        }

        // build.prop requires the vendor_file label.
        let vf = td.join("vendor_file.bin");
        if !vf.is_file() {
            let _ = std::fs::write(&vf, b"u:object_r:vendor_file:s0\0");
        }
        let (dir, name) = match vpath.rsplit_once('/') {
            Some((d, n)) if !d.is_empty() => (d, n),
            Some((_, n)) => ("/", n),
            None => ("/", vpath),
        };
        let cmds = vec![
            format!("cd {dir}"),
            format!(
                "ea_set -f {} {name} security.selinux",
                forward_slashes(&vf)
            ),
        ];
        let mut quiet = Logger::new();
        let _ = ops.run_best_effort(image, &cmds, td, &mut quiet);
    }
    if changed == 0 {
        log.log("[WARN] Could not write the VoLTE show props (build.prop dump/write failed).");
    }
    Ok(changed)
}

/// Dumps `/build.prop` and `/default.prop`, logs at most six allow-listed
/// identity lines and returns the raw text (used by chip refinement).
pub fn identify(image: &Path, td: &Path, ops: &mut dyn Ext4Ops, log: &mut Logger) -> String {
    let dumps = [("/build.prop", "hd_id_build.prop"), ("/default.prop", "hd_id_default.prop")];
    let cmds: Vec<String> = dumps
        .iter()
        .map(|(src, name)| format!("dump -p {src} {}", forward_slashes(&td.join(name))))
        .collect();
    let mut quiet = Logger::new();
    if ops.run_best_effort(image, &cmds, td, &mut quiet).is_err() {
        return String::new();
    }
    let mut blob: Vec<u8> = Vec::new();
    for (_, name) in dumps {
        let p = td.join(name);
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.is_file() && meta.len() > 8 {
                if let Ok(data) = std::fs::read(&p) {
                    blob.push(b'\n');
                    blob.extend_from_slice(&data);
                }
            }
        }
    }
    if blob.len() < 20 {
        return String::new();
    }
    let text = String::from_utf8_lossy(&blob).into_owned();
    let mut info: Vec<String> = Vec::new();
    for ln in text.lines() {
        let s = ln.trim();
        let Some((k, v)) = s.split_once('=') else {
            continue;
        };
        if IDENTITY_KEYS.contains(&k.trim()) {
            info.push(format!("{}={}", k.trim(), v.trim()));
            if info.len() >= 6 {
                break;
            }
        }
    }
    if !info.is_empty() {
        log.log(format!("[INFO] {}", info.join(" | ")));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[derive(Default)]
    struct FakeOps {
        dumps: HashMap<String, Vec<u8>>,
        commands: Vec<Vec<String>>,
        written: Vec<(String, Vec<u8>)>,
        fail_dump: bool,
        fail_write: bool,
    }

    impl Ext4Ops for FakeOps {
        fn run(
            &mut self,
            _image: &Path,
            commands: &[String],
            _workdir: &Path,
            _log: &mut Logger,
        ) -> Result<String> {
            let mut is_write = false;
            for c in commands {
                let dump = c.strip_prefix("dump -p ").or_else(|| c.strip_prefix("dump "));
                if let Some(rest) = dump {
                    if self.fail_dump {
                        return Err(Error::build("dump failed"));
                    }
                    let mut parts = rest.splitn(2, ' ');
                    let vpath = parts.next().unwrap_or("");
                    let host = parts.next().unwrap_or("");
                    if let Some(data) = self.dumps.get(vpath) {
                        std::fs::write(host, data).unwrap();
                    }
                } else if let Some(rest) = c.strip_prefix("write ") {
                    is_write = true;
                    let mut parts = rest.splitn(2, ' ');
                    let host = parts.next().unwrap_or("");
                    let name = parts.next().unwrap_or("");
                    self.written
                        .push((name.to_owned(), std::fs::read(host).unwrap_or_default()));
                }
            }
            if is_write && self.fail_write {
                return Err(Error::build("write failed"));
            }
            self.commands.push(commands.to_vec());
            Ok(String::new())
        }

        fn path_exists(&mut self, _image: &Path, _vpath: &str, _workdir: &Path) -> Result<bool> {
            Ok(false)
        }
    }

    fn quiet() -> Logger {
        Logger::new().with_sink(Vec::<String>::new())
    }

    #[test]
    fn show_props_has_75_unique_keys_and_python_values() {
        assert_eq!(HD_VOLTE_SHOW_PROPS.len(), 75);
        let keys: Vec<String> = HD_VOLTE_SHOW_PROPS.iter().map(|l| key_of(l)).collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 75);
        assert_eq!(HD_VOLTE_SHOW_PROPS[0], "persist.dbg.ims_volte_enable=1");
        assert_eq!(HD_VOLTE_SHOW_PROPS[5], "persist.radio.calls.on.ims=1");
        assert_eq!(HD_VOLTE_SHOW_PROPS[74], "ro.vivo.volte.support=1");
        assert_eq!(HD_VOLTE_SHOW_PROPS[37], "persist.sys.oppo.region=VN");
        assert_eq!(HD_VOLTE_SHOW_PROPS[68], "persist.radio.VT_ENABLE=1");
    }

    #[test]
    fn every_prop_is_appended_once_and_comments_survive() {
        let dir = tmp("volte_props_append");
        let body = b"# comment\n\nro.unknown=keep\nro.product.model=X\n";
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), body.to_vec());
        let mut log = quiet();
        let changed = upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 75);
        assert_eq!(ops.written.len(), 1);
        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
        assert!(text.starts_with("# comment\n\nro.unknown=keep\nro.product.model=X\n"));
        assert!(text.contains("\n# HD VoLTE show switch\n"));
        for line in HD_VOLTE_SHOW_PROPS {
            assert_eq!(text.matches(line).count(), 1, "{line}");
        }
        assert!(ops.commands.iter().flatten().any(|c| c.starts_with("ea_set -f ")));
        let label = std::fs::read(dir.join("vendor_file.bin")).unwrap();
        assert_eq!(label, b"u:object_r:vendor_file:s0\0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_wrong_values_are_replaced_and_counted_as_dirty() {
        let dir = tmp("volte_props_replace");
        let mut ops = FakeOps::default();
        ops.dumps.insert(
            "/build.prop".to_owned(),
            b"persist.mtk.volte_support=0\npersist.sys.oppo.region=TH\n".to_vec(),
        );
        let mut log = quiet();
        let changed = upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 75);
        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
        assert!(text.contains("persist.mtk.volte_support=1\n"));
        assert!(text.contains("persist.sys.oppo.region=VN\n"));
        assert_eq!(text.matches("persist.mtk.volte_support=1").count(), 1);
        assert!(!text.contains("persist.mtk.volte_support=0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_match_case_insensitively_but_values_stay_verbatim() {
        let dir = tmp("volte_props_case");
        let mut ops = FakeOps::default();
        ops.dumps
            .insert("/build.prop".to_owned(), b"PERSIST.RADIO.VT_ENABLE=0\n".to_vec());
        let mut log = quiet();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap(), 75);
        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
        assert!(text.contains("persist.radio.VT_ENABLE=1"));
        assert_eq!(text.matches("persist.radio.VT_ENABLE=1").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let dir = tmp("volte_props_idempotent");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), b"ro.xxxx=1\n".to_vec());
        let mut log = quiet();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap(), 75);
        let first = ops.written.clone();
        let mut ops2 = FakeOps::default();
        ops2.dumps
            .insert("/build.prop".to_owned(), first[0].1.clone());
        let calls_before = ops2.commands.len();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops2, &mut log).unwrap(), 0);
        assert!(ops2.written.is_empty(), "nothing to rewrite");
        assert_eq!(ops2.commands.len(), calls_before + 6, "one dump per property file");
        assert!(ops2
            .commands
            .iter()
            .flatten()
            .all(|c| c.starts_with("dump ")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_short_files_are_skipped() {
        let dir = tmp("volte_props_skip");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/default.prop".to_owned(), b"short\n".to_vec());
        let mut log = Logger::new();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap(), 0);
        assert!(ops.written.is_empty());
        let dir2 = tmp("volte_props_skip2");
        let mut ops2 = FakeOps {
            fail_dump: true,
            ..Default::default()
        };
        assert_eq!(upsert(Path::new("img"), &dir2, &mut ops2, &mut log).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn a_write_failure_is_logged_and_does_not_stop_the_other_files() {
        let dir = tmp("volte_props_write_fail");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), b"ro.xxxx=1\n".to_vec());
        ops.dumps.insert("/default.prop".to_owned(), b"ro.yyyy=1\n".to_vec());
        ops.fail_write = true;
        let mut log = quiet();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap(), 0);
        assert_eq!(ops.written.len(), 2, "both files were attempted");
        assert!(
            !ops.commands.iter().flatten().any(|c| c.starts_with("ea_set -f ")),
            "no label is applied after a failed write"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn identify_fixture() -> (PathBuf, FakeOps) {
        let dir = tmp("volte_props_identify");
        let mut ops = FakeOps::default();
        let body = b"ro.product.model=CPH1803\nro.serialno=SECRET123\npersist.radio.imei=999\n\
ro.product.device=op4k7f\nro.build.version.release=9\nro.build.version.oplusrom=A.34\n\
ro.product.name=x\nro.build.product=y\nro.oppo.market.name=z\nro.vendor.oplus.market.name=w\n\
ro.product.oppo.model=v\n";
        ops.dumps.insert("/build.prop".to_owned(), body.to_vec());
        (dir, ops)
    }

    struct Collect(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

    impl crate::logging::Sink for Collect {
        fn emit(&mut self, line: &str) {
            self.0.borrow_mut().push(line.to_owned());
        }
    }

    #[test]
    fn identify_logs_at_most_six_allowlisted_lines() {
        let (dir, mut ops) = identify_fixture();
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(Collect(lines.clone()));
        let text = identify(Path::new("img"), &dir, &mut ops, &mut log);
        assert!(text.contains("ro.serialno=SECRET123"), "raw text is returned");
        let captured = lines.borrow().clone();
        assert_eq!(captured.len(), 1);
        let line = &captured[0];
        let fields: Vec<&str> = line.trim_start_matches("[INFO] ").split(" | ").collect();
        assert_eq!(fields.len(), 6);
        assert_eq!(fields[0], "ro.product.model=CPH1803");
        assert!(!line.contains("SECRET123"));
        assert!(!line.contains("imei"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identify_rejects_a_short_blob() {
        let dir = tmp("volte_props_identify_short");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), b"x=1\n".to_vec());
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        assert_eq!(identify(Path::new("img"), &dir, &mut ops, &mut log), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identify_returns_empty_when_the_dump_fails() {
        let dir = tmp("volte_props_identify_fail");
        let mut ops = FakeOps {
            fail_dump: true,
            ..Default::default()
        };
        let mut log = Logger::new();
        assert_eq!(identify(Path::new("img"), &dir, &mut ops, &mut log), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
