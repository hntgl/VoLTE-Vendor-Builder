//! AVB footer inspection, partition fitting and hashtree rebuild.
//!
//! Ports `_parse_avbf` (`hd_vendor_build.py:529`), `avb_info_fast` (`:554`),
//! `fit_avb_part` (`:608`), `avb_module` (`:1256`) and `rebuild_hashtree`
//! (`:1271`). `avbtool.py` is executed as a subprocess (decision 8.2/1); the
//! frozen parameters are exactly the ones the Python call passes.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::config::{AVB_MAGIC, BLOCK};
use crate::error::{Error, Result};
use crate::ext4_tools::capture;
use crate::logging::Logger;
use crate::slot::force_slot_size;

const DIGEST_LEN: u64 = 32;
/// `64 * 1024 + 4096` from `fit_avb_part`.
const META_LEN: u64 = 64 * 1024 + 4 * 1024;
const FOOTER_LEN: usize = 64;
const SCAN_LEN: u64 = 32 * 1024 * 1024;
/// `avbtool.py` rewrites a multi-GB image, so it gets a generous but finite
/// deadline instead of blocking the build forever.
const AVB_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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

/// Integrity scheme of the descriptor that named the image partition. `None`
/// means no hashtree/hash descriptor was found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorKind {
    #[default]
    None,
    /// Tag 2: a whole-partition hash.
    HashOnly,
    /// Tag 1: a dm-verity hashtree.
    Hashtree,
}

/// Result of `avb_info_fast` (`:554`). Defaults are the Python dict defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvbInfo {
    pub has_footer: bool,
    /// True only when the footer parsed from the last 64 bytes of the file.
    /// A footer found by the backward scan means the image was padded or
    /// modified after signing, so the pipeline refuses to build from it.
    pub footer_at_exact_tail: bool,
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
    /// Descriptor tags this build does not know how to read. Recorded so the
    /// pipeline can warn instead of pretending the vbmeta was fully parsed.
    pub unsupported_descriptor_tags: Vec<u64>,
    /// Tag 4 references another partition. The rebuild cannot reproduce it.
    pub has_chain_descriptor: bool,
    /// Which descriptor (Tag 1 or Tag 2) named the image partition.
    pub descriptor_kind: DescriptorKind,
}

impl AvbInfo {
    pub fn empty(image_size: u64) -> Self {
        AvbInfo {
            has_footer: false,
            footer_at_exact_tail: false,
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
            unsupported_descriptor_tags: Vec::new(),
            has_chain_descriptor: false,
            descriptor_kind: DescriptorKind::None,
        }
    }
}

/// `_parse_avbf`: the 36-byte big-endian footer prefix, with the same size
/// constraints. `footer_end` is where the footer is assumed to end.
pub(crate) fn parse_footer(blob: &[u8], footer_end: u64) -> Option<AvbFooter> {
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

    if footer.vbmeta_size == 0 || footer.vbmeta_size > MAX_VBMETA {
        return Err(Error::build(format!(
            "AVB footer declares a {} byte vbmeta (allowed 1..={MAX_VBMETA})",
            footer.vbmeta_size
        )));
    }
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(footer.vbmeta_offset))?;
    let mut vb = vec![0u8; footer.vbmeta_size as usize];
    f.read_exact(&mut vb)?;

    let meta = parse_vbmeta(&vb).map_err(|e| {
        Error::build(format!(
            "AVB vbmeta at offset {}: {e}",
            footer.vbmeta_offset
        ))
    })?;
    if let Some(name) = meta.partition_name {
        info.partition_name = name;
    }
    if let Some(alg) = meta.hash_algorithm {
        info.hash_algorithm = alg;
    }
    if let Some(salt) = meta.salt {
        info.salt = salt;
    }
    if let Some(roots) = meta.fec_roots {
        info.fec_roots = roots;
    }
    info.props = meta.props;
    info.unsupported_descriptor_tags = meta.unsupported_tags;
    info.has_chain_descriptor = meta.has_chain_descriptor;
    info.descriptor_kind = meta.descriptor_kind;
    Ok(info)
}

/// The largest vbmeta `avbtool.py` writes (`MAX_VBMETA_SIZE`).
const MAX_VBMETA: u64 = 64 * 1024;
/// `AvbVBMetaImageHeader.SIZE`.
const VBMETA_HEADER_LEN: usize = 256;
/// `AvbHashtreeDescriptor.SIZE`.
const HASHTREE_DESCRIPTOR_LEN: usize = 180;
/// `AvbPropertyDescriptor.SIZE`.
const PROPERTY_DESCRIPTOR_LEN: usize = 32;
/// `AvbHashDescriptor.SIZE` (16-byte header + 116 byte fixed body).
const HASH_DESCRIPTOR_LEN: usize = 132;
/// `AvbChainPartitionDescriptor.SIZE` (16-byte header + 76 byte fixed body).
const CHAIN_PARTITION_DESCRIPTOR_LEN: usize = 92;

/// Fields the rebuild needs from the vbmeta descriptors. `None` means the
/// image has no descriptor that carries the field, not that the field was
/// guessed. Only hashtree/hash descriptors (tags 1/2) name the image being
/// rebuilt: the first one picks its partition and later descriptors of that
/// same partition fill only the fields still `None`. Chain descriptors (tag 4)
/// reference another partition and never select the image partition.
#[derive(Debug, Default, PartialEq, Eq)]
struct VbmetaMeta {
    partition_name: Option<String>,
    hash_algorithm: Option<String>,
    salt: Option<String>,
    fec_roots: Option<u32>,
    props: Vec<(String, String)>,
    unsupported_tags: Vec<u64>,
    has_chain_descriptor: bool,
    descriptor_kind: DescriptorKind,
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn be_u64(bytes: &[u8]) -> u64 {
    let mut q = [0u8; 8];
    q.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(q)
}

fn round8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn fixed_string(field: &[u8], what: &str) -> std::result::Result<String, String> {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end])
        .map(str::to_owned)
        .map_err(|_| format!("{what} is not UTF-8"))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Selects the first image descriptor's partition name and reports whether
