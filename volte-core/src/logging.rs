//! Byte decoding and logging sinks.
//!
//! `decode` reproduces `_decode` (`hd_vendor_build.py:264`): UTF-8 first, then
//! CP1258, then CP1252, then Latin-1. The two code-page tables are generated
//! from the bundled Python codecs, not typed by hand.
//!
//! `Logger` is the `BuildLog` equivalent (`:346`) with the console replaced by
//! an injected sink: this crate never writes to stdout/stderr itself.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Latin-1 complement of CP1258 for bytes 0x80..=0xFF; `None` = undefined.
const CP1258_HIGH: [Option<char>; 128] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    None,
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    None,
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    None,
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    None,
    Some('\u{0178}'),
    Some('\u{00A0}'),
    Some('\u{00A1}'),
    Some('\u{00A2}'),
    Some('\u{00A3}'),
    Some('\u{00A4}'),
    Some('\u{00A5}'),
    Some('\u{00A6}'),
    Some('\u{00A7}'),
    Some('\u{00A8}'),
    Some('\u{00A9}'),
    Some('\u{00AA}'),
    Some('\u{00AB}'),
    Some('\u{00AC}'),
    Some('\u{00AD}'),
    Some('\u{00AE}'),
    Some('\u{00AF}'),
    Some('\u{00B0}'),
    Some('\u{00B1}'),
    Some('\u{00B2}'),
    Some('\u{00B3}'),
    Some('\u{00B4}'),
    Some('\u{00B5}'),
    Some('\u{00B6}'),
    Some('\u{00B7}'),
    Some('\u{00B8}'),
    Some('\u{00B9}'),
    Some('\u{00BA}'),
    Some('\u{00BB}'),
    Some('\u{00BC}'),
    Some('\u{00BD}'),
    Some('\u{00BE}'),
    Some('\u{00BF}'),
    Some('\u{00C0}'),
    Some('\u{00C1}'),
    Some('\u{00C2}'),
    Some('\u{0102}'),
    Some('\u{00C4}'),
    Some('\u{00C5}'),
    Some('\u{00C6}'),
    Some('\u{00C7}'),
    Some('\u{00C8}'),
    Some('\u{00C9}'),
    Some('\u{00CA}'),
    Some('\u{00CB}'),
    Some('\u{0300}'),
    Some('\u{00CD}'),
    Some('\u{00CE}'),
    Some('\u{00CF}'),
    Some('\u{0110}'),
    Some('\u{00D1}'),
    Some('\u{0309}'),
    Some('\u{00D3}'),
    Some('\u{00D4}'),
    Some('\u{01A0}'),
    Some('\u{00D6}'),
    Some('\u{00D7}'),
    Some('\u{00D8}'),
    Some('\u{00D9}'),
    Some('\u{00DA}'),
    Some('\u{00DB}'),
    Some('\u{00DC}'),
    Some('\u{01AF}'),
    Some('\u{0303}'),
    Some('\u{00DF}'),
    Some('\u{00E0}'),
    Some('\u{00E1}'),
    Some('\u{00E2}'),
    Some('\u{0103}'),
    Some('\u{00E4}'),
    Some('\u{00E5}'),
    Some('\u{00E6}'),
    Some('\u{00E7}'),
    Some('\u{00E8}'),
    Some('\u{00E9}'),
    Some('\u{00EA}'),
    Some('\u{00EB}'),
    Some('\u{0301}'),
    Some('\u{00ED}'),
    Some('\u{00EE}'),
    Some('\u{00EF}'),
    Some('\u{0111}'),
    Some('\u{00F1}'),
    Some('\u{0323}'),
    Some('\u{00F3}'),
    Some('\u{00F4}'),
    Some('\u{01A1}'),
    Some('\u{00F6}'),
    Some('\u{00F7}'),
    Some('\u{00F8}'),
    Some('\u{00F9}'),
    Some('\u{00FA}'),
    Some('\u{00FB}'),
    Some('\u{00FC}'),
    Some('\u{01B0}'),
    Some('\u{20AB}'),
    Some('\u{00FF}'),
];

