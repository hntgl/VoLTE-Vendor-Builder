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
use crate::error::{Error, ErrorKind, Result};
use crate::ext4_tools::{self, Ext4Ops, Fsck};
use crate::flags;
use crate::init_rc;
use crate::logging::{fmt_time, FileSink, Logger};
use crate::path_id;
use crate::payload;
use crate::profile;
use crate::props;
use crate::report;
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
        let path = std::env::temp_dir().join(format!("volte_patch_{}_{nanos}", std::process::id()));
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

/// Extra destination space reserved beyond the two image-sized files: AVB may
/// pad to a MiB block and the slot lock rounds the file, and the vbmeta rewrite
/// needs scratch that is not part of the image payload.
const AVB_HEADROOM: u64 = 64 * 1024 * 1024;

/// Peak bytes the build needs on each kind of volume.
///
/// Destination: the output image, one spare image-sized reserve and
/// [`AVB_HEADROOM`]. Temp: the working image, plus the unsparsed copy when the
/// source is sparse. The old peak also covered the removed pre-AVB backup, so
/// this stays conservative.
fn space_need(raw_sz: u64, sparse: bool) -> (u64, u64) {
    let dest = raw_sz.saturating_mul(2).saturating_add(AVB_HEADROOM);
    let temp = raw_sz.saturating_add(if sparse { raw_sz } else { 0 });
    (dest, temp)
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
    if path_id::same_file(&path, src) {
        path = folder.join(format!("{base}_VolLTE_out.img"));
    }
    Ok((folder, path))
}

