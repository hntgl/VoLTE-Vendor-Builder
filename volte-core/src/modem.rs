//! FAT16 NON-HLOS modem image engine (BUILD MODEM / BUILD FIX MODEM).
//!
//! Port of `qc_modem_volte.py` (the engine) and the CLI half of
//! `build_all_in_one.py` (`run_modem_build`, `run_cli_fix`): puts the
//! Viettel VoLTE MBN config ahead of other carriers in `oem_sw.txt` /
//! `mbn_sw.txt` on a Qualcomm NON-HLOS modem partition image, or injects a
//! Viettel MBN from a donor image (or a raw blob) when the destination
//! modem does not have one. Image size never changes.
//!
//! Intentionally not ported (GUI-only conveniences, not requested for this
//! CLI integration, and not covered by any test here):
//! - the Tk GUI itself;
//! - zip-file input (`modem.zip` containing `modem.bin`/`modem.img`);
//! - the `%APPDATA%\HoangDucTool` MBN cache and the sibling-folder
//!   auto-discovery of a donor modem (`find_mbn` in `build_all_in_one.py`);
//!   pass `--donor` or `--mbn` explicitly instead;
//! - `_viettel_line_from_texts`, `_nl_join`, `_rel_to_parts`: dead code in
//!   the Python reference (defined, never called), not ported.
//!
//! New hardening not present in the Python reference, applied for the same
//! reason as the vendor builder's A01/A01b/G02 guard (`path_id::same_file`):
//! an `--out` that resolves to the same file as the modem being read is a
//! build error instead of a silent overwrite of the source.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::logging::Logger;
use crate::path_id;

/// FAT16 "end of chain" marker.
pub const FAT16_EOF: u16 = 0xFFFF;
/// FAT16 "bad cluster" marker; chains stop at or above this value.
pub const FAT16_BAD: u16 = 0xFFF7;

const ATTR_ARC: u8 = 0x20;
const ATTR_DIR: u8 = 0x10;
const ATTR_LFN: u8 = 0x0F;

/// 8.3 name of the "." directory entry: one dot padded with ten spaces.
/// Routing it through `encode_83` would yield an all-space name, which
/// `decode_83` turns into `""`; `iter_dir` only filters literal "."/"..",
/// so that entry would look like a child directory pointing at itself.
const DOT_ENTRY_NAME: [u8; 11] = *b".          ";

/// Below this size an `mcfg_sw.mbn` is treated as missing/corrupt, matching
/// the Python reference's `MBN_MIN_SIZE`.
pub const MBN_MIN_SIZE: usize = 1000;

/// The relative path Viettel's MBN lives at; logged and written into
/// `oem_sw.txt` / `mbn_sw.txt` verbatim (lower-cased, like the Python
/// reference's `want`).
pub const VIETTEL_REL: &str = "mcfg_sw/generic/sea/viettel/commerci/vietnam/mcfg_sw.mbn";

const BASE_PARTS: [&str; 5] = ["image", "modem_pr", "mcfg", "configs", "mcfg_sw"];
const OEM_SW_PARTS: [&str; 6] = [
    "image",
    "modem_pr",
    "mcfg",
    "configs",
    "mcfg_sw",
    "oem_sw.txt",
];
const MBN_SW_PARTS: [&str; 6] = [
    "image",
    "modem_pr",
    "mcfg",
    "configs",
    "mcfg_sw",
    "mbn_sw.txt",
];
const VIETTEL_PARTS: [&str; 11] = [
    "image",
    "modem_pr",
    "mcfg",
    "configs",
    "mcfg_sw",
    "generic",
    "sea",
    "viettel",
    "commerci",
    "vietnam",
    "mcfg_sw.mbn",
];

const BAD_FAT_MSG: &str =
    "Not a Qualcomm NON-HLOS FAT16 modem image. Select modem.bin / modem.img - not vendor/md1img.";

// ---------------------------------------------------------------------
// Small text helpers (Latin-1 in, Latin-1 out, like the Python reference)
// ---------------------------------------------------------------------

fn latin1_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn latin1_encode_str(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect()
}

fn latin1_encode_limited(s: &str, max_chars: usize) -> Vec<u8> {
    s.chars()
        .take(max_chars)
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect()
}

/// `str.splitlines()` for the line endings this image format actually uses
/// (`\n`, `\r\n`, bare `\r`). The Python reference also treats a handful of
/// exotic Unicode separators as line breaks; those never occur in a carrier
/// whitelist text file, so this port does not special-case them.
fn split_universal_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut cur));
            }
            '\n' => {
                lines.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn part_match(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    !a.is_empty() && !b.is_empty() && a.eq_ignore_ascii_case(b)
}

fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h: Vec<u8> = haystack.iter().map(u8::to_ascii_lowercase).collect();
    let n: Vec<u8> = needle.iter().map(u8::to_ascii_lowercase).collect();
    h.windows(n.len()).any(|w| w == n.as_slice())
}

fn decode_83(raw: &[u8]) -> String {
    let mut buf = [b' '; 11];
    let n = raw.len().min(11);
    buf[..n].copy_from_slice(&raw[..n]);
    let mut base: Vec<u8> = buf[0..8].to_vec();
    let mut ext: Vec<u8> = buf[8..11].to_vec();
    while matches!(base.last(), Some(b' ') | Some(0)) {
        base.pop();
    }
    while matches!(ext.last(), Some(b' ') | Some(0)) {
        ext.pop();
    }
    if base.first() == Some(&0x05) {
        base[0] = 0xE5;
    }
    let base_s = latin1_decode(&base);
    let ext_s = latin1_decode(&ext);
    if ext_s.is_empty() {
        base_s
    } else {
        format!("{base_s}.{ext_s}")
    }
}

fn lfn_checksum(name83: &[u8]) -> u8 {
    let mut s: u32 = 0;
    for &b in name83.iter().take(11) {
        s = (((s & 1) << 7) + (s >> 1) + b as u32) & 0xFF;
    }
    s as u8
}

/// Joins LFN (long file name) directory entries into a name; `""` if the
/// entries are not a consistent, checksum-matching LFN run for `name83`.
fn decode_lfn(mut entries: Vec<(u8, u8, [u8; 32])>, name83: &[u8]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    entries.sort_by_key(|e| e.0 & 0x3F);
    let masked: Vec<u8> = entries.iter().map(|e| e.0 & 0x3F).collect();
    if entries.len() > u8::MAX as usize {
        return String::new();
    }
    let expect: Vec<u8> = (1..=entries.len() as u8).collect();
    if masked != expect {
        return String::new();
    }
    if entries.last().unwrap().0 & 0x40 == 0 {
        return String::new();
    }
    let chk = lfn_checksum(name83);
    if entries.iter().any(|e| e.1 != chk) {
        return String::new();
    }
    const OFFSETS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    let mut chars: Vec<char> = Vec::new();
    for (_seq, _chk, raw) in entries.iter() {
        for &off in OFFSETS.iter() {
            let c = u16::from_le_bytes([raw[off], raw[off + 1]]);
            if c == 0 {
                return chars.into_iter().collect();
            }
            if c == 0xFFFF {
                continue;
            }
            if let Some(ch) = char::from_u32(c as u32) {
                chars.push(ch);
            }
        }
    }
    chars.into_iter().collect()
}

fn encode_83(name: &str) -> [u8; 11] {
    let upper = name.to_uppercase();
    let (base, ext) = match upper.rfind('.') {
        Some(pos) => (&upper[..pos], &upper[pos + 1..]),
        None => (upper.as_str(), ""),
    };
    let b = latin1_encode_limited(base, 8);
    let e = latin1_encode_limited(ext, 3);
    let mut raw = [b' '; 11];
    raw[0..b.len()].copy_from_slice(&b);
    raw[8..8 + e.len()].copy_from_slice(&e);
    raw
}

