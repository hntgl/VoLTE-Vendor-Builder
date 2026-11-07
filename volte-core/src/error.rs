//! Recoverable failures of the builder.
//!
//! Mirrors Python's `BuildError` (a single exception type carrying a message)
//! while keeping a machine-readable kind so callers can distinguish a genuine
//! EXT4 refusal from an I/O failure.

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Expected build failure, message comes from the pipeline.
    Build,
    /// Filesystem or stream error.
    Io,
    /// The image is not EXT4 (`debugfs: Bad magic number`).
    BadMagic,
    /// A required external tool or payload file is missing.
    Missing,
    /// Invalid command line.
    Usage,
}

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error { kind, message: message.into() }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::Build, message)
    }

    pub fn bad_magic(message: impl Into<String>) -> Self {
        Error::new(ErrorKind::BadMagic, message)
    }

    pub fn missing(what: &str, path: &Path) -> Self {
        Error::new(
            ErrorKind::Missing,
            format!("missing {what}: {}", path.display()),
        )
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(ErrorKind::Io, e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_error_display_is_the_message() {
        let e = Error::build("Thieu payload: overlay.bin");
        assert_eq!(e.to_string(), "Thieu payload: overlay.bin");
        assert_eq!(e.kind(), ErrorKind::Build);
    }

    #[test]
    fn io_error_maps_to_io_kind() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let e = Error::from(io);
        assert_eq!(e.kind(), ErrorKind::Io);
    }

    #[test]
    fn missing_names_the_exact_path() {
        let p = Path::new(r"E:\Build_app\Volte\tool\avbtool.py");
        let e = Error::missing("tool", p);
        assert_eq!(e.kind(), ErrorKind::Missing);
        assert!(e.message().contains(r"E:\Build_app\Volte\tool\avbtool.py"));
    }

    #[test]
    fn bad_magic_kind_is_distinct() {
        assert_eq!(Error::bad_magic("debugfs: Bad magic number").kind(), ErrorKind::BadMagic);
    }
}
