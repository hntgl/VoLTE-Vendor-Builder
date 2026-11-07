//! AVB footer inspection, partition fitting and hashtree rebuild.
//!
//! Ports `_parse_avbf` (`hd_vendor_build.py:529`), `avb_info_fast` (`:554`),
//! `fit_avb_part` (`:608`), `avb_module` (`:1256`) and `rebuild_hashtree`
//! (`:1271`). `avbtool.py` is executed as a subprocess (decision 8.2/1); the
//! frozen parameters are exactly the ones the Python call passes.

use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{AVB_MAGIC, BLOCK};
use crate::error::{Error, Result};
use crate::logging::{decode, Logger};
use crate::slot::force_slot_size;

const DIGEST_LEN: u64 = 32;
/// `64 * 1024 + 4096` from `fit_avb_part`.
const META_LEN: u64 = 64 * 1024 + 4 * 1024;
const FOOTER_LEN: usize = 64;
const SCAN_LEN: u64 = 32 * 1024 * 1024;

/// Parsed `AvbFooter` (`_parse_avbf`, `:529`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvbFooter {
    pub footer_major: u32,
    pub footer_minor: u32,
    pub original_image_size: u64,
    pub partition_size: u64,
    pub vbmeta_offset: u64,
    pub vbmeta_size: u64,
}

/// Result of `avb_info_fast` (`:554`). Defaults are the Python dict defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvbInfo {
    pub has_footer: bool,
    pub partition_name: String,
    pub hash_algorithm: String,
    pub salt: String,
    pub fec_roots: u32,
    pub image_size: u64,
    pub partition_size: u64,
    pub original_image_size: u64,
    pub vbmeta_offset: u64,
    pub vbmeta_size: u64,
    pub footer_major: u32,
    pub footer_minor: u32,
    pub props: Vec<(String, String)>,
}

impl AvbInfo {
    pub fn empty(image_size: u64) -> Self {
        AvbInfo {
            has_footer: false,
            partition_name: "vendor".to_string(),
            hash_algorithm: "sha256".to_string(),
            salt: String::new(),
            fec_roots: 0,
            image_size,
            partition_size: image_size,
            original_image_size: 0,
            vbmeta_offset: 0,
            vbmeta_size: 0,
            footer_major: 0,
            footer_minor: 0,
            props: Vec::new(),
        }
    }
}

/// `_parse_avbf`: the 36-byte big-endian footer prefix, with the same size
/// constraints. `footer_end` is where the footer is assumed to end.
pub fn parse_footer(blob: &[u8], footer_end: u64) -> Option<AvbFooter> {
    if blob.len() < FOOTER_LEN || blob[..4] != AVB_MAGIC {
        return None;
    }
    // struct.unpack("!4s2LQQQ", blob[:36])
    let major = u32::from_be_bytes([blob[4], blob[5], blob[6], blob[7]]);
    let minor = u32::from_be_bytes([blob[8], blob[9], blob[10], blob[11]]);
    let mut q = [0u8; 8];
    let orig = {
        q.copy_from_slice(&blob[12..20]);
        u64::from_be_bytes(q)
    };
    let voff = {
        q.copy_from_slice(&blob[20..28]);
        u64::from_be_bytes(q)
    };
    let vsz = {
        q.copy_from_slice(&blob[28..36]);
        u64::from_be_bytes(q)
    };
    if orig < 1024 * 1024 || orig >= footer_end {
        return None;
    }
    if voff == 0 || voff >= footer_end {
        return None;
    }
    Some(AvbFooter {
        footer_major: major,
        footer_minor: minor,
        original_image_size: orig,
        partition_size: footer_end,
        vbmeta_offset: voff,
        vbmeta_size: vsz,
    })
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .rev()
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn fill_from_vbmeta(path: &Path, footer: &AvbFooter) -> Result<AvbInfo> {
    let mut info = AvbInfo::empty(std::fs::metadata(path)?.len());
    info.has_footer = true;
    info.footer_major = footer.footer_major;
    info.footer_minor = footer.footer_minor;
    info.original_image_size = footer.original_image_size;
    info.partition_size = footer.partition_size;
    info.vbmeta_offset = footer.vbmeta_offset;
    info.vbmeta_size = footer.vbmeta_size;

    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(footer.vbmeta_offset))?;
    let take = footer.vbmeta_size.min(65_536) as usize;
    let mut vb = vec![0u8; take];
    let n = f.read(&mut vb)?;
    vb.truncate(n);
    for alg in ["sha256", "sha512", "sha1"] {
        if find_last(&vb, alg.as_bytes()).is_some() {
            info.hash_algorithm = alg.to_string();
            break;
        }
    }
    if find_last(&vb, b"vendor").is_some() {
        info.partition_name = "vendor".to_string();
    }
    Ok(info)
}

