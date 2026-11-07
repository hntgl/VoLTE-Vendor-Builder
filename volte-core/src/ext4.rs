//! `debugfs` and `e2fsck` backends (decision 8.2/1: external tools, no static
//! link to `libext2fs`).
//!
//! Port of `run_debugfs` (`hd_vendor_build.py:384`), `debugfs_exists`
//! (`:407`), `run_e2fsck` (`:431`) and `read_vendor_flag_bit` (`:730`).
//!
//! The external tools are treated exactly as Python treats them:
//! - every command batch becomes `workdir/hd_debugfs_cmds.txt`, UTF-8 with LF
//!   and a trailing newline, replayed by `debugfs -w -f` with the working
//!   directory set to `workdir`.
//! - a non-EXT4 image is recognised by its *text* ("bad magic number" /
//!   "not a valid ext"), because debugfs 1.44.5 exits 0 on that error.
//! - `write src /abs/name` stores the leading slash in the entry name (e2fsck
//!   then reports an illegal name), so callers use `cd` + bare names like
//!   `overlay_cmds` does.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::flags::flag_bit_from_dumps;
use crate::logging::{decode, Logger};

const RUN_TIMEOUT: Duration = Duration::from_secs(3600);
const STAT_TIMEOUT: Duration = Duration::from_secs(600);
const FSCK_TIMEOUT: Duration = Duration::from_secs(3600);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Command-file name written into the working directory before every batch
/// (`hd_debugfs_cmds.txt`, `:388`).
pub const CMD_FILE: &str = "hd_debugfs_cmds.txt";

/// The two ext4 verbs the pipeline needs. Implemented by `DebugfsBackend`,
/// mocked by callers that must not spawn a process.
pub trait Ext4Ops {
    /// Runs one command batch. A command that fails is a build error, so a
    /// mutation that did not happen is never reported as success.
    fn run(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        log: &mut Logger,
    ) -> Result<String>;

    /// Runs a probe batch whose failures are expected and ignored: candidate
    /// dumps, property files that may not exist, cleanup entries that may be
    /// absent. `debugfs -f` already continues after a failure, so the default
    /// forwards to [`run`](Self::run); backends with a fail-fast `run` override
    /// this with a per-command loop.
    fn run_best_effort(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        log: &mut Logger,
    ) -> Result<String> {
        self.run(image, commands, workdir, log)
    }

    fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool>;
}

/// `e2fsck -fn` behind a trait so the verdict rule is testable.
pub trait Fsck {
    fn check(&mut self, image: &Path, workdir: &Path) -> Result<String>;
}

fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

#[derive(Debug)]
struct Captured {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Captured {
    /// Python: `(_decode(stdout) + "\n" + _decode(stderr)).strip()`.
    fn text(&self) -> String {
        format!("{}\n{}", decode(&self.stdout), decode(&self.stderr))
            .trim()
            .to_owned()
    }
}

fn capture(cmd: &mut Command, timeout: Duration) -> Result<Captured> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    quiet(cmd);
    let mut child = cmd.spawn()?;
    let mut out_pipe = child.stdout.take().expect("stdout is piped");
    let mut err_pipe = child.stderr.take().expect("stderr is piped");
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        let _ = out_tx.send(buf);
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        let _ = err_tx.send(buf);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // The reader threads are left to drain and die: a grandchild
                // can keep the pipe open after the direct child is killed, and
                // joining them here would defeat the deadline.
                return Err(Error::build(format!(
                    "tool timed out after {} s",
                    timeout.as_secs()
                )));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let _ = out_thread.join();
    let _ = err_thread.join();
    Ok(Captured {
        code: status.code().unwrap_or(-1),
        stdout: out_rx.recv().unwrap_or_default(),
        stderr: err_rx.recv().unwrap_or_default(),
    })
}

fn reject_non_ext4(text: &str) -> Result<()> {
    let low = text.to_lowercase();
    if low.contains("bad magic number") || low.contains("not a valid ext") {
        return Err(Error::bad_magic(
            "not an EXT4 image (debugfs: bad magic number)",
        ));
    }
    Ok(())
}

/// `debugfs.exe` lookup: the tool directory first, then the bare name on PATH
/// (`:385-387`).
pub struct DebugfsBackend {
    exe: PathBuf,
}

impl DebugfsBackend {
    pub fn new(tool_dir: &Path) -> Self {
        let exe = tool_dir.join("debugfs.exe");
        let exe = if exe.is_file() {
            exe
        } else {
            PathBuf::from("debugfs.exe")
        };
        DebugfsBackend { exe }
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }
}

