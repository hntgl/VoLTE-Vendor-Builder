//! File-identity comparison for source/destination guards.
//!
//! `PathBuf` equality is case-sensitive and knows nothing about symlinks or
//! hard links, so a case-only `--out` alias passed the old guard and deleted
//! the source (audit A01b/G02). Ask the filesystem instead: `same-file`
//! compares volume/file index on Windows and device/inode on Unix. Only when a
//! path cannot be opened (typically because the destination does not exist
//! yet) does the check fall back to an absolute, case-folded comparison.

use std::path::Path;

/// True when both paths name the same file, or would name the same file once
/// the missing one is created.
pub fn same_file(a: &Path, b: &Path) -> bool {
    match same_file::is_same_file(a, b) {
        Ok(same) => same,
        Err(_) => absolute_eq(a, b),
    }
}

fn absolute_eq(a: &Path, b: &Path) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(x), Ok(y)) => path_eq(&x, &y),
        _ => path_eq(a, b),
    }
}

fn path_eq(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// True when both directories live on the same volume, so their disk-space
/// needs must be added together. Unknown identity is treated as "same" to keep
/// the caller's probe conservative.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        fn volume(p: &Path) -> Option<String> {
            let canon = std::fs::canonicalize(p).ok()?;
            let first = canon.components().next()?;
            Some(first.as_os_str().to_string_lossy().into_owned())
        }
        match (volume(a), volume(b)) {
            (Some(x), Some(y)) => x.eq_ignore_ascii_case(&y),
            _ => true,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => true,
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (a, b);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn a_directory_and_its_child_share_a_volume() {
        let dir = tmp("volte_path_id_volume");
        let child = dir.join("sub");
        std::fs::create_dir_all(&child).unwrap();
        assert!(same_volume(&dir, &child));
        assert!(same_volume(&dir, &dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identical_paths_are_the_same_file() {
        let dir = tmp("volte_path_id_same");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"x").unwrap();
        assert!(same_file(&p, &p));
        assert!(!same_file(&p, &dir.join("other.bin")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_and_absolute_paths_are_the_same_file() {
        let cwd = std::env::current_dir().unwrap();
        let rel = Path::new("Cargo.toml");
        assert!(cwd.join(rel).is_file(), "test must run in the crate root");
        assert!(same_file(rel, &cwd.join(rel)));
    }

    #[cfg(windows)]
    #[test]
    fn case_only_alias_is_the_same_file_on_windows() {
        let dir = tmp("volte_path_id_case");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"x").unwrap();
        let alias = dir.join("IMG.BIN");
        assert!(
            alias.is_file(),
            "the test needs a case-insensitive filesystem"
        );
        assert!(same_file(&p, &alias));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlink_is_the_same_file_when_the_platform_allows_it() {
        let dir = tmp("volte_path_id_symlink");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"x").unwrap();
        let link = dir.join("link.bin");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&p, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&p, &link);
        if made.is_err() {
            return;
        }
        assert!(same_file(&p, &link));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_link_is_the_same_file_when_the_platform_allows_it() {
        let dir = tmp("volte_path_id_hardlink");
        let p = dir.join("img.bin");
        std::fs::write(&p, b"x").unwrap();
        let link = dir.join("hard.bin");
        if std::fs::hard_link(&p, &link).is_err() {
            return;
        }
        assert!(same_file(&p, &link));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