/// `avb_info_fast`: last 64 bytes first, then a backward scan of the final
/// 32 MiB for a footer displaced by padding.
pub fn avb_info_fast(path: &Path) -> Result<AvbInfo> {
    let size = std::fs::metadata(path)?.len();
    if size < FOOTER_LEN as u64 {
        return Ok(AvbInfo::empty(size));
    }
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(size - FOOTER_LEN as u64))?;
    let mut blob = [0u8; FOOTER_LEN];
    f.read_exact(&mut blob)?;
    if let Some(footer) = parse_footer(&blob, size) {
        return fill_from_vbmeta(path, &footer);
    }

    let scan = size.min(SCAN_LEN);
    f.seek(SeekFrom::Start(size - scan))?;
    let mut chunk = vec![0u8; scan as usize];
    f.read_exact(&mut chunk)?;
    let mut end = chunk.len();
    while let Some(pos) = find_last(&chunk[..end], &AVB_MAGIC) {
        if pos + FOOTER_LEN <= chunk.len() {
            let footer_end = size - scan + pos as u64 + FOOTER_LEN as u64;
            if let Some(footer) = parse_footer(&chunk[pos..pos + FOOTER_LEN], footer_end) {
                return fill_from_vbmeta(path, &footer);
            }
        }
        if pos == 0 {
            break;
        }
        end = pos;
    }
    Ok(AvbInfo::empty(size))
}

/// `tree_size` inside `fit_avb_part`: total hashtree bytes for a partition.
pub fn tree_size(mut size: u64) -> u64 {
    let mut total = 0u64;
    while size > BLOCK {
        let num_blocks = size.div_ceil(BLOCK);
        let level = (num_blocks * DIGEST_LEN).div_ceil(BLOCK) * BLOCK;
        total += level;
        size = level;
    }
    total
}

/// `fit_avb_part` (`:608`): smallest MiB-aligned partition that holds the
/// image plus hashtree, vbmeta and footer; 48 rounds then a 64 MiB fallback.
pub fn fit_avb_part(img_sz: u64, hint: u64) -> u64 {
    let mut need = hint.max(img_sz);
    for _ in 0..48 {
        need = need.div_ceil(1048576) * 1048576;
        if !need.is_multiple_of(BLOCK) {
            need = need.div_ceil(BLOCK) * BLOCK;
        }
        let max_img = need.saturating_sub(tree_size(need)).saturating_sub(META_LEN);
        if max_img >= img_sz {
            return need;
        }
        need = img_sz + tree_size(need) + META_LEN + 8 * 1048576;
    }
    (img_sz + 64 * 1048576).div_ceil(1048576) * 1048576
}

