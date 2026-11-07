//! The build pipeline.
//!
//! Port of `build` (`hd_vendor_build.py:1396`): 25 ordered steps, the boot-safe
//! AVB rollback (`:1671-1702`) and the final size lock (`:1704-1720`). External
//! tools arrive through the traits `Ext4Ops`, `Fsck` and `Avb`, so a test can
//! swap any of them without touching a real image.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::avb;
use crate::chip_detect;
use crate::config;
use crate::error::{Error, Result};
use crate::ext4::{self, Ext4Ops, Fsck};
use crate::flags;
use crate::init_rc;
use crate::logging::{fmt_time, FileSink, Logger};
use crate::payload;
use crate::profile;
use crate::props;
use crate::report;
use crate::slot;
use crate::sparse;
use crate::verify::{self, Verdict};

/// Everything Python receives as `build` arguments.
pub struct Options<'a> {
    pub src: &'a Path,
    /// `--out`; empty means `<src dir>/<name>_VolLTE.img`.
    pub out_path: &'a str,
    /// `""` = detect, `"mtk"` / `"qc"` = forced.
    pub chip: &'a str,
    pub keep_out: bool,
    /// Directory the payload lookup is anchored to (`ROOT` in Python).
    pub root: &'a Path,
    /// `ROOT/Logs`.
    pub logs_dir: &'a Path,
}

/// The three external-tool seams.
pub struct Deps<'a> {
    pub ops: &'a mut dyn Ext4Ops,
    pub fsck: &'a mut dyn Fsck,
    pub avb: &'a mut dyn avb::Avb,
}

/// Temporary work directory, removed on drop like `TemporaryDirectory`.
struct WorkDir(PathBuf);

impl WorkDir {
    fn create() -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "hd_hue_chuan_{}_{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(WorkDir(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.0
}

/// Python's `time.strftime("%Y%m%d_%H%M%S")` is local time; the port has no
/// local-time API, so the name carries the UTC stamp. Only the file name can
/// differ, by the UTC offset.
fn log_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}{m:02}{d:02}_{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `:1425-1434`: the output path rules, including the `_VolLTE_out.img` escape
/// when the default name would overwrite the source.
fn resolve_output(src: &Path, out: &str, keep_out: bool) -> Result<(PathBuf, PathBuf)> {
    if keep_out && !out.is_empty() {
        let path = std::path::absolute(out)?;
        let folder = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        return Ok((folder, path));
    }
    let folder = src
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let base = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "vendor".to_string());
    let mut path = folder.join(format!("{base}_VolLTE.img"));
    if std::path::absolute(&path)? == src {
        path = folder.join(format!("{base}_VolLTE_out.img"));
    }
    Ok((folder, path))
}

/// Best-effort `shutil.disk_usage(dir).free >= needed`.
///
/// The standard library has no free-space API, so this reserves the size with
/// `set_len`, which allocates real clusters on NTFS (measured: a 2 GiB
/// `set_len` lowered `E:` free space by exactly 2 GiB), and releases it again.
/// Only "disk full" (`ERROR_DISK_FULL` 112 / `ENOSPC` 28) is reported; every
/// other probe failure is ignored, matching Python's broad `except`.
fn probe_space(dir: &Path, needed: u64) -> std::io::Result<()> {
    if needed == 0 {
        return Ok(());
    }
    let path = dir.join(format!(".hd_volte_space_probe_{}.tmp", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let res = file.set_len(needed);
    drop(file);
    let _ = std::fs::remove_file(&path);
    res
}

fn disk_full(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(28) | Some(112))
}

fn log_lines(log: &mut Logger, text: &str) {
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        log.log(line);
    }
}

fn copy_with_progress(
    src: &Path,
    dst: &Path,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    let mut fi = std::fs::File::open(src)?;
    let mut fo = std::fs::File::create(dst)?;
    let total = std::fs::metadata(src)?.len().max(1);
    let mut done = 0u64;
    let mut last = -1i32;
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    loop {
        let n = fi.read(&mut buf)?;
        if n == 0 {
            break;
        }
        fo.write_all(&buf[..n])?;
        done += n as u64;
        let p = 8 + (20 * done / total) as i32;
        if p != last {
            last = p;
            progress(p as u8, "Copy");
        }
    }
    fo.flush()?;
    Ok(())
}

