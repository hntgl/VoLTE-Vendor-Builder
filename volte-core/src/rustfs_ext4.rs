//! `rust-fs-ext4` backend, the default `Ext4Ops` engine since 2026-11-07.
//!
//! Interprets the same EXT4 command strings the pipeline emits (the `debugfs`
//! grammar of the Python reference), so
//! the pipeline can be pointed at either engine without changing call sites.
//! The parser covers exactly the verbs the pipeline emits: `cd`, `mkdir`,
//! `rmdir`, `rm`, `write`, `set_inode_field`, `ea_set -f`, `dump` and
//! `dump -p`. A failing command aborts [`Ext4Ops::run`] with the command and
//! its cause, so a batch never reports success for work that did not happen;
//! [`Ext4Ops::run_best_effort`] keeps the probe semantics instead. Host paths
//! are taken as the free-form remainder of the line, so a work directory with
//! spaces or non-ASCII characters reaches `write`, `ea_set` and `dump` intact.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs_ext4::block_io::{BlockDevice, FileDevice};
use fs_ext4::error::Error as FsError;
use fs_ext4::fs::Filesystem;

use crate::error::{Error, Result};
use crate::ext4_tools::Ext4Ops;
use crate::logging::Logger;

/// `Ext4Ops` over the pure-Rust ext4 driver.
pub struct RustFsExt4Backend;

impl RustFsExt4Backend {
    pub fn new() -> Self {
        RustFsExt4Backend
    }
}

impl Default for RustFsExt4Backend {
    fn default() -> Self {
        Self::new()
    }
}

fn map_fs(e: FsError) -> Error {
    match e {
        FsError::BadMagic { .. } => {
            Error::bad_magic("not an EXT4 image (rust-fs-ext4: bad magic number)")
        }
        other => Error::build(format!("rust-fs-ext4: {other}")),
    }
}

/// debugfs resolves host files against the per-batch working directory.
fn host_path(workdir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workdir.join(p)
    }
}

fn read_host(workdir: &Path, name: &str) -> std::result::Result<Vec<u8>, String> {
    let path = host_path(workdir, name);
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_host(workdir: &Path, name: &str, data: &[u8]) -> std::result::Result<(), String> {
    let path = host_path(workdir, name);
    std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))
}

/// A dump allocates the inode's declared size. Bound that claim by the image
/// file (a regular file cannot exceed its filesystem) and make the allocation
/// fallible, so a corrupt inode is an error instead of an OOM kill.
fn dump_buffer(declared: u64, image_len: u64) -> std::result::Result<Vec<u8>, String> {
    if declared > image_len {
        return Err(format!(
            "file claims {declared} bytes, more than the {image_len}-byte image"
        ));
    }
    let len = usize::try_from(declared)
        .map_err(|_| format!("file size {declared} does not fit memory"))?;
    let mut buf = Vec::new();
    buf.try_reserve_exact(len)
        .map_err(|_| format!("cannot allocate {len} bytes for the dump"))?;
    buf.resize(len, 0);
    Ok(buf)
}

fn parse_u32(text: &str) -> std::result::Result<u32, String> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => u32::from_str_radix(digits, 16),
        None => text.parse::<u32>(),
    }
    .map_err(|_| format!("invalid number: {text}"))
}

/// debugfs parses `set_inode_field ... mode` as octal unless `0x` prefixed.
fn parse_mode(text: &str) -> std::result::Result<u16, String> {
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(digits) => u16::from_str_radix(digits, 16),
        None => u16::from_str_radix(text, 8),
    }
    .map_err(|_| format!("invalid mode: {text}"))
}

/// Splits the verb off a command line.
fn split_verb(command: &str) -> (&str, &str) {
    match command.split_once(char::is_whitespace) {
        Some((verb, args)) => (verb, args.trim_start()),
        None => (command, ""),
    }
}

/// Splits the first whitespace-delimited token off `args`; the remainder keeps
/// embedded whitespace, which is how host paths with spaces survive.
fn split_token(args: &str) -> (&str, &str) {
    let args = args.trim_start();
    match args.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail.trim_start()),
        None => (args, ""),
    }
}

