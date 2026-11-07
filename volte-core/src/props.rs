//! VoLTE property injection and device identification.
//!
//! Port of `inject_volte_show_props` (`hd_vendor_build.py:1055`) and
//! `dump_device_props` (`:1167`). `HD_VOLTE_SHOW_PROPS` is byte-identical to the
//! Python tuple: 75 lines, 75 distinct keys, values copied verbatim.

use std::path::Path;

use crate::env::forward_slashes;
use crate::error::Result;
use crate::ext4_tools::Ext4Ops;
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

/// ASCII-lowercased key of a prop line, as bytes. Matching on bytes keeps
/// lines that are not valid UTF-8 intact.
fn key_bytes(line: &[u8]) -> Vec<u8> {
    let Some(eq) = line.iter().position(|&b| b == b'=') else {
        return Vec::new();
    };
    let key = trim_bytes(&line[..eq]);
    key.iter().map(u8::to_ascii_lowercase).collect()
}

fn trim_bytes(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(line.len());
    let end = line
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &line[start..end]
}

fn wanted(line: &[u8]) -> Option<&'static str> {
    let key = key_bytes(line);
    if key.is_empty() {
        return None;
    }
    HD_VOLTE_SHOW_PROPS
        .iter()
        .rev()
        .find(|w| key_bytes(w.as_bytes()) == key)
        .copied()
}

/// Splits bytes into `(line, terminator)` pairs. CRLF is kept as its own
/// terminator and a final line without `\n` has an empty terminator.
fn split_lines(raw: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in raw.iter().enumerate() {
        if b == b'\n' {
            let mut body = &raw[start..i];
            let mut term: &[u8] = b"\n";
            if body.ends_with(b"\r") {
                body = &body[..body.len() - 1];
                term = b"\r\n";
            }
            out.push((body, term));
            start = i + 1;
        }
    }
    if start < raw.len() {
        out.push((&raw[start..], b""));
    }
    out
}

/// Replaces the file content in place. The backend's `write` keeps the inode,
/// so its mode, owner and SELinux label survive; the Python reference deletes
/// the file first and then forces root/0644, which this port intentionally
/// does not do.
fn write_back(image: &Path, td: &Path, vpath: &str, host: &Path, ops: &mut dyn Ext4Ops) -> Result<()> {
    let cmds = vec![format!("write {} {vpath}", forward_slashes(host))];
    let mut quiet = Logger::new();
    ops.run(image, &cmds, td, &mut quiet).map(|_| ())
}