/// 32 random bytes as lower-case hex (`os.urandom(32).hex()`). The bytes come
/// from the OS-seeded `RandomState` keys; this salt only needs to be fresh per
/// build, not a cryptographic secret.
pub fn random_salt_hex() -> String {
    let state = RandomState::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut out = [0u8; 32];
    for (i, slot) in out.chunks_mut(8).enumerate() {
        let mut h = state.build_hasher();
        h.write_u128(now);
        h.write_usize(i);
        slot.copy_from_slice(&h.finish().to_be_bytes());
    }
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The frozen parameter set of the Python `add_hashtree_footer` call (`:1306`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashtreeRequest {
    pub image: PathBuf,
    pub partition_size: u64,
    pub partition_name: String,
    pub hash_algorithm: String,
    pub salt: String,
    pub props: Vec<(String, String)>,
}

impl HashtreeRequest {
    /// CLI form of the same arguments, for `avbtool.py add_hashtree_footer`.
    pub fn cli_args(&self) -> Vec<String> {
        let mut args = vec![
            "add_hashtree_footer".to_string(),
            "--image".to_string(),
            self.image.to_string_lossy().into_owned(),
            "--partition_size".to_string(),
            self.partition_size.to_string(),
            "--partition_name".to_string(),
            self.partition_name.clone(),
            "--hash_algorithm".to_string(),
            self.hash_algorithm.clone(),
            "--block_size".to_string(),
            "4096".to_string(),
            "--salt".to_string(),
            self.salt.clone(),
            "--algorithm".to_string(),
            "NONE".to_string(),
            "--rollback_index".to_string(),
            "0".to_string(),
            "--rollback_index_location".to_string(),
            "0".to_string(),
            "--flags".to_string(),
            "0".to_string(),
            "--do_not_generate_fec".to_string(),
            "--fec_num_roots".to_string(),
            "2".to_string(),
        ];
        for (k, v) in &self.props {
            args.push("--prop".to_string());
            args.push(format!("{k}:{v}"));
        }
        args
    }
}

/// Backend that appends the hashtree/footer.
pub trait Avb {
    fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()>;
}

/// `avb_module`: the embedded `avbtool.py`, run through the embedded Python.
#[derive(Debug)]
pub struct PythonAvbtool {
    python: PathBuf,
    avbtool: PathBuf,
}

impl PythonAvbtool {
    pub fn new(python: PathBuf, avbtool: PathBuf) -> Result<Self> {
        if !avbtool.is_file() {
            return Err(Error::missing("tool", &avbtool));
        }
        Ok(PythonAvbtool { python, avbtool })
    }
}

impl Avb for PythonAvbtool {
    fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
        let mut cmd = Command::new(&self.python);
        cmd.arg(&self.avbtool).args(req.cli_args());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let out = cmd
            .output()
            .map_err(|e| Error::build(format!("cannot run avbtool.py: {e}")))?;
        if !out.status.success() {
            let text = decode(&out.stderr);
            let text = text.trim();
            let text: String = text.chars().take(240).collect();
            return Err(Error::build(format!(
                "avbtool.py failed ({}): {text}",
                out.status.code().map_or("signal".to_string(), |c| c.to_string())
            )));
        }
        Ok(())
    }
}

/// `rebuild_hashtree` (`:1271`).
///
/// `Ok(Some(info))` means a footer was rebuilt, verified against the pre-call
/// vbmeta region, and the image was padded to the locked slot when needed;
/// `Ok(None)` means the hashtree did not fit and the original footer/EXT4 tail
/// must be kept — Python returns `False` in that case and the pipeline restores
/// its pre-AVB snapshot. `Err` means the backend failed or left the old footer
/// untouched; it is never reported as success.
pub fn rebuild_hashtree(
    image: &Path,
    info: &AvbInfo,
    part_size: u64,
    backend: &mut dyn Avb,
    log: &mut Logger,
) -> Result<Option<AvbInfo>> {
    let lock = info.partition_size;
    let mut part_size = part_size;
    let img_sz = std::fs::metadata(image)?.len();
    if lock > 0 {
        part_size = lock;
    } else if !info.has_footer || part_size <= img_sz {
        part_size = fit_avb_part(img_sz, part_size);
        if !info.has_footer {
            log.log("[AVB] No original footer - creating a new hashtree (padded).");
        }
    }

    let salt = if info.salt.is_empty() {
        random_salt_hex()
    } else {
        info.salt.clone()
    };
    let req = HashtreeRequest {
        image: image.to_path_buf(),
        partition_size: part_size,
        partition_name: if info.partition_name.is_empty() {
            "vendor".to_string()
        } else {
            info.partition_name.clone()
        },
        hash_algorithm: if info.hash_algorithm.is_empty() {
            "sha256".to_string()
        } else {
            info.hash_algorithm.clone()
        },
        salt,
        props: info.props.clone(),
    };

    log.log("[STATUS] Erasing old AVB footer...");
    log.log("[STATUS] Rebuilding AVB hashtree...");
    if lock > 0 {
        log.log(format!(
            "[AVB] locking partition_size={lock} (EDL slot, no growth)."
        ));
    }

    // Python's `for attempt in range(5)` retries nothing: every path that does
    // not return a footer breaks out of the loop at the end of the first pass.
    log.log(format!(
        "[AVB] add_hashtree partition_size={} (attempt 1)",
        req.partition_size
    ));
    let old_vbmeta = if info.has_footer && info.vbmeta_size > 0 {
        Some(vbmeta_digest(image, info.vbmeta_offset, info.vbmeta_size)?)
    } else {
        None
    };
    backend.add_hashtree_footer(&req)?;

    let after = avb_info_fast(image)?;
    if after.has_footer {
        if let Some(before) = old_vbmeta {
            let unchanged = after.vbmeta_offset == info.vbmeta_offset
                && after.vbmeta_size == info.vbmeta_size
                && vbmeta_digest(image, after.vbmeta_offset, after.vbmeta_size)? == before;
            if unchanged {
                return Err(Error::build(
                    "AVB backend returned Ok but the vbmeta region is unchanged - \
                     the old footer was kept, so the rebuild is not verified",
                ));
            }
        }
        let cur = std::fs::metadata(image)?.len();
        if lock > 0 && cur > lock {
            log.log(format!(
                "[WARN] Hashtree grew to {cur} > slot {lock} - not truncating (would bootloop)."
            ));
            return Ok(None);
        }
        if lock > 0 && cur < lock {
            force_slot_size(image, lock, false, "AVB-PAD", log)?;
        }
        log.log("[OK] Old AVB footer replaced.");
        log.log("[AVB] Rebuild hashtree completed.");
        for line in crate::report::avb_lines(&after) {
            log.log(line);
        }
        return Ok(Some(after));
    }
    log.log("[WARN] Hashtree does not fit the slot - keeping the original EXT4 overlay.");
    log.log("[WARN] Could not create a real AVB footer - keeping the original footer (avoid bootloop).");
    Ok(None)
}

/// SHA-256 of a vbmeta region, used to prove a rebuild actually replaced the
/// old footer. A re-written footer with a fresh salt always changes these bytes;
/// an untouched region means the backend did not rebuild anything.
fn vbmeta_digest(image: &Path, offset: u64, size: u64) -> Result<String> {
    let mut f = File::open(image)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut hasher = crate::sha256::Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut left = size;
    while left > 0 {
        let n = (left as usize).min(buf.len());
        f.read_exact(&mut buf[..n])?;
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(crate::sha256::hex_upper(&hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::PathBuf;
    use std::rc::Rc;

    struct SharedSink(Rc<RefCell<Vec<String>>>);

    impl crate::logging::Sink for SharedSink {
        fn emit(&mut self, line: &str) {
            self.0.borrow_mut().push(line.to_owned());
        }
    }

    fn catching_logger() -> (Logger, Rc<RefCell<Vec<String>>>) {
        let lines = Rc::new(RefCell::new(Vec::new()));
        (Logger::new().with_sink(SharedSink(lines.clone())), lines)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn footer_bytes(major: u32, minor: u32, orig: u64, voff: u64, vsz: u64) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[..4].copy_from_slice(&AVB_MAGIC);
        b[4..8].copy_from_slice(&major.to_be_bytes());
        b[8..12].copy_from_slice(&minor.to_be_bytes());
        b[12..20].copy_from_slice(&orig.to_be_bytes());
        b[20..28].copy_from_slice(&voff.to_be_bytes());
        b[28..36].copy_from_slice(&vsz.to_be_bytes());
        b
    }

    fn image_with_footer(dir: &Path, padding: u64) -> PathBuf {
        let path = dir.join("vendor.img");
        let size: u64 = 2 * 1024 * 1024;
        let mut data = vec![0u8; size as usize];
        data[1024 * 1024..1024 * 1024 + 13].copy_from_slice(b"sha512\0vendor");
        let footer_at = size - 64 - padding;
        data[footer_at as usize..footer_at as usize + 64]
            .copy_from_slice(&footer_bytes(1, 0, 1024 * 1024, 1024 * 1024, 4096));
        std::fs::write(&path, &data).unwrap();
        path
    }

    #[test]
    fn parse_accepts_valid_footer() {
        let b = footer_bytes(1, 2, 2 * 1024 * 1024, 1024 * 1024, 8192);
        let f = parse_footer(&b, 4 * 1024 * 1024).expect("valid footer");
        assert_eq!(f.footer_major, 1);
        assert_eq!(f.footer_minor, 2);
        assert_eq!(f.original_image_size, 2 * 1024 * 1024);
        assert_eq!(f.vbmeta_offset, 1024 * 1024);
        assert_eq!(f.vbmeta_size, 8192);
        assert_eq!(f.partition_size, 4 * 1024 * 1024);
    }

    #[test]
    fn parse_rejects_magic_size_and_offsets() {
        let mut b = footer_bytes(1, 0, 2 * 1024 * 1024, 1024 * 1024, 4096);
        b[0] = b'X';
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none());

        let b = footer_bytes(1, 0, 512 * 1024, 1024 * 1024, 4096);
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none(), "orig < 1 MiB");

        let b = footer_bytes(1, 0, 5 * 1024 * 1024, 1024 * 1024, 4096);
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none(), "orig >= footer_end");

        let b = footer_bytes(1, 0, 2 * 1024 * 1024, 0, 4096);
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none(), "voff == 0");

        let b = footer_bytes(1, 0, 2 * 1024 * 1024, 4 * 1024 * 1024, 4096);
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none(), "voff >= footer_end");

        assert!(parse_footer(&[0u8; 10], 4 * 1024 * 1024).is_none(), "short blob");
    }

    #[test]
    fn info_reads_footer_at_end_and_detects_sha512() {
        let dir = tmp("volte_avb_info_end");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        assert_eq!(info.hash_algorithm, "sha512");
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.image_size, 2 * 1024 * 1024);
        assert_eq!(info.partition_size, 2 * 1024 * 1024);
        assert_eq!(info.original_image_size, 1024 * 1024);
        assert_eq!(info.salt, "");
        assert!(info.props.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn info_scans_back_past_padding() {
        let dir = tmp("volte_avb_info_scan");
        let p = image_with_footer(&dir, 8192);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        assert_eq!(info.partition_size, 2 * 1024 * 1024 - 8192);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn info_defaults_without_footer() {
        let dir = tmp("volte_avb_info_none");
        let p = dir.join("plain.img");
        std::fs::write(&p, vec![0u8; 4096]).unwrap();
        let info = avb_info_fast(&p).unwrap();
        assert!(!info.has_footer);
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.partition_size, 4096);
        assert_eq!(info.image_size, 4096);
        assert_eq!(info.original_image_size, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_partition_size_properties() {
        for img in [1u64, 4096, 100_000, 5_000_000, 100 * 1024 * 1024, 1_200_000_000] {
            let part = fit_avb_part(img, 0);
            assert_eq!(part % 1048576, 0, "MiB aligned for {img}");
            assert!(part >= img, "holds the image for {img}");
            assert!(
                part - tree_size(part) - META_LEN >= img,
                "leaves room for tree+meta for {img}"
            );
        }
        let hint = fit_avb_part(1000, 64 * 1024 * 1024);
        assert_eq!(hint, 64 * 1024 * 1024, "hint wins when already large enough");
    }

    #[test]
    fn tree_size_uses_32_byte_digests() {
        assert_eq!(tree_size(BLOCK), 0);
        assert_eq!(tree_size(2 * BLOCK), BLOCK);
        assert_eq!(tree_size(1024 * BLOCK), 36864);
    }

    #[test]
    fn cli_args_carry_the_frozen_parameters() {
        let req = HashtreeRequest {
            image: PathBuf::from(r"E:\work\vendor.img"),
            partition_size: 12345678,
            partition_name: "vendor".to_string(),
            hash_algorithm: "sha256".to_string(),
            salt: "aabb".to_string(),
            props: vec![("k".to_string(), "v".to_string())],
        };
        let args = req.cli_args();
        let joined = args.join(" ");
        assert!(args[0] == "add_hashtree_footer");
        assert!(joined.contains("--algorithm NONE"));
        assert!(joined.contains("--do_not_generate_fec"));
        assert!(joined.contains("--fec_num_roots 2"));
        assert!(joined.contains("--block_size 4096"));
        assert!(joined.contains("--hash_algorithm sha256"));
        assert!(joined.contains("--salt aabb"));
        assert!(joined.contains("--rollback_index 0"));
        assert!(joined.contains("--flags 0"));
        assert!(joined.contains("--prop k:v"));
        assert!(!joined.contains("--key"));
        assert!(!joined.contains("--generate_fec "));
    }

    #[test]
    fn random_salt_is_64_lower_hex_chars_and_changes() {
        let a = random_salt_hex();
        let b = random_salt_hex();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn missing_avbtool_names_the_exact_path() {
        let dir = tmp("volte_avb_missing_tool");
        let tool = dir.join("tool").join("avbtool.py");
        let err = PythonAvbtool::new(PathBuf::from("python.exe"), tool.clone()).unwrap_err();
        assert_eq!(err.kind(), crate::error::ErrorKind::Missing);
        assert!(err.message().contains(&tool.display().to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct NoopBackend;
    impl Avb for NoopBackend {
        fn add_hashtree_footer(&mut self, _req: &HashtreeRequest) -> Result<()> {
            Err(Error::build("simulated avbtool failure"))
        }
    }

    struct FooterWriter {
        grow: u64,
    }

    impl Avb for FooterWriter {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            let size = req.partition_size;
            let mut f = std::fs::OpenOptions::new().read(true).write(true).open(&req.image)?;
            let footer = footer_bytes(1, 0, 1024 * 1024, 1024 * 1024, 4096);
            if self.grow == 0 {
                f.seek(SeekFrom::Start(size - 64))?;
                f.write_all(&footer)?;
                f.set_len(size)?;
            } else {
                f.seek(SeekFrom::End(0))?;
                f.write_all(&vec![0u8; (self.grow - 64) as usize])?;
                f.write_all(&footer)?;
            }
            Ok(())
        }
    }

    fn plain_image(dir: &Path, size: u64) -> PathBuf {
        let p = dir.join("vendor.img");
        std::fs::write(&p, vec![0u8; size as usize]).unwrap();
        p
    }

    #[test]
    fn a_failing_backend_is_an_error_and_keeps_the_image_untouched() {
        let dir = tmp("volte_avb_rebuild_fail");
        let p = plain_image(&dir, 2 * 1024 * 1024);
        let before = std::fs::read(&p).unwrap();
        let info = avb_info_fast(&p).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = rebuild_hashtree(&p, &info, 0, &mut NoopBackend, &mut log).unwrap_err();
        assert!(err.message().contains("simulated avbtool failure"), "{err}");
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 2 * 1024 * 1024);
        assert_eq!(std::fs::read(&p).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_backend_over_an_old_footer_is_never_reported_as_success() {
        let dir = tmp("volte_avb_rebuild_fail_old_footer");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        let (mut log, lines) = catching_logger();
        rebuild_hashtree(&p, &info, 0, &mut NoopBackend, &mut log).unwrap_err();
        let text = lines.borrow().join("\n");
        assert!(!text.contains("Rebuild hashtree completed."), "{text}");
        assert!(!text.contains("[OK] Old AVB footer"), "{text}");
        assert!(!text.contains("[OK] AVB"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct RewriteFooter {
        change_vbmeta: bool,
    }

    impl Avb for RewriteFooter {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            let size = req.partition_size;
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&req.image)?;
            if self.change_vbmeta {
                f.seek(SeekFrom::Start(1024 * 1024 + 8))?;
                f.write_all(b"new")?;
            }
            f.seek(SeekFrom::Start(size - 64))?;
            f.write_all(&footer_bytes(1, 0, 1024 * 1024, 1024 * 1024, 4096))?;
            f.set_len(size)?;
            Ok(())
        }
    }

    #[test]
    fn a_backend_that_returns_ok_without_rebuilding_is_an_error() {
        let dir = tmp("volte_avb_rebuild_unchanged");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = rebuild_hashtree(
            &p,
            &info,
            0,
            &mut RewriteFooter { change_vbmeta: false },
            &mut log,
        )
        .unwrap_err();
        assert!(err.message().contains("unchanged"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verified_rebuild_reports_the_new_footer() {
        let dir = tmp("volte_avb_rebuild_verified");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        let (mut log, lines) = catching_logger();
        let out = rebuild_hashtree(
            &p,
            &info,
            0,
            &mut RewriteFooter { change_vbmeta: true },
            &mut log,
        )
        .unwrap()
        .expect("footer rebuilt");
        assert!(out.has_footer);
        let text = lines.borrow().join("\n");
        assert!(text.contains("[OK] Old AVB footer replaced."), "{text}");
        assert!(text.contains("[AVB] Rebuild hashtree completed."), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebuild_detects_footer_and_keeps_the_locked_size() {
        let dir = tmp("volte_avb_rebuild_ok");
        let p = plain_image(&dir, 2 * 1024 * 1024);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_size, 2 * 1024 * 1024);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let out = rebuild_hashtree(&p, &info, 0, &mut FooterWriter { grow: 0 }, &mut log)
            .unwrap()
            .expect("footer rebuilt");
        assert!(out.has_footer);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 2 * 1024 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebuild_refuses_to_shrink_a_grown_image() {
        let dir = tmp("volte_avb_rebuild_grow");
        let p = plain_image(&dir, 2 * 1024 * 1024);
        let info = avb_info_fast(&p).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let out = rebuild_hashtree(&p, &info, 0, &mut FooterWriter { grow: 4096 }, &mut log).unwrap();
        assert!(out.is_none());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 2 * 1024 * 1024 + 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