impl Ext4Ops for DebugfsBackend {
    fn run(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        log: &mut Logger,
    ) -> Result<String> {
        let cmdfile = workdir.join(CMD_FILE);
        let body = format!("{}\n", commands.join("\n"));
        std::fs::write(&cmdfile, body)?;
        let mut cmd = Command::new(&self.exe);
        cmd.arg("-w")
            .arg("-f")
            .arg(&cmdfile)
            .arg(image)
            .current_dir(workdir);
        let out = capture(&mut cmd, RUN_TIMEOUT)?;
        let text = out.text();
        reject_non_ext4(&text)?;
        if out.code != 0 {
            log.log(format!("[WARN] debugfs rc={}", out.code));
        }
        Ok(text)
    }

    fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("-R")
            .arg(format!("stat {vpath}"))
            .arg(image)
            .current_dir(workdir);
        let out = capture(&mut cmd, STAT_TIMEOUT)?;
        let text = out.text();
        reject_non_ext4(&text)?;
        Ok(!text.to_lowercase().contains("file not found"))
    }
}

/// `e2fsck.exe` lookup, same fallback rule as debugfs (`:432-434`).
pub struct E2fsck {
    exe: PathBuf,
}

impl E2fsck {
    pub fn new(tool_dir: &Path) -> Self {
        let exe = tool_dir.join("e2fsck.exe");
        let exe = if exe.is_file() {
            exe
        } else {
            PathBuf::from("e2fsck.exe")
        };
        E2fsck { exe }
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }
}

/// Last 400 characters, Python's `out[-400:]`.
fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(n)).collect()
}

impl Fsck for E2fsck {
    /// `e2fsck -fn`: rc 0 is clean, 1 means corrected errors (impossible with
    /// `-n`), anything else aborts the build with the tail of the output.
    fn check(&mut self, image: &Path, workdir: &Path) -> Result<String> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("-fn").arg(image).current_dir(workdir);
        let out = capture(&mut cmd, FSCK_TIMEOUT)?;
        let text = out.text();
        if out.code != 0 && out.code != 1 {
            return Err(Error::build(format!(
                "e2fsck rc={}: {}",
                out.code,
                tail_chars(&text, 400)
            )));
        }
        Ok(text)
    }
}