/// Upserts the 75 properties into every property file that exists and returns
/// Python's `changed` count (added + fixed lines). Per-file failures are logged
/// and skipped, never fatal.
///
/// The file is edited as bytes: untouched lines keep their exact bytes and
/// terminators (CRLF included), only lines whose key matches are replaced, and
/// duplicate keys are left in place. Unlike the Python reference, the image
/// file is overwritten instead of deleted, so its mode, owner and label are
/// preserved, and the vendor_file label is only applied to `/build.prop`.
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

        let mut out: Vec<u8> = Vec::with_capacity(raw.len() + 2048);
        let mut seen: Vec<Vec<u8>> = Vec::new();
        let mut dirty = 0usize;
        let mut term: &[u8] = b"\n";
        let mut term_set = false;
        for (body, line_end) in split_lines(&raw) {
            if !term_set && !line_end.is_empty() {
                term = line_end;
                term_set = true;
            }
            let s = trim_bytes(body);
            if s.is_empty() || s[0] == b'#' || !s.contains(&b'=') {
                out.extend_from_slice(body);
                out.extend_from_slice(line_end);
                continue;
            }
            match wanted(s) {
                Some(want) => {
                    if s != want.as_bytes() {
                        out.extend_from_slice(want.as_bytes());
                        dirty += 1;
                    } else {
                        out.extend_from_slice(body);
                    }
                    out.extend_from_slice(line_end);
                    seen.push(key_bytes(s));
                }
                None => {
                    out.extend_from_slice(body);
                    out.extend_from_slice(line_end);
                }
            }
        }

        let mut add: Vec<&str> = Vec::new();
        for line in HD_VOLTE_SHOW_PROPS {
            let key = key_bytes(line.as_bytes());
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            add.push(line);
            seen.push(key);
        }
        if add.is_empty() && dirty == 0 {
            continue;
        }

        if out.last() != Some(&b'\n') {
            out.extend_from_slice(term);
        }
        if !add.is_empty() {
            out.extend_from_slice(term);
            out.extend_from_slice(b"# HD VoLTE show switch");
            out.extend_from_slice(term);
            for (i, line) in add.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(term);
                }
                out.extend_from_slice(line.as_bytes());
            }
            out.extend_from_slice(term);
        }
        std::fs::write(&host, &out)?;

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
                let why: String = e.message().chars().take(120).collect();
                log.log(format!("[WARN] Write {vpath}: {why}"));
                continue;
            }
        }

        // Only build.prop is required to carry the vendor_file label; the other
        // files keep whatever label their inode already had.
        if vpath == "/build.prop" {
            let vf = td.join("vendor_file.bin");
            if !vf.is_file() {
                let _ = std::fs::write(&vf, b"u:object_r:vendor_file:s0\0");
            }
            let cmds = vec![format!(
                "ea_set -f {} {vpath} security.selinux",
                forward_slashes(&vf)
            )];
            let mut quiet = Logger::new();
            let _ = ops.run_best_effort(image, &cmds, td, &mut quiet);
        }
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
        write_error: Option<String>,
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
            if is_write {
                if let Some(msg) = &self.write_error {
                    return Err(Error::build(msg.clone()));
                }
                if self.fail_write {
                    return Err(Error::build("write failed"));
                }
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
        let keys: Vec<String> = HD_VOLTE_SHOW_PROPS
            .iter()
            .map(|l| String::from_utf8(key_bytes(l.as_bytes())).unwrap())
            .collect();
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

    #[test]
    fn a_multibyte_error_message_is_truncated_by_char_without_panicking() {
        let dir = tmp("volte_props_utf8_error");
        let mut ops = FakeOps {
            write_error: Some(format!("{}ế{}", "x".repeat(119), "y".repeat(10))),
            ..Default::default()
        };
        ops.dumps
            .insert("/build.prop".to_owned(), b"ro.xxxx=1\n".to_vec());
        let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(Collect(lines.clone()));
        let changed = upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 0);
        let captured = lines.borrow().clone();
        assert!(
            captured
                .last()
                .is_some_and(|l| l.contains("Could not write the VoLTE show props")),
            "{captured:?}"
        );
        let why = captured[0]
            .strip_prefix("[WARN] Write /build.prop: ")
            .expect("the warning keeps its prefix");
        assert_eq!(why, format!("{}ế", "x".repeat(119)));
        assert_eq!(why.chars().count(), 120);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crlf_duplicates_and_invalid_bytes_survive_the_rewrite() {
        let dir = tmp("volte_props_bytes");
        let body = b"ro.xxxx=1\r\n# keep \xFF\xFE\r\npersist.mtk.volte_support=0\r\n\
PERSIST.RADIO.VT_ENABLE=0\r\nlast.noeol=1";
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), body.to_vec());
        let mut log = quiet();
        let changed = upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 75);
        let out = &ops.written[0].1;

        assert!(
            out.starts_with(b"ro.xxxx=1\r\n# keep \xFF\xFE\r\n"),
            "untouched lines keep their exact bytes"
        );
        assert!(contains(out, b"persist.mtk.volte_support=1\r\n"), "{out:?}");
        assert!(contains(out, b"persist.radio.VT_ENABLE=1\r\n"), "{out:?}");
        assert!(!contains(out, b"persist.mtk.volte_support=0"));
        assert!(
            contains(out, b"last.noeol=1\r\n\r\n# HD VoLTE show switch\r\n"),
            "the appended block follows the detected CRLF style"
        );
        assert!(!contains(out, b"\n\n# HD VoLTE show switch\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_keys_are_all_corrected_and_kept() {
        let dir = tmp("volte_props_dupes");
        let mut ops = FakeOps::default();
        ops.dumps.insert(
            "/build.prop".to_owned(),
            b"persist.mtk.volte_support=0\npersist.mtk.volte_support=2\n".to_vec(),
        );
        let mut log = quiet();
        let changed = upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 76, "two fixed lines plus 74 new keys");
        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
        assert_eq!(text.matches("persist.mtk.volte_support=1\n").count(), 2);
    }

    #[test]
    fn the_rewrite_preserves_inode_metadata_instead_of_resetting_it() {
        let dir = tmp("volte_props_metadata");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/build.prop".to_owned(), b"ro.xxxx=1\n".to_vec());
        let mut log = quiet();
        upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap();
        let flat: Vec<&String> = ops.commands.iter().flatten().collect();
        assert!(
            !flat.iter().any(|c| c.starts_with("rm ") || c.contains("set_inode_field")),
            "{flat:?}"
        );
        assert!(
            flat.iter().any(|c| c.starts_with("write ") && c.ends_with(" /build.prop")),
            "the file is rewritten in place by absolute path: {flat:?}"
        );
        assert!(
            flat.iter().any(|c| c.starts_with("ea_set -f ") && c.ends_with(" /build.prop security.selinux")),
            "{flat:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_build_prop_gets_the_vendor_file_label() {
        let dir = tmp("volte_props_label_scope");
        let mut ops = FakeOps::default();
        ops.dumps.insert("/default.prop".to_owned(), b"ro.yyyy=1\n".to_vec());
        let mut log = quiet();
        assert_eq!(upsert(Path::new("img"), &dir, &mut ops, &mut log).unwrap(), 75);
        let flat: Vec<&String> = ops.commands.iter().flatten().collect();
        assert!(
            !flat.iter().any(|c| c.starts_with("ea_set ")),
            "{flat:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
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