/// Splits the last whitespace-delimited token off `args`.
fn split_last_token(args: &str) -> Option<(&str, &str)> {
    let args = args.trim_end();
    let (head, tail) = args.rsplit_once(char::is_whitespace)?;
    Some((head.trim_end(), tail))
}

struct Session {
    fs: Filesystem,
    cwd: String,
    /// Size of the backing image file. A regular file inside the image can
    /// never be larger, so a corrupt inode cannot drive a huge allocation.
    image_len: u64,
}

impl Session {
    fn full_path(&self, path: &str) -> String {
        if path.starts_with('/') {
            path.to_owned()
        } else if self.cwd == "/" {
            format!("/{path}")
        } else {
            format!("{}/{path}", self.cwd)
        }
    }

    fn inode_of(&self, path: &str) -> std::result::Result<u32, String> {
        self.fs
            .lookup_path_bytes(path.as_bytes())
            .map_err(|e| e.to_string())
    }

    /// Runs one EXT4 command line (the `debugfs` grammar). `Err` carries the
    /// per-command failure text; the caller decides whether that aborts the batch.
    fn execute(&mut self, command: &str, workdir: &Path) -> std::result::Result<(), String> {
        let (verb, args) = split_verb(command);
        match verb {
            "cd" => {
                if args.is_empty() {
                    return Err("cd: missing directory".to_owned());
                }
                let path = self.full_path(args);
                let ino = self.inode_of(&path)?;
                if !self.fs.stat_ino(ino).map_err(|e| e.to_string())?.is_dir() {
                    return Err(format!("{path}: not a directory"));
                }
                self.cwd = path;
                Ok(())
            }
            "mkdir" => {
                if args.is_empty() {
                    return Err("mkdir: missing path".to_owned());
                }
                let path = self.full_path(args);
                self.fs
                    .apply_mkdir(&path, 0o755)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "rmdir" => {
                if args.is_empty() {
                    return Err("rmdir: missing path".to_owned());
                }
                let path = self.full_path(args);
                self.fs.apply_rmdir(&path).map_err(|e| e.to_string())
            }
            "rm" => {
                if args.is_empty() {
                    return Err("rm: missing path".to_owned());
                }
                let path = self.full_path(args);
                // Removing an absent path is idempotent cleanup, which the
                // overlay batches rely on (`rm` of an optional features file).
                match self.fs.apply_unlink(&path) {
                    Ok(()) => Ok(()),
                    Err(FsError::NotFound) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            }
            "write" => {
                // The image name is the last token; everything before it is the
                // host path, spaces included.
                let (host, name) =
                    split_last_token(args).ok_or("write: expected <host> <name>".to_owned())?;
                if host.is_empty() || name.is_empty() {
                    return Err("write: expected <host> <name>".to_owned());
                }
                let data = read_host(workdir, host)?;
                let path = self.full_path(name);
                match self.fs.lookup_path_bytes(path.as_bytes()) {
                    Ok(_) => self
                        .fs
                        .apply_replace_file_content(&path, &data)
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    Err(FsError::NotFound) => {
                        self.fs
                            .apply_create(&path, 0o644)
                            .map_err(|e| e.to_string())?;
                        if data.is_empty() {
                            return Ok(());
                        }
                        self.fs
                            .apply_pwrite(&path, 0, &data)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            "set_inode_field" => {
                let (name, rest) = split_token(args);
                let (field, value) = split_token(rest);
                if name.is_empty() || field.is_empty() || value.is_empty() {
                    return Err("set_inode_field: expected <name> <field> <value>".to_owned());
                }
                let path = self.full_path(name);
                let ino = self.inode_of(&path)?;
                match field {
                    // debugfs writes the full 16-bit mode; the driver keeps the
                    // file-type bits and applies the low 12 bits.
                    "mode" => {
                        let mode = parse_mode(value)?;
                        self.fs
                            .apply_chmod_ino(ino, mode)
                            .map_err(|e| e.to_string())
                    }
                    "uid" => {
                        let uid = parse_u32(value)?;
                        self.fs
                            .apply_chown_ino(ino, uid, u32::MAX)
                            .map_err(|e| e.to_string())
                    }
                    "gid" => {
                        let gid = parse_u32(value)?;
                        self.fs
                            .apply_chown_ino(ino, u32::MAX, gid)
                            .map_err(|e| e.to_string())
                    }
                    other => Err(format!("unsupported inode field: {other}")),
                }
            }
            "ea_set" => {
                let (flag, rest) = split_verb(args);
                if flag != "-f" {
                    return Err(format!("ea_set: unsupported flag {flag}"));
                }
                // `ea_set -f <host> <name> <attr>`: the host is the free-form
                // middle, so peel the attribute and name off the end.
                let (host_and_name, attr) = split_last_token(rest)
                    .ok_or("ea_set: expected <host> <name> <attr>".to_owned())?;
                let (host, name) = split_last_token(host_and_name)
                    .ok_or("ea_set: expected <host> <name> <attr>".to_owned())?;
                if host.is_empty() || name.is_empty() || attr.is_empty() {
                    return Err("ea_set: expected <host> <name> <attr>".to_owned());
                }
                let value = read_host(workdir, host)?;
                let path = self.full_path(name);
                self.fs
                    .apply_setxattr(&path, attr, &value)
                    .map_err(|e| e.to_string())
            }
            "dump" => {
                // `dump [-p] <vpath> <host>`: the image path is one token and
                // the host is the rest of the line, spaces included.
                let (first, rest) = split_token(args);
                let (vpath, host) = if first == "-p" {
                    split_token(rest)
                } else {
                    (first, rest)
                };
                if vpath.is_empty() || host.is_empty() {
                    return Err("dump: expected <path> <host file>".to_owned());
                }
                let path = self.full_path(vpath);
                let ino = self.inode_of(&path)?;
                let inode = self.fs.stat_ino(ino).map_err(|e| e.to_string())?;
                if !inode.is_file() {
                    return Err(format!("{path}: not a regular file"));
                }
                let mut buf =
                    dump_buffer(inode.size, self.image_len).map_err(|e| format!("{path}: {e}"))?;
                let read = self
                    .fs
                    .read_ino(ino, 0, &mut buf)
                    .map_err(|e| e.to_string())?;
                buf.truncate(read);
                write_host(workdir, host, &buf)
            }
            other => Err(format!("unsupported command: {other}")),
        }
    }
}

impl RustFsExt4Backend {
    fn open_session(image: &Path) -> Result<Session> {
        let image_len = std::fs::metadata(image)?.len();
        let dev: Arc<dyn BlockDevice> = Arc::new(FileDevice::open_path_rw(image).map_err(map_fs)?);
        let fs = Filesystem::mount(dev).map_err(map_fs)?;
        Ok(Session {
            fs,
            cwd: "/".to_owned(),
            image_len,
        })
    }
}

impl Ext4Ops for RustFsExt4Backend {
    fn run(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        _log: &mut Logger,
    ) -> Result<String> {
        let mut session = Self::open_session(image)?;
        for command in commands {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            if let Err(message) = session.execute(command, workdir) {
                // Release the mount cleanly, then report the failing command:
                // a batch must never claim success for work it did not do.
                let _ = session.fs.finish();
                return Err(Error::build(format!("ext4: {command}: {message}")));
            }
        }
        session.fs.finish().map_err(map_fs)?;
        Ok(String::new())
    }

    fn run_best_effort(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        _log: &mut Logger,
    ) -> Result<String> {
        let mut session = Self::open_session(image)?;
        let mut out = String::new();
        for command in commands {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            if let Err(message) = session.execute(command, workdir) {
                out.push_str(&format!("ext4: {command}: {message}\n"));
            }
        }
        session.fs.finish().map_err(map_fs)?;
        Ok(out)
    }

    fn path_exists(&mut self, image: &Path, vpath: &str, _workdir: &Path) -> Result<bool> {
        let dev: Arc<dyn BlockDevice> = Arc::new(FileDevice::open_path(image).map_err(map_fs)?);
        let fs = Filesystem::mount(dev).map_err(map_fs)?;
        match fs.lookup_path_bytes(vpath.as_bytes()) {
            Ok(_) => Ok(true),
            Err(FsError::NotFound) | Err(FsError::NotADirectory) => Ok(false),
            Err(e) => Err(map_fs(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use std::process::Command;

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
        let status = Command::new(tool_dir().join("mke2fs.exe"))
            .arg("-q")
            .arg("-t")
            .arg("ext4")
            .arg("-b")
            .arg("4096")
            .arg("-O")
            .arg("dir_index,^metadata_csum_seed,^orphan_file")
            .arg("-F")
            .arg(&img)
            .arg("16384")
            .current_dir(dir)
            .status()
            .expect("run mke2fs");
        assert!(status.success(), "mke2fs failed: {status}");
        img
    }

    fn quiet() -> Logger {
        Logger::new().with_sink(Vec::<String>::new())
    }

    fn run(ops: &mut RustFsExt4Backend, img: &Path, dir: &Path, cmds: &[&str]) -> String {
        let commands: Vec<String> = cmds.iter().map(|c| (*c).to_owned()).collect();
        ops.run(img, &commands, dir, &mut quiet()).expect("batch")
    }

    #[test]
    fn dump_buffer_bounds_the_claim_by_the_image_size() {
        assert_eq!(dump_buffer(0, 4096).unwrap().len(), 0);
        assert_eq!(dump_buffer(4096, 4096).unwrap().len(), 4096);
        let err = dump_buffer(4097, 4096).unwrap_err();
        assert!(err.contains("more than the 4096-byte image"), "{err}");
    }

    #[test]
    fn writes_a_file_dumps_it_back_and_reports_existence() {
        let dir = tmp("volte_rustfs_write");
        let img = make_ext4(&dir, "write.img");
        std::fs::write(dir.join("host.bin"), b"hello rust ext4\n").unwrap();
        let mut ops = RustFsExt4Backend::new();
        run(
            &mut ops,
            &img,
            &dir,
            &[
                "mkdir /etc",
                "cd /etc",
                "write host.bin payload.bin",
                "set_inode_field payload.bin mode 0x81A4",
                "set_inode_field payload.bin uid 0",
                "set_inode_field payload.bin gid 0",
                "dump /etc/payload.bin back.bin",
            ],
        );
        assert_eq!(
            std::fs::read(dir.join("back.bin")).unwrap(),
            b"hello rust ext4\n"
        );
        assert!(ops.path_exists(&img, "/etc/payload.bin", &dir).unwrap());
        assert!(!ops.path_exists(&img, "/etc/missing", &dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_inode_field_mode_and_gid_and_the_selinux_label_land_on_disk() {
        let dir = tmp("volte_rustfs_meta");
        let img = make_ext4(&dir, "meta.img");
        let label: &[u8] = b"u:object_r:vendor_overlay_file:s0\0";
        std::fs::write(dir.join("payload.bin"), b"apk bytes").unwrap();
        std::fs::write(dir.join("overlay.bin"), label).unwrap();
        let mut ops = RustFsExt4Backend::new();
        run(
            &mut ops,
            &img,
            &dir,
            &[
                "mkdir /overlay",
                "cd /overlay",
                "write payload.bin payload.apk",
                "set_inode_field payload.apk mode 0x81A4",
                "set_inode_field payload.apk uid 0",
                "set_inode_field payload.apk gid 0",
                "ea_set -f overlay.bin payload.apk security.selinux",
                "cd /overlay",
                "set_inode_field . gid 2000",
            ],
        );
        let dev: Arc<dyn BlockDevice> = Arc::new(FileDevice::open_path(&img).unwrap());
        let fs = Filesystem::mount(dev).unwrap();
        let ino = fs.lookup_path_bytes(b"/overlay/payload.apk").unwrap();
        let inode = fs.stat_ino(ino).unwrap();
        assert_eq!(inode.mode, 0o100644);
        assert_eq!(inode.uid, 0);
        assert_eq!(inode.gid, 0);
        let raw = fs.read_inode_raw(ino).unwrap();
        assert!(
            raw.windows(label.len()).any(|w| w == label),
            "SELinux label bytes are not in the inode"
        );
        let root = fs.lookup_path_bytes(b"/overlay").unwrap();
        assert_eq!(fs.stat_ino(root).unwrap().gid, 2000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removes_files_and_empty_directories() {
        let dir = tmp("volte_rustfs_remove");
        let img = make_ext4(&dir, "remove.img");
        std::fs::write(dir.join("f.bin"), b"x").unwrap();

        let mut ops = RustFsExt4Backend::new();
        run(
            &mut ops,
            &img,
            &dir,
            &["mkdir /a", "write f.bin /a/f", "rm /a/f", "rmdir /a"],
        );
        assert!(!ops.path_exists(&img, "/a", &dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_command_is_an_error_and_stops_the_batch() {
        let dir = tmp("volte_rustfs_fail_closed");
        let img = make_ext4(&dir, "fail.img");
        let mut ops = RustFsExt4Backend::new();
        let commands: Vec<String> = ["mkdir /a", "rmdir /missing", "mkdir /b"]
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let err = ops.run(&img, &commands, &dir, &mut quiet()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Build);
        assert!(
            err.message().contains("rmdir /missing"),
            "{}",
            err.message()
        );
        assert!(
            err.message().to_lowercase().contains("not found"),
            "{}",
            err.message()
        );
        assert!(
            ops.path_exists(&img, "/a", &dir).unwrap(),
            "commands before the failure stay applied"
        );
        assert!(
            !ops.path_exists(&img, "/b", &dir).unwrap(),
            "the batch stops at the failing command"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_after_rm_is_reported_instead_of_succeeding() {
        // The audit's C01 shape: props::write_back removes the target first, so
        // a failed write used to be reported as success with the file gone.
        let dir = tmp("volte_rustfs_rm_then_fail");
        let img = make_ext4(&dir, "rmfail.img");
        std::fs::write(dir.join("seed.prop"), b"ro.x=1\n").unwrap();
        let mut ops = RustFsExt4Backend::new();
        run(&mut ops, &img, &dir, &["write seed.prop build.prop"]);
        let commands: Vec<String> = vec![
            "rm /build.prop".to_owned(),
            "write missing-host.prop build.prop".to_owned(),
        ];
        let err = ops.run(&img, &commands, &dir, &mut quiet()).unwrap_err();
        assert!(err.message().contains("write missing-host.prop build.prop"));
        assert!(
            !ops.path_exists(&img, "/build.prop", &dir).unwrap(),
            "the rm did happen; the important part is that it is not reported as success"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_replacement_keeps_the_existing_file_intact() {
        let dir = tmp("volte_rustfs_keep_old");
        let img = make_ext4(&dir, "keep.img");
        std::fs::write(dir.join("old.prop"), b"ro.old=1\n").unwrap();
        let mut ops = RustFsExt4Backend::new();
        run(&mut ops, &img, &dir, &["write old.prop build.prop"]);
        let err = ops
            .run(
                &img,
                &["write missing-host.prop build.prop".to_owned()],
                &dir,
                &mut quiet(),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Build);
        run(&mut ops, &img, &dir, &["dump /build.prop back.prop"]);
        assert_eq!(std::fs::read(dir.join("back.prop")).unwrap(), b"ro.old=1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removing_an_absent_path_is_idempotent() {
        let dir = tmp("volte_rustfs_rm_absent");
        let img = make_ext4(&dir, "rmabsent.img");
        let mut ops = RustFsExt4Backend::new();
        run(&mut ops, &img, &dir, &["rm /missing", "mkdir /x"]);
        assert!(ops.path_exists(&img, "/x", &dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn best_effort_reports_failures_and_keeps_going() {
        let dir = tmp("volte_rustfs_best_effort");
        let img = make_ext4(&dir, "best.img");
        let mut ops = RustFsExt4Backend::new();
        let commands: Vec<String> = ["rmdir /missing", "mkdir /x"]
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let out = ops
            .run_best_effort(&img, &commands, &dir, &mut quiet())
            .expect("probe batch");
        assert!(out.contains("not found"), "{out}");
        assert!(ops.path_exists(&img, "/x", &dir).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_paths_with_spaces_and_non_ascii_survive_write_and_dump() {
        let dir = tmp("volte_rustfs_spaces");
        let img = make_ext4(&dir, "spaces.img");
        let work = dir.join("audit dir").join("thư mục làm việc");
        std::fs::create_dir_all(&work).unwrap();
        let host = work.join("seed file.prop");
        std::fs::write(&host, b"ro.keep=1\n").unwrap();
        let back = work.join("out file.prop");
        let commands: Vec<String> = vec![
            format!("write {} /build.prop", crate::env::forward_slashes(&host)),
            format!("dump -p /build.prop {}", crate::env::forward_slashes(&back)),
        ];
        let mut ops = RustFsExt4Backend::new();
        ops.run(&img, &commands, &work, &mut quiet()).unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), b"ro.keep=1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn props_upsert_works_from_a_work_directory_with_spaces() {
        let dir = tmp("volte_rustfs_props_spaces");
        let img = make_ext4(&dir, "props.img");
        let seed = dir.join("seed.prop");
        std::fs::write(&seed, b"ro.keep=1\n").unwrap();
        let mut ops = RustFsExt4Backend::new();
        let seed_cmd = format!("write {} build.prop", crate::env::forward_slashes(&seed));
        ops.run(&img, &[seed_cmd], &dir, &mut quiet()).unwrap();

        let td = dir.join("audit dir");
        std::fs::create_dir_all(&td).unwrap();
        let mut log = quiet();
        let changed = crate::props::upsert(&img, &td, &mut ops, &mut log).unwrap();
        assert_eq!(changed, 75, "a space in the workdir must not disable props");

        let back = td.join("check.prop");
        let dump_cmd = format!("dump /build.prop {}", crate::env::forward_slashes(&back));
        ops.run(&img, &[dump_cmd], &td, &mut quiet()).unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(&back).unwrap()).into_owned();
        assert!(text.contains("ro.keep=1"));
        assert!(text.contains("persist.dbg.ims_volte_enable=1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ea_set_accepts_a_host_path_with_spaces() {
        let dir = tmp("volte_rustfs_ea_spaces");
        let img = make_ext4(&dir, "ea.img");
        let label: &[u8] = b"u:object_r:vendor_overlay_file:s0\0";
        let work = dir.join("audit dir");
        std::fs::create_dir_all(&work).unwrap();
        let label_host = work.join("overlay label.bin");
        std::fs::write(&label_host, label).unwrap();
        let payload = work.join("payload file.apk");
        std::fs::write(&payload, b"apk bytes").unwrap();

        let commands: Vec<String> = vec![
            format!(
                "write {} payload.apk",
                crate::env::forward_slashes(&payload)
            ),
            format!(
                "ea_set -f {} payload.apk security.selinux",
                crate::env::forward_slashes(&label_host)
            ),
        ];
        let mut ops = RustFsExt4Backend::new();
        ops.run(&img, &commands, &work, &mut quiet()).unwrap();

        let dev: Arc<dyn BlockDevice> = Arc::new(FileDevice::open_path(&img).unwrap());
        let fs = Filesystem::mount(dev).unwrap();
        let ino = fs.lookup_path_bytes(b"/payload.apk").unwrap();
        let raw = fs.read_inode_raw(ino).unwrap();
        assert!(
            raw.windows(label.len()).any(|w| w == label),
            "the SELinux label must land from a spaced host path"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_non_ext4_file_is_refused_with_bad_magic() {
        let dir = tmp("volte_rustfs_badmagic");
        let img = dir.join("zeros.img");
        std::fs::write(&img, vec![0u8; 1 << 20]).unwrap();
        let mut ops = RustFsExt4Backend::new();
        let err = ops
            .run(&img, &["mkdir /x".to_owned()], &dir, &mut quiet())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::BadMagic, "{}", err.message());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