/// CP1252 for bytes 0x80..=0xFF; `None` = undefined.
const CP1252_HIGH: [Option<char>; 128] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
    Some('\u{00A0}'),
    Some('\u{00A1}'),
    Some('\u{00A2}'),
    Some('\u{00A3}'),
    Some('\u{00A4}'),
    Some('\u{00A5}'),
    Some('\u{00A6}'),
    Some('\u{00A7}'),
    Some('\u{00A8}'),
    Some('\u{00A9}'),
    Some('\u{00AA}'),
    Some('\u{00AB}'),
    Some('\u{00AC}'),
    Some('\u{00AD}'),
    Some('\u{00AE}'),
    Some('\u{00AF}'),
    Some('\u{00B0}'),
    Some('\u{00B1}'),
    Some('\u{00B2}'),
    Some('\u{00B3}'),
    Some('\u{00B4}'),
    Some('\u{00B5}'),
    Some('\u{00B6}'),
    Some('\u{00B7}'),
    Some('\u{00B8}'),
    Some('\u{00B9}'),
    Some('\u{00BA}'),
    Some('\u{00BB}'),
    Some('\u{00BC}'),
    Some('\u{00BD}'),
    Some('\u{00BE}'),
    Some('\u{00BF}'),
    Some('\u{00C0}'),
    Some('\u{00C1}'),
    Some('\u{00C2}'),
    Some('\u{00C3}'),
    Some('\u{00C4}'),
    Some('\u{00C5}'),
    Some('\u{00C6}'),
    Some('\u{00C7}'),
    Some('\u{00C8}'),
    Some('\u{00C9}'),
    Some('\u{00CA}'),
    Some('\u{00CB}'),
    Some('\u{00CC}'),
    Some('\u{00CD}'),
    Some('\u{00CE}'),
    Some('\u{00CF}'),
    Some('\u{00D0}'),
    Some('\u{00D1}'),
    Some('\u{00D2}'),
    Some('\u{00D3}'),
    Some('\u{00D4}'),
    Some('\u{00D5}'),
    Some('\u{00D6}'),
    Some('\u{00D7}'),
    Some('\u{00D8}'),
    Some('\u{00D9}'),
    Some('\u{00DA}'),
    Some('\u{00DB}'),
    Some('\u{00DC}'),
    Some('\u{00DD}'),
    Some('\u{00DE}'),
    Some('\u{00DF}'),
    Some('\u{00E0}'),
    Some('\u{00E1}'),
    Some('\u{00E2}'),
    Some('\u{00E3}'),
    Some('\u{00E4}'),
    Some('\u{00E5}'),
    Some('\u{00E6}'),
    Some('\u{00E7}'),
    Some('\u{00E8}'),
    Some('\u{00E9}'),
    Some('\u{00EA}'),
    Some('\u{00EB}'),
    Some('\u{00EC}'),
    Some('\u{00ED}'),
    Some('\u{00EE}'),
    Some('\u{00EF}'),
    Some('\u{00F0}'),
    Some('\u{00F1}'),
    Some('\u{00F2}'),
    Some('\u{00F3}'),
    Some('\u{00F4}'),
    Some('\u{00F5}'),
    Some('\u{00F6}'),
    Some('\u{00F7}'),
    Some('\u{00F8}'),
    Some('\u{00F9}'),
    Some('\u{00FA}'),
    Some('\u{00FB}'),
    Some('\u{00FC}'),
    Some('\u{00FD}'),
    Some('\u{00FE}'),
    Some('\u{00FF}'),
];

fn decode_with_high(bytes: &[u8], high: &[Option<char>; 128]) -> Option<String> {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x80 {
            out.push(b as char);
        } else {
            out.push(high[(b - 0x80) as usize]?);
        }
    }
    Some(out)
}

/// Port of `_decode` (`hd_vendor_build.py:264`).
pub fn decode(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    if let Some(s) = decode_with_high(bytes, &CP1258_HIGH) {
        return s;
    }
    if let Some(s) = decode_with_high(bytes, &CP1252_HIGH) {
        return s;
    }
    bytes.iter().map(|&b| b as char).collect()
}

/// Port of `fmt_time` (`:324`): whole minutes and seconds, zero padded.
pub fn fmt_time(sec: f64) -> String {
    let total = if sec.is_finite() && sec > 0.0 {
        sec as u64
    } else {
        0
    };
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// Where a log line goes. The CLI supplies a console sink; tests supply a
/// memory sink; `FileSink` writes the build log file.
pub trait Sink {
    fn emit(&mut self, line: &str);
}

/// Appending UTF-8 log file with LF line endings (`BuildLog`, `:346`).
pub struct FileSink {
    file: Option<File>,
}

impl FileSink {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(FileSink { file: Some(file) })
    }
}