/// this descriptor belongs to the selected partition. Only tags 1/2 may call
/// this: their `partition_name` is the image they describe. A descriptor from
/// any other partition may not contribute metadata.
fn select_image_partition(meta: &mut VbmetaMeta, name: &str) -> bool {
    if let Some(chosen) = meta.partition_name.as_deref() {
        return chosen == name;
    }
    meta.partition_name = Some(name.to_owned());
    true
}

/// First non-empty algorithm and first salt win. Only descriptors of the
/// selected partition reach this helper.
fn merge_hash_fields(meta: &mut VbmetaMeta, algorithm: String, salt: String) {
    if meta.hash_algorithm.is_none() && !algorithm.is_empty() {
        meta.hash_algorithm = Some(algorithm);
    }
    if meta.salt.is_none() {
        meta.salt = Some(salt);
    }
}

/// Parses `AvbVBMetaImageHeader` plus its descriptor block. Tags are read from
/// the descriptor header, never searched for as text; tags this build does not
/// understand are recorded in `unsupported_tags` rather than dropped silently.
/// Any length that does not match the descriptor's own size formula is an
/// error, so a truncated image can never be read as metadata.
fn parse_vbmeta(vb: &[u8]) -> std::result::Result<VbmetaMeta, String> {
    if vb.len() < VBMETA_HEADER_LEN {
        return Err(format!(
            "{} bytes is shorter than the {VBMETA_HEADER_LEN}-byte header",
            vb.len()
        ));
    }
    if &vb[..4] != b"AVB0" {
        return Err("bad vbmeta magic (expected AVB0)".to_owned());
    }
    let auth = be_u64(&vb[12..20]);
    let aux = be_u64(&vb[20..28]);
    let desc_offset = be_u64(&vb[96..104]);
    let desc_size = be_u64(&vb[104..112]);
    let aux_end = (VBMETA_HEADER_LEN as u64)
        .checked_add(auth)
        .and_then(|n| n.checked_add(aux))
        .ok_or("auth/aux block sizes overflow")?;
    if aux_end > vb.len() as u64 {
        return Err(format!(
            "auth+aux blocks end at {aux_end}, outside the {}-byte vbmeta",
            vb.len()
        ));
    }
    let start = VBMETA_HEADER_LEN as u64 + auth + desc_offset;
    let end = start
        .checked_add(desc_size)
        .ok_or("descriptor size overflows")?;
    if end > aux_end {
        return Err(format!(
            "descriptors end at {end}, past the auxiliary block end {aux_end}"
        ));
    }

    let mut meta = VbmetaMeta::default();
    let mut pos = start as usize;
    let end = end as usize;
    while pos < end {
        if pos + 16 > end {
            return Err(format!("truncated descriptor header at offset {pos}"));
        }
        let tag = be_u64(&vb[pos..pos + 8]);
        let nbf = be_u64(&vb[pos + 8..pos + 16]);
        if !nbf.is_multiple_of(8) {
            return Err(format!("descriptor at {pos} has unaligned size {nbf}"));
        }
        let total = 16usize
            .checked_add(nbf as usize)
            .ok_or("descriptor size overflows")?;
        let last = pos.checked_add(total).ok_or("descriptor end overflows")?;
        if last > end {
            return Err(format!(
                "descriptor at {pos} ends at {last}, past the descriptor block end {end}"
            ));
        }
        let body = &vb[pos..last];
        match tag {
            0 => {
                if let Some((key, value)) = parse_property(body, pos)? {
                    meta.props.push((key, value));
                }
            }
            1 => parse_hashtree(body, pos, &mut meta)?,
            2 => parse_hash_descriptor(body, pos, &mut meta)?,
            4 => parse_chain_partition_descriptor(body, pos, &mut meta)?,
            _ => meta.unsupported_tags.push(tag),
        }
        pos = last;
    }
    Ok(meta)
}

fn parse_hashtree(
    body: &[u8],
    pos: usize,
    meta: &mut VbmetaMeta,
) -> std::result::Result<(), String> {
    if body.len() < HASHTREE_DESCRIPTOR_LEN {
        return Err(format!(
            "hashtree descriptor at {pos} is shorter than {HASHTREE_DESCRIPTOR_LEN} bytes"
        ));
    }
    let nbf = be_u64(&body[8..16]) as usize;
    let fec_roots = be_u32(&body[52..56]);
    let algorithm = fixed_string(&body[72..104], "hash algorithm")?;
    let name_len = be_u32(&body[104..108]) as usize;
    let salt_len = be_u32(&body[108..112]) as usize;
    let digest_len = be_u32(&body[112..116]) as usize;
    let dynamic = HASHTREE_DESCRIPTOR_LEN
        .checked_add(name_len)
        .and_then(|n| n.checked_add(salt_len))
        .and_then(|n| n.checked_add(digest_len))
        .ok_or("hashtree descriptor lengths overflow")?;
    if dynamic > body.len() {
        return Err(format!(
            "hashtree descriptor at {pos} declares {dynamic} bytes but carries {}",
            body.len()
        ));
    }
    let expected = round8(dynamic.saturating_sub(16));
    if nbf != expected {
        return Err(format!(
            "hashtree descriptor at {pos} declares {nbf} following bytes, expected {expected}"
        ));
    }
    let name_at = HASHTREE_DESCRIPTOR_LEN;
    let salt_at = name_at + name_len;
    let salt = &body[salt_at..salt_at + salt_len];
    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;

    if select_image_partition(meta, name) {
        merge_hash_fields(meta, algorithm, hex_lower(salt));
        if meta.fec_roots.is_none() {
            meta.fec_roots = Some(fec_roots);
        }
        meta.descriptor_kind = DescriptorKind::Hashtree;
    }
    Ok(())
}