/// Puts the Viettel line first in an `oem_sw.txt` / `mbn_sw.txt` blob.
/// Returns `(new_blob, already_had_the_line)`. Port of
/// `_oem_put_viettel_first` (`qc_modem_volte.py:155`); the written line is
/// the lower-cased `viettel_rel`, matching the Python reference exactly.
fn oem_put_viettel_first(blob: &[u8], viettel_rel: &str) -> (Vec<u8>, bool) {
    let has_crlf = blob.windows(2).any(|w| w == b"\r\n");
    let nl = if has_crlf { "\r\n" } else { "\n" };
    let rel = if viettel_rel.trim().is_empty() {
        VIETTEL_REL
    } else {
        viettel_rel
    };
    let want = rel.trim().to_lowercase();
    let text = latin1_decode(blob);
    let mut had = false;
    let mut keep: Vec<String> = Vec::new();
    for line in split_universal_lines(&text) {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        if s.to_lowercase() == want {
            had = true;
            continue;
        }
        keep.push(s.to_string());
    }
    let mut out = String::new();
    out.push_str(&want);
    out.push_str(nl);
    for s in &keep {
        out.push_str(s);
        out.push_str(nl);
    }
    (latin1_encode_str(&out), had)
}

fn mb(n: usize) -> f64 {
    n as f64 / (1024.0 * 1024.0)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

// ---------------------------------------------------------------------
// FAT16
// ---------------------------------------------------------------------

/// One resolved directory entry: a file, or a directory (`attr & ATTR_DIR`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub attr: u8,
    pub cluster: u32,
    pub size: u32,
    dir_cluster: u32,
    ent_index: usize,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attr & ATTR_DIR != 0
    }
}

/// A FAT16 NON-HLOS image held entirely in memory. Every operation keeps
/// the backing buffer (`data`) the same length; nothing here grows or
/// shrinks the image itself (only the directory tree and cluster contents
/// inside it change).
#[derive(Debug)]
pub struct Fat16Image {
    pub data: Vec<u8>,
    bps: u32,
    nfats: u32,
    rootent: u32,
    spf: u32,
    cluster_size: u32,
    fat_off: usize,
    root_off: usize,
    data_off: usize,
    max_cluster: u32,
    oem: [u8; 8],
}

impl Fat16Image {
    pub fn new(data: Vec<u8>) -> Result<Self> {
        let n = data.len();
        if n < 512 {
            return Err(Error::bad_magic(BAD_FAT_MSG));
        }
        let bs = &data[0..512];
        let bps = u16::from_le_bytes([bs[11], bs[12]]) as u32;
        let spc = bs[13] as u32;
        let reserved = u16::from_le_bytes([bs[14], bs[15]]) as u32;
        let nfats = bs[16] as u32;
        let rootent = u16::from_le_bytes([bs[17], bs[18]]) as u32;
        let total16 = u16::from_le_bytes([bs[19], bs[20]]) as u32;
        let spf = u16::from_le_bytes([bs[22], bs[23]]) as u32;
        let total32 = u32::from_le_bytes([bs[32], bs[33], bs[34], bs[35]]);
        let total = if total32 != 0 { total32 } else { total16 };

        let ok = matches!(bps, 512 | 1024 | 2048 | 4096)
            && spc > 0
            && (spc & (spc - 1)) == 0
            && reserved >= 1
            && nfats >= 1
            && spf > 0
            && total > 0
            && rootent > 0;
        if !ok {
            return Err(Error::bad_magic(BAD_FAT_MSG));
        }

        let cluster_size = bps * spc;
        let fat_off = (reserved * bps) as usize;
        let root_off = ((reserved + nfats * spf) * bps) as usize;
        let root_bytes = (rootent * 32) as usize;
        let data_off = root_off + root_bytes;
        if data_off > n {
            return Err(Error::bad_magic(BAD_FAT_MSG));
        }
        let root_sectors = (root_bytes as u32) / bps;
        let used = reserved
            .checked_add(nfats * spf)
            .and_then(|v| v.checked_add(root_sectors));
        let data_sectors = match used.and_then(|u| total.checked_sub(u)) {
            Some(v) => v,
            None => return Err(Error::bad_magic(BAD_FAT_MSG)),
        };
        let data_clusters = data_sectors / spc;
        if data_clusters < 3 {
            return Err(Error::bad_magic(BAD_FAT_MSG));
        }
        // A truncated image can still pass `data_off > n` (the boot sector,
        // FATs and root directory survive) while its data region does not.
        // Every cluster access assumes this range exists, so reject it here.
        let data_end = (data_clusters as usize)
            .checked_mul(cluster_size as usize)
            .and_then(|bytes| data_off.checked_add(bytes));
        match data_end {
            Some(end) if end <= n => {}
            _ => return Err(Error::bad_magic(BAD_FAT_MSG)),
        }
        let max_cluster = data_clusters + 1;
        let mut oem = [0u8; 8];
        oem.copy_from_slice(&bs[3..11]);

        Ok(Fat16Image {
            data,
            bps,
            nfats,
            rootent,
            spf,
            cluster_size,
            fat_off,
            root_off,
            data_off,
            max_cluster,
            oem,
        })
    }

    pub(crate) fn cluster_size(&self) -> u32 {
        self.cluster_size
    }

    /// The 8-byte OEM id in the boot sector, trimmed of spaces/NUL.
    pub(crate) fn oem_label(&self) -> String {
        latin1_decode(&self.oem)
            .trim_end_matches(['\u{0}', ' '])
            .to_string()
    }

    fn clus_off(&self, c: u32) -> usize {
        self.data_off + (c as usize - 2) * self.cluster_size as usize
    }

    fn per_clus(&self) -> usize {
        self.cluster_size as usize / 32
    }

    fn fat_get(&self, c: u32) -> u16 {
        let off = self.fat_off + c as usize * 2;
        if off + 2 > self.data.len() {
            return 0;
        }
        u16::from_le_bytes([self.data[off], self.data[off + 1]])
    }

