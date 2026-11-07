//! External `e2fsck` runner and the EXT4 backend traits.
//!
//! After the Đợt 2.5 parity check the product has a single EXT4 engine: the
//! in-process `rust-fs-ext4` backend (`crate::rustfs_ext4`). This module keeps
//! the [`Ext4Ops`] contract the pipeline codes against and the external
//! `e2fsck -fn` verdict ([`Fsck`], [`E2fsck`]).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::flags::flag_bit_from_dumps;
use crate::logging::{decode, Logger};

const FSCK_TIMEOUT: Duration = Duration::from_secs(3600);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The two ext4 verbs the pipeline needs.
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
    /// absent. Backends with a fail-fast `run` override this with a
    /// per-command loop.
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

/// `e2fsck.exe` lookup: the tool directory first, then the bare name on PATH
/// (`:432-434`).
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
        let out = capture(&mut cmd, FSCK_TIMEOUT).map_err(|e| {
            Error::new(
                e.kind(),
                format!("e2fsck ({}): {}", self.exe.display(), e.message()),
            )
        })?;
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
    use crate::rustfs_ext4::RustFsExt4Backend;

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
        quiet(&mut cmd);
        let status = cmd.status().expect("run mke2fs");
        assert!(status.success(), "mke2fs failed: {status}");
        assert_eq!(std::fs::metadata(&img).unwrap().len(), 16384 * 4096);
        img
    }

    #[test]
    fn e2fsck_falls_back_to_the_bare_name() {
        let fsck = E2fsck::new(Path::new(r"E:\no\such\tool"));
        assert_eq!(fsck.exe(), Path::new("e2fsck.exe"));
    }

    #[test]
    fn a_missing_e2fsck_executable_is_an_error() {
        let dir = tmp("volte_ext4_fsck_missing");
        let img = make_ext4(&dir, "clean.img");
        let mut fsck = E2fsck { exe: dir.join("no-such-e2fsck.exe") };
        let err = fsck.check(&img, &dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Io);
        assert!(err.message().contains("e2fsck"), "{err}");
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

        // s_magic sits 56 bytes into the superblock at offset 1024; zeroing it
        // makes e2fsck exit non-zero with the tail carrying the reason.
        let mut bytes = std::fs::read(&img).unwrap();
        bytes[1024 + 56..1024 + 58].fill(0);
        std::fs::write(&img, &bytes).unwrap();
        let err = fsck.check(&img, &dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Build);
        assert!(err.message().starts_with("e2fsck rc="), "{}", err);
        assert!(
            err.message().to_lowercase().contains("bad magic"),
            "{}",
            err
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vendor_flag_bit_reads_the_dumped_build_prop() {
        let dir = tmp("volte_ext4_flagbit");
        let img = make_ext4(&dir, "flag.img");
        let mut ops = RustFsExt4Backend::new();
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