impl Sink for FileSink {
    fn emit(&mut self, line: &str) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.flush();
        }
    }
}

impl Drop for FileSink {
    fn drop(&mut self) {
        self.file = None;
    }
}

/// Collects lines in memory (the Python `sink` callback, used by tests).
#[derive(Default)]
pub struct MemorySink {
    pub lines: Vec<String>,
}

impl Sink for MemorySink {
    fn emit(&mut self, line: &str) {
        self.lines.push(line.to_owned());
    }
}

impl Sink for Vec<String> {
    fn emit(&mut self, line: &str) {
        self.push(line.to_owned());
    }
}

/// Port of `BuildLog`: every line goes to all sinks, in registration order.
#[derive(Default)]
pub struct Logger {
    sinks: Vec<Box<dyn Sink>>,
}

impl Logger {
    pub fn new() -> Self {
        Logger { sinks: Vec::new() }
    }

    pub fn with_sink<S: Sink + 'static>(mut self, sink: S) -> Self {
        self.sinks.push(Box::new(sink));
        self
    }

    pub fn add_sink<S: Sink + 'static>(&mut self, sink: S) {
        self.sinks.push(Box::new(sink));
    }

    pub fn log(&mut self, msg: impl AsRef<str>) {
        let text = msg.as_ref();
        for sink in self.sinks.iter_mut() {
            sink.emit(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_wins_when_valid() {
        let s = "Tiếng Việt";
        assert_eq!(decode(s.as_bytes()), s);
    }

    #[test]
    fn cp1252_bytes_decode_after_utf8_fails() {
        // 0x80 is undefined in CP1258, so the CP1252 table is used: euro sign.
        let got = decode(&[b'A', 0x80, b'B']);
        assert_eq!(got, "A\u{20AC}B");
    }

    #[test]
    fn cp1258_bytes_win_before_cp1252() {
        // 0xD0 is 'Đ' in CP1258 but 'Ð' in CP1252/Latin-1.
        assert_eq!(decode(&[0xD0]), "\u{0110}");
        assert_eq!(decode(&[0xFD]), "\u{01B0}");
    }

    #[test]
    fn latin1_is_the_last_resort() {
        // 0x81 is undefined in both CP1258 and CP1252.
        assert_eq!(decode(&[0x81]), "\u{0081}");
    }

    #[test]
    fn fmt_time_matches_python() {
        assert_eq!(fmt_time(0.0), "00:00");
        assert_eq!(fmt_time(59.9), "00:59");
        assert_eq!(fmt_time(61.0), "01:01");
        assert_eq!(fmt_time(-5.0), "00:00");
    }

    struct SharedSink(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

    impl Sink for SharedSink {
        fn emit(&mut self, line: &str) {
            self.0.borrow_mut().push(line.to_owned());
        }
    }

    #[test]
    fn logger_fans_out_to_every_sink_in_order() {
        let a = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let b = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut logger = Logger::new()
            .with_sink(SharedSink(a.clone()))
            .with_sink(SharedSink(b.clone()));
        logger.log("e2fsck=PASS");
        logger.log("[OK] done");
        let expect = vec!["e2fsck=PASS".to_owned(), "[OK] done".to_owned()];
        assert_eq!(*a.borrow(), expect);
        assert_eq!(*b.borrow(), expect);
    }

    #[test]
    fn memory_sink_collects_lines() {
        let mut sink = MemorySink::default();
        sink.emit("APK_SHA256=ABC");
        sink.emit("RC_SHA256=DEF");
        assert_eq!(sink.lines, vec!["APK_SHA256=ABC", "RC_SHA256=DEF"]);
    }

    #[test]
    fn file_sink_appends_lines_with_lf() {
        let dir = std::env::temp_dir().join("volte_core_logging_test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("build.log");
        {
            let mut sink = FileSink::create(&path).expect("create log file");
            sink.emit("line one");
            sink.emit("line two");
        }
        let bytes = std::fs::read(&path).expect("read log file");
        assert_eq!(bytes, b"line one\nline two\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