/// Tag 2, `AvbHashDescriptor`: a whole-partition hash. `digest_len == 0` is a
/// persistent digest and therefore valid.
fn parse_hash_descriptor(
    body: &[u8],
    pos: usize,
    meta: &mut VbmetaMeta,
) -> std::result::Result<(), String> {
    if body.len() < HASH_DESCRIPTOR_LEN {
        return Err(format!(
            "hash descriptor at {pos} is shorter than {HASH_DESCRIPTOR_LEN} bytes"
        ));
    }
    let nbf = be_u64(&body[8..16]) as usize;
    let algorithm = fixed_string(&body[24..56], "hash algorithm")?;
    let name_len = be_u32(&body[56..60]) as usize;
    let salt_len = be_u32(&body[60..64]) as usize;
    let digest_len = be_u32(&body[64..68]) as usize;
    let dynamic = HASH_DESCRIPTOR_LEN
        .checked_add(name_len)
        .and_then(|n| n.checked_add(salt_len))
        .and_then(|n| n.checked_add(digest_len))
        .ok_or("hash descriptor lengths overflow")?;
    if dynamic > body.len() {
        return Err(format!(
            "hash descriptor at {pos} declares {dynamic} bytes but carries {}",
            body.len()
        ));
    }
    let expected = round8(dynamic.saturating_sub(16));
    if nbf != expected {
        return Err(format!(
            "hash descriptor at {pos} declares {nbf} following bytes, expected {expected}"
        ));
    }
    let name_at = HASH_DESCRIPTOR_LEN;
    let salt_at = name_at + name_len;
    let salt = &body[salt_at..salt_at + salt_len];
    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;

    if select_image_partition(meta, name) {
        merge_hash_fields(meta, algorithm, hex_lower(salt));
        if meta.descriptor_kind == DescriptorKind::None {
            meta.descriptor_kind = DescriptorKind::HashOnly;
        }
    }
    Ok(())
}

/// Tag 4, `AvbChainPartitionDescriptor`: names another partition signed by a
/// public key. It carries no hash algorithm, salt or FEC roots, and its
/// `partition_name` is not the image being rebuilt, so it selects nothing.
/// The flag records that a rebuild would drop this chain reference.
fn parse_chain_partition_descriptor(
    body: &[u8],
    pos: usize,
    meta: &mut VbmetaMeta,
) -> std::result::Result<(), String> {
    if body.len() < CHAIN_PARTITION_DESCRIPTOR_LEN {
        return Err(format!(
            "chain partition descriptor at {pos} is shorter than {CHAIN_PARTITION_DESCRIPTOR_LEN} bytes"
        ));
    }
    let nbf = be_u64(&body[8..16]) as usize;
    let name_len = be_u32(&body[20..24]) as usize;
    let public_key_len = be_u32(&body[24..28]) as usize;
    let dynamic = CHAIN_PARTITION_DESCRIPTOR_LEN
        .checked_add(name_len)
        .and_then(|n| n.checked_add(public_key_len))
        .ok_or("chain partition descriptor lengths overflow")?;
    if dynamic > body.len() {
        return Err(format!(
            "chain partition descriptor at {pos} declares {dynamic} bytes but carries {}",
            body.len()
        ));
    }
    let expected = round8(dynamic.saturating_sub(16));
    if nbf != expected {
        return Err(format!(
            "chain partition descriptor at {pos} declares {nbf} following bytes, expected {expected}"
        ));
    }
    let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
    let _name = std::str::from_utf8(&body[name_at..name_at + name_len])
        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
    meta.has_chain_descriptor = true;
    Ok(())
}

