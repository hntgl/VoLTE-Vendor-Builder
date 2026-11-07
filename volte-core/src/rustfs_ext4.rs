//! `rust-fs-ext4` backend, the default `Ext4Ops` engine since 2026-11-07.
//!
//! Interprets the same `debugfs` command strings `DebugfsBackend` receives, so
//! the pipeline can be pointed at either engine without changing call sites.
//! The parser covers exactly the verbs the pipeline emits: `cd`, `mkdir`,
//! `rmdir`, `rm`, `write`, `set_inode_field`, `ea_set -f`, `dump` and
//! `dump -p`. Like debugfs in `-f` mode, a failing command is reported and the
//! batch continues; only a mount or flush failure aborts the call.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs_ext4::block_io::{BlockDevice, FileDevice};
use fs_ext4::error::Error as FsError;
use fs_ext4::fs::Filesystem;

use crate::error::{Error, Result};
use crate::ext4::Ext4Ops;
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

struct Session {
    fs: Filesystem,
    cwd: String,
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

    /// Runs one `debugfs` command line. `Err` carries debugfs' per-command
    /// failure text; the caller keeps going.
    fn execute(
        &mut self,
        command: &str,
        workdir: &Path,
    ) -> std::result::Result<(), String> {
        let mut parts = command.split_whitespace();
        let verb = parts.next().unwrap_or("");
        match verb {
            "cd" => {
                let target = parts.next().ok_or("cd: missing directory")?;
                let path = self.full_path(target);
                let ino = self.inode_of(&path)?;
                if !self.fs.stat_ino(ino).map_err(|e| e.to_string())?.is_dir() {
                    return Err(format!("{path}: not a directory"));
                }
                self.cwd = path;
                Ok(())
            }
            "mkdir" => {
                let target = parts.next().ok_or("mkdir: missing path")?;
                let path = self.full_path(target);
                self.fs
                    .apply_mkdir(&path, 0o755)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            "rmdir" => {
                let target = parts.next().ok_or("rmdir: missing path")?;
                let path = self.full_path(target);
                self.fs.apply_rmdir(&path).map_err(|e| e.to_string())
            }
            "rm" => {
                let target = parts.next().ok_or("rm: missing path")?;
                let path = self.full_path(target);
                self.fs.apply_unlink(&path).map_err(|e| e.to_string())
            }
            "write" => {
                let host = parts.next().ok_or("write: missing host file")?;
                let name = parts.next().ok_or("write: missing image name")?;
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
                let name = parts.next().ok_or("set_inode_field: missing name")?;
                let field = parts.next().ok_or("set_inode_field: missing field")?;
                let value = parts.next().ok_or("set_inode_field: missing value")?;
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
                let flag = parts.next().ok_or("ea_set: missing flag")?;
                if flag != "-f" {
                    return Err(format!("ea_set: unsupported flag {flag}"));
                }
                let host = parts.next().ok_or("ea_set: missing host file")?;
                let name = parts.next().ok_or("ea_set: missing image name")?;
                let attr = parts.next().ok_or("ea_set: missing attribute")?;
                let value = read_host(workdir, host)?;
                let path = self.full_path(name);
                self.fs
                    .apply_setxattr(&path, attr, &value)
                    .map_err(|e| e.to_string())
            }
            "dump" => {
                let first = parts.next().ok_or("dump: missing path")?;
                let (vpath, host) = if first == "-p" {
                    (
                        parts.next().ok_or("dump: missing path")?,
                        parts.next().ok_or("dump: missing host file")?,
                    )
                } else {
                    (
                        first,
                        parts.next().ok_or("dump: missing host file")?,
                    )
                };
                let path = self.full_path(vpath);
                let ino = self.inode_of(&path)?;
                let inode = self.fs.stat_ino(ino).map_err(|e| e.to_string())?;
                if !inode.is_file() {
                    return Err(format!("{path}: not a regular file"));
                }
                let mut buf = vec![0u8; inode.size as usize];
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

impl Ext4Ops for RustFsExt4Backend {
    fn run(
        &mut self,
        image: &Path,
        commands: &[String],
        workdir: &Path,
        _log: &mut Logger,
    ) -> Result<String> {
        let dev: Arc<dyn BlockDevice> =
            Arc::new(FileDevice::open_path_rw(image).map_err(map_fs)?);
        let fs = Filesystem::mount(dev).map_err(map_fs)?;
        let mut session = Session {
            fs,
            cwd: "/".to_owned(),
        };
        let mut out = String::new();
        for command in commands {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            if let Err(message) = session.execute(command, workdir) {
                out.push_str(&format!("debugfs: {command}: {message}\n"));
            }
        }
        session.fs.finish().map_err(map_fs)?;
        Ok(out)
    }

    fn path_exists(&mut self, image: &Path, vpath: &str, _workdir: &Path) -> Result<bool> {
        let dev: Arc<dyn BlockDevice> =
            Arc::new(FileDevice::open_path(image).map_err(map_fs)?);
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
        assert!(ops
            .path_exists(&img, "/etc/payload.bin", &dir)
            .unwrap());
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
        let dev: Arc<dyn BlockDevice> =
            Arc::new(FileDevice::open_path(&img).unwrap());
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
    fn a_failed_command_is_reported_and_the_batch_continues() {
        let dir = tmp("volte_rustfs_continue");
        let img = make_ext4(&dir, "continue.img");
        let mut ops = RustFsExt4Backend::new();
        let out = run(&mut ops, &img, &dir, &["rm /missing", "mkdir /x"]);
        assert!(out.contains("not found"), "{out}");
        assert!(ops.path_exists(&img, "/x", &dir).unwrap());
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