    fn fat_set(&mut self, c: u32, val: u16) {
        for i in 0..self.nfats {
            let off = self.fat_off + (i * self.spf * self.bps) as usize + c as usize * 2;
            if off + 2 <= self.data.len() {
                self.data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
    }

    fn chain(&self, start: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = start;
        let mut seen = std::collections::HashSet::new();
        while c >= 2 && c <= self.max_cluster && c < FAT16_BAD as u32 && !seen.contains(&c) {
            out.push(c);
            seen.insert(c);
            let nxt = self.fat_get(c) as u32;
            if nxt >= FAT16_BAD as u32 || nxt < 2 {
                break;
            }
            if nxt > self.max_cluster {
                break;
            }
            c = nxt;
        }
        out
    }

    fn alloc(&mut self, n: usize) -> Result<Vec<u32>> {
        if n == 0 {
            return Ok(Vec::new());
        }
        let mut got = Vec::new();
        let maxc = self.max_cluster;
        let mut c: u32 = 2;
        while c <= maxc && got.len() < n {
            let off = self.clus_off(c);
            if off + self.cluster_size as usize <= self.data.len() && self.fat_get(c) == 0 {
                got.push(c);
                self.fat_set(c, FAT16_EOF);
            }
            c += 1;
        }
        if got.len() < n {
            return Err(Error::build(
                "FAT out of free clusters - not enough room to write the MBN.",
            ));
        }
        Ok(got)
    }

    fn link_chain(&mut self, clusters: &[u32]) {
        for (i, &c) in clusters.iter().enumerate() {
            let next = if i + 1 < clusters.len() {
                clusters[i + 1] as u16
            } else {
                FAT16_EOF
            };
            self.fat_set(c, next);
        }
    }

    fn read_clusters(&self, start: u32, size: usize) -> Vec<u8> {
        if size == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for c in self.chain(start) {
            let off = self.clus_off(c);
            let end = (off + self.cluster_size as usize).min(self.data.len());
            if off < end {
                out.extend_from_slice(&self.data[off..end]);
            }
            if out.len() >= size {
                break;
            }
        }
        if out.len() > size {
            out.truncate(size);
        }
        out
    }

    fn write_clusters(&mut self, clusters: &[u32], blob: &[u8]) {
        let mut pos = 0usize;
        for &c in clusters {
            let off = self.clus_off(c);
            let end_blob = (pos + self.cluster_size as usize).min(blob.len());
            if pos < end_blob {
                let chunk = &blob[pos..end_blob];
                let end_data = (off + chunk.len()).min(self.data.len());
                if end_data > off {
                    let n = end_data - off;
                    self.data[off..off + n].copy_from_slice(&chunk[..n]);
                }
            }
            pos += self.cluster_size as usize;
            if pos >= blob.len() {
                break;
            }
        }
    }

    fn dir_nslots(&self, dir_cluster: u32) -> usize {
        if dir_cluster == 0 {
            self.rootent as usize
        } else {
            self.chain(dir_cluster).len() * self.per_clus()
        }
    }

    fn slot_abs(&self, dir_cluster: u32, index: usize) -> Option<usize> {
        let off = if dir_cluster == 0 {
            self.root_off + index * 32
        } else {
            let per = self.per_clus();
            let ch = self.chain(dir_cluster);
            let ci = index / per;
            if ci >= ch.len() {
                return None;
            }
            self.clus_off(ch[ci]) + (index % per) * 32
        };
        if off.checked_add(32)? > self.data.len() {
            return None;
        }
        Some(off)
    }

    fn iter_dir(&self, dir_cluster: u32) -> Vec<Entry> {
        let nslots = self.dir_nslots(dir_cluster);
        let mut lfn: Vec<(u8, u8, [u8; 32])> = Vec::new();
        let mut out = Vec::new();
        for i in 0..nslots {
            let off = match self.slot_abs(dir_cluster, i) {
                Some(o) if o + 32 <= self.data.len() => o,
                _ => break,
            };
            let ent = &self.data[off..off + 32];
            if ent[0] == 0x00 {
                break;
            }
            let attr = ent[11];
            if attr == ATTR_LFN {
                let seq = ent[0];
                if seq & 0x40 != 0 {
                    lfn.clear();
                }
                let mut raw = [0u8; 32];
                raw.copy_from_slice(ent);
                lfn.push((seq, ent[13], raw));
                continue;
            }
            let entries = std::mem::take(&mut lfn);
            if ent[0] == 0xE5 {
                continue;
            }
            let lfn_name = decode_lfn(entries, &ent[0..11]);
            let name = if !lfn_name.is_empty() {
                lfn_name
            } else {
                decode_83(&ent[0..11])
            };
            if name == "." || name == ".." {
                continue;
            }
            out.push(Entry {
                name,
                attr,
                cluster: u16::from_le_bytes([ent[26], ent[27]]) as u32,
                size: u32::from_le_bytes([ent[28], ent[29], ent[30], ent[31]]),
                dir_cluster,
                ent_index: i,
            });
        }
        out
    }

    /// Resolves a `/`-style path of exact (case-insensitive) component
    /// names from the root. No 8.3 truncation, no prefix matching.
    pub fn find(&self, parts: &[&str]) -> Option<Entry> {
        let mut cur: u32 = 0;
        let mut ent: Option<Entry> = None;
        for part in parts.iter().copied() {
            let mut found = None;
            for it in self.iter_dir(cur) {
                if part_match(&it.name, part) {
                    found = Some(it);
                    break;
                }
            }
            let e = found?;
            cur = if e.is_dir() { e.cluster } else { 0 };
            ent = Some(e);
        }
        ent
    }

    fn find_child(&self, dir_cluster: u32, name: &str) -> Option<Entry> {
        self.iter_dir(dir_cluster)
            .into_iter()
            .find(|it| part_match(&it.name, name))
    }

    fn free_slot(&self, dir_cluster: u32, count: usize) -> Option<usize> {
        let nslots = self.dir_nslots(dir_cluster);
        let mut run = 0usize;
        let mut start = 0usize;
        for i in 0..nslots {
            let off = self.slot_abs(dir_cluster, i)?;
            let b = *self.data.get(off)?;
            if b == 0x00 || b == 0xE5 {
                if run == 0 {
                    start = i;
                }
                run += 1;
                if run >= count {
                    return Some(start);
                }
            } else {
                run = 0;
            }
        }
        None
    }

    fn grow_dir(&mut self, dir_cluster: u32) -> Result<usize> {
        if dir_cluster == 0 {
            return Err(Error::build("FAT16 root directory is full."));
        }
        let new = self.alloc(1)?[0];
        let ch = self.chain(dir_cluster);
        if let Some(&last) = ch.last() {
            self.fat_set(last, new as u16);
        }
        self.fat_set(new, FAT16_EOF);
        let off = self.clus_off(new);
        let csz = self.cluster_size as usize;
        self.data[off..off + csz].fill(0);
        Ok(ch.len() * self.per_clus())
    }

    fn alloc_dirent(&mut self, dir_cluster: u32, count: usize) -> Result<usize> {
        if let Some(idx) = self.free_slot(dir_cluster, count) {
            return Ok(idx);
        }
        self.grow_dir(dir_cluster)
    }

    fn put_ent(
        &mut self,
        off: usize,
        name83: &[u8; 11],
        attr: u8,
        cluster: u32,
        size: u32,
    ) -> Result<()> {
        if off + 32 > self.data.len() {
            return Err(Error::build(
                "Directory entry position is outside the image.",
            ));
        }
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(name83);
        e[11] = attr;
        e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        self.data[off..off + 32].copy_from_slice(&e);
        Ok(())
    }

    /// Creates `name` under `parent_cluster` (0 = root) if it does not
    /// already exist as a directory there.
    fn mkdir(&mut self, parent_cluster: u32, name: &str) -> Result<Entry> {
        for it in self.iter_dir(parent_cluster) {
            if part_match(&it.name, name) && it.is_dir() {
                return Ok(it);
            }
        }
        let cl = self.alloc(1)?[0];
        let off = self.clus_off(cl);
        let csz = self.cluster_size as usize;
        self.data[off..off + csz].fill(0);
        self.fat_set(cl, FAT16_EOF);
        self.put_ent(off, &DOT_ENTRY_NAME, ATTR_DIR, cl, 0)?;
        self.put_ent(off + 32, &encode_83(".."), ATTR_DIR, parent_cluster, 0)?;
        let idx = self.alloc_dirent(parent_cluster, 1)?;
        let aoff = self
            .slot_abs(parent_cluster, idx)
            .ok_or_else(|| Error::build("Directory slot is outside the image."))?;
        self.put_ent(aoff, &encode_83(name), ATTR_DIR, cl, 0)?;
        Ok(Entry {
            name: name.to_uppercase(),
            attr: ATTR_DIR,
            cluster: cl,
            size: 0,
            dir_cluster: parent_cluster,
            ent_index: idx,
        })
    }

    pub(crate) fn read_file(&self, entry: &Entry) -> Vec<u8> {
        self.read_clusters(entry.cluster, entry.size as usize)
    }

    /// Rewrites `entry`'s content, growing/shrinking its cluster chain as
    /// needed, and updates its directory slot (size + first cluster) in
    /// place. Returns the entry with the new cluster/size.
    pub(crate) fn write_file(&mut self, entry: &Entry, blob: &[u8]) -> Result<Entry> {
        let need = if blob.is_empty() {
            0
        } else {
            blob.len().div_ceil(self.cluster_size as usize)
        };
        let mut cur = self.chain(entry.cluster);
        if need == 0 {
            for &c in &cur {
                self.fat_set(c, 0);
            }
            cur.clear();
        } else if cur.is_empty() {
            cur = self.alloc(need)?;
            self.link_chain(&cur);
        } else if cur.len() < need {
            let extra = self.alloc(need - cur.len())?;
            self.fat_set(*cur.last().unwrap(), extra[0] as u16);
            self.link_chain(&extra);
            cur.extend(extra);
        } else if cur.len() > need {
            for &c in &cur[need..] {
                self.fat_set(c, 0);
            }
            cur.truncate(need);
            if let Some(&last) = cur.last() {
                self.fat_set(last, FAT16_EOF);
            }
        }
        if !blob.is_empty() && !cur.is_empty() {
            self.write_clusters(&cur, blob);
        }
        let mut updated = entry.clone();
        updated.cluster = cur.first().copied().unwrap_or(0);
        updated.size = blob.len() as u32;
        if let Some(off) = self.slot_abs(entry.dir_cluster, entry.ent_index) {
            if off + 11 <= self.data.len() {
                let mut name83 = [0u8; 11];
                name83.copy_from_slice(&self.data[off..off + 11]);
                self.put_ent(off, &name83, updated.attr, updated.cluster, updated.size)?;
            }
        }
        Ok(updated)
    }

    fn walk(&self, start_cluster: u32, prefix: &[String]) -> Vec<(Entry, Vec<String>)> {
        let mut visited = std::collections::HashSet::new();
        self.walk_into(start_cluster, prefix, &mut visited)
    }

    fn walk_into(
        &self,
        start_cluster: u32,
        prefix: &[String],
        visited: &mut std::collections::HashSet<u32>,
    ) -> Vec<(Entry, Vec<String>)> {
        if !visited.insert(start_cluster) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for it in self.iter_dir(start_cluster) {
            let mut parts = prefix.to_vec();
            parts.push(it.name.clone());
            if it.is_dir() {
                out.extend(self.walk_into(it.cluster, &parts, visited));
            } else {
                out.push((it, parts));
            }
        }
        out
    }

    fn has_viettel_dir(&self) -> bool {
        let base = match self.find(&BASE_PARTS) {
            Some(b) if b.is_dir() => b,
            _ => return false,
        };
        let gen = match self.find_child(base.cluster, "generic") {
            Some(g) if g.is_dir() => g,
            _ => return false,
        };
        for carrier in ["sea", "row"] {
            if let Some(car) = self.find_child(gen.cluster, carrier) {
                if car.is_dir()
                    && self
                        .find_child(car.cluster, "viettel")
                        .map(|v| v.is_dir())
                        .unwrap_or(false)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Finds a usable (>1000 byte) Viettel `mcfg_sw.mbn` under `sea` or
    /// `row`. Returns the entry plus the relative path it was found at.
    pub(crate) fn find_viettel_mbn(&self) -> (Option<Entry>, String) {
        let base = match self.find(&BASE_PARTS) {
            Some(b) if b.is_dir() => b,
            _ => return (None, String::new()),
        };
        let gen = match self.find_child(base.cluster, "generic") {
            Some(g) if g.is_dir() => g,
            _ => return (None, String::new()),
        };
        for carrier in ["sea", "row"] {
            let car = match self.find_child(gen.cluster, carrier) {
                Some(c) if c.is_dir() => c,
                _ => continue,
            };
            let vt = match self.find_child(car.cluster, "viettel") {
                Some(v) if v.is_dir() => v,
                _ => continue,
            };
            let prefix = vec![gen.name.clone(), car.name.clone(), vt.name.clone()];
            for (entry, parts) in self.walk(vt.cluster, &prefix) {
                if entry.size as usize <= MBN_MIN_SIZE {
                    continue;
                }
                if part_match(&entry.name, "mcfg_sw.mbn") {
                    let low: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
                    let expect = [
                        "generic",
                        carrier,
                        "viettel",
                        "commerci",
                        "vietnam",
                        "mcfg_sw.mbn",
                    ];
                    let rel = if low.iter().map(String::as_str).eq(expect.iter().copied()) {
                        format!("generic/{carrier}/viettel/commerci/vietnam/mcfg_sw.mbn")
                    } else {
                        parts.join("/")
                    };
                    return (Some(entry), rel);
                }
            }
        }
        (None, String::new())
    }

    pub(crate) fn list_sea_carriers(&self) -> Vec<String> {
        let base = match self.find(&BASE_PARTS) {
            Some(b) if b.is_dir() => b,
            _ => return Vec::new(),
        };
        let gen = match self.find_child(base.cluster, "generic") {
            Some(g) if g.is_dir() => g,
            _ => return Vec::new(),
        };
        let sea = match self.find_child(gen.cluster, "sea") {
            Some(s) if s.is_dir() => s,
            _ => return Vec::new(),
        };
        self.iter_dir(sea.cluster)
            .into_iter()
            .filter(|it| it.is_dir())
            .map(|it| it.name)
            .collect()
    }
}

/// `looks_like_qc_modem`: true if `path` parses as a FAT16 NON-HLOS image.
pub fn looks_like_qc_modem(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(data) => Fat16Image::new(data).is_ok(),
        Err(_) => false,
    }
}

fn ensure_dir(fat: &mut Fat16Image, parts: &[&str]) -> Result<u32> {
    let mut cur: u32 = 0;
    for name in parts.iter().copied() {
        match fat.find_child(cur, name) {
            Some(ent) if ent.is_dir() => cur = ent.cluster,
            Some(_) => {
                return Err(Error::build(format!(
                    "Modem path is blocked by a file: {name}"
                )))
            }
            None => cur = fat.mkdir(cur, name)?.cluster,
        }
    }
    Ok(cur)
}

/// Writes/creates the Viettel `mcfg_sw.mbn` with `blob`'s content.
fn inject_mbn(fat: &mut Fat16Image, blob: &[u8]) -> Result<Entry> {
    if let Some(entry) = fat.find(&VIETTEL_PARTS) {
        return fat.write_file(&entry, blob);
    }
    let parent = ensure_dir(fat, &VIETTEL_PARTS[..VIETTEL_PARTS.len() - 1])?;
    let name = VIETTEL_PARTS[VIETTEL_PARTS.len() - 1];
    let need =
        ((blob.len() + fat.cluster_size() as usize - 1) / fat.cluster_size() as usize).max(1);
    let clusters = fat.alloc(need)?;
    fat.link_chain(&clusters);
    fat.write_clusters(&clusters, blob);
    let idx = fat.alloc_dirent(parent, 1)?;
    let off = fat
        .slot_abs(parent, idx)
        .ok_or_else(|| Error::build("Directory slot is outside the image."))?;
    fat.put_ent(
        off,
        &encode_83(name),
        ATTR_ARC,
        clusters[0],
        blob.len() as u32,
    )?;
    Ok(Entry {
        name: name.to_uppercase(),
        attr: ATTR_ARC,
        cluster: clusters[0],
        size: blob.len() as u32,
        dir_cluster: parent,
        ent_index: idx,
    })
}

// ---------------------------------------------------------------------
// Orchestration: scan / build / fix (file I/O + logging happen here, like
// `pipeline::run` does for the vendor builder)
// ---------------------------------------------------------------------

/// Result of `scan_modem_profiles`.
pub struct ScanReport {
    pub carriers: Vec<String>,
    pub has_viettel_mbn: bool,
    pub viettel_rel: String,
    pub viettel_size: u64,
}

/// Result of a BUILD MODEM / BUILD FIX MODEM run.
#[derive(Debug)]
pub struct ModemReport {
    pub dst: PathBuf,
    pub size: u64,
    pub viettel_rel: String,
    pub already_had_viettel_first: bool,
}

fn default_dst(src: &Path, suffix: &str) -> PathBuf {
    let parent = src.parent().unwrap_or_else(|| Path::new(""));
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = match src.extension() {
        Some(e) => format!(".{}", e.to_string_lossy()),
        None => ".bin".to_string(),
    };
    parent.join(format!("{stem}{suffix}{ext}"))
}

fn resolve_dst(src: &Path, out: Option<&Path>, suffix: &str) -> PathBuf {
    match out {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => default_dst(src, suffix),
    }
}

/// Refuses an `--out` that would overwrite the image being read (A01-style
/// guard; see the module doc comment).
fn guard_identity(src: &Path, dst: &Path) -> Result<()> {
    if path_id::same_file(src, dst) {
        return Err(Error::build(format!(
            "Output path is the same as the source modem image ({}). Refusing to overwrite the source.",
            dst.display()
        )));
    }
    Ok(())
}

fn log_post_build_notes(log: &mut Logger, is_fix: bool) {
    log.log("-- AFTER BUILD --");
    log.log(format!(
        "1) Flash the {} file into the modem partition",
        if is_fix {
            "modem_volte_fix"
        } else {
            "modem_volte"
        }
    ));
    log.log("2) Flash the vendor image already BUILD VoLTE'd");
    log.log(
        "3) NOTE: if there is no signal after flashing, wipe modemst1 and modemst2, then reboot",
    );
    log.log("4) Viettel SIM + 4G");
}

/// SCAN MODEM PROFILES: lists carriers / Viettel MBN / oem_sw.txt without
/// writing anything.
pub(crate) fn scan_modem_profiles(src: &Path, log: &mut Logger) -> Result<ScanReport> {
    let data = std::fs::read(src)?;
    let data_len = data.len();
    let fat = Fat16Image::new(data).map_err(|_| Error::bad_magic(BAD_FAT_MSG))?;
    log.log("-- SCAN MODEM PROFILES --");
    log.log(format!(
        "Size: {:.1} MB | OEM: {}",
        mb(data_len),
        fat.oem_label()
    ));
    let carriers = fat.list_sea_carriers();
    log.log(format!(
        "SEA carriers ({}): {}",
        carriers.len(),
        carriers.join(", ")
    ));
    let (vent, rel) = fat.find_viettel_mbn();
    let (has_viettel, viettel_size) = match &vent {
        Some(e) => {
            log.log(format!("Viettel MBN: YES ({} bytes) - {rel}", e.size));
            (true, e.size as u64)
        }
        None => {
            log.log("Viettel MBN: NOT present in this modem.");
            log.log("-> Cannot enable the Viettel signal without this MBN file.");
            (false, 0)
        }
    };
    if let Some(oem) = fat.find(&OEM_SW_PARTS) {
        let blob = fat.read_file(&oem);
        let text = latin1_decode(&blob);
        let lines = split_universal_lines(&text)
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .count();
        let has = contains_ci(&blob, b"viettel");
        log.log(format!(
            "oem_sw.txt: {lines} lines | Viettel whitelist: {}",
            if has { "YES" } else { "NOT YET" }
        ));
    } else {
        log.log("oem_sw.txt: NOT FOUND");
    }
    Ok(ScanReport {
        carriers,
        has_viettel_mbn: has_viettel,
        viettel_rel: if has_viettel { rel } else { String::new() },
        viettel_size,
    })
}

/// BUILD MODEM: the modem already has a Viettel MBN; move it to the front
/// of `oem_sw.txt` (and `mbn_sw.txt`, if present). Port of
/// `build_modem_volte` (`qc_modem_volte.py:687`), wrapped with the scan
/// step `run_modem_build` (`build_all_in_one.py:45`) does first.
pub fn build_modem_volte(src: &Path, out: Option<&Path>, log: &mut Logger) -> Result<ModemReport> {
    let dst = resolve_dst(src, out, "_volte");
    guard_identity(src, &dst)?;

    scan_modem_profiles(src, log)?;

    log.log("-- PATCH CARRIER MBN (Viettel) --");
    let data = std::fs::read(src)?;
    log.log(format!("Reading modem ({:.1} MB)...", mb(data.len())));
    let mut fat = Fat16Image::new(data).map_err(|_| Error::bad_magic(BAD_FAT_MSG))?;
    log.log(format!(
        "FAT16 cluster {} - OEM {}",
        fat.cluster_size(),
        fat.oem_label()
    ));

    let carriers = fat.list_sea_carriers();
    let vt_dir = fat.has_viettel_dir();
    log.log(format!(
        "SEA: {} carriers | VIETTEL dir: {}",
        carriers.len(),
        if vt_dir { "YES" } else { "NO" }
    ));

    let oem = fat.find(&OEM_SW_PARTS).ok_or_else(|| {
        Error::build("oem_sw.txt not found in the modem (not a valid Qualcomm NON-HLOS image).")
    })?;
    let (vt_entry, _rel) = fat.find_viettel_mbn();
    if vt_entry.is_none() {
        return Err(Error::build(
            "No Viettel MBN in this modem (sea/viettel/.../mcfg_sw.mbn).\n\
             This modem does not support the Viettel network - use a different modem.",
        ));
    }
    log.log(format!("Viettel MBN: {VIETTEL_REL}"));

    let oem_blob = fat.read_file(&oem);
    let (new_oem, already) = oem_put_viettel_first(&oem_blob, VIETTEL_REL);
    fat.write_file(&oem, &new_oem)?;
    log.log(if already {
        "oem_sw.txt: already has Viettel first."
    } else {
        "oem_sw.txt: moved Viettel to the front."
    });

    if let Some(mb_entry) = fat.find(&MBN_SW_PARTS) {
        let mb_blob = fat.read_file(&mb_entry);
        let (new_mb, had) = oem_put_viettel_first(&mb_blob, VIETTEL_REL);
        if !had {
            fat.write_file(&mb_entry, &new_mb)?;
            log.log("mbn_sw.txt: added the Viettel line.");
        }
    }

    log.log(format!("Writing modem to: {}", dst.display()));
    std::fs::write(&dst, &fat.data)?;
    log.log(format!("Written: {}", dst.display()));
    log_post_build_notes(log, false);

    Ok(ModemReport {
        dst,
        size: fat.data.len() as u64,
        viettel_rel: VIETTEL_REL.to_string(),
        already_had_viettel_first: already,
    })
}

/// BUILD FIX MODEM from a raw MBN blob (already read into memory). Port of
/// `build_modem_fix_from_mbn` (`qc_modem_volte.py:820`).
pub fn build_modem_fix_from_mbn(
    dest: &Path,
    mbn: &[u8],
    out: Option<&Path>,
    log: &mut Logger,
) -> Result<ModemReport> {
    if mbn.len() < MBN_MIN_SIZE {
        return Err(Error::build("Source Viettel MBN is too small/corrupt."));
    }
    let dst = resolve_dst(dest, out, "_volte_fix");
    guard_identity(dest, &dst)?;

    log.log("-- BUILD FIX MODEM (missing Viettel network) --");
    log.log(format!("Viettel MBN: {} bytes", mbn.len()));
    let data = std::fs::read(dest)?;
    log.log(format!(
        "TARGET: {} ({:.1} MB)",
        file_name(dest),
        mb(data.len())
    ));
    let mut fat = Fat16Image::new(data).map_err(|_| Error::bad_magic(BAD_FAT_MSG))?;

    inject_mbn(&mut fat, mbn)?;
    log.log(format!("Loaded Viettel MBN into: {VIETTEL_REL}"));

    let oem = fat
        .find(&OEM_SW_PARTS)
        .ok_or_else(|| Error::build("oem_sw.txt not found in the target modem."))?;
    let oem_blob = fat.read_file(&oem);
    let (new_oem, _had) = oem_put_viettel_first(&oem_blob, VIETTEL_REL);
    fat.write_file(&oem, &new_oem)?;
    log.log("oem_sw.txt: moved Viettel to the front.");

    if let Some(mb_entry) = fat.find(&MBN_SW_PARTS) {
        let mb_blob = fat.read_file(&mb_entry);
        let (new_mb, had2) = oem_put_viettel_first(&mb_blob, VIETTEL_REL);
        if !had2 {
            fat.write_file(&mb_entry, &new_mb)?;
            log.log("mbn_sw.txt: added the Viettel line.");
        }
    }

    log.log(format!("Writing fixed modem to: {}", dst.display()));
    std::fs::write(&dst, &fat.data)?;
    log.log(format!("Written: {}", dst.display()));
    log_post_build_notes(log, true);

    Ok(ModemReport {
        dst,
        size: fat.data.len() as u64,
        viettel_rel: VIETTEL_REL.to_string(),
        already_had_viettel_first: false,
    })
}

/// BUILD FIX MODEM, taking the Viettel MBN from a donor modem image. Port
/// of `build_modem_fix_viettel` (`qc_modem_volte.py:899`).
pub fn build_modem_fix_viettel(
    dest: &Path,
    donor: &Path,
    out: Option<&Path>,
    log: &mut Logger,
) -> Result<ModemReport> {
    let donor_data = std::fs::read(donor)?;
    let dfat = Fat16Image::new(donor_data).map_err(|_| Error::bad_magic(BAD_FAT_MSG))?;
    let (vent, rel) = dfat.find_viettel_mbn();
    let vent = vent.ok_or_else(|| {
        Error::build(
            "Donor modem has no Viettel MBN. Use a Vietnamese modem that already has Viettel.",
        )
    })?;
    let blob = dfat.read_file(&vent);
    log.log(format!("DONOR: {}", file_name(donor)));
    log.log(format!("Donor Viettel MBN: {} bytes - {rel}", blob.len()));

    build_modem_fix_from_mbn(dest, &blob, out, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::MemorySink;

    /// Builds a minimal but valid, blank FAT16 image: `data_clusters`
    /// clusters of `512` bytes, one FAT copy, a 16-entry root directory.
    fn blank_image(data_clusters: u32) -> Vec<u8> {
        let bps: u16 = 512;
        let spc: u8 = 1;
        let reserved: u16 = 1;
        let nfats: u8 = 1;
        let rootent: u16 = 16;
        let spf: u16 = 1;
        let root_sectors = (rootent as u32 * 32) / bps as u32;
        let total_sectors =
            reserved as u32 + nfats as u32 * spf as u32 + root_sectors + data_clusters;
        let mut data = vec![0u8; total_sectors as usize * bps as usize];
        data[3..11].copy_from_slice(b"TESTOEM1");
        data[11..13].copy_from_slice(&bps.to_le_bytes());
        data[13] = spc;
        data[14..16].copy_from_slice(&reserved.to_le_bytes());
        data[16] = nfats;
        data[17..19].copy_from_slice(&rootent.to_le_bytes());
        data[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
        data[22..24].copy_from_slice(&spf.to_le_bytes());
        data[32..36].copy_from_slice(&0u32.to_le_bytes());
        data
    }

    fn fresh_image() -> Fat16Image {
        Fat16Image::new(blank_image(64)).expect("blank image parses")
    }

    /// Creates `BASE_PARTS/generic/<carrier>/viettel/commerci/vietnam/` and
    /// writes an `mcfg_sw.mbn` of `mbn_len` bytes under it, plus empty
    /// `oem_sw.txt`/`mbn_sw.txt` at the base. Returns the image.
    fn image_with_viettel(carrier: &str, mbn_len: usize) -> Fat16Image {
        let mut fat = fresh_image();
        let mut cur: u32 = 0;
        for part in BASE_PARTS.iter().copied().chain(
            ["generic", carrier, "viettel", "commerci", "vietnam"]
                .iter()
                .copied(),
        ) {
            cur = fat.mkdir(cur, part).expect("mkdir").cluster;
        }
        let idx = fat.alloc_dirent(cur, 1).unwrap();
        let off = fat.slot_abs(cur, idx).unwrap();
        let blob = vec![0xABu8; mbn_len];
        let clusters = fat
            .alloc(
                ((mbn_len + fat.cluster_size() as usize - 1) / fat.cluster_size() as usize).max(1),
            )
            .unwrap();
        fat.link_chain(&clusters);
        fat.write_clusters(&clusters, &blob);
        fat.put_ent(
            off,
            &encode_83("mcfg_sw.mbn"),
            ATTR_ARC,
            clusters[0],
            blob.len() as u32,
        )
        .unwrap();

        // oem_sw.txt / mbn_sw.txt at BASE_PARTS, listing a couple of other
        // carriers ahead of Viettel, like a real whitelist would.
        let base = fat.find(&BASE_PARTS).unwrap();
        for name in ["oem_sw.txt", "mbn_sw.txt"] {
            let idx = fat.alloc_dirent(base.cluster, 1).unwrap();
            let off = fat.slot_abs(base.cluster, idx).unwrap();
            fat.put_ent(off, &encode_83(name), ATTR_ARC, 0, 0).unwrap();
            let entry = Entry {
                name: name.to_string(),
                attr: ATTR_ARC,
                cluster: 0,
                size: 0,
                dir_cluster: base.cluster,
                ent_index: idx,
            };
            fat.write_file(
                &entry,
                b"mcfg_sw/generic/row/other/commerci/thailand/mcfg_sw.mbn\r\n",
            )
            .unwrap();
        }
        fat
    }

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join("volte_core_modem_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    // -- boot sector validation --------------------------------------

    #[test]
    fn a_too_short_buffer_is_rejected() {
        let e = Fat16Image::new(vec![0u8; 100]).unwrap_err();
        assert_eq!(e.kind(), crate::error::ErrorKind::BadMagic);
    }

    #[test]
    fn a_zero_bytes_per_sector_is_rejected() {
        let mut data = blank_image(8);
        data[11..13].copy_from_slice(&0u16.to_le_bytes());
        assert!(Fat16Image::new(data).is_err());
    }

    #[test]
    fn a_non_power_of_two_sectors_per_cluster_is_rejected() {
        let mut data = blank_image(8);
        data[13] = 3;
        assert!(Fat16Image::new(data).is_err());
    }

    #[test]
    fn fewer_than_three_data_clusters_is_rejected() {
        let data = blank_image(2);
        assert!(Fat16Image::new(data).is_err());
    }

    #[test]
    fn a_blank_image_parses_and_reports_its_oem() {
        let fat = fresh_image();
        assert_eq!(fat.oem_label(), "TESTOEM1");
        assert_eq!(fat.cluster_size(), 512);
    }

    // -- A4: a truncated image must fail cleanly, never panic ----------

    #[test]
    fn a4_a_valid_image_is_still_accepted() {
        let data = blank_image(64);
        let len = data.len();
        let fat = Fat16Image::new(data).expect("a well-formed image must still parse");
        assert_eq!(fat.cluster_size(), 512);
        assert_eq!(
            fat.data.len(),
            len,
            "the data-region bound must accept an image whose data ends exactly at EOF"
        );
    }

    #[test]
    fn a4_a_truncated_data_region_is_rejected_by_new() {
        let fat = fresh_image();
        let full = fat.data.len();
        let mut data = fat.data;
        data.truncate(full / 2);
        let err = Fat16Image::new(data).unwrap_err();
        assert_eq!(
            err.kind(),
            crate::error::ErrorKind::BadMagic,
            "a data region cut in half must be rejected as a bad FAT16 image: {err}"
        );
    }

    #[test]
    fn a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking() {
        let mut fat = fresh_image();
        let mut cur = 0u32;
        for _ in 0..50 {
            cur = fat.mkdir(cur, "d").expect("mkdir").cluster;
        }
        let full = fat.data.len();
        fat.data.truncate(full / 2);
        assert!(
            fat.clus_off(cur) >= fat.data.len(),
            "fixture must place the last directory past the cut"
        );
        assert_eq!(
            fat.free_slot(cur, 1),
            None,
            "a slot outside the truncated buffer must be None, not an index panic"
        );
    }

    #[test]
    fn a4_fix_modem_from_mbn_on_a_truncated_image_returns_err() {
        let fat = fresh_image();
        let full = fat.data.len();
        let mut data = fat.data;
        data.truncate(full / 2);
        let dest = write_temp("truncated_modem.bin", &data);
        let mbn = vec![0x11u8; MBN_MIN_SIZE + 20];
        let mut log = Logger::new();
        let err = build_modem_fix_from_mbn(&dest, &mbn, None, &mut log).unwrap_err();
        assert_eq!(
            err.kind(),
            crate::error::ErrorKind::BadMagic,
            "fix-modem must refuse a truncated image before touching it: {err}"
        );
    }

    // -- 4b: walking must terminate on deep or cyclic directory trees -----

    #[test]
    fn a4b_deep_directories_are_walked_without_overflowing_the_stack() {
        let mut fat = fresh_image();
        let mut cur = 0u32;
        for _ in 0..50 {
            cur = fat.mkdir(cur, "d").expect("mkdir").cluster;
        }
        let idx = fat.alloc_dirent(cur, 1).unwrap();
        let off = fat.slot_abs(cur, idx).unwrap();
        fat.put_ent(off, &encode_83("leaf.bin"), ATTR_ARC, 0, 0)
            .unwrap();

        let found = fat.walk(0, &[]);
        assert_eq!(found.len(), 1, "the walk must reach the leaf exactly once");
        assert_eq!(found[0].1.len(), 51);
        assert_eq!(found[0].1.last().unwrap(), "LEAF.BIN");
    }

    #[test]
    fn a4b_cycle_guard_skips_a_self_referencing_directory() {
        let mut fat = fresh_image();
        let d = fat.mkdir(0, "cycle").expect("mkdir");
        let off = fat.clus_off(d.cluster);
        fat.put_ent(off, &[b' '; 11], ATTR_DIR, d.cluster, 0)
            .unwrap();

        let found = fat.walk(d.cluster, &[]);
        assert!(
            found.is_empty(),
            "a self-referencing directory must be skipped, not followed forever"
        );
    }

    // -- 8.3 / LFN name codecs -----------------------------------------

    #[test]
    fn decode_83_round_trips_a_simple_name() {
        let raw = encode_83("mcfg_sw.mbn");
        assert_eq!(decode_83(&raw), "MCFG_SW.MBN");
    }

    #[test]
    fn decode_83_handles_a_name_with_no_extension() {
        let raw = encode_83("viettel");
        assert_eq!(decode_83(&raw), "VIETTEL");
    }

    #[test]
    fn part_match_is_case_insensitive_and_exact() {
        assert!(part_match("Viettel", "VIETTEL"));
        assert!(!part_match("viettel2", "viettel"));
        assert!(!part_match("", "viettel"));
    }

    // -- oem_sw.txt / mbn_sw.txt rewrite --------------------------------

    #[test]
    fn viettel_line_moves_to_the_front_and_dedups() {
        let blob = b"mcfg_sw/generic/row/a/commerci/x/mcfg_sw.mbn\r\n\
mcfg_sw/generic/sea/viettel/commerci/vietnam/mcfg_sw.mbn\r\n\
mcfg_sw/generic/row/b/commerci/y/mcfg_sw.mbn\r\n";
        let (out, had) = oem_put_viettel_first(blob, VIETTEL_REL);
        assert!(had);
        let text = latin1_decode(&out);
        let lines: Vec<&str> = text.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert_eq!(lines[0], VIETTEL_REL);
        assert_eq!(
            lines.len(),
            3,
            "the duplicate Viettel line must not survive"
        );
    }

    #[test]
    fn viettel_line_is_added_when_absent_and_lf_is_preserved() {
        let blob = b"mcfg_sw/generic/row/a/commerci/x/mcfg_sw.mbn\n";
        let (out, had) = oem_put_viettel_first(blob, VIETTEL_REL);
        assert!(!had);
        let text = latin1_decode(&out);
        assert!(text.starts_with(VIETTEL_REL));
        assert!(
            !text.contains("\r\n"),
            "LF input must stay LF, not become CRLF"
        );
    }

    #[test]
    fn empty_oem_blob_still_gets_the_viettel_line() {
        let (out, had) = oem_put_viettel_first(b"", VIETTEL_REL);
        assert!(!had);
        assert_eq!(latin1_decode(&out), format!("{VIETTEL_REL}\n"));
    }

    // -- directory tree: mkdir / find / write_file ----------------------

    #[test]
    fn mkdir_is_idempotent_and_find_resolves_the_path() {
        let mut fat = fresh_image();
        let mut cur = 0u32;
        for part in BASE_PARTS {
            cur = fat.mkdir(cur, part).unwrap().cluster;
        }
        // Calling mkdir again on an existing directory must not allocate a
        // second entry or a new cluster chain.
        let again = fat.mkdir(0, BASE_PARTS[0]).unwrap();
        let first = fat.find(&BASE_PARTS[..1]).unwrap();
        assert_eq!(again.cluster, first.cluster);
        assert!(fat.find(&BASE_PARTS).is_some());
    }

    #[test]
    fn write_file_then_read_file_round_trips() {
        let mut fat = fresh_image();
        let base = fat.mkdir(0, "oem_sw.txt_holder").unwrap();
        let idx = fat.alloc_dirent(base.cluster, 1).unwrap();
        let off = fat.slot_abs(base.cluster, idx).unwrap();
        fat.put_ent(off, &encode_83("file.txt"), ATTR_ARC, 0, 0)
            .unwrap();
        let entry = Entry {
            name: "FILE.TXT".to_string(),
            attr: ATTR_ARC,
            cluster: 0,
            size: 0,
            dir_cluster: base.cluster,
            ent_index: idx,
        };
        let content = b"hello viettel";
        let updated = fat.write_file(&entry, content).unwrap();
        assert_eq!(fat.read_file(&updated), content);

        // Shrinking then growing again must not corrupt the content.
        let updated = fat.write_file(&updated, b"x").unwrap();
        assert_eq!(fat.read_file(&updated), b"x");
        let bigger = vec![0x42u8; fat.cluster_size() as usize * 3 + 10];
        let updated = fat.write_file(&updated, &bigger).unwrap();
        assert_eq!(fat.read_file(&updated), bigger);
    }

    #[test]
    fn image_size_never_changes_across_writes() {
        let mut fat = fresh_image();
        let before = fat.data.len();
        let base = fat.mkdir(0, "d").unwrap();
        let idx = fat.alloc_dirent(base.cluster, 1).unwrap();
        let off = fat.slot_abs(base.cluster, idx).unwrap();
        fat.put_ent(off, &encode_83("f.bin"), ATTR_ARC, 0, 0)
            .unwrap();
        let entry = Entry {
            name: "F.BIN".into(),
            attr: ATTR_ARC,
            cluster: 0,
            size: 0,
            dir_cluster: base.cluster,
            ent_index: idx,
        };
        let big = vec![7u8; fat.cluster_size() as usize * 4];
        fat.write_file(&entry, &big).unwrap();
        assert_eq!(
            fat.data.len(),
            before,
            "write_file must not resize the image"
        );
    }

    // -- Viettel discovery -----------------------------------------------

    #[test]
    fn find_viettel_mbn_finds_a_large_enough_sea_entry() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE + 10);
        let (entry, rel) = fat.find_viettel_mbn();
        let entry = entry.expect("viettel mbn should be found");
        assert_eq!(entry.size as usize, MBN_MIN_SIZE + 10);
        assert_eq!(rel, "generic/sea/viettel/commerci/vietnam/mcfg_sw.mbn");
    }

    #[test]
    fn find_viettel_mbn_rejects_an_entry_at_or_below_the_minimum_size() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE);
        let (entry, _rel) = fat.find_viettel_mbn();
        assert!(
            entry.is_none(),
            "exactly MBN_MIN_SIZE must still count as missing"
        );
    }

    #[test]
    fn find_viettel_mbn_also_checks_the_row_carrier() {
        let fat = image_with_viettel("row", MBN_MIN_SIZE + 1);
        let (entry, rel) = fat.find_viettel_mbn();
        assert!(entry.is_some());
        assert_eq!(rel, "generic/row/viettel/commerci/vietnam/mcfg_sw.mbn");
    }

    #[test]
    fn list_sea_carriers_lists_only_directories() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE + 1);
        let carriers = fat.list_sea_carriers();
        assert!(carriers.iter().any(|c| c.eq_ignore_ascii_case("viettel")));
    }

    // -- end-to-end BUILD MODEM / BUILD FIX MODEM ------------------------

    #[test]
    fn build_modem_volte_moves_viettel_first_and_keeps_the_image_size() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE + 50);
        let src = write_temp("has_viettel.bin", &fat.data);
        let src_len = fat.data.len();
        let mut log = Logger::new().with_sink(MemorySink::default());