fn parse_property(
    body: &[u8],
    pos: usize,
) -> std::result::Result<Option<(String, String)>, String> {
    if body.len() < PROPERTY_DESCRIPTOR_LEN {
        return Err(format!(
            "property descriptor at {pos} is shorter than {PROPERTY_DESCRIPTOR_LEN} bytes"
        ));
    }
    let nbf = be_u64(&body[8..16]) as usize;
    let key_len = be_u64(&body[16..24]) as usize;
    let value_len = be_u64(&body[24..32]) as usize;
    let expected = round8(PROPERTY_DESCRIPTOR_LEN - 16 + key_len + 1 + value_len + 1);
    if nbf != expected {
        return Err(format!(
            "property descriptor at {pos} declares {nbf} following bytes, expected {expected}"
        ));
    }
    let key_at = PROPERTY_DESCRIPTOR_LEN;
    let value_at = key_at + key_len + 1;
    let value_end = value_at + value_len;
    if value_end + 1 > body.len() {
        return Err(format!("property descriptor at {pos} overruns its bytes"));
    }
    let key = std::str::from_utf8(&body[key_at..key_at + key_len])
        .map_err(|_| format!("property key at {pos} is not UTF-8"))?
        .to_owned();
    let value = std::str::from_utf8(&body[value_at..value_end])
        .map_err(|_| format!("property {key} at {pos} has a non-UTF-8 value"))?
        .to_owned();
    Ok(Some((key, value)))
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
        let mut info = fill_from_vbmeta(path, &footer)?;
        info.footer_at_exact_tail = true;
        return Ok(info);
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
pub(crate) fn tree_size(mut size: u64) -> u64 {
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
pub(crate) fn fit_avb_part(img_sz: u64, hint: u64) -> u64 {
    let mut need = hint.max(img_sz);
    for _ in 0..48 {
        need = need.div_ceil(1048576) * 1048576;
        if !need.is_multiple_of(BLOCK) {
            need = need.div_ceil(BLOCK) * BLOCK;
        }
        let max_img = need
            .saturating_sub(tree_size(need))
            .saturating_sub(META_LEN);
        if max_img >= img_sz {
            return need;
        }
        need = img_sz + tree_size(need) + META_LEN + 8 * 1048576;
    }
    (img_sz + 64 * 1048576).div_ceil(1048576) * 1048576
}

/// 32 bytes from the OS CSPRNG as lower-case hex (`os.urandom(32).hex()`).
/// A failing entropy source aborts the build instead of falling back to a
/// predictable salt.
pub(crate) fn random_salt_hex() -> Result<String> {
    let mut out = [0u8; 32];
    getrandom::fill(&mut out)
        .map_err(|e| Error::build(format!("cannot read the OS random source: {e}")))?;
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
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
    pub(crate) fn cli_args(&self) -> Vec<String> {
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
        let out = capture(&mut cmd, AVB_TIMEOUT)
            .map_err(|e| Error::build(format!("avbtool.py did not finish: {e}")))?;
        if out.code != 0 {
            let text = out.text();
            let text = text.trim();
            let text: String = text.chars().take(240).collect();
            return Err(Error::build(format!(
                "avbtool.py failed ({}): {text}",
                out.code
            )));
        }
        Ok(())
    }
}

/// `rebuild_hashtree` (`:1271`).
///
/// The bytes from `original_image_size` to the end of the file (old hashtree,
/// vbmeta, footer and padding) are read into memory first; the file itself is
/// not touched before the backend runs. `avbtool` rewrites that tail in place,
/// so a failed rebuild, or a tree that does not fit the locked slot, is rolled
/// back from the saved bytes. `Ok(Some(info))` means a footer was rebuilt and
/// the image was padded to the locked slot when needed; `Ok(None)` means the
/// hashtree grew past the slot and the tail was restored; `Err` means the
/// backend failed or left no valid footer, and the tail was restored first.
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

    // The caller (pipeline) only reaches here with a footer at the exact tail,
    // whose `original_image_size` is always below the file size.
    let boundary = info.original_image_size;
    if boundary > img_sz {
        return Err(Error::build(format!(
            "AVB footer says the original image is {boundary} bytes but the file is only {img_sz} bytes - refusing to rebuild from contradictory metadata"
        )));
    }
    let mut tail_bytes = Vec::new();
    {
        let mut f = File::open(image)?;
        f.seek(SeekFrom::Start(boundary))?;
        f.read_to_end(&mut tail_bytes)?;
    }

    let salt = if info.salt.is_empty() {
        random_salt_hex()?
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
    if let Err(e) = backend.add_hashtree_footer(&req) {
        return Err(restore_tail_or_report(image, boundary, &tail_bytes, e));
    }

    let after = match avb_info_fast(image) {
        Ok(after) => after,
        Err(e) => return Err(restore_tail_or_report(image, boundary, &tail_bytes, e)),
    };
    if !after.has_footer {
        let e = Error::build("AVB backend returned Ok but no AVB footer exists after the rebuild");
        return Err(restore_tail_or_report(image, boundary, &tail_bytes, e));
    }
    let cur = match std::fs::metadata(image) {
        Ok(m) => m.len(),
        Err(e) => {
            return Err(restore_tail_or_report(
                image,
                boundary,
                &tail_bytes,
                Error::from(e),
            ))
        }
    };
    if lock > 0 && cur > lock {
        log.log(format!(
            "[WARN] Hashtree grew to {cur} > slot {lock} - not truncating (would bootloop)."
        ));
        restore_tail(image, boundary, &tail_bytes)?;
        return Ok(None);
    }
    if lock > 0 && cur < lock {
        if let Err(e) = force_slot_size(image, lock, false, "AVB-PAD", log) {
            return Err(restore_tail_or_report(image, boundary, &tail_bytes, e));
        }
    }
    log.log("[OK] Old AVB footer replaced.");
    log.log("[AVB] Rebuild hashtree completed.");
    for line in crate::report::avb_lines(&after) {
        log.log(line);
    }
    Ok(Some(after))
}

/// Puts the saved tail back: `boundary` becomes the file size again and the
/// bytes that followed it are rewritten, so the image is byte-for-byte what it
/// was before the backend ran.
fn restore_tail(image: &Path, boundary: u64, tail_bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).open(image)?;
    f.set_len(boundary)?;
    if !tail_bytes.is_empty() {
        f.seek(SeekFrom::Start(boundary))?;
        f.write_all(tail_bytes)?;
        f.set_len(boundary + tail_bytes.len() as u64)?;
    }
    f.sync_all()?;
    Ok(())
}

/// Restores the tail and keeps the original error. When the restore itself
/// fails the combined message is still an `Err`, never a success.
fn restore_tail_or_report(image: &Path, boundary: u64, tail_bytes: &[u8], error: Error) -> Error {
    match restore_tail(image, boundary, tail_bytes) {
        Ok(()) => error,
        Err(restore) => Error::build(format!(
            "{} (and the image tail could not be restored: {})",
            error.message(),
            restore.message()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::PathBuf;
    use std::rc::Rc;

    /// 704-byte vbmeta written by the bundled `tool/avbtool.py` 1.3.0.
    /// `fixtures/avb/README.md` records how it was produced and what
    /// `avbtool.py info_image` reports for it.
    const GOLDEN_VBMETA: &[u8] = include_bytes!("../../fixtures/avb/vbmeta_vendor_golden.bin");
    const GOLDEN_VBMETA_SHA256: &str =
        "1C5338CD481F787AE627389681932A40BD38CF6F6CC284577819AD2197F1E81C";
    const GOLDEN_SALT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

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
        data[1024 * 1024..1024 * 1024 + GOLDEN_VBMETA.len()].copy_from_slice(GOLDEN_VBMETA);
        let footer_at = size - 64 - padding;
        data[footer_at as usize..footer_at as usize + 64].copy_from_slice(&footer_bytes(
            1,
            0,
            1024 * 1024,
            1024 * 1024,
            GOLDEN_VBMETA.len() as u64,
        ));
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
        assert!(
            parse_footer(&b, 4 * 1024 * 1024).is_none(),
            "orig >= footer_end"
        );

        let b = footer_bytes(1, 0, 2 * 1024 * 1024, 0, 4096);
        assert!(parse_footer(&b, 4 * 1024 * 1024).is_none(), "voff == 0");

        let b = footer_bytes(1, 0, 2 * 1024 * 1024, 4 * 1024 * 1024, 4096);
        assert!(
            parse_footer(&b, 4 * 1024 * 1024).is_none(),
            "voff >= footer_end"
        );

        assert!(
            parse_footer(&[0u8; 10], 4 * 1024 * 1024).is_none(),
            "short blob"
        );
    }

    #[test]
    fn the_golden_fixture_matches_its_reference_digest() {
        assert_eq!(GOLDEN_VBMETA.len(), 704);
        assert_eq!(
            crate::sha256::hex_upper(&crate::sha256::digest(GOLDEN_VBMETA)),
            GOLDEN_VBMETA_SHA256
        );
    }

    #[test]
    fn info_reads_the_golden_vbmeta_fields() {
        let dir = tmp("volte_avb_info_end");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        assert!(info.footer_at_exact_tail);
        assert_eq!(
            info.props,
            vec![
                ("com.android.build.vendor".to_owned(), "v2".to_owned()),
                (
                    "com.android.build.vendor.extra".to_owned(),
                    "hello".to_owned()
                ),
            ]
        );
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.salt, GOLDEN_SALT);
        assert_eq!(info.fec_roots, 0);
        assert_eq!(info.image_size, 2 * 1024 * 1024);
        assert_eq!(info.partition_size, 2 * 1024 * 1024);
        assert_eq!(info.original_image_size, 1024 * 1024);
        assert_eq!(info.vbmeta_offset, 1024 * 1024);
        assert_eq!(info.vbmeta_size, 704);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_or_malformed_vbmeta_is_an_error() {
        let dir = tmp("volte_avb_bad_vbmeta");
        let footer_at = 2 * 1024 * 1024 - 64;
        let set_vsz = |vsz: u64| {
            let p = image_with_footer(&dir, 0);
            let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.seek(SeekFrom::Start(footer_at + 28)).unwrap();
            f.write_all(&vsz.to_be_bytes()).unwrap();
            p
        };

        let p = image_with_footer(&dir, 0);
        let mut f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.seek(SeekFrom::Start(1024 * 1024)).unwrap();
        f.write_all(b"XXXX").unwrap();
        drop(f);
        let err = avb_info_fast(&p).unwrap_err();
        assert!(err.message().contains("bad vbmeta magic"), "{err}");

        for vsz in [0u64, 64 * 1024 + 1] {
            let err = avb_info_fast(&set_vsz(vsz)).unwrap_err();
            assert!(err.message().contains("byte vbmeta"), "{err}");
        }

        let err = avb_info_fast(&set_vsz(100)).unwrap_err();
        assert!(
            err.message().contains("shorter than the 256-byte header"),
            "{err}"
        );

        let err = avb_info_fast(&set_vsz(300)).unwrap_err();
        assert!(
            err.message().contains("outside the 300-byte vbmeta"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn info_scans_back_past_padding() {
        let dir = tmp("volte_avb_info_scan");
        let p = image_with_footer(&dir, 8192);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        assert!(
            !info.footer_at_exact_tail,
            "a footer found by the backward scan is not at the tail"
        );
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
        assert!(!info.footer_at_exact_tail);
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.partition_size, 4096);
        assert_eq!(info.image_size, 4096);
        assert_eq!(info.original_image_size, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_partition_size_properties() {
        for img in [
            1u64,
            4096,
            100_000,
            5_000_000,
            100 * 1024 * 1024,
            1_200_000_000,
        ] {
            let part = fit_avb_part(img, 0);
            assert_eq!(part % 1048576, 0, "MiB aligned for {img}");
            assert!(part >= img, "holds the image for {img}");
            assert!(
                part - tree_size(part) - META_LEN >= img,
                "leaves room for tree+meta for {img}"
            );
        }
        let hint = fit_avb_part(1000, 64 * 1024 * 1024);
        assert_eq!(
            hint,
            64 * 1024 * 1024,
            "hint wins when already large enough"
        );
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
        let a = random_salt_hex().unwrap();
        let b = random_salt_hex().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn twenty_five_salts_are_all_distinct() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..25 {
            assert!(seen.insert(random_salt_hex().unwrap()), "salt repeated");
        }
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
        salt: Option<[u8; 32]>,
    }

    /// Offset of the hashtree salt inside the golden vbmeta, derived from the
    /// descriptor header rather than hard-coded.
    fn golden_salt_offset() -> usize {
        let auth = be_u64(&GOLDEN_VBMETA[12..20]) as usize;
        let desc_offset = be_u64(&GOLDEN_VBMETA[96..104]) as usize;
        let start = VBMETA_HEADER_LEN + auth + desc_offset;
        let name_len = be_u32(&GOLDEN_VBMETA[start + 104..start + 108]) as usize;
        start + HASHTREE_DESCRIPTOR_LEN + name_len
    }

    impl Avb for FooterWriter {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            let size = req.partition_size;
            let mut vb = GOLDEN_VBMETA.to_vec();
            if let Some(salt) = self.salt {
                let at = golden_salt_offset();
                vb[at..at + 32].copy_from_slice(&salt);
            }
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&req.image)?;
            f.seek(SeekFrom::Start(1024 * 1024))?;
            f.write_all(&vb)?;
            let footer = footer_bytes(1, 0, 1024 * 1024, 1024 * 1024, vb.len() as u64);
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
        let before_sha = crate::sha256::file_hex_upper(&p).unwrap();
        let before = std::fs::read(&p).unwrap();
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_footer);
        let (mut log, lines) = catching_logger();
        rebuild_hashtree(&p, &info, 0, &mut NoopBackend, &mut log).unwrap_err();
        let text = lines.borrow().join("\n");
        assert!(!text.contains("Rebuild hashtree completed."), "{text}");
        assert!(!text.contains("[OK] Old AVB footer"), "{text}");
        assert!(!text.contains("[OK] AVB"), "{text}");
        assert_eq!(
            crate::sha256::file_hex_upper(&p).unwrap(),
            before_sha,
            "the saved tail restores the image byte for byte"
        );
        assert_eq!(std::fs::read(&p).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct SilentBackend;
    impl Avb for SilentBackend {
        fn add_hashtree_footer(&mut self, _req: &HashtreeRequest) -> Result<()> {
            Ok(())
        }
    }

    /// Without zeroing the old footer first, a backend that writes nothing is
    /// indistinguishable from an idempotent avbtool rebuild: the existing
    /// footer is still valid and at the tail. The real backend reports its own
    /// failures through `Err`, which is the path that matters here.
    #[test]
    fn a_backend_that_writes_nothing_leaves_the_existing_footer_in_place() {
        let dir = tmp("volte_avb_rebuild_silent");
        let p = image_with_footer(&dir, 0);
        let before_sha = crate::sha256::file_hex_upper(&p).unwrap();
        let before = std::fs::read(&p).unwrap();
        let info = avb_info_fast(&p).unwrap();
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let out = rebuild_hashtree(&p, &info, 0, &mut SilentBackend, &mut log)
            .unwrap()
            .expect("the existing footer is still valid");
        assert!(out.has_footer);
        assert_eq!(crate::sha256::file_hex_upper(&p).unwrap(), before_sha);
        assert_eq!(std::fs::read(&p).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verified_rebuild_reports_the_new_footer() {
        let dir = tmp("volte_avb_rebuild_verified");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        let fresh_salt = *b"0123456789abcdef0123456789abcdef";
        let (mut log, lines) = catching_logger();
        let out = rebuild_hashtree(
            &p,
            &info,
            0,
            &mut FooterWriter {
                grow: 0,
                salt: Some(fresh_salt),
            },
            &mut log,
        )
        .unwrap()
        .expect("footer rebuilt");
        assert!(out.has_footer);
        assert_eq!(
            out.salt,
            hex_lower(&fresh_salt),
            "the new footer is re-read"
        );
        assert_eq!(out.partition_name, "vendor");
        assert_eq!(out.props, info.props);
        let text = lines.borrow().join("\n");
        assert!(text.contains("[OK] Old AVB footer replaced."), "{text}");
        assert!(text.contains("[AVB] Rebuild hashtree completed."), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct RecordingBackend {
        seen: Rc<RefCell<Option<HashtreeRequest>>>,
        inner: FooterWriter,
    }

    impl Avb for RecordingBackend {
        fn add_hashtree_footer(&mut self, req: &HashtreeRequest) -> Result<()> {
            *self.seen.borrow_mut() = Some(req.clone());
            self.inner.add_hashtree_footer(req)
        }
    }

    #[test]
    fn the_rebuild_request_carries_the_parsed_metadata() {
        let dir = tmp("volte_avb_rebuild_request");
        let p = image_with_footer(&dir, 0);
        let info = avb_info_fast(&p).unwrap();
        let seen = Rc::new(RefCell::new(None));
        let mut backend = RecordingBackend {
            seen: seen.clone(),
            inner: FooterWriter {
                grow: 0,
                salt: None,
            },
        };
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        rebuild_hashtree(&p, &info, 0, &mut backend, &mut log)
            .unwrap()
            .expect("footer rebuilt");
        let req = seen.borrow().clone().expect("backend called");
        assert_eq!(req.partition_size, 2 * 1024 * 1024, "the locked slot");
        assert_eq!(req.partition_name, "vendor");
        assert_eq!(req.hash_algorithm, "sha256");
        assert_eq!(req.salt, GOLDEN_SALT);
        assert_eq!(req.props, info.props);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rebuild_detects_footer_and_keeps_the_locked_size() {
        let dir = tmp("volte_avb_rebuild_ok");
        let p = plain_image(&dir, 2 * 1024 * 1024);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_size, 2 * 1024 * 1024);
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let out = rebuild_hashtree(
            &p,
            &info,
            0,
            &mut FooterWriter {
                grow: 0,
                salt: None,
            },
            &mut log,
        )
        .unwrap()
        .expect("footer rebuilt");
        assert!(out.has_footer);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 2 * 1024 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hashtree_that_grows_past_the_slot_restores_the_tail() {
        let dir = tmp("volte_avb_rebuild_grow");
        let p = image_with_footer(&dir, 0);
        let before_sha = crate::sha256::file_hex_upper(&p).unwrap();
        let info = avb_info_fast(&p).unwrap();
        let (mut log, lines) = catching_logger();
        let out = rebuild_hashtree(
            &p,
            &info,
            0,
            &mut FooterWriter {
                grow: 4096,
                salt: None,
            },
            &mut log,
        )
        .unwrap();
        assert!(out.is_none());
        let text = lines.borrow().join("\n");
        assert!(text.contains("[WARN] Hashtree grew to"), "{text}");
        assert_eq!(
            std::fs::metadata(&p).unwrap().len(),
            2 * 1024 * 1024,
            "the saved tail is restored, not left grown"
        );
        assert_eq!(
            crate::sha256::file_hex_upper(&p).unwrap(),
            before_sha,
            "the image is byte-for-byte what it was before the backend"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Test-local 8-byte alignment. Deliberately not `round8`, so a bug in the
    /// production helper cannot hide behind the fixture builder.
    fn a2_pad8(n: usize) -> usize {
        n.div_ceil(8) * 8
    }

    /// `AvbHashDescriptor` built from the AOSP layout alone: 16-byte header
    /// (tag + num_bytes_following) then 116 fixed bytes then name/salt/digest.
    fn a2_hash_descriptor(
        image_size: u64,
        algorithm: &str,
        name: &str,
        salt: &[u8],
        digest: &[u8],
    ) -> Vec<u8> {
        let mut d = vec![0u8; 132];
        d[0..8].copy_from_slice(&2u64.to_be_bytes());
        d[16..24].copy_from_slice(&image_size.to_be_bytes());
        let alg = algorithm.as_bytes();
        assert!(alg.len() < 32, "hash_algorithm must fit in char[32]");
        d[24..24 + alg.len()].copy_from_slice(alg);
        d[56..60].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[60..64].copy_from_slice(&(salt.len() as u32).to_be_bytes());
        d[64..68].copy_from_slice(&(digest.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(salt);
        d.extend_from_slice(digest);
        let nbf = a2_pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    /// `AvbChainPartitionDescriptor`: 16-byte header, 76 fixed bytes, then
    /// partition name and public key.
    fn a2_chain_descriptor(partition: &str, public_key: &[u8], rollback: u32) -> Vec<u8> {
        let mut d = vec![0u8; 92];
        d[0..8].copy_from_slice(&4u64.to_be_bytes());
        d[16..20].copy_from_slice(&rollback.to_be_bytes());
        d[20..24].copy_from_slice(&(partition.len() as u32).to_be_bytes());
        d[24..28].copy_from_slice(&(public_key.len() as u32).to_be_bytes());
        d.extend_from_slice(partition.as_bytes());
        d.extend_from_slice(public_key);
        let nbf = a2_pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    /// `AvbHashtreeDescriptor` from the AOSP field order: 16-byte header, then
    /// 164 fixed bytes (`fec_num_roots` at 52, `hash_algorithm` at 72), then
    /// partition name, salt and root digest.
    fn a2_hashtree_descriptor(
        image_size: u64,
        algorithm: &str,
        name: &str,
        salt: &[u8],
        digest: &[u8],
        fec_roots: u32,
    ) -> Vec<u8> {
        let mut d = vec![0u8; 180];
        d[0..8].copy_from_slice(&1u64.to_be_bytes());
        d[16..20].copy_from_slice(&1u32.to_be_bytes()); // dm_verity_version
        d[20..28].copy_from_slice(&image_size.to_be_bytes());
        d[44..48].copy_from_slice(&4096u32.to_be_bytes()); // data_block_size
        d[48..52].copy_from_slice(&4096u32.to_be_bytes()); // hash_block_size
        d[52..56].copy_from_slice(&fec_roots.to_be_bytes());
        let alg = algorithm.as_bytes();
        assert!(alg.len() < 32, "hash_algorithm must fit in char[32]");
        d[72..72 + alg.len()].copy_from_slice(alg);
        d[104..108].copy_from_slice(&(name.len() as u32).to_be_bytes());
        d[108..112].copy_from_slice(&(salt.len() as u32).to_be_bytes());
        d[112..116].copy_from_slice(&(digest.len() as u32).to_be_bytes());
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(salt);
        d.extend_from_slice(digest);
        let nbf = a2_pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    /// Descriptor with an arbitrary tag and payload, used to prove unknown
    /// tags are recorded instead of silently dropped.
    fn a2_unknown_descriptor(tag: u64, payload: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 16];
        d[0..8].copy_from_slice(&tag.to_be_bytes());
        d.extend_from_slice(payload);
        let nbf = a2_pad8(d.len() - 16);
        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
        d.resize(16 + nbf, 0);
        d
    }

    /// Minimal `AvbVBMetaImageHeader` (256 bytes) with no auth/aux blocks and
    /// the descriptors immediately after `desc_offset = 0`.
    fn a2_vbmeta_with(descriptors: &[Vec<u8>]) -> Vec<u8> {
        let mut vb = vec![0u8; 256];
        vb[..4].copy_from_slice(b"AVB0");
        let desc_size: usize = descriptors.iter().map(Vec::len).sum();
        // The descriptor block lives inside the auxiliary data block, so the
        // header's aux size must cover it (`parse_vbmeta` checks this bound).
        vb[20..28].copy_from_slice(&(desc_size as u64).to_be_bytes());
        vb[104..112].copy_from_slice(&(desc_size as u64).to_be_bytes());
        for d in descriptors {
            vb.extend_from_slice(d);
        }
        vb
    }

    /// 2 MiB image with `vb` at 1 MiB and a matching AVB footer at the end.
    fn a2_image_with_vbmeta(dir: &Path, vb: &[u8]) -> PathBuf {
        let path = dir.join("vendor.img");
        let size: u64 = 2 * 1024 * 1024;
        let mut data = vec![0u8; size as usize];
        data[1024 * 1024..1024 * 1024 + vb.len()].copy_from_slice(vb);
        let footer_at = size - 64;
        data[footer_at as usize..footer_at as usize + 64].copy_from_slice(&footer_bytes(
            1,
            0,
            1024 * 1024,
            1024 * 1024,
            vb.len() as u64,
        ));
        std::fs::write(&path, &data).unwrap();
        path
    }

    #[test]
    fn a2_hash_descriptor_sets_partition_name_and_algorithm() {
        let dir = tmp("volte_avb_a2_hash");
        let vb = a2_vbmeta_with(&[a2_hash_descriptor(
            4096,
            "sha1",
            "vendor_dlkm",
            &[0xde, 0xad, 0xbe, 0xef],
            &[], // digest_len == 0 means a persistent digest, not an error
        )]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor_dlkm");
        assert_eq!(info.hash_algorithm, "sha1");
        assert_eq!(info.salt, "deadbeef");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_chain_only_does_not_override_image_partition_fallback() {
        let dir = tmp("volte_avb_a2_chain");
        let vb = a2_vbmeta_with(&[a2_chain_descriptor("boot", &[0x11, 0x22, 0x33, 0x44], 1)]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(
            info.partition_name, "vendor",
            "a chain reference names another partition, not the image being rebuilt"
        );
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.salt, "");
        assert_eq!(info.fec_roots, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped() {
        let dir = tmp("volte_avb_a2_unknown");
        let vb = a2_vbmeta_with(&[a2_unknown_descriptor(99, &[0xaa; 8])]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.unsupported_descriptor_tags, vec![99]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_chain_before_hash_same_partition_merges_fields() {
        let dir = tmp("volte_avb_a2_merge_chain_hash");
        let vb = a2_vbmeta_with(&[
            a2_chain_descriptor("vendor_dlkm", &[0x01, 0x02, 0x03], 1),
            a2_hash_descriptor(
                4096,
                "sha512",
                "vendor_dlkm",
                &[0xaa, 0xbb, 0xcc, 0xdd],
                &[],
            ),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor_dlkm");
        assert_eq!(info.hash_algorithm, "sha512");
        assert_eq!(info.salt, "aabbccdd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_chain_before_hashtree_does_not_select_image_partition() {
        let dir = tmp("volte_avb_a2_chain_before_tree");
        let vb = a2_vbmeta_with(&[
            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
            a2_hashtree_descriptor(
                4096,
                "sha512",
                "vendor",
                &[0xaa, 0xbb, 0xcc, 0xdd],
                &[0x55; 32],
                2,
            ),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(
            info.partition_name, "vendor",
            "the hashtree descriptor names the image partition"
        );
        assert_eq!(info.hash_algorithm, "sha512");
        assert_eq!(info.salt, "aabbccdd");
        assert_eq!(info.fec_roots, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_chain_reference_does_not_mix_hash_metadata() {
        let dir = tmp("volte_avb_a2_merge_mixed_partitions");
        let vb = a2_vbmeta_with(&[
            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
            a2_hash_descriptor(4096, "sha1", "vendor_dlkm", &[0x11, 0x22, 0x33, 0x44], &[]),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(
            info.partition_name, "vendor_dlkm",
            "the chain reference must not take over the image partition name"
        );
        assert_eq!(
            info.hash_algorithm, "sha1",
            "hash must come from the descriptor that names the image partition"
        );
        assert_eq!(info.salt, "11223344");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_multiple_chain_descriptors_are_handled_correctly() {
        let dir = tmp("volte_avb_a2_chain_multiple");
        let vb = a2_vbmeta_with(&[
            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
            a2_chain_descriptor("system", &[0x04, 0x05, 0x06], 2),
            a2_hashtree_descriptor(
                4096,
                "sha256",
                "vendor",
                &[0xde, 0xad, 0xbe, 0xef],
                &[0x77; 32],
                1,
            ),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.salt, "deadbeef");
        assert_eq!(info.fec_roots, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_hash_before_hashtree_same_partition_fills_fec_only() {
        let dir = tmp("volte_avb_a2_merge_hash_tree");
        let vb = a2_vbmeta_with(&[
            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
            a2_hashtree_descriptor(
                4096,
                "sha256",
                "vendor",
                &[0x11, 0x22, 0x33, 0x44],
                &[0x55; 32],
                2,
            ),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.hash_algorithm, "sha512", "first algorithm wins");
        assert_eq!(info.salt, "aabbccdd", "first salt wins");
        assert_eq!(info.fec_roots, 2, "hashtree still fills the missing FEC");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_hashtree_before_hash_same_partition_keeps_first_values() {
        let dir = tmp("volte_avb_a2_merge_tree_hash");
        let vb = a2_vbmeta_with(&[
            a2_hashtree_descriptor(
                4096,
                "sha256",
                "vendor",
                &[0x11, 0x22, 0x33, 0x44],
                &[0x55; 32],
                2,
            ),
            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(info.hash_algorithm, "sha256");
        assert_eq!(info.salt, "11223344");
        assert_eq!(info.fec_roots, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_hashtree_for_another_partition_does_not_fill_missing_fields() {
        let dir = tmp("volte_avb_a2_merge_tree_other");
        let vb = a2_vbmeta_with(&[
            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
            a2_hashtree_descriptor(
                4096,
                "sha256",
                "vendor_dlkm",
                &[0x11, 0x22, 0x33, 0x44],
                &[0x55; 32],
                4,
            ),
        ]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert_eq!(info.partition_name, "vendor");
        assert_eq!(
            info.fec_roots, 0,
            "FEC from another partition must be ignored"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_chain_descriptor_is_flagged_in_avb_info() {
        let dir = tmp("volte_avb_a2_chain_flag");
        let vb = a2_vbmeta_with(&[a2_chain_descriptor("boot", &[0x11, 0x22, 0x33], 1)]);
        let p = a2_image_with_vbmeta(&dir, &vb);
        let info = avb_info_fast(&p).unwrap();
        assert!(info.has_chain_descriptor);
        assert_eq!(info.descriptor_kind, DescriptorKind::None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a2_descriptor_kind_distinguishes_hash_from_hashtree() {
        let dir = tmp("volte_avb_a2_kind");
        let hash = a2_image_with_vbmeta(
            &dir,
            &a2_vbmeta_with(&[a2_hash_descriptor(4096, "sha1", "vendor", &[0xaa], &[])]),
        );
        assert_eq!(
            avb_info_fast(&hash).unwrap().descriptor_kind,
            DescriptorKind::HashOnly
        );

        let tree = a2_image_with_vbmeta(
            &dir,
            &a2_vbmeta_with(&[a2_hashtree_descriptor(
                4096,
                "sha256",
                "vendor",
                &[0xbb],
                &[0x55; 32],
                2,
            )]),
        );
        assert_eq!(
            avb_info_fast(&tree).unwrap().descriptor_kind,
            DescriptorKind::Hashtree
        );

        let mixed = a2_image_with_vbmeta(
            &dir,
            &a2_vbmeta_with(&[
                a2_hash_descriptor(4096, "sha1", "vendor", &[0xcc], &[]),
                a2_hashtree_descriptor(4096, "sha256", "vendor", &[0xdd], &[0x66; 32], 2),
            ]),
        );
        assert_eq!(
            avb_info_fast(&mixed).unwrap().descriptor_kind,
            DescriptorKind::Hashtree,
            "a hashtree descriptor wins over a hash descriptor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct CountingBackend {
        calls: usize,
    }

    impl Avb for CountingBackend {
        fn add_hashtree_footer(&mut self, _req: &HashtreeRequest) -> Result<()> {
            self.calls += 1;
            Ok(())
        }
    }

    #[test]
    fn original_size_larger_than_the_file_is_refused_before_the_backend() {
        let dir = tmp("volte_avb_orig_too_big");
        let p = plain_image(&dir, 2 * 1024 * 1024);
        let before = std::fs::read(&p).unwrap();
        let mut info = AvbInfo::empty(2 * 1024 * 1024);
        info.partition_size = 2 * 1024 * 1024;
        info.original_image_size = 3 * 1024 * 1024;
        let mut backend = CountingBackend { calls: 0 };
        let mut log = Logger::new().with_sink(Vec::<String>::new());
        let err = rebuild_hashtree(&p, &info, 0, &mut backend, &mut log).unwrap_err();
        assert_eq!(backend.calls, 0, "the backend must not be reached");
        let msg = err.message();
        assert!(msg.contains("3145728"), "{msg}");
        assert!(msg.contains("2097152"), "{msg}");
        assert_eq!(std::fs::read(&p).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
