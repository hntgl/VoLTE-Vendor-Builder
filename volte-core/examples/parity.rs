//! Phase 2.5 parity harness (measurement only, not used by the product).
//!
//! Two commands:
//!
//! ```text
//! parity build <debugfs|rustfs> <src> <out> <qc|mtk> <root> <logs> <tool>
//! parity manifest <image> <out.tsv>
//! ```
//!
//! `build` runs the real pipeline (real `e2fsck`, real `avbtool.py`) with the
//! chosen EXT4 backend. `manifest` walks an image read-only through the same
//! pure-Rust driver and writes one TSV line per path: escaped path, type, mode,
//! uid, gid, size, mtime, and the SHA-256 of the file bytes (or of the symlink
//! target). Comparing the two manifests is the content/metadata parity check;
//! timestamps are recorded but not part of the verdict.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use fs_ext4::block_io::{BlockDevice, FileDevice};
use fs_ext4::fs::Filesystem;
use fs_ext4::inode::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG, S_IFSOCK};
use volte_core::avb::PythonAvbtool;
use volte_core::ext4::{DebugfsBackend, E2fsck};
use volte_core::logging::{Logger, Sink};
use volte_core::pipeline::{self, Deps, Options};
use volte_core::rustfs_ext4::RustFsExt4Backend;
use volte_core::sha256::{hex_upper, Sha256};

struct Stdout;

impl Sink for Stdout {
    fn emit(&mut self, line: &str) {
        println!("{line}");
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[ERROR] {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("build") => build(args),
        Some("manifest") => manifest(args),
        _ => Err("usage: parity build <debugfs|rustfs> <src> <out> <qc|mtk> <root> <logs> <tool>\n\
                  \x20      parity manifest <image> <out.tsv>"
            .to_string()),
    }
}

fn arg<'a>(args: &'a [String], i: usize, name: &str) -> Result<&'a str, String> {
    args.get(i)
        .map(String::as_str)
        .ok_or_else(|| format!("missing argument {i} ({name})"))
}

fn build(args: &[String]) -> Result<(), String> {
    let kind = arg(args, 1, "backend")?;
    let src = PathBuf::from(arg(args, 2, "src")?);
    let out = arg(args, 3, "out")?;
    let chip = arg(args, 4, "chip")?;
    let root = PathBuf::from(arg(args, 5, "root")?);
    let logs = PathBuf::from(arg(args, 6, "logs")?);
    let tool = PathBuf::from(arg(args, 7, "tool")?);

    let python = {
        let bundled = tool.join("python").join("python.exe");
        if bundled.is_file() {
            bundled
        } else {
            PathBuf::from("python.exe")
        }
    };
    let mut avb = PythonAvbtool::new(python, tool.join("avbtool.py"))
        .map_err(|e| format!("avbtool: {e}"))?;
    let mut fsck = E2fsck::new(&tool);
    let mut log = Logger::new().with_sink(Stdout);
    let mut progress = |_n: u8, _label: &str| {};
    let opts = Options {
        src: &src,
        out_path: out,
        chip,
        keep_out: true,
        root: &root,
        logs_dir: &logs,
    };

    let started = std::time::Instant::now();
    let result = match kind {
        "debugfs" => {
            let mut ops = DebugfsBackend::new(&tool);
            let mut deps = Deps { ops: &mut ops, fsck: &mut fsck, avb: &mut avb };
            pipeline::run(&opts, &mut deps, &mut log, &mut progress)
        }
        "rustfs" => {
            let mut ops = RustFsExt4Backend::new();
            let mut deps = Deps { ops: &mut ops, fsck: &mut fsck, avb: &mut avb };
            pipeline::run(&opts, &mut deps, &mut log, &mut progress)
        }
        other => return Err(format!("unknown backend: {other}")),
    };
    match result {
        Ok(path) => {
            println!(
                "[PARITY] result=OK backend={kind} out={} elapsed={:.1}s",
                path.display(),
                started.elapsed().as_secs_f64()
            );
            Ok(())
        }
        Err(e) => {
            println!("[PARITY] result=ERR backend={kind} {}", e.message());
            Err(e.message().to_string())
        }
    }
}

fn manifest(args: &[String]) -> Result<(), String> {
    let image = PathBuf::from(arg(args, 1, "image")?);
    let out = PathBuf::from(arg(args, 2, "out")?);

    let dev: Arc<dyn BlockDevice> =
        Arc::new(FileDevice::open_path(&image).map_err(|e| format!("open: {e}"))?);
    let fs = Filesystem::mount(dev).map_err(|e| format!("mount: {e}"))?;
    let root = fs
        .read_dir_ino(2u32)
        .map_err(|e| format!("read root: {e}"))?;

    let mut lines: Vec<String> = Vec::new();
    for entry in root {
        if entry.name == b"." || entry.name == b".." {
            continue;
        }
        walk(&fs, entry.inode, &format!("/{}", escape(&entry.name)), &mut lines)?;
    }
    lines.sort();
    std::fs::write(&out, lines.join("\n") + "\n").map_err(|e| format!("write: {e}"))?;
    println!("[PARITY] manifest={} entries={}", out.display(), lines.len());
    Ok(())
}

fn walk(
    fs: &Filesystem,
    ino: u32,
    path: &str,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let inode = fs.stat_ino(ino).map_err(|e| format!("stat {path}: {e}"))?;
    let kind = match inode.mode & S_IFMT {
        S_IFDIR => "d",
        S_IFREG => "f",
        S_IFLNK => "l",
        S_IFCHR => "c",
        S_IFBLK => "b",
        S_IFIFO => "p",
        S_IFSOCK => "s",
        _ => "?",
    };
    let digest = match kind {
        "f" => file_digest(fs, ino, inode.size, path)?,
        "l" => {
            let target = fs
                .read_link_ino(ino)
                .map_err(|e| format!("readlink {path}: {e}"))?;
            hex_upper(&digest_bytes(&target))
        }
        _ => "-".to_string(),
    };
    lines.push(format!(
        "{path}\t{kind}\t{:o}\t{}\t{}\t{}\t{}\t{digest}",
        inode.mode, inode.uid, inode.gid, inode.size, inode.mtime
    ));
    if kind == "d" {
        let entries = fs
            .read_dir_ino(ino)
            .map_err(|e| format!("read {path}: {e}"))?;
        for entry in entries {
            if entry.name == b"." || entry.name == b".." {
                continue;
            }
            walk(
                fs,
                entry.inode,
                &format!("{path}/{}", escape(&entry.name)),
                lines,
            )?;
        }
    }
    Ok(())
}

fn digest_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finish()
}

fn file_digest(fs: &Filesystem, ino: u32, size: u64, path: &str) -> Result<String, String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut off = 0u64;
    while off < size {
        let n = fs
            .read_ino(ino, off, &mut buf)
            .map_err(|e| format!("read {path} at {off}: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        off += n as u64;
    }
    Ok(hex_upper(&h.finish()))
}

fn escape(name: &[u8]) -> String {
    let mut s = String::new();
    for &b in name {
        match b {
            b'\\' => s.push_str("\\\\"),
            b'\t' => s.push_str("\\t"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s
}