        let report = build_modem_volte(&src, None, &mut log).expect("build modem");
        assert_eq!(report.size as usize, src_len);
        assert!(!report.already_had_viettel_first);

        let out_bytes = std::fs::read(&report.dst).unwrap();
        let out_fat = Fat16Image::new(out_bytes).unwrap();
        let oem_entry = out_fat.find(&OEM_SW_PARTS).unwrap();
        let oem_text = latin1_decode(&out_fat.read_file(&oem_entry));
        assert!(oem_text.starts_with(VIETTEL_REL));

        // Source file on disk must be untouched (A01-style guard holds even
        // though nothing here forced a collision).
        assert_eq!(std::fs::read(&src).unwrap().len(), src_len);
    }

    #[test]
    fn build_modem_volte_refuses_a_modem_without_viettel() {
        let fat = fresh_image();
        let src = write_temp("no_viettel.bin", &fat.data);
        // No oem_sw.txt either, so this must fail on the oem_sw.txt check
        // before ever reaching the "no Viettel MBN" branch.
        let mut log = Logger::new();
        let err = build_modem_volte(&src, None, &mut log).unwrap_err();
        assert!(err.message().contains("oem_sw.txt"));
    }

    #[test]
    fn build_modem_volte_refuses_out_equal_to_source() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE + 10);
        let src = write_temp("collide.bin", &fat.data);
        let mut log = Logger::new();
        let err = build_modem_volte(&src, Some(src.as_path()), &mut log).unwrap_err();
        assert!(err.message().contains("Refusing to overwrite"));
    }

    #[test]
    fn build_modem_fix_from_mbn_injects_a_missing_viettel_mbn() {
        let mut fat = fresh_image();
        // Build the oem_sw.txt / mbn_sw.txt base tree without any Viettel
        // directory at all, so inject_mbn has to create it from scratch.
        let mut cur = 0u32;
        for part in BASE_PARTS {
            cur = fat.mkdir(cur, part).unwrap().cluster;
        }
        let idx = fat.alloc_dirent(cur, 1).unwrap();
        let off = fat.slot_abs(cur, idx).unwrap();
        fat.put_ent(off, &encode_83("oem_sw.txt"), ATTR_ARC, 0, 0)
            .unwrap();
        let oem_entry = Entry {
            name: "OEM_SW.TXT".into(),
            attr: ATTR_ARC,
            cluster: 0,
            size: 0,
            dir_cluster: cur,
            ent_index: idx,
        };
        fat.write_file(
            &oem_entry,
            b"mcfg_sw/generic/row/a/commerci/x/mcfg_sw.mbn\r\n",
        )
        .unwrap();

        let dest = write_temp("missing_viettel.bin", &fat.data);
        let dest_len = fat.data.len();
        let mbn_blob = vec![0x11u8; MBN_MIN_SIZE + 20];
        let mut log = Logger::new();

        let report = build_modem_fix_from_mbn(&dest, &mbn_blob, None, &mut log).expect("fix build");
        assert_eq!(report.size as usize, dest_len);
        assert!(report.dst.to_string_lossy().contains("_volte_fix"));

        let out_bytes = std::fs::read(&report.dst).unwrap();
        let out_fat = Fat16Image::new(out_bytes).unwrap();
        let (vent, _rel) = out_fat.find_viettel_mbn();
        let vent = vent.expect("the injected mbn must now be found");
        assert_eq!(out_fat.read_file(&vent), mbn_blob);
    }

    #[test]
    fn build_modem_fix_from_mbn_rejects_a_too_small_blob() {
        let dest = write_temp("whatever.bin", &fresh_image().data);
        let mut log = Logger::new();
        let err = build_modem_fix_from_mbn(&dest, &[0u8; 10], None, &mut log).unwrap_err();
        assert!(err.message().contains("too small"));
    }

    #[test]
    fn build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image() {
        let donor_fat = image_with_viettel("sea", MBN_MIN_SIZE + 30);
        let donor = write_temp("donor.bin", &donor_fat.data);

        let mut dest_fat = fresh_image();
        let mut cur = 0u32;
        for part in BASE_PARTS {
            cur = dest_fat.mkdir(cur, part).unwrap().cluster;
        }
        let idx = dest_fat.alloc_dirent(cur, 1).unwrap();
        let off = dest_fat.slot_abs(cur, idx).unwrap();
        dest_fat
            .put_ent(off, &encode_83("oem_sw.txt"), ATTR_ARC, 0, 0)
            .unwrap();
        let oem_entry = Entry {
            name: "OEM_SW.TXT".into(),
            attr: ATTR_ARC,
            cluster: 0,
            size: 0,
            dir_cluster: cur,
            ent_index: idx,
        };
        dest_fat
            .write_file(
                &oem_entry,
                b"mcfg_sw/generic/row/a/commerci/x/mcfg_sw.mbn\r\n",
            )
            .unwrap();
        let dest = write_temp("fix_target.bin", &dest_fat.data);
        let mut log = Logger::new();

        let report =
            build_modem_fix_viettel(&dest, &donor, None, &mut log).expect("fix from donor");
        let out_bytes = std::fs::read(&report.dst).unwrap();
        let out_fat = Fat16Image::new(out_bytes).unwrap();
        assert!(out_fat.find_viettel_mbn().0.is_some());
    }

    #[test]
    fn build_modem_fix_viettel_refuses_a_donor_without_viettel() {
        let donor = write_temp("donor_no_viettel.bin", &fresh_image().data);
        let dest = write_temp("dest_whatever.bin", &fresh_image().data);
        let mut log = Logger::new();
        let err = build_modem_fix_viettel(&dest, &donor, None, &mut log).unwrap_err();
        assert!(err.message().contains("Donor modem has no Viettel MBN"));
    }

    #[test]
    fn looks_like_qc_modem_is_true_only_for_a_parsable_image() {
        let good = write_temp("good.bin", &fresh_image().data);
        let bad = write_temp("bad.bin", b"not a modem image at all");
        assert!(looks_like_qc_modem(&good));
        assert!(!looks_like_qc_modem(&bad));
        assert!(!looks_like_qc_modem(Path::new("/nonexistent/path.bin")));
    }

    #[test]
    fn scan_modem_profiles_reports_viettel_presence() {
        let fat = image_with_viettel("sea", MBN_MIN_SIZE + 5);
        let src = write_temp("scan_me.bin", &fat.data);
        let mut log = Logger::new();
        let report = scan_modem_profiles(&src, &mut log).expect("scan");
        assert!(report.has_viettel_mbn);
        assert_eq!(report.viettel_size, (MBN_MIN_SIZE + 5) as u64);
        assert!(report
            .carriers
            .iter()
            .any(|c| c.eq_ignore_ascii_case("viettel")));
    }

    #[test]
    fn default_dst_appends_the_suffix_before_the_extension() {
        assert_eq!(
            default_dst(Path::new("modem.bin"), "_volte"),
            PathBuf::from("modem_volte.bin")
        );
        assert_eq!(
            default_dst(Path::new("modem"), "_volte_fix"),
            PathBuf::from("modem_volte_fix.bin")
        );
    }
}