/// Best-effort free-space reservation.
///
/// The standard library has no free-space API and this crate forbids `unsafe`,
/// so the probe extends a file with `set_len` and relies on the filesystem
/// allocating the requested space. That holds on NTFS (measured: a 2 GiB
/// `set_len` lowered `E:` free space by exactly 2 GiB); on filesystems with
/// delayed allocation the probe is advisory only, which is why every write in
/// the build still turns `ENOSPC` into a hard error. Only "disk full"
/// (`ERROR_DISK_FULL` 112 / `ENOSPC` 28) is reported; every other probe failure
/// is inconclusive and ignored.
fn probe_space(dir: &Path, needed: u64) -> std::io::Result<()> {
    if needed == 0 {
        return Ok(());
    }
    let path = dir.join(format!(".volte_space_probe_{}.tmp", std::process::id()));
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

fn copy_with_progress(src: &Path, dst: &Path, progress: &mut dyn FnMut(u8, &str)) -> Result<()> {
    if path_id::same_file(src, dst) {
        return Err(Error::build(format!(
            "Refusing to copy {} onto itself; the source was not modified.",
            src.display()
        )));
    }
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
    let log_path = opts
        .logs_dir
        .join(format!("build_vendor_volte_{}.log", log_stamp()));
    log.add_sink(FileSink::create(&log_path)?);

    let (folder, out_path) = resolve_output(&src, opts.out_path, opts.keep_out)?;
    if path_id::same_file(&src, &out_path) {
        return Err(Error::build(format!(
            "Refusing to build: the output path is the same file as the source image ({}). \
             Choose another --out path; the source was not modified.",
            out_path.display()
        )));
    }
    std::fs::create_dir_all(&folder)?;
    let compressed_sz = std::fs::metadata(&src)?.len();
    // The sparse header declares the real size; an invalid one is rejected here,
    // before any output is created.
    let sparse_sz = sparse::output_size(&src)?;
    let src_is_sparse = sparse_sz.is_some();
    let raw_sz = sparse_sz.unwrap_or(compressed_sz);
    let temp_dir = std::env::temp_dir();

    if raw_sz > 0 {
        let (dest_need, temp_need) = space_need(raw_sz, src_is_sparse);
        let shared_volume = path_id::same_volume(&folder, &temp_dir);
        let dest_probe = if shared_volume {
            probe_space(&folder, dest_need.saturating_add(temp_need))
        } else {
            probe_space(&folder, dest_need)
        };
        if let Err(e) = dest_probe {
            if disk_full(&e) {
                let need = if shared_volume {
                    dest_need.saturating_add(temp_need)
                } else {
                    dest_need
                };
                return Err(Error::build(format!(
                    "NOT ENOUGH DISK SPACE! The destination drive cannot hold {:.1} MB.",
                    mb(need)
                )));
            }
        }
        if !shared_volume {
            if let Err(e) = probe_space(&temp_dir, temp_need) {
                if disk_full(&e) {
                    log.log(format!(
                        "[WARN] The temp drive has less than {:.1} MB free - clean it up before BUILD.",
                        mb(temp_need)
                    ));
                }
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
    log.log("VoLTE Vendor Patcher");
    log.log("Version: 2.9.2 / V298");
    log.log("Mode   : STANDARD - e2fsck + AVB signature, EDL-sized");
    log.log("Engine : HT");
    log.log(format!("Chip   : {}", if qc { "QUALCOMM" } else { "MTK" }));
    log.log(format!("Input  : {}", src.display()));
    log.log(format!("Output : {}", out_path.display()));
    log.log(format!("Log    : {}", log_path.display()));
    log.log("=".repeat(64));
    log.log("[STATUS] Inspecting source image...");
    progress(3, "Inspect");

    let work_dir = WorkDir::create()?;
    let td = work_dir.path();

    let work_src = if src_is_sparse {
        log.log("[STATUS] Android sparse image - unsparsing...");
        progress(5, "Unsparse");
        let raw = td.join("vendor_raw.img");
        match sparse::unsparse(&src, &raw)? {
            sparse::Unsparse::Converted { .. } => raw,
            sparse::Unsparse::NotSparse => {
                return Err(Error::build("Unsparse failed - cannot inject VoLTE."))
            }
        }
    } else {
        src.clone()
    };

    log.log("[STATUS] Copying source image...");
    progress(8, "Copy");
    if out_path.is_file() && !path_id::same_file(&out_path, &src) {
        // Keep the previous output as a single-generation backup so a failed
        // build cannot destroy the image the user is currently flashing.
        let bak = {
            let mut s = out_path.as_os_str().to_owned();
            s.push(".bak");
            PathBuf::from(s)
        };
        if bak.is_file() {
            let _ = std::fs::remove_file(&bak);
        }
        if let Err(e) = std::fs::rename(&out_path, &bak) {
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
        return Err(Error::build("Copy vendor failed (disk/file is open)."));
    }
    let work = out_path.clone();

    progress(30, "AVB info");
    let mut avb_info = avb::avb_info_fast(&work)?;
    if avb_info.has_footer && !avb_info.footer_at_exact_tail {
        return Err(Error::build(
            "AVB footer found but not positioned at the end of the partition \
             (found via backward scan, not at the expected tail offset). This \
             usually means the source vendor.img was already modified or \
             corrupted by a previous process. Refusing to build from a \
             suspect source image - re-dump the original vendor.img and retry.",
        ));
    }
    if !avb_info.unsupported_descriptor_tags.is_empty() {
        for tag in &avb_info.unsupported_descriptor_tags {
            log.log(format!(
                "[WARN] AVB vbmeta: unsupported descriptor tag {tag}"
            ));
        }
        let tags = avb_info
            .unsupported_descriptor_tags
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::build(format!(
            "input image has unsupported AVB descriptor tag(s) {tags}; the rebuild cannot \
             preserve them (Tag 3 is the kernel command line descriptor) - refusing to rebuild"
        )));
    }
    if avb_info.has_chain_descriptor {
        return Err(Error::build(
            "input image has a chain partition descriptor (Tag 4); the current rebuild does not preserve chain descriptors - refusing to rebuild to avoid breaking the AVB verification chain",
        ));
    }
    if avb_info.descriptor_kind == avb::DescriptorKind::HashOnly {
        return Err(Error::build(
            "input image uses a whole-partition hash descriptor (Tag 2); the rebuild can only \
             produce a hashtree descriptor (Tag 1), so the format would change - refusing to rebuild",
        ));
    }
    if avb_info.fec_roots > 0 {
        return Err(Error::build(format!(
            "input image has FEC (fec_roots={}) but the rebuild always runs with \
             --do_not_generate_fec; dropping FEC would change the verified-boot metadata - \
             refusing to rebuild",
            avb_info.fec_roots
        )));
    }
    if avb_info.has_footer
        && (avb_info.data_block_size != config::BLOCK as u32
            || avb_info.hash_block_size != config::BLOCK as u32)
    {
        return Err(Error::build(format!(
            "input image uses {} / {} byte hashtree blocks, but the rebuild only supports \
             {block}-byte blocks - refusing to write different geometry",
            avb_info.data_block_size,
            avb_info.hash_block_size,
            block = config::BLOCK
        )));
    }
    if avb_info.persistent_digest {
        return Err(Error::build(
            "input image uses a persistent on-device root digest (digest_len = 0); the rebuild \
             cannot refresh a digest that is not stored in the descriptor - refusing to rebuild",
        ));
    }
    if avb_info.has_footer {
        log.log("[AVB] Existing AVB footer detected; original AVB parameters will be preserved.");
        for line in report::avb_lines_input(&avb_info) {
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
    log.log("EXT4 engine: rust-fs-ext4 (in-process)");
    for c in &clean {
        log.log(format!("ext4: {c}"));
    }
    // The clean batch removes paths that may legitimately be absent.
    match deps.ops.run_best_effort(&work, &clean, td, log) {
        Ok(out) => log_lines(log, &out),
        Err(e) => log.log(format!("[WARN] ext4 clean: {}", trunc(e.message(), 200))),
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
                "ext4 overlay injection failed: {}",
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
    let orig_bit = ext4_tools::read_vendor_flag_bit(&work, td, deps.ops);
    let nflag = match flags::patch_flags_keep_size(&work, &work, 0, log) {
        Ok(n) => n,
        Err(e) => {
            // A read/write failure means we do not know the flag state; it must
            // not be reported as "0" and the build must not continue.
            let phase = match e.kind() {
                ErrorKind::Read => "reading",
                ErrorKind::Write => "writing",
                _ => "accessing",
            };
            let why = trunc(e.message(), 160);
            log.log(format!("[ERROR] FLAGS {phase}: {why}"));
            return Err(Error::build(format!(
                "VoLTE flag patch failed while {phase} the image - refusing to continue: {why}"
            )));
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
    let payload_ok = verified
        .as_ref()
        .map(|v| v.apk == Verdict::Verified && v.rc == Verdict::Verified)
        .unwrap_or(false);
    if payload_ok {
        log.log("[STEP] Overlay APK + RC are inside the image (VoLTE switch).");
    } else {
        return Err(Error::build(
            "Overlay APK/RC not verified inside the image - refusing to export",
        ));
    }

    log.log("e2fsck -fn");
    progress(78, "e2fsck");
    let fsck_status = match deps.fsck.check(&work, td) {
        Ok(out) => {
            log_lines(log, &out);
            log.log("e2fsck=PASS");
            report::FsckStatus::Pass
        }
        Err(e) => {
            log.log(format!("[ERROR] e2fsck: {}", trunc(e.message(), 200)));
            log.log("e2fsck=FAIL");
            return Err(Error::build(format!(
                "e2fsck failed - refusing to report the image as flashable: {}",
                trunc(e.message(), 200)
            )));
        }
    };

    let slot_size = orig_sz;
    progress(82, "AVB");
    let avb_token: &str = if !avb_info.has_footer {
        log.log("[STEP] Source image has no AVB footer - skipping AVB (no footer added).");
        "SKIP"
    } else {
        log.log("[STATUS] Generating hashtree for the image (FEC disabled)...");
        let avb_reason = match avb::rebuild_hashtree(&work, &avb_info, orig_sz, deps.avb, log) {
            Ok(Some(_)) => None,
            Ok(None) => Some("the rebuilt hashtree grew past the slot".to_string()),
            Err(e) => {
                let text = trunc(e.message(), 200);
                log.log(format!("[WARN] AVB hashtree: {text}"));
                Some(text)
            }
        };
        match avb_reason {
            None => {
                log.log("[OK] AVB structure + EDL size verified - bootloader acceptance not tested.");
                "PASS"
            }
            Some(reason) => {
                log.log("[ERROR] AVB rebuild failed - refusing to export the patched image.");
                // rebuild_hashtree restores the saved tail on every failure
                // path, but the filesystem was already patched: exporting it
                // with the restored (old) AVB metadata would bootloop a device
                // with verified boot. Confirm the restore, then drop the file.
                let cur = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
                let footer_ok = avb::avb_info_fast(&work)
                    .map(|i| i.has_footer)
                    .unwrap_or(false);
                let _ = std::fs::remove_file(&work);
                if !footer_ok || slot_size == 0 || cur != slot_size {
                    return Err(Error::build(format!(
                        "AVB rebuild failed: {reason}. The image tail could not be restored \
                         to its pre-AVB state ({cur} bytes, footer={footer_ok}) - no image was exported."
                    )));
                }
                return Err(Error::build(format!(
                    "AVB rebuild failed: {reason}. The patched filesystem would not match the \
                     AVB metadata, so no image was exported."
                )));
            }
        }
    };

    let out_sz = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
    if slot_size > 0 && out_sz != slot_size {
        return Err(Error::build(format!(
            "STANDARD build: AVB OK but the size is wrong ({out_sz} != {slot_size}) - refusing to truncate."
        )));
    }
    log.log(format!(
        "[OK] STANDARD size matches the slot: {out_sz} (0x{out_sz:X})"
    ));

    progress(96, "Done");
    let out_display = out_path.display().to_string();
    log.log(format!("{}{}", config::TOKEN_FILE, out_display));
    log.log(format!("{}{}", config::TOKEN_SIZE, out_sz));

    let summary = report::Summary {
        out_path: &out_display,
        out_size: out_sz,
        e2fsck: fsck_status,
        avb_hashtree: avb_token,
        avb_signature: if avb_token == "PASS" { "DROPPED" } else { "NONE" },
        flags: on,
        apk_sha256: Some(&apk_sha),
        rc_sha256: Some(&rc_sha),
        total_time: &fmt_time(t0.elapsed().as_secs_f64()),
    };
    for line in report::summary_lines(&summary) {
        log.log(line);
    }

    let final_size = std::fs::metadata(&work).map(|m| m.len()).unwrap_or(0);
    let body = report::report_body(
        &out_display,
        final_size,
        fsck_status,
        avb_token,
        summary.avb_signature,
    );
    let _ = std::fs::write(opts.logs_dir.join("vendor_VolLTE_report.txt"), body);

    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avb::HashtreeRequest;
    use crate::ext4_tools::E2fsck;
    use crate::logging::Sink;
    use crate::rustfs_ext4::RustFsExt4Backend;
    use std::cell::RefCell;
    use std::io::{Seek, SeekFrom, Write};
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
            Err(_) => Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("tool"),
        };
        assert!(
            dir.join("mke2fs.exe").is_file(),
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

    /// 32 MiB filesystem padded to the 64 MiB slot, so AVB metadata can live
    /// in the tail without overwriting filesystem blocks.
    fn make_ext4_with_slack(dir: &Path, name: &str) -> PathBuf {
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
            .arg("8192")
            .current_dir(dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let status = cmd.status().expect("run mke2fs");
        assert!(status.success(), "mke2fs failed: {status}");
        let f = std::fs::OpenOptions::new().write(true).open(&img).unwrap();
        f.set_len(64 * 1024 * 1024).unwrap();
        img
    }

    /// Test-local AOSP descriptor builders. They deliberately use literal
    /// offsets instead of production helpers so a production bug cannot hide
    /// behind the fixture.
    fn pad8(n: usize) -> usize {
        n.div_ceil(8) * 8
    }

    fn hashtree_descriptor(name: &str, fec_roots: u32) -> Vec<u8> {
        let salt = [0x11u8, 0x22, 0x33, 0x44];
        let digest = [0x55u8; 32];
        let mut d = vec![0u8; 180];
        d[0..8].copy_from_slice(&1u64.to_be_bytes());
        d[16..20].copy_from_slice(&1u32.to_be_bytes());
        d[20..28].copy_from_slice(&4096u64.to_be_bytes());
        d[44..48].copy_from_slice(&4096u32.to_be_bytes());
        d[48..52].copy_from_slice(&4096u32.to_be_bytes());
        d[52..56].copy_from_slice(&fec_roots.to_be_bytes());
        d[72..78].copy_from_slice(b"sha256");
        d[104..108].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[108..112].copy_from_slice(&(salt.len() as u32).to_be_bytes());
        d[112..116].copy_from_slice(&(digest.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(&salt);
        d.extend_from_slice(&digest);
        let nbf = pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    fn hashtree_descriptor_with_digest(
        name: &str,
        image_size: u64,
        fec_roots: u32,
        digest: &[u8],
    ) -> Vec<u8> {
        let salt = [0x11u8, 0x22, 0x33, 0x44];
        let mut d = vec![0u8; 180];
        d[0..8].copy_from_slice(&1u64.to_be_bytes());
        d[16..20].copy_from_slice(&1u32.to_be_bytes());
        d[20..28].copy_from_slice(&image_size.to_be_bytes());
        d[44..48].copy_from_slice(&4096u32.to_be_bytes());
        d[48..52].copy_from_slice(&4096u32.to_be_bytes());
        d[52..56].copy_from_slice(&fec_roots.to_be_bytes());
        d[72..78].copy_from_slice(b"sha256");
        d[104..108].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[108..112].copy_from_slice(&(salt.len() as u32).to_be_bytes());
        d[112..116].copy_from_slice(&(digest.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(&salt);
        d.extend_from_slice(digest);
        let nbf = pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    fn hashtree_descriptor_with_blocks(name: &str, data: u32, hash: u32) -> Vec<u8> {
        let mut d = hashtree_descriptor(name, 0);
        // The parser checks image_size against the declared data block size.
        d[20..28].copy_from_slice(&(u64::from(data) * 4).to_be_bytes());
        d[44..48].copy_from_slice(&data.to_be_bytes());
        d[48..52].copy_from_slice(&hash.to_be_bytes());
        d
    }

    fn persistent_hashtree_descriptor(name: &str) -> Vec<u8> {
        let mut d = hashtree_descriptor(name, 0);
        d[112..116].copy_from_slice(&0u32.to_be_bytes());
        let nbf = pad8(180 + name.len() + 4 - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.truncate(16 + nbf);
        d
    }

    fn hash_descriptor(name: &str) -> Vec<u8> {
        let salt = [0xaa, 0xbb, 0xcc, 0xdd];
        let mut d = vec![0u8; 132];
        d[0..8].copy_from_slice(&2u64.to_be_bytes());
        d[16..24].copy_from_slice(&4096u64.to_be_bytes());
        d[24..29].copy_from_slice(b"sha1\0");
        d[56..60].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[60..64].copy_from_slice(&(salt.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(&salt);
        let nbf = pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    fn chain_descriptor(name: &str) -> Vec<u8> {
        let key = [0x42u8; 16];
        let mut d = vec![0u8; 92];
        d[0..8].copy_from_slice(&4u64.to_be_bytes());
        d[16..20].copy_from_slice(&1u32.to_be_bytes());
        d[20..24].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[24..28].copy_from_slice(&(key.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(&key);
        let nbf = pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    /// Tag 3 (`AvbKernelCmdlineDescriptor`) is not reproduced by the rebuild.
    fn kernel_cmdline_descriptor(cmdline: &str) -> Vec<u8> {
        let mut d = vec![0u8; 24];
        d[0..8].copy_from_slice(&3u64.to_be_bytes());
        d[16..20].copy_from_slice(&1u32.to_be_bytes());
        d[20..24].copy_from_slice(&(cmdline.len() as u32).to_be_bytes());
        d.extend_from_slice(cmdline.as_bytes());
        let nbf = pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    fn vbmeta_with(descriptors: &[Vec<u8>]) -> Vec<u8> {
        let mut vb = vec![0u8; 256];
        vb[..4].copy_from_slice(b"AVB0");
        let desc_size: usize = descriptors.iter().map(Vec::len).sum();
        let aux = desc_size.div_ceil(64) * 64;
        vb[20..28].copy_from_slice(&(aux as u64).to_be_bytes());
        vb[104..112].copy_from_slice(&(desc_size as u64).to_be_bytes());
        for d in descriptors {
            vb.extend_from_slice(d);
        }
        vb.resize(256 + aux, 0);
        vb
    }

    fn footer_bytes(orig: u64, voff: u64, vsz: u64) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[..4].copy_from_slice(&config::AVB_MAGIC);
        b[4..8].copy_from_slice(&1u32.to_be_bytes());
        b[12..20].copy_from_slice(&orig.to_be_bytes());
        b[20..28].copy_from_slice(&voff.to_be_bytes());
        b[28..36].copy_from_slice(&vsz.to_be_bytes());
        b
    }

    /// Writes a valid vbmeta + footer ending at `end`, moving the file size to
    /// `end`. Used both for an in-place rebuild and for a grown hashtree. The
    /// digest covers the bytes the verifier will hash, so a successful path is
    /// accepted by `verify_hashtree_integrity`.
    fn write_avb_tail(image: &Path, end: u64, name: &str, fec_roots: u32) -> Result<()> {
        let vb_off = end - 4096;
        // The fixture only covers the first MiB; the verifier hashes exactly
        // this range, and no pipeline test asserts the reported original size.
        let covered = 1024 * 1024;
        let salt = [0x11u8, 0x22, 0x33, 0x44];
        let root = avb::test_merkle_root(image, covered, covered, &salt, "sha256")?;
        let vb = vbmeta_with(&[hashtree_descriptor_with_digest(
            name, covered, fec_roots, &root,
        )]);
        let footer_at = end - 64;
        assert!(vb_off + vb.len() as u64 <= footer_at);
        let mut f = std::fs::OpenOptions::new().write(true).open(image)?;
        f.set_len(end)?;
        f.seek(SeekFrom::Start(vb_off))?;
        f.write_all(&vb)?;
        f.write_all(&vec![0u8; (footer_at - vb_off - vb.len() as u64) as usize])?;
        f.write_all(&footer_bytes(covered, vb_off, vb.len() as u64))?;
        f.sync_all()?;
        Ok(())
    }

    fn attach_avb_footer(img: &Path, descriptors: &[Vec<u8>]) {
        let vb = vbmeta_with(descriptors);
        let size = std::fs::metadata(img).unwrap().len();
        let vb_off = size - 4096;
        attach_avb_tail(img, &vb, vb_off, size);
    }

    fn attach_avb_tail(img: &Path, vb: &[u8], vb_off: u64, end: u64) {
        let footer_at = end - 64;
        assert!(vb_off + vb.len() as u64 <= footer_at);
        let mut f = std::fs::OpenOptions::new().write(true).open(img).unwrap();
        f.seek(SeekFrom::Start(vb_off)).unwrap();
        f.write_all(vb).unwrap();
        f.write_all(&vec![0u8; (footer_at - vb_off - vb.len() as u64) as usize])
            .unwrap();
        f.write_all(&footer_bytes(vb_off, vb_off, vb.len() as u64))
            .unwrap();
        f.sync_all().unwrap();
    }

    struct FooterAvb {
        calls: usize,
    }

    impl avb::Avb for FooterAvb {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            self.calls += 1;
            if let Some(td) = req.image.parent() {
                assert!(
                    !td.join("vendor_pre_avb.img").exists(),
                    "the file-wide pre-AVB backup must not be created"
                );
            }
            write_avb_tail(&req.image, req.partition_size, &req.partition_name, 0)
        }
    }

    struct GrowingFooterAvb {
        calls: usize,
    }

    impl avb::Avb for GrowingFooterAvb {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            self.calls += 1;
            let grown = req.partition_size + 4096;
            write_avb_tail(&req.image, grown, &req.partition_name, 0)
        }
    }

    fn seed_payloads(root: &Path) {
        let dir = root.join("payload");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(config::APK_NAME), vec![b'A'; 1200]).unwrap();
        std::fs::write(dir.join(config::RC_NAME_MTK), b"on boot\n    setprop x 1\n").unwrap();
        std::fs::write(dir.join(config::RC_NAME_QC), b"on boot\n    setprop y 1\n").unwrap();
        std::fs::write(
            dir.join("overlay.bin"),
            b"u:object_r:vendor_overlay_file:s0\0",
        )
        .unwrap();
        std::fs::write(
            dir.join("config.bin"),
            b"u:object_r:vendor_configs_file:s0\0",
        )
        .unwrap();
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
            assert!(
                req.image.is_file(),
                "the AVB step must run on the output copy"
            );
            Ok(())
        }
    }

    /// Never reached by the source-protection tests: the guard fires first.
    struct NoopOps;

    impl Ext4Ops for NoopOps {
        fn run(
            &mut self,
            _image: &Path,
            _commands: &[String],
            _workdir: &Path,
            _log: &mut Logger,
        ) -> Result<String> {
            Ok(String::new())
        }

        fn path_exists(&mut self, _image: &Path, _vpath: &str, _workdir: &Path) -> Result<bool> {
            Ok(false)
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

    struct AlwaysFailFsck {
        calls: usize,
    }

    impl Fsck for AlwaysFailFsck {
        fn check(&mut self, _image: &Path, _workdir: &Path) -> Result<String> {
            self.calls += 1;
            Err(Error::build("e2fsck simulated failure (exit 4)"))
        }
    }

    struct FailingAvb {
        calls: usize,
    }

    impl avb::Avb for FailingAvb {
        fn add_hashtree_footer(&mut self, _req: &HashtreeRequest) -> Result<()> {
            self.calls += 1;
            Err(Error::build("simulated avbtool.py failure"))
        }
    }

    #[test]
    fn output_path_defaults_next_to_the_source() {
        let src = std::path::absolute("D:/rom/vendor.img").unwrap();
        let (folder, out) = resolve_output(&src, "", false).unwrap();
        assert_eq!(folder, std::path::absolute("D:/rom").unwrap());
        assert_eq!(
            out,
            std::path::absolute("D:/rom/vendor_VolLTE.img").unwrap()
        );
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

    fn run_with_out(src: &Path, out: &str, root: &Path, logs: &Path) -> Result<PathBuf> {
        let mut ops = NoopOps;
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let mut log = Logger::new();
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src,
            out_path: out,
            chip: "mtk",
            keep_out: true,
            root,
            logs_dir: logs,
        };
        run(&opts, &mut deps, &mut log, &mut progress)
    }

    fn bak_path(out: &Path) -> PathBuf {
        let mut s = out.as_os_str().to_owned();
        s.push(".bak");
        PathBuf::from(s)
    }

    /// Runs a full build with the real ext4 engine and the given fsck/AVB seams.
    fn run_full_build(
        src: &Path,
        out: &str,
        root: &Path,
        logs: &Path,
        fsck: &mut dyn Fsck,
        avb: &mut dyn avb::Avb,
        lines: &Rc<RefCell<Vec<String>>>,
    ) -> Result<PathBuf> {
        let mut ops = RustFsExt4Backend::new();
        let mut deps = Deps {
            ops: &mut ops,
            fsck,
            avb,
        };
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src,
            out_path: out,
            chip: "mtk",
            keep_out: !out.is_empty(),
            root,
            logs_dir: logs,
        };
        run(&opts, &mut deps, &mut log, &mut progress)
    }

    fn assert_source_intact(src: &Path, len: u64, hash: &str) {
        assert!(src.is_file(), "the source must still exist");
        assert_eq!(std::fs::metadata(src).unwrap().len(), len);
        assert_eq!(crate::sha256::file_hex_upper(src).unwrap(), hash);
    }

    #[test]
    fn output_equal_to_the_source_is_refused_and_the_source_is_untouched() {
        let dir = tmp("volte_pipeline_out_equals_src");
        let src = dir.join("vendor.img");
        std::fs::write(&src, vec![b'A'; 4096]).unwrap();
        let before = crate::sha256::file_hex_upper(&src).unwrap();
        let out = src.to_string_lossy().into_owned();
        let logs = dir.join("Logs");

        let err = run_with_out(&src, &out, &dir, &logs).unwrap_err();
        assert!(
            err.message().contains("same file as the source image"),
            "{}",
            err.message()
        );
        assert_source_intact(&src, 4096, &before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_source_with_absolute_output_is_refused() {
        let dir = tmp("volte_pipeline_relative_source");
        let abs_src = std::env::current_dir().unwrap().join("Cargo.toml");
        assert!(abs_src.is_file(), "the test must run in the crate root");
        let before = crate::sha256::file_hex_upper(&abs_src).unwrap();
        let len = std::fs::metadata(&abs_src).unwrap().len();
        let out = abs_src.to_string_lossy().into_owned();
        let logs = dir.join("Logs");

        let err = run_with_out(Path::new("Cargo.toml"), &out, &dir, &logs).unwrap_err();
        assert!(
            err.message().contains("same file as the source image"),
            "{}",
            err.message()
        );
        assert_source_intact(&abs_src, len, &before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn case_only_output_alias_is_refused_and_the_source_is_untouched() {
        let dir = tmp("volte_pipeline_case_alias");
        let src = dir.join("vendor.img");
        std::fs::write(&src, vec![b'C'; 4096]).unwrap();
        let out = dir.join("VENDOR.IMG");
        assert!(
            out.is_file(),
            "the test needs a case-insensitive filesystem"
        );
        let hash = crate::sha256::file_hex_upper(&src).unwrap();
        let out_s = out.to_string_lossy().into_owned();
        let logs = dir.join("Logs");

        let err = run_with_out(&src, &out_s, &dir, &logs).unwrap_err();
        assert!(
            err.message().contains("same file as the source image"),
            "{}",
            err.message()
        );
        assert_source_intact(&src, 4096, &hash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlinked_output_is_refused_when_the_platform_allows_it() {
        let dir = tmp("volte_pipeline_symlink_out");
        let src = dir.join("vendor.img");
        std::fs::write(&src, vec![b'D'; 4096]).unwrap();
        let link = dir.join("out.img");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&src, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&src, &link);
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let hash = crate::sha256::file_hex_upper(&src).unwrap();
        let out = link.to_string_lossy().into_owned();
        let logs = dir.join("Logs");

        let err = run_with_out(&src, &out, &dir, &logs).unwrap_err();
        assert!(
            err.message().contains("same file as the source image"),
            "{}",
            err.message()
        );
        assert_source_intact(&src, 4096, &hash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_linked_output_is_refused_when_the_platform_allows_it() {
        let dir = tmp("volte_pipeline_hardlink_out");
        let src = dir.join("vendor.img");
        std::fs::write(&src, vec![b'E'; 4096]).unwrap();
        let link = dir.join("out.img");
        if std::fs::hard_link(&src, &link).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let hash = crate::sha256::file_hex_upper(&src).unwrap();
        let out = link.to_string_lossy().into_owned();
        let logs = dir.join("Logs");

        let err = run_with_out(&src, &out, &dir, &logs).unwrap_err();
        assert!(
            err.message().contains("same file as the source image"),
            "{}",
            err.message()
        );
        assert_source_intact(&src, 4096, &hash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_stamp_has_the_python_shape() {
        let stamp = log_stamp();
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert_eq!(&stamp[8..9], "_");
        assert!(stamp
            .chars()
            .filter(|c| *c != '_')
            .all(|c| c.is_ascii_digit()));
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
    fn space_need_counts_the_copy_and_the_temp_headroom() {
        let raw = 100 * 1024 * 1024;
        let (dest, temp) = space_need(raw, false);
        assert_eq!(dest, raw * 2 + AVB_HEADROOM);
        assert_eq!(temp, raw, "a raw source only needs the working copy");

        let (dest, temp) = space_need(raw, true);
        assert_eq!(dest, raw * 2 + AVB_HEADROOM);
        assert_eq!(
            temp,
            raw * 2,
            "the unsparsed copy and the working image coexist"
        );
    }

    #[test]
    fn space_need_saturates_instead_of_overflowing() {
        let (dest, temp) = space_need(u64::MAX, true);
        assert_eq!(dest, u64::MAX);
        assert_eq!(temp, u64::MAX);
    }

    #[test]
    fn missing_source_is_a_build_error() {
        let dir = tmp("volte_pipeline_missing");
        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
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
        assert!(
            err.message().starts_with("File not found:"),
            "{}",
            err.message()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_build_reaches_the_final_tokens_on_a_synthetic_image() {
        let dir = tmp("volte_pipeline_full");
        let tool = tool_dir();
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = CountingFsck {
            inner: E2fsck::new(&tool),
            calls: 0,
        };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
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
        assert_eq!(
            crate::sha256::file_hex_upper(&src).unwrap(),
            src_sha,
            "source untouched"
        );
        assert_eq!(fsck.calls, 1);
        assert_eq!(avb.calls, 1);

        let text = lines.borrow().join("\n");
        for need in [
            "[CHIP] Forced MEDIATEK branch",
            "e2fsck=PASS",
            "AVB_HASHTREE=PASS",
            "VOLTE_FLAGS=0",
            "AVB_ALGORITHM=NONE",
            "AVB_SIGNATURE=DROPPED",
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
            assert!(
                marks.contains(&expected),
                "progress {expected} missing: {marks:?}"
            );
        }

        let commands = std::fs::read_to_string(logs.join("commands_last.txt")).unwrap();
        assert!(
            commands.contains("\n\n"),
            "clean and write blocks are separated"
        );
        assert!(commands.starts_with("rm /overlay/OPPO_VoLTE_Overlay.apk\n"));
        assert!(commands.contains("\ncd /overlay\n"));
        let report = std::fs::read_to_string(logs.join("vendor_VolLTE_report.txt")).unwrap();
        assert_eq!(report.lines().count(), 7, "{report}");
        assert!(report.starts_with("File="));
        assert!(report.contains("\ne2fsck=PASS\n"));
        assert!(report.contains("\nAVB_SIGNATURE=DROPPED\n"));
        assert!(report.ends_with("FLASH=NO\n"));
        let log_files: Vec<_> = std::fs::read_dir(&logs)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("build_vendor_volte_")
            })
            .collect();
        assert_eq!(log_files.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_out_path_that_differs_from_the_source_is_accepted() {
        let dir = tmp("volte_pipeline_explicit_out");
        let tool = tool_dir();
        let src = make_ext4_with_slack(&dir, "vendor.img");
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_dir = dir.join("output");
        std::fs::create_dir_all(&out_dir).unwrap();
        let out_path = out_dir.join("vendor_volte_custom.img");
        let out_s = out_path.to_string_lossy().into_owned();

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = CountingFsck {
            inner: E2fsck::new(&tool),
            calls: 0,
        };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: &out_s,
            chip: "mtk",
            keep_out: true,
            root: &dir,
            logs_dir: &logs,
        };
        let out = run(&opts, &mut deps, &mut log, &mut progress).unwrap();

        assert_eq!(
            out,
            std::path::absolute(&out_path).unwrap(),
            "a normal explicit --out is used as given"
        );
        assert_eq!(
            crate::sha256::file_hex_upper(&src).unwrap(),
            src_sha,
            "the source is untouched"
        );
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len()
        );
        let text = lines.borrow().join("\n");
        assert!(text.contains("[OUTPUT] "), "{text}");
        assert!(text.contains("e2fsck=PASS"), "{text}");
        assert!(
            text.contains("APK_SHA256=") && !text.contains("APK_SHA256=UNVERIFIED"),
            "the overlay must be verified through the default backend\n{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_first_build_creates_no_backup() {
        let dir = tmp("volte_pipeline_bak_first");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let out = run_full_build(&src, "", &dir, &logs, &mut fsck, &mut avb, &lines).unwrap();

        assert!(out.is_file());
        assert!(
            !bak_path(&out).exists(),
            "a first build must not create a backup"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_output_is_kept_as_a_single_generation_backup() {
        let dir = tmp("volte_pipeline_bak_existing");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_path = dir.join("vendor_VolLTE.img");
        std::fs::write(&out_path, b"previous output").unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let out = run_full_build(&src, "", &dir, &logs, &mut fsck, &mut avb, &lines).unwrap();

        assert_eq!(out, out_path);
        assert_eq!(
            std::fs::read(bak_path(&out)).unwrap(),
            b"previous output",
            "the backup must hold the exact previous output"
        );
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len(),
            "the output itself must be the new build"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_backup_is_replaced_by_the_previous_output() {
        let dir = tmp("volte_pipeline_bak_replace");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_path = dir.join("vendor_VolLTE.img");
        std::fs::write(&out_path, b"second generation").unwrap();
        std::fs::write(bak_path(&out_path), b"first generation").unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let out = run_full_build(&src, "", &dir, &logs, &mut fsck, &mut avb, &lines).unwrap();

        assert_eq!(
            std::fs::read(bak_path(&out)).unwrap(),
            b"second generation",
            "the backup must be the output that existed before this build"
        );
        let extra: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("vendor_VolLTE.img.bak."))
            .collect();
        assert!(
            extra.is_empty(),
            "only one backup generation must exist, found {extra:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_custom_out_path_gets_the_same_backup_treatment() {
        let dir = tmp("volte_pipeline_bak_custom_out");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_path = dir.join("custom_output.img");
        std::fs::write(&out_path, b"custom previous").unwrap();
        let out_s = out_path.to_string_lossy().into_owned();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let out = run_full_build(&src, &out_s, &dir, &logs, &mut fsck, &mut avb, &lines).unwrap();

        assert_eq!(out, std::path::absolute(&out_path).unwrap());
        assert_eq!(
            std::fs::read(bak_path(&out)).unwrap(),
            b"custom previous",
            "--out must not delete the previous file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_build_keeps_the_previous_output_backup() {
        let dir = tmp("volte_pipeline_bak_failure");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_path = dir.join("vendor_VolLTE.img");
        std::fs::write(&out_path, b"good previous output").unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = AlwaysFailFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let err = run_full_build(&src, "", &dir, &logs, &mut fsck, &mut avb, &lines).unwrap_err();

        assert!(err.message().contains("e2fsck failed"), "{err}");
        assert_eq!(
            std::fs::read(bak_path(&out_path)).unwrap(),
            b"good previous output",
            "a failure after the copy must leave the backup intact"
        );
        assert!(
            !logs.join("vendor_VolLTE_report.txt").exists(),
            "a failed build must not report an output"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rename_failure_only_warns_and_the_build_continues() {
        let dir = tmp("volte_pipeline_bak_rename_fail");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out_path = dir.join("vendor_VolLTE.img");
        std::fs::write(&out_path, b"old output").unwrap();
        // A directory at the backup path makes rename fail on every platform
        // while leaving out_path writable, which is the case under test.
        let bak = bak_path(&out_path);
        std::fs::create_dir(&bak).unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };

        let out = run_full_build(&src, "", &dir, &logs, &mut fsck, &mut avb, &lines).unwrap();

        let text = lines.borrow().join("\n");
        assert!(
            text.contains("[WARN] Destination file is open/locked:"),
            "a failed rename must only warn\n{text}"
        );
        assert!(
            bak.is_dir(),
            "the failed rename must not alter the backup path"
        );
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len(),
            "the build must continue and write the new output"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_e2fsck_failure_fails_the_build_before_avb() {
        let dir = tmp("volte_pipeline_fsck_fail");
        let src = make_ext4(&dir, "vendor.img");
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = AlwaysFailFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().contains("e2fsck failed"), "{err}");
        assert_eq!(fsck.calls, 1);
        assert_eq!(
            avb.calls, 0,
            "the AVB step must not run after a failed fsck"
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);

        let text = lines.borrow().join("\n");
        assert!(text.contains("e2fsck=FAIL"), "{text}");
        assert!(!text.contains("[OK] EDL size matches"), "{text}");
        assert!(!text.contains("AVB_HASHTREE="), "{text}");
        assert!(!text.contains("FLASH=NO"), "{text}");
        assert!(!text.contains("[OK] AVB"), "{text}");
        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_no_footer_image_that_fills_the_slot_skips_avb_and_exports() {
        let dir = tmp("volte_pipeline_no_footer_full_slot");
        let src = make_ext4(&dir, "vendor.img");
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FailingAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        assert!(!avb::avb_info_fast(&src).unwrap().has_footer);
        let out = run(&opts, &mut deps, &mut log, &mut progress).unwrap();

        assert_eq!(
            avb.calls, 0,
            "a source without a footer must not reach avbtool"
        );
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len()
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);

        let text = lines.borrow().join("\n");
        assert!(
            text.contains(
                "[STEP] Source image has no AVB footer - skipping AVB (no footer added)."
            ),
            "{text}"
        );
        assert!(text.contains("AVB_HASHTREE=SKIP"), "{text}");
        assert!(text.contains("[OUTPUT] "), "{text}");
        assert!(!text.contains("[AVB] add_hashtree"), "{text}");
        assert!(!text.contains("[STATUS] Generating hashtree"), "{text}");
        assert!(!text.contains("Pre-AVB snapshot"), "{text}");
        let report = std::fs::read_to_string(logs.join("vendor_VolLTE_report.txt")).unwrap();
        assert!(report.contains("AVB_HASHTREE=SKIP"), "{report}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_footer_that_is_not_at_the_tail_stops_the_build_before_processing() {
        let dir = tmp("volte_pipeline_footer_misplaced");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let mut f = std::fs::OpenOptions::new().append(true).open(&src).unwrap();
        f.write_all(&[0u8; 4096]).unwrap();
        drop(f);
        let src_len = std::fs::metadata(&src).unwrap().len();
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        let info = avb::avb_info_fast(&src).unwrap();
        assert!(info.has_footer);
        assert!(
            !info.footer_at_exact_tail,
            "the fixture must only expose the footer through the backward scan"
        );
        let logs = dir.join("Logs");

        let mut ops = NoopOps;
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FailingAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(
            err.message()
                .contains("not positioned at the end of the partition"),
            "{err}"
        );
        assert_eq!(fsck.calls, 0, "e2fsck must not run");
        assert_eq!(avb.calls, 0, "avbtool must not run");
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);
        assert_eq!(std::fs::metadata(&src).unwrap().len(), src_len);

        let text = lines.borrow().join("\n");
        assert!(!text.contains("EXT4 engine:"), "{text}");
        assert!(!text.contains("e2fsck -fn"), "{text}");
        assert!(!text.contains("e2fsck="), "{text}");
        assert!(!text.contains("[STATUS] Generating hashtree"), "{text}");
        assert!(!text.contains("AVB_HASHTREE="), "{text}");
        assert!(!text.contains("[OUTPUT] "), "{text}");
        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_avb_backend_failure_aborts_the_build() {
        let dir = tmp("volte_pipeline_avb_fail");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FailingAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert_eq!(avb.calls, 1);
        assert!(
            err.message().contains("AVB rebuild failed"),
            "{err}"
        );
        assert!(
            err.message().contains("simulated avbtool.py failure"),
            "the original reason must survive: {err}"
        );
        let out = dir.join("vendor_VolLTE.img");
        assert!(
            !out.exists(),
            "a failed AVB rebuild must not leave a flashable output behind"
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);

        let text = lines.borrow().join("\n");
        assert!(text.contains("[WARN] AVB hashtree:"), "{text}");
        assert!(
            text.contains("[ERROR] AVB rebuild failed - refusing to export"),
            "{text}"
        );
        assert!(!text.contains("AVB_HASHTREE="), "{text}");
        assert!(!text.contains("[OUTPUT] "), "{text}");
        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hashtree_that_grows_past_the_slot_aborts_the_build() {
        let dir = tmp("volte_pipeline_avb_grow");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = GrowingFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert_eq!(avb.calls, 1);
        assert!(
            err.message().contains("grew past the slot"),
            "the original reason must survive: {err}"
        );
        let out = dir.join("vendor_VolLTE.img");
        assert!(
            !out.exists(),
            "a hashtree that does not fit the slot must not leave an output behind"
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);

        let text = lines.borrow().join("\n");
        assert!(text.contains("[WARN] Hashtree grew to"), "{text}");
        assert!(
            text.contains("[ERROR] AVB rebuild failed - refusing to export"),
            "{text}"
        );
        assert!(!text.contains("AVB_HASHTREE="), "{text}");
        assert!(!text.contains("[OUTPUT] "), "{text}");
        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The filesystem is written before the AVB step, so a failed rebuild must
    /// drop the already-patched copy instead of exporting it with stale AVB
    /// metadata.
    #[test]
    fn a_patched_image_is_not_exported_when_the_avb_rebuild_fails() {
        let dir = tmp("volte_pipeline_avb_fail_patched");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");
        let out = dir.join("explicit_out.img");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FailingAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: out.to_str().unwrap(),
            chip: "mtk",
            keep_out: true,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().contains("simulated avbtool.py failure"), "{err}");
        assert!(
            !out.exists(),
            "the output copy must be removed after a failed AVB rebuild"
        );
        assert!(
            lines
                .borrow()
                .iter()
                .any(|l| l.contains("EXT4 engine: rust-fs-ext4")),
            "the filesystem must have been patched before the AVB step ran"
        );
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_successful_avb_rebuild_exports_the_image() {
        let dir = tmp("volte_pipeline_avb_ok");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        let src_sha = crate::sha256::file_hex_upper(&src).unwrap();
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let out = run(&opts, &mut deps, &mut log, &mut progress).unwrap();
        assert_eq!(avb.calls, 1);
        assert_eq!(crate::sha256::file_hex_upper(&src).unwrap(), src_sha);
        assert_eq!(
            std::fs::metadata(&out).unwrap().len(),
            std::fs::metadata(&src).unwrap().len()
        );
        let text = lines.borrow().join("\n");
        assert!(text.contains("AVB_HASHTREE=PASS"), "{text}");
        assert!(!text.contains("Pre-AVB snapshot"), "{text}");
        assert!(
            text.contains("[STATUS] Generating hashtree for the image (FEC disabled)..."),
            "{text}"
        );
        assert!(!text.contains("Generating hashtree and FEC"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_with_a_chain_descriptor_is_refused_before_avb() {
        let dir = tmp("volte_pipeline_chain");
        let src = make_ext4(&dir, "vendor.img");
        attach_avb_footer(&src, &[chain_descriptor("boot")]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(
            err.message().contains("chain partition descriptor"),
            "{err}"
        );
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_with_an_unsupported_descriptor_tag_is_refused_before_avb() {
        let dir = tmp("volte_pipeline_unknown_tag");
        let src = make_ext4(&dir, "vendor.img");
        attach_avb_footer(
            &src,
            &[
                hashtree_descriptor("vendor", 0),
                kernel_cmdline_descriptor("androidboot.selinux=permissive"),
            ],
        );
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(
            err.message().contains("unsupported AVB descriptor tag(s) 3"),
            "{err}"
        );
        assert!(err.message().contains("kernel command line"), "{err}");
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let text = lines.borrow().join("\n");
        assert!(
            text.contains("[WARN] AVB vbmeta: unsupported descriptor tag 3"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hash_only_input_is_refused() {
        let dir = tmp("volte_pipeline_tag2");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hash_descriptor("vendor")]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        assert_eq!(
            avb::avb_info_fast(&src).unwrap().descriptor_kind,
            avb::DescriptorKind::HashOnly
        );
        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(
            err.message().contains("whole-partition hash descriptor (Tag 2)"),
            "{err}"
        );
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_with_fec_is_refused() {
        let dir = tmp("volte_pipeline_fec");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 2)]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        assert_eq!(avb::avb_info_fast(&src).unwrap().fec_roots, 2);
        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().contains("fec_roots=2"), "{err}");
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_default_hashtree_block_size_is_refused() {
        let dir = tmp("volte_pipeline_block_size");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor_with_blocks("vendor", 16384, 4096)]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().contains("16384"), "{err}");
        assert!(err.message().contains("4096-byte blocks"), "{err}");
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_persistent_root_digest_is_refused() {
        let dir = tmp("volte_pipeline_persistent_digest");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[persistent_hashtree_descriptor("vendor")]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        assert!(avb::avb_info_fast(&src).unwrap().persistent_digest);
        let err = run(&opts, &mut deps, &mut log, &mut progress).unwrap_err();
        assert!(err.message().contains("persistent on-device root digest"), "{err}");
        assert_eq!(avb.calls, 0, "the AVB backend must not be reached");
        assert_eq!(fsck.calls, 0, "the build must stop before e2fsck");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_without_fec_does_not_warn_about_fec() {
        let dir = tmp("volte_pipeline_no_fec");
        let src = make_ext4_with_slack(&dir, "vendor.img");
        attach_avb_footer(&src, &[hashtree_descriptor("vendor", 0)]);
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = RustFsExt4Backend::new();
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = FooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        run(&opts, &mut deps, &mut log, &mut progress).unwrap();
        let text = lines.borrow().join("\n");
        assert!(!text.contains("FEC will be dropped"), "{text}");
        assert!(!text.contains("Generating hashtree and FEC"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real backend, but it flips one byte of the overlay APK inside the
    /// image immediately after the overlay write batch (A3). Verification then
    /// has to notice the mismatch before the pipeline exports anything.
    struct CorruptingApkOps {
        inner: RustFsExt4Backend,
        corrupted: bool,
    }

    impl Ext4Ops for CorruptingApkOps {
        fn run(
            &mut self,
            image: &Path,
            commands: &[String],
            workdir: &Path,
            log: &mut Logger,
        ) -> Result<String> {
            let out = self.inner.run(image, commands, workdir, log)?;
            let wrote_apk = commands
                .iter()
                .any(|c| c.starts_with("write ") && c.ends_with(&format!(" {}", config::APK_NAME)));
            if wrote_apk && !self.corrupted {
                let host = workdir.join("a3_corrupt_dump.apk");
                let dump = format!(
                    "dump -p /overlay/{} {}",
                    config::APK_NAME,
                    crate::env::forward_slashes(&host)
                );
                self.inner
                    .run(image, &[dump], workdir, &mut Logger::new())?;
                let mut bytes = std::fs::read(&host)?;
                assert!(!bytes.is_empty(), "the injected APK must not be empty");
                bytes[0] ^= 0xFF;
                std::fs::write(&host, &bytes)?;
                let replace = format!(
                    "write {} {}",
                    crate::env::forward_slashes(&host),
                    config::APK_NAME
                );
                self.inner.run(
                    image,
                    &["cd /overlay".to_owned(), replace],
                    workdir,
                    &mut Logger::new(),
                )?;
                self.corrupted = true;
            }
            Ok(out)
        }

        fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
            self.inner.path_exists(image, vpath, workdir)
        }
    }

    #[test]
    fn a3_payload_mismatch_after_injection_aborts_build() {
        let dir = tmp("volte_pipeline_a3_payload_mismatch");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = CorruptingApkOps {
            inner: RustFsExt4Backend::new(),
            corrupted: false,
        };
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let result = run(&opts, &mut deps, &mut log, &mut progress);
        assert!(
            ops.corrupted,
            "the test must corrupt the APK inside the image after injection"
        );

        let text = lines.borrow().join("\n");
        assert!(text.contains("content mismatch"), "{text}");
        assert!(text.contains("APK_SHA256=UNVERIFIED"), "{text}");

        match result {
            Ok(out) => panic!(
                "A3: the build exported {} even though the overlay APK was corrupted after injection\n{text}",
                out.display()
            ),
            Err(err) => assert_eq!(
                err.message(),
                "Overlay APK/RC not verified inside the image - refusing to export"
            ),
        }
        assert!(!text.contains("[OUTPUT] "), "{text}");
        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real backend, but it removes the output image right after the flag
    /// probe dump (A6). The flag patch then has to report a read failure
    /// instead of silently treating it as "0 flags".
    struct DroppingFlagProbeOps {
        inner: RustFsExt4Backend,
    }

    impl Ext4Ops for DroppingFlagProbeOps {
        fn run(
            &mut self,
            image: &Path,
            commands: &[String],
            workdir: &Path,
            log: &mut Logger,
        ) -> Result<String> {
            let out = self.inner.run(image, commands, workdir, log);
            if commands.iter().any(|c| c.ends_with("flg_build.prop")) {
                let _ = std::fs::remove_file(image);
            }
            out
        }

        fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
            self.inner.path_exists(image, vpath, workdir)
        }
    }

    #[test]
    fn a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero() {
        let dir = tmp("volte_pipeline_a6_flag_read");
        let src = make_ext4(&dir, "vendor.img");
        seed_payloads(&dir);
        let logs = dir.join("Logs");

        let mut ops = DroppingFlagProbeOps {
            inner: RustFsExt4Backend::new(),
        };
        let mut fsck = PassingFsck { calls: 0 };
        let mut avb = NoFooterAvb { calls: 0 };
        let mut deps = Deps {
            ops: &mut ops,
            fsck: &mut fsck,
            avb: &mut avb,
        };
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
        let mut progress = |_n: u8, _l: &str| {};
        let opts = Options {
            src: &src,
            out_path: "",
            chip: "mtk",
            keep_out: false,
            root: &dir,
            logs_dir: &logs,
        };

        let result = run(&opts, &mut deps, &mut log, &mut progress);
        let text = lines.borrow().join("\n");
        match result {
            Ok(out) => panic!(
                "A6: the build exported {} after the flag patch could not read the image\n{text}",
                out.display()
            ),
            Err(err) => assert!(
                err.message().contains("flag patch failed while reading"),
                "A6: expected a flag read error, got: {}\n{text}",
                err.message()
            ),
        }
        assert!(!text.contains("[OUTPUT] "), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
