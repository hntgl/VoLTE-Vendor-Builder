//! Tool and executable roots, ported from `tool_dir` (`hd_vendor_build.py:277`).
//!
//! Python anchors every path to the script directory (`ROOT`); the Rust port
//! anchors to `current_exe().parent()` so the same layout works for the built
//! binary.

use std::path::{Path, PathBuf};

/// Candidate order of `tool_dir`: `tool\` → `tools\` → the root itself, chosen
/// by the presence of `avbtool.py` or `e2fsck.exe`. Falls back to the root.
pub fn tool_dir(root: &Path) -> PathBuf {
    for cand in [root.join("tool"), root.join("tools"), root.to_path_buf()] {
        if cand.join("avbtool.py").is_file() || cand.join("e2fsck.exe").is_file() {
            return cand;
        }
    }
    root.to_path_buf()
}

/// Directory that contains the running executable.
pub fn exe_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The pipeline keeps Python's `"\\"` → `"/"` conversion so the command lists
/// stay byte-identical; host paths never reach the in-process engine with a
/// Windows separator.
pub fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn tool_dir_prefers_tool_over_tools_and_root() {
        let root = tmp("volte_env_tool_priority");
        std::fs::create_dir_all(root.join("tool")).unwrap();
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("tool").join("avbtool.py"), b"#").unwrap();
        std::fs::write(root.join("tools").join("e2fsck.exe"), b"MZ").unwrap();
        std::fs::write(root.join("e2fsck.exe"), b"MZ").unwrap();
        assert_eq!(tool_dir(&root), root.join("tool"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tool_dir_falls_back_to_tools_then_root() {
        let root = tmp("volte_env_tool_fallback");
        std::fs::create_dir_all(root.join("tools")).unwrap();
        std::fs::write(root.join("tools").join("e2fsck.exe"), b"MZ").unwrap();
        assert_eq!(tool_dir(&root), root.join("tools"));

        std::fs::remove_file(root.join("tools").join("e2fsck.exe")).unwrap();
        std::fs::write(root.join("avbtool.py"), b"#").unwrap();
        assert_eq!(tool_dir(&root), root);

        std::fs::remove_file(root.join("avbtool.py")).unwrap();
        assert_eq!(tool_dir(&root), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn forward_slashes_replaces_backslashes() {
        assert_eq!(
            forward_slashes(Path::new(r"E:\Build_app\x\payload.apk")),
            "E:/Build_app/x/payload.apk"
        );
    }

    #[test]
    fn exe_root_points_at_the_running_directory() {
        let root = exe_root();
        assert!(root.is_dir(), "exe root must exist: {}", root.display());
    }
}