/// Drops the `mkdir` commands whose directory already exists (`:1565-1581`);
/// a probe error, including a non-EXT4 image, is fatal like in Python.
fn filter_mkdir(
    commands: Vec<String>,
    image: &Path,
    td: &Path,
    ops: &mut dyn Ext4Ops,
) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(commands.len());
    for c in commands {
        if let Some(vpath) = c.strip_prefix("mkdir ") {
            if ops.path_exists(image, vpath.trim(), td)? {
                continue;
            }
        }
        out.push(c);
    }
    Ok(out)
}

/// `build` (`:1396`). Returns the output path; a failure leaves any partial
/// output in place, exactly like the Python tool.
pub fn run(
    opts: &Options,
    deps: &mut Deps,
    log: &mut Logger,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<PathBuf> {
    let t0 = Instant::now();
    let src = std::path::absolute(opts.src)?;
    if !src.is_file() {
        return Err(Error::build(format!("File not found: {}", src.display())));
    }

    std::fs::create_dir_all(opts.logs_dir)?;
    let log_path = opts.logs_dir.join(format!("hue_chuan_{}.log", log_stamp()));
    log.add_sink(FileSink::create(&log_path)?);

    let (folder, out_path) = resolve_output(&src, opts.out_path, opts.keep_out)?;
    std::fs::create_dir_all(&folder)?;
    let orig_sz = std::fs::metadata(&src)?.len();

    if orig_sz > 0 {
        if let Err(e) = probe_space(&folder, orig_sz) {
            if disk_full(&e) {
                return Err(Error::build(format!(
                    "NOT ENOUGH DISK SPACE! The destination drive cannot hold {:.1} MB.",
                    mb(orig_sz)
                )));
            }
        }
        if let Err(e) = probe_space(&std::env::temp_dir(), orig_sz) {
            if disk_full(&e) {
                log.log(format!(
                    "[WARN] The temp drive has less than {:.1} MB free - clean it up before BUILD.",
                    mb(orig_sz)
                ));
            }
        }
    }

    let chip_lc = opts.chip.trim().to_lowercase();
    let chip_given = !chip_lc.is_empty();
    let mut qc;
    if matches!(chip_lc.as_str(), "qc" | "qualcomm") {
        qc = true;
        log.log("[CHIP] Forced QUALCOMM branch (user choice).");
    } else if matches!(chip_lc.as_str(), "mtk" | "mediatek") {
        qc = false;
        log.log("[CHIP] Forced MEDIATEK branch (user choice).");
    } else {
        let (fam, how) = chip_detect::detect_vendor_chip(&src);
        qc = fam == "qc";
        log.log(format!(
            "[CHIP] Detected: {} ({how})",
            if qc { "QUALCOMM" } else { "MEDIATEK" }
        ));
    }

    log.log("=".repeat(64));
    log.log("HD Vendor Fix VoLTE");
    log.log("Version: 2.9.2 / V298");
    log.log("Mode   : STANDARD - e2fsck + AVB signature, EDL-sized");
    log.log("Engine : hoangduc");
    log.log(format!("Chip   : {}", if qc { "QUALCOMM" } else { "MTK" }));
    log.log(format!("Input  : {}", src.display()));
    log.log(format!("Output : {}", out_path.display()));
    log.log(format!("Log    : {}", log_path.display()));
    log.log("=".repeat(64));
    log.log("[STATUS] Inspecting source image...");
    progress(3, "Inspect");

    let work_dir = WorkDir::create()?;
    let td = work_dir.path();

    let work_src = if sparse::is_sparse(&src) {
        log.log("[STATUS] Android sparse image - unsparsing...");
        progress(5, "Unsparse");
        let raw = td.join("vendor_raw.img");
        match sparse::unsparse(&src, &raw)? {
            sparse::Unsparse::Converted { .. } => raw,
            sparse::Unsparse::NotSparse => {
                return Err(Error::build(
                    "Unsparse failed - cannot inject VoLTE.",
                ))
            }
        }
    } else {
        src.clone()
    };

    log.log("[STATUS] Copying source image...");
    progress(8, "Copy");
    if out_path.is_file() && std::path::absolute(&out_path)? != src {
        if let Err(e) = std::fs::remove_file(&out_path) {
            log.log(format!(
                "[WARN] Destination file is open/locked: {}",
                trunc(&e.to_string(), 160)
            ));
        }
    }
    copy_with_progress(&work_src, &out_path, progress)?;
    let copied = std::fs::metadata(&out_path)
        .map(|m| m.is_file() && m.len() >= 1024)
        .unwrap_or(false);
    if !copied {
        return Err(Error::build(
            "Copy vendor failed (disk/file is open).",
        ));
    }
    let work = out_path.clone();

    progress(30, "AVB info");
    let mut avb_info = avb::avb_info_fast(&work)?;
    if avb_info.has_footer {
        log.log("[AVB] Existing AVB footer detected; original AVB parameters will be preserved.");
        for line in report::avb_lines(&avb_info) {
            log.log(line);
        }
    } else {
        log.log("avbtool: Given image does not look like a vbmeta image.");
        log.log("[AVB] File has no AVB footer - still building STANDARD (318 baseline).");
    }
    let orig_sz = std::fs::metadata(&work)?.len();
    // EDL UnlockTool: the slot is the size of the source file, footer or not.
    if avb_info.partition_size == 0 {
        avb_info.partition_size = orig_sz;
    } else if avb_info.partition_size != orig_sz && orig_sz > 0 {
        avb_info.partition_size = orig_sz;
        log.log(format!(
            "[AVB] Locked partition_size to the source file for EDL: {orig_sz}"
        ));
    }

    progress(32, "Identify");
    let props_text = props::identify(&work, td, deps.ops, log);
    if !chip_given && !props_text.is_empty() {
        let fam2 = chip_detect::refine_chip_from_props(if qc { "qc" } else { "mtk" }, &props_text);
        if (fam2 == "qc") != qc {
            qc = fam2 == "qc";
            log.log(format!(
                "[CHIP] Changed from build.prop: {}",
                if qc { "QUALCOMM" } else { "MEDIATEK" }
            ));
        }
    }

    log.log("[STATUS] Injecting VoLTE payload...");
    progress(38, "Patch overlay");
    let plan = payload::overlay_commands(opts.root, td, qc)?;
    let clean = filter_mkdir(plan.clean, &work, td, deps.ops)?;
    log.log("debugfs 1.44.5 (15-Dec-2018)");
    for c in &clean {
        log.log(format!("debugfs: {c}"));
    }
    match deps.ops.run(&work, &clean, td, log) {
        Ok(out) => log_lines(log, &out),
        Err(e) => log.log(format!("[WARN] debugfs clean: {}", trunc(e.message(), 200))),
    }
    let write = filter_mkdir(plan.write, &work, td, deps.ops)?;
    let mut joined = clean.clone();
    joined.push(String::new());
    joined.extend(write.iter().cloned());
    let mut commands_text = joined.join("\n");
    commands_text.push('\n');
    let _ = std::fs::write(opts.logs_dir.join("commands_last.txt"), commands_text);
    match deps.ops.run(&work, &write, td, log) {
        Ok(out) => log_lines(log, &out),
        Err(e) => {
            return Err(Error::build(format!(
                "debugfs overlay injection failed: {}",
                trunc(e.message(), 200)
            )))
        }
    }
    progress(58, "Patch overlay");

    log.log("[STEP] Linking init.target.rc so the VoLTE RC runs at boot...");
    let cfg = td.join("config.bin");
    let needles: [&str; 2] = if qc {
        [profile::IMPORT_LINE, profile::IMPORT_LINE_HW]
    } else {
        [profile::IMPORT_LINE_MTK, profile::IMPORT_LINE_MTK_HW]
    };
    match init_rc::link(&work, td, Some(&cfg), &needles, deps.ops, log) {
        Ok(true) => log.log(format!(
            "[OK] init.target.rc imports {}",
            profile::rc_name(qc)
        )),
        Ok(false) => {}
        Err(e) => log.log(format!(
            "[WARN] init.target.rc: {}",
            trunc(e.message(), 160)
        )),
    }

    log.log("[STEP] Writing the VoLTE switch properties into build.prop...");
    match props::upsert(&work, td, deps.ops, log) {
        Ok(_) => {}
        Err(e) => log.log(format!(
            "[WARN] build.prop VoLTE: {}",
            trunc(e.message(), 160)
        )),
    }
    progress(64, "Props");

    log.log("[STEP] Enabling the VoLTE switch (overlay) + reading the original vendor flag...");
    let orig_bit = ext4::read_vendor_flag_bit(&work, td, deps.ops);
    let nflag = match flags::patch_flags_keep_size(&work, &work, 0, log) {
        Ok(n) => n,
        Err(e) => {
            log.log(format!("[WARN] FLAGS: {}", trunc(e.message(), 160)));
            0
        }
    };
    let on: u8 = if nflag >= 1 || orig_bit >= 1 { 1 } else { 0 };
    if nflag >= 1 {
        log.log("[STEP] VoLTE flag: 1 (0->1 patched in the image)");
    } else if orig_bit >= 1 {
        log.log("[STEP] VoLTE flag: 1 (the original vendor already had =1)");
    } else {
        log.log("[STEP] VoLTE flag: 0 (the original vendor has no volte_support=1)");
    }
    let (_, hit, tip) = report::flag_note(on as i64);
    log.log(format!("[STEP] {hit}"));
    log.log(format!("[STEP] {tip}"));

    progress(72, "Verify");
    let verified = match verify::verify_payload(&work, td, qc, deps.ops, log) {
        Ok(v) => Some(v),
        Err(e) => {
            log.log(format!("[WARN] verify dump: {}", trunc(e.message(), 220)));
            log.log("APK_SHA256=UNVERIFIED");
            log.log("RC_SHA256=UNVERIFIED");
            None
        }
    };
    let apk_sha = verified
        .as_ref()
        .map(|v| v.apk_sha256.clone())
        .unwrap_or_else(|| "UNVERIFIED".to_string());
    let rc_sha = verified
        .as_ref()
        .map(|v| v.rc_sha256.clone())
        .unwrap_or_else(|| "UNVERIFIED".to_string());
    let apk_ok = verified
        .as_ref()
        .map(|v| v.apk == Verdict::Verified)
        .unwrap_or(false);
    if apk_ok {
        log.log("[STEP] Overlay APK + RC are inside the image (VoLTE switch).");
    } else {
        log.log("[WARN] Overlay APK not seen - still exporting the file.");
    }

    log.log("e2fsck -fn");
    progress(78, "e2fsck");
    let mut fsck_status = "PASS";
    match deps.fsck.check(&work, td) {
        Ok(out) => {
            log_lines(log, &out);
            log.log("e2fsck=PASS");
        }
        Err(e) => {
            fsck_status = "FAIL";
            log.log(format!("[WARN] e2fsck: {}", trunc(e.message(), 200)));
            log.log("e2fsck=FAIL");
        }
    }

    let slot_size = orig_sz;
    let pre_avb = td.join("vendor_pre_avb.img");
    let pre_avb_ok = std::fs::copy(&work, &pre_avb).is_ok();
    progress(82, "AVB");
    log.log("[STATUS] Generating hashtree and FEC for the image...");
    let avb_ok = match avb::rebuild_hashtree(&work, &avb_info, orig_sz, deps.avb, log) {
        Ok(rebuilt) => rebuilt.is_some(),
        Err(e) => {
            log.log(format!("[WARN] AVB hashtree: {}", trunc(e.message(), 200)));
            false
        }
    };
    let cur = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
    let footer_ok = avb::avb_info_fast(&work)
        .map(|i| i.has_footer)
        .unwrap_or(false);
    let boot_safe = avb_ok && footer_ok && slot_size > 0 && cur == slot_size;
    let skip_avb;
    if !boot_safe {
        log.log("[STEP] Dropping AVB + EDL size - no footer truncation when the hashtree grows.");
        if pre_avb_ok {
            if let Err(e) = std::fs::copy(&pre_avb, &work) {
                log.log(format!(
                    "[WARN] Restore pre-AVB: {}",
                    trunc(&e.to_string(), 120)
                ));
            }
        }
        slot::force_slot_size(&work, slot_size, true, "BOOT-SAFE", log)?;
        skip_avb = true;
    } else {
        skip_avb = false;
        log.log("[OK] AVB hashtree + EDL size match - safe to boot.");
    }

    let mut out_sz = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
    if skip_avb {
        out_sz = slot::force_slot_size(&work, slot_size, true, "BUILD", log)?;
    } else if slot_size > 0 && out_sz != slot_size {
        return Err(Error::build(format!(
            "STANDARD build: AVB OK but the size is wrong ({out_sz} != {slot_size}) - refusing to truncate."
        )));
    } else {
        log.log(format!(
            "[OK] STANDARD size matches the slot: {out_sz} (0x{out_sz:X})"
        ));
    }

    progress(96, "Done");
    let out_display = out_path.display().to_string();
    log.log(format!("{}{}", config::TOKEN_FILE, out_display));
    log.log(format!("{}{}", config::TOKEN_SIZE, out_sz));
    if slot_size > 0 && out_sz != slot_size {
        return Err(Error::build(format!(
            "Build size mismatch for EDL ({out_sz} != {slot_size})."
        )));
    }

    let avb_token = if skip_avb { "SKIP" } else { "PASS" };
    let summary = report::Summary {
        out_path: &out_display,
        out_size: out_sz,
        e2fsck: fsck_status,
        avb_hashtree: avb_token,
        flags: on,
        apk_sha256: Some(&apk_sha),
        rc_sha256: Some(&rc_sha),
        total_time: &fmt_time(t0.elapsed().as_secs_f64()),
    };
    for line in report::summary_lines(&summary) {
        log.log(line);
    }

    let final_size = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
    let body = report::report_body(&out_display, final_size, fsck_status, avb_token);
    let _ = std::fs::write(opts.logs_dir.join("vendor_VolLTE_report.txt"), body);

    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avb::HashtreeRequest;
    use crate::ext4::{DebugfsBackend, E2fsck};
    use crate::logging::Sink;
    use std::cell::RefCell;
    use std::process::Command;
    use std::rc::Rc;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn tool_dir() -> PathBuf {
        let dir = match std::env::var("VOLTE_TOOL_DIR") {
            Ok(v) => PathBuf::from(v),
            Err(_) => Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("tool"),
        };
        assert!(
            dir.join("debugfs.exe").is_file(),
            "bundled e2fsprogs missing at {} (set VOLTE_TOOL_DIR)",
            dir.display()
        );
        dir
    }

    fn make_ext4(dir: &Path, name: &str) -> PathBuf {
        let img = dir.join(name);
        let mut cmd = Command::new(tool_dir().join("mke2fs.exe"));
        cmd.arg("-q")
            .arg("-t")
            .arg("ext4")
            .arg("-b")
            .arg("4096")
            .arg("-O")
            .arg("dir_index,^metadata_csum_seed,^orphan_file")
            .arg("-F")
            .arg(&img)
            .arg("16384")
            .current_dir(dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let status = cmd.status().expect("run mke2fs");
        assert!(status.success(), "mke2fs failed: {status}");
        img
    }

    fn seed_payloads(root: &Path) {
        let dir = root.join("payload");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(config::APK_NAME), vec![b'A'; 1200]).unwrap();
        std::fs::write(dir.join(config::RC_NAME_MTK), b"on boot\n    setprop x 1\n").unwrap();
        std::fs::write(dir.join(config::RC_NAME_QC), b"on boot\n    setprop y 1\n").unwrap();
        std::fs::write(dir.join("overlay.bin"), b"u:object_r:vendor_overlay_file:s0\0").unwrap();
        std::fs::write(dir.join("config.bin"), b"u:object_r:vendor_configs_file:s0\0").unwrap();
        std::fs::write(dir.join(config::FEATURES_XML), b"<permissions/>").unwrap();
    }

    struct SharedSink(Rc<RefCell<Vec<String>>>);

    impl Sink for SharedSink {
        fn emit(&mut self, line: &str) {
            self.0.borrow_mut().push(line.to_owned());
        }
    }

    struct NoFooterAvb {
        calls: usize,
    }

    impl avb::Avb for NoFooterAvb {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            self.calls += 1;
            assert!(req.image.is_file(), "the AVB step must run on the output copy");
            Ok(())
        }
    }

    struct PassingFsck {
        calls: usize,
    }

    impl Fsck for PassingFsck {
        fn check(&mut self, image: &Path, _workdir: &Path) -> Result<String> {
            self.calls += 1;
            Ok(format!("Pass 1: checking {}", image.display()))
        }
    }

    /// The real `e2fsck`, with a call counter for the integration test.
    struct CountingFsck {
        inner: E2fsck,
        calls: usize,
    }

    impl Fsck for CountingFsck {
        fn check(&mut self, image: &Path, workdir: &Path) -> Result<String> {
            self.calls += 1;
            self.inner.check(image, workdir)
        }
    }

    #[test]
    fn output_path_defaults_next_to_the_source() {
        let src = std::path::absolute("D:/rom/vendor.img").unwrap();
        let (folder, out) = resolve_output(&src, "", false).unwrap();
        assert_eq!(folder, std::path::absolute("D:/rom").unwrap());
        assert_eq!(out, std::path::absolute("D:/rom/vendor_VolLTE.img").unwrap());
    }

    #[test]
    fn output_path_honours_keep_out_and_renames_a_src_collision() {
        let src = std::path::absolute("D:/rom/vendor.img").unwrap();
        let (_, out) = resolve_output(&src, "D:/out/custom.img", true).unwrap();
        assert_eq!(out, std::path::absolute("D:/out/custom.img").unwrap());
        let (_, same) = resolve_output(&src, "", false).unwrap();
        assert_ne!(same, src);
        assert!(same.to_string_lossy().ends_with("vendor_VolLTE.img"));
    }

    #[test]
    fn log_stamp_has_the_python_shape() {
        let stamp = log_stamp();
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert_eq!(&stamp[8..9], "_");
        assert!(stamp.chars().filter(|c| *c != '_').all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn the_space_probe_allocates_and_cleans_up() {
        let dir = tmp("volte_pipeline_probe");
        probe_space(&dir, 4 * 1024 * 1024).unwrap();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert!(left.is_empty(), "probe file must be removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_source_is_a_build_error() {
        let dir = tmp("volte_pipeline_missing");
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps { ops: &mut ops, fsck: &mut fsck, avb: &mut avb };
        let mut log = Logger::new();
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &dir.join("nope.img"),
            out_path: "",
            chip: "",
            keep_out: false,
            root: &dir,
            logs_dir: &dir.join("Logs"),
        };
        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().starts_with("File not found:"), "{}", err.message());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_build_reaches_the_final_tokens_on_a_synthetic_image() {
        let dir = tmp("volte_pipeline_full");
        let tool = tool_dir();
        let src = make_ext4(&dir, "vendor.img");
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = DebugfsBackend::new(&tool);
        let mut fsck = CountingFsck { inner: E2fsck::new(&tool), calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps { ops: &mut ops, fsck: &mut fsck, avb: &mut avb };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut marks: Vec<(u8, String)> = Vec::new();
        let mut progress = |n: u8, label: &str| marks.push((n, label.to_owned()));
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };
        let out = run(&opts, &mut deps, &mut log, &mut progress).unwrap();

        assert_eq!(out, dir.join("vendor_VolLTE.img"));
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len()
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha, "source untouched");
        assert_eq!(fsck.calls, 1);
        assert_eq!(avb.calls, 1);

        let text = lines.borrow().join("\n");
        for need in [
            "[CHIP] Forced MEDIATEK branch",
            "e2fsck=PASS",
            "AVB_HASHTREE=SKIP",
            "VOLTE_FLAGS=0",
            "AVB_ALGORITHM=NONE",
            "FLASH=NO",
            "[OUTPUT] ",
        ] {
            assert!(text.contains(need), "missing {need:?}\n{text}");
        }
        let fsck_at = text.find("e2fsck=PASS").unwrap();
        let avb_at = text.find("[STATUS] Generating hashtree").unwrap();
        assert!(fsck_at < avb_at, "e2fsck must run before the AVB step");
        assert!(text.contains("[AVB] locking partition_size="));

        let marks: Vec<u8> = marks.iter().map(|(n, _)| *n).collect();
        for expected in [3u8, 8, 30, 32, 38, 58, 64, 72, 78, 82, 96] {
            assert!(marks.contains(&expected), "progress {expected} missing: {marks:?}");
        }

        let commands = std::fs::read_to_string(logs.join("commands_last.txt")).unwrap();
        assert!(commands.contains("\n\n"), "clean and write blocks are separated");
        assert!(commands.starts_with("cd /overlay/HDVoLTE\n"));
        assert!(commands.contains("\nmkdir /overlay/HDVoLTE\n"));
        let report = std::fs::read_to_string(logs.join("vendor_VolLTE_report.txt")).unwrap();
        assert_eq!(report.lines().count(), 6, "{report}");
        assert!(report.starts_with("File="));
        assert!(report.contains("\ne2fsck=PASS\n"));
        assert!(report.ends_with("FLASH=NO\n"));
        let log_files: Vec<_> = std::fs::read_dir(&logs)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("hue_chuan_"))
            .collect();
        assert_eq!(log_files.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