/// `read_vendor_flag_bit` (`:730`): dump the two property files, then let
/// `flag_bit_from_dumps` decide. Every failure is swallowed, exactly like the
/// Python `try/except`.
pub fn read_vendor_flag_bit(image: &Path, workdir: &Path, ops: &mut dyn Ext4Ops) -> u8 {
    let commands = vec![
        "dump /default.prop hd_flg_def.prop".to_owned(),
        "dump /build.prop hd_flg_build.prop".to_owned(),
    ];
    let mut sink = Logger::new();
    let _ = ops.run_best_effort(image, &commands, workdir, &mut sink);
    flag_bit_from_dumps(workdir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn quiet_log() -> Logger {
        Logger::new().with_sink(Vec::<String>::new())
    }

    /// The bundled e2fsprogs in this project's own `tool\`. Tests fail instead
    /// of skipping when it is absent: the module is unverified without it.
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
        quiet(&mut cmd);
        let status = cmd.status().expect("run mke2fs");
        assert!(status.success(), "mke2fs failed: {status}");
        assert_eq!(std::fs::metadata(&img).unwrap().len(), 16384 * 4096);
        img
    }

    #[test]
    fn exe_falls_back_to_the_bare_name() {
        let backend = DebugfsBackend::new(Path::new(r"E:\no\such\tool"));
        assert_eq!(backend.exe(), Path::new("debugfs.exe"));
        let fsck = E2fsck::new(Path::new(r"E:\no\such\tool"));
        assert_eq!(fsck.exe(), Path::new("e2fsck.exe"));
    }

    #[test]
    fn command_file_is_lf_only_and_commands_reach_the_image() {
        let dir = tmp("volte_ext4_run");
        let img = make_ext4(&dir, "run.img");
        std::fs::write(dir.join("host.txt"), b"hello\n").unwrap();
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut log = quiet_log();
        let out = ops
            .run(
                &img,
                &[
                    "write host.txt hello.txt".to_owned(),
                    "ls -l /".to_owned(),
                ],
                &dir,
                &mut log,
            )
            .unwrap();
        assert!(out.contains("Allocated inode"), "{out}");
        assert!(out.contains("hello.txt"), "{out}");
        let cmds = std::fs::read(dir.join(CMD_FILE)).unwrap();
        assert_eq!(cmds, b"write host.txt hello.txt\nls -l /\n");
        assert_eq!(std::fs::metadata(&img).unwrap().len(), 16384 * 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_file_joins_an_empty_batch_to_a_single_newline() {
        let dir = tmp("volte_ext4_empty_batch");
        let img = make_ext4(&dir, "empty.img");
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut log = quiet_log();
        ops.run(&img, &[], &dir, &mut log).unwrap();
        assert_eq!(std::fs::read(dir.join(CMD_FILE)).unwrap(), b"\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_exists_matches_the_stat_probe() {
        let dir = tmp("volte_ext4_exists");
        let img = make_ext4(&dir, "exists.img");
        std::fs::write(dir.join("host.txt"), b"x\n").unwrap();
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut log = quiet_log();
        ops.run(
            &img,
            &["write host.txt hello.txt".to_owned()],
            &dir,
            &mut log,
        )
        .unwrap();
        assert!(ops.path_exists(&img, "/hello.txt", &dir).unwrap());
        assert!(ops.path_exists(&img, "/lost+found", &dir).unwrap());
        assert!(!ops.path_exists(&img, "/nope.txt", &dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_magic_is_detected_by_text_not_exit_code() {
        let dir = tmp("volte_ext4_badmagic");
        let junk = dir.join("junk1m.img");
        std::fs::write(&junk, vec![b'A'; 1024 * 1024]).unwrap();
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut log = quiet_log();
        let err = ops
            .run(&junk, &["ls -l /".to_owned()], &dir, &mut log)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::BadMagic);
        let err = ops.path_exists(&junk, "/", &dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::BadMagic);

        // A 1024-byte file only yields "short read"; debugfs exits 0 and the
        // build is not refused, exactly like Python.
        let short = dir.join("short.img");
        std::fs::write(&short, vec![0u8; 1024]).unwrap();
        let out = ops
            .run(&short, &["ls -l /".to_owned()], &dir, &mut log)
            .unwrap();
        assert!(out.to_lowercase().contains("short read"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fsck_accepts_a_clean_image_and_rejects_errors_with_the_tail() {
        let dir = tmp("volte_ext4_fsck");
        let img = make_ext4(&dir, "clean.img");
        let mut fsck = E2fsck::new(&tool_dir());
        let out = fsck.check(&img, &dir).unwrap();
        assert!(out.contains("Pass 5"), "{out}");
        assert!(out.contains("clean.img:"), "{out}");

        // `write src /abs.txt` stores the leading slash in the name; e2fsck
        // then reports an illegal name and exits 4 (measured).
        std::fs::write(dir.join("host.txt"), b"x\n").unwrap();
        let mut ops = DebugfsBackend::new(&tool_dir());
        let mut log = quiet_log();
        ops.run(&img, &["write host.txt /abs.txt".to_owned()], &dir, &mut log)
            .unwrap();
        let err = fsck.check(&img, &dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Build);
        assert!(err.message().starts_with("e2fsck rc="), "{}", err);
        assert!(
            err.message().contains("WARNING: Filesystem still has errors"),
            "{}",
            err
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vendor_flag_bit_reads_the_dumped_build_prop() {
        let dir = tmp("volte_ext4_flagbit");
        let img = make_ext4(&dir, "flag.img");
        let mut ops = DebugfsBackend::new(&tool_dir());
        assert_eq!(read_vendor_flag_bit(&img, &dir, &mut ops), 0);

        std::fs::write(dir.join("build.prop"), b"ro.x=1\npersist.mtk.volte_support=1\n").unwrap();
        let mut log = quiet_log();
        ops.run(
            &img,
            &["write build.prop build.prop".to_owned()],
            &dir,
            &mut log,
        )
        .unwrap();
        assert_eq!(read_vendor_flag_bit(&img, &dir, &mut ops), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_chars_counts_characters_not_bytes() {
        let s = "aé漢😀b";
        assert_eq!(tail_chars(s, 4), "é漢😀b");
        assert_eq!(tail_chars(s, 99), s);
        assert_eq!(tail_chars("", 4), "");
    }

    #[cfg(windows)]
    #[test]
    fn a_hung_tool_is_killed_at_the_deadline() {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg("ping -n 10 127.0.0.1");
        let started = Instant::now();
        let err = capture(&mut cmd, Duration::from_millis(300)).unwrap_err();
        assert!(err.message().contains("timed out"), "{}", err);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
