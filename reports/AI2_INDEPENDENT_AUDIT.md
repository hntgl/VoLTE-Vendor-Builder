# AI2 Independent Audit — VoLTE-Vendor-Builder

**Audit type:** independent, read-only source and behaviour audit
**Audit date:** 2026-11-07
**Repository:** `E:\Build_app\Volte\VoLTE-Vendor-Builder`
**Workspace state:** no `.git` directory; `CLAUDE.md` present; `VoLTE-Vendor-Builder.zip` absent at audit time
**Toolchain:** `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0`, `rustfmt 1.10.0-stable`, `clippy 0.1.99`
**Auditor rule:** no file inside the repository was modified. All reproduction code lives in an external Cargo project at `C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro`.

---

## 1. Verdict and executive summary

### Verdict: **NOT READY**

The project compiles, passes clippy with `-D warnings`, and all 145 existing tests pass. However, the audit reproduced **four Critical data-integrity / silent-success defects** and **seven High-severity defects** that can destroy the user's source image, delete files inside the output image while reporting success, or report an unverified/old AVB footer as a successful rebuild. Several of these are on the default CLI path (`RustFsExt4Backend`), not only on test-only seams.

### Finding counts

| Status | Count |
|---|---:|
| Reproduced | 11 |
| Confirmed (static evidence) | 13 |
| Needs verification | 0 findings (5 unverified areas, §12) |
| False positive / rejected | 1 |
| **Total** | **25** |

| Severity | Count |
|---|---:|
| Critical | 4 |
| High | 7 |
| Medium | 7 |
| Low | 6 |
| Informational | 1 |
| **Total** | **25** |

### Critical and High findings (short list)

| ID | Severity | Status | Title |
|---|---|---|---|
| A01 | Critical | Reproduced | `--out` equal to the source truncates the source to 0 bytes |
| A01b | Critical | Reproduced | Case-only output alias deletes the source on Windows |
| C01 | Critical | Reproduced | Default ext4 backend returns `Ok` after `rm` + failed `write`; data loss reported as success |
| D01 | Critical | Reproduced | Failing AVB backend over an existing footer returns `Ok(Some)` and logs success |
| A02 | High | Confirmed | Pre-AVB backup is never verified; failed restore only warns and the build continues |
| A04 | High | Reproduced | `e2fsck` failure still returns `Ok`; summary starts with `[OK] EDL size matches`; CLI exits 0 |
| B01 | High | Reproduced | Sparse image ending in `DONT_CARE` produces a short/empty output file |
| C02 | High | Reproduced | `split_whitespace` breaks every host path containing a space |
| D03 | High | Confirmed | `fill_from_vbmeta` never recovers `salt`, FEC roots, props, or the real partition name |
| E01 | High | Reproduced | Stale host dump files are reported as `Verified` with the stale file's hash |
| G01 | High | Reproduced | Root-level `/init.target.rc` is deleted and rewritten under `/etc/init/hw` while the log claims success |

### Why NOT READY

1. A normal user can pass `--out` pointing at the input file (or a case-variant on Windows) and lose the input. This is not an exotic API misuse: `--out` is a documented CLI flag.
2. The default CLI backend (`RustFsExt4Backend`, `volte-cli/src/main.rs:180`) intentionally swallows per-command failures. A failed write after an `rm` leaves the file deleted, and the pipeline continues.
3. `e2fsck=FAIL` and a stale/failed AVB rebuild can both end in exit code 0 with `[OK]` summary lines. A caller cannot distinguish a good image from a damaged one.
4. The test suite does not exercise the default backend in the pipeline integration test, uses a no-op AVB backend, and validates payloads by size only. The passing test count therefore does not cover the failure modes above.

The fix set is concrete and local (input/output identity check, fail-closed command execution, propagate fsck/verify failures, stricter sparse validation). Until those land, the tool should not be used on irreplaceable images.

---

## 2. Scope, constraints, and method

### In scope

- All Rust source in `volte-core` (20 modules) and `volte-cli`.
- All Cargo manifests and `Cargo.lock`.
- README, `docs/PORT_STATUS.md`, `tests/README.md`, `fixtures/README.md`, profiles.
- Behavioural comparison with the Python reference `E:\Build_app\Volte\VoLTE\vendor_patcher\hd_vendor_build.py` and the authoritative `E:\Build_app\Volte\VoLTE\vendor_patcher\tool\avbtool.py` where needed.

### Out of scope / not performed

- No source, manifest, test, or configuration edits.
- No writes to the repository except this `reports/` directory.
- No flashing to a real device and no destructive disk-full or concurrency experiments.
- No dependency CVE scan: `cargo-audit`, `cargo-deny`, and `cargo-nextest` are not installed.
- No Miri: the component is unavailable for `stable-x86_64-pc-windows-msvc`.

### Method

1. Read the whole Rust codebase and the relevant Python reference paths.
2. Re-derive each prior finding from current code; never assume the prior wording was correct.
3. Build an external repro crate that depends on `volte-core` by path and exercises public APIs with synthetic ext4/sparse/AVB data.
4. Reproduce only non-destructive cases. Mark everything else `Confirmed (static)` or `Needs verification`.
5. Run the full build/test/lint gates on the current tree.
6. Classify severity by user-visible impact, not by code elegance.

### Evidence-class definitions used in this report

| Class | Meaning |
|---|---|
| **Reproduced** | An external test executed the current code and observed the defect. |
| **Confirmed (static)** | The defect is provable from the current code path; a safe repro was not possible or not necessary. |
| **Needs verification** | Evidence is insufficient or the trigger is environmental. |
| **False positive** | The claimed defect does not hold against current code or the authoritative reference. |

---

## 3. Repository and build inventory

### Workspace

```
VoLTE-Vendor-Builder/
├─ Cargo.toml                 workspace: volte-core, volte-cli; [patch] rust-fs-core
├─ Cargo.lock
├─ README.md
├─ CLAUDE.md
├─ docs/PORT_STATUS.md
├─ profiles/                  (README only at audit time)
├─ fixtures/                  (README only at audit time)
├─ tests/                     (README only at audit time)
├─ reports/                   created by this audit
├─ volte-core/                20 modules, 5,871 lines
└─ volte-cli/                 main.rs, 290 lines
```

Total Rust source: **6,161 lines** across 21 files (excluding `target/`).

### Dependency surface

| Crate | Source |
|---|---|
| `rust-fs-ext4` | git `antimatter-studios/rust-fs-ext4`, rev `1aaea0793baba99c69488c02c2b510adb14bb501` |
| `rust-fs-core` | git `antimatter-studios/rust-fs-core`, rev `3c2340a047f5ddfac2837d0c61d327ff4f4a9247` |
| external tools | `e2fsck.exe`, `mke2fs.exe`, `debugfs.exe`, bundled Python, `avbtool.py` |

`cargo tree -d` reports no duplicate dependency versions.

### Default execution path

`volte-cli/src/main.rs:180` selects `RustFsExt4Backend`. The pipeline integration test, by contrast, uses `DebugfsBackend`. This is the single most important test/production mismatch in the project (see §9).

---

## 4. Toolchain and command results

All commands were run in `E:\Build_app\Volte\VoLTE-Vendor-Builder` on the current tree.

| Command | Exit | Result |
|---|---:|---|
| `cargo check --workspace --all-targets` | 0 | Success |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 | Success, zero warnings |
| `cargo test --workspace` | 0 | **145 passed**: 10 CLI + 135 `volte-core`; 0 failed, 0 ignored |
| `cargo fmt --all -- --check` | 1 | **FAIL**: 110 diff blocks across the workspace (style only) |
| `cargo tree -d` | 0 | No duplicate dependencies |
| `cargo audit` | — | Not installed |
| `cargo deny` | — | Not installed |
| `cargo nextest` | — | Not installed |
| `cargo miri --version` | 1 | `miri` component unavailable for stable MSVC |

### Interpretation

- **Build and lint are green.** No compiler or clippy warning blocks a release.
- **Formatting is not green.** 110 blocks is not a one-line drift; it indicates `cargo fmt` is not part of the project's gate. This is a tooling/process finding, not a correctness finding.
- **Test count is not coverage.** 145 tests pass, but the behaviour that ships (default Rust ext4 backend) is not the behaviour under integration test, and AVB is replaced by a no-op in the pipeline test.
- **Dependency risk is unknown.** With no `cargo-audit`/`cargo-deny`, the git-pinned `rust-fs-ext4`/`rust-fs-core` revisions have not been checked against advisories.

---

## 5. Agent Skills used

### Loaded and read

| Skill | Use in this audit |
|---|---|
| `rust-code-review` | Two-axis review (standards + spec), severity calibration, smell baseline |
| `rust-testing` | Test-quality assessment: unit vs integration, fixtures, differential parity, golden tests |
| `sota-rust` | Error/panic policy, unsafe discipline, supply chain, async/process handling, release hardening |
| `type-driven-design` | Illegal-state analysis for output paths, backends, AVB state, and "success" tokens |

### Found but only partially read

| Resource | Note |
|---|---|
| `SMELLS.md` (repository smell baseline referenced by the review skill) | Read partially; the detailed findings below are derived from current code directly, not copied from it |

### Not applicable / not used

The remaining available skills (Axum, GraphQL, gRPC, TUI, Python reverse engineering, report publishing, port-from-Python, async-rust) do not match a pure Rust CLI/static-image audit. `port-to-rust`/`rust-port` were not needed because the Python reference was used only as behavioural evidence, not as a porting guide.

---

## 6. Prior findings A01–F02: independent verification

Status legend: **R** = Reproduced, **C** = Confirmed (static), **FP** = False positive.

| ID | Severity | Status | Exact location | Verdict in one line |
|---|---|---|---|---|
| A01 | Critical | R | `pipeline.rs:119-142`, `176-202`, `317-333` | `--out` == source: source is truncated to 0, build fails after the damage |
| A01b | Critical | R | `pipeline.rs:317-325` | Case-only alias bypasses `absolute(out) != src`; `remove_file` deletes the source |
| A02 | High | C | `pipeline.rs:495-522` | Backup success is only `.is_ok()`; no size/hash check; failed restore warns and continues |
| A03 | Medium | C | `pipeline.rs:151-164`, `246-263` | Space probe underestimates peak use (copy + temp + AVB growth) |
| A04 | High | R | `pipeline.rs:481-492`, `552-571`; `report.rs:72` | `e2fsck=FAIL` still returns `Ok` and prints `[OK] EDL size matches`; CLI exit 0 |
| B01 | High | R | `sparse.rs:109-113` | Trailing `DONT_CARE` seeks but does not extend; output shorter than `written` |
| B02 | Medium | C | `sparse.rs:56-80`, `109-113` | Header/version/block-size/chunk-size/final-size not validated; `n as i64` can overflow |
| C01 | Critical | R | `rustfs_ext4.rs:244-271`, `106-167`; `props.rs:137-146`; `init_rc.rs:97-127` | Per-command failures are text; `rm` + failed `write` returns `Ok` |
| C02 | High | R | `rustfs_ext4.rs:111` | `split_whitespace` truncates host paths containing spaces; props silently not written |
| D01 | Critical | R | `avb.rs:398-416` | Failed backend over an old footer returns `Ok(Some)` and logs "completed" |
| D02 | Low | **FP** | `avb.rs:274-276` | `--do_not_generate_fec` + `--fec_num_roots` are compatible in `avbtool.py`; only the log text is misleading |
| D03 | High | C | `avb.rs:122-148` | `salt`, FEC roots, props never parsed; partition name defaults to `vendor` |
| D04 | Low | C | `avb.rs:218-236` | Salt is 64-bit SipHash chunks + timestamp, not `os.urandom(32)` |
| E01 | High | R | `verify.rs:71-130` | Stale host files pass the size threshold and are hashed as if freshly dumped |
| E02 | Medium | C | `payload.rs:114-145`; `verify.rs:93-119`; `props.rs:220` | Duplicate writes and stale copies; host copy errors propagate, image-side write failures are swallowed |
| F01 | Medium | R | `props.rs:232` | Byte slice at `min(120)` panics on a multibyte UTF-8 boundary |
| F02 | Medium | C | `props.rs:165-220` | Lossy UTF-8, CRLF normalization, duplicate keys kept, mode/uid/gid reset, blanket `ea_set` |

### Notes on the reconstruction limit

No prior audit report exists in the repository at audit time (`reports/` was absent), and the workspace is not a git repository. The prior findings were reconstructed from the audit brief/checkpoint. Their *claims* were re-derived from current code; the original wording cannot be quoted and is not relied on.

---

## 7. Detailed findings — prior set (11-part template)

Each finding below uses the same 11-part template:
**ID / title** (heading), **severity**, **status**, **exact location**, **current code**, **trigger**, **observed behaviour/evidence**, **expected behaviour**, **impact**, **fix recommendation**.

---

### A01 — `--out` equal to the source truncates the source to zero bytes

1. **Severity:** Critical
2. **Status:** Reproduced
3. **Location:** `volte-core/src/pipeline.rs:119-142` (`resolve_output`), `176-202` (`copy_with_progress`), `317-333` (collision check + copy)
4. **Current code:**
```rust
// resolve_output, keep_out branch — no comparison with `src`
if keep_out && !out.is_empty() {
    let path = std::path::absolute(out)?;
    ...
    return Ok((folder, path));
}

// copy_with_progress
let mut fi = std::fs::File::open(src)?;
let mut fo = std::fs::File::create(dst)?;   // truncates dst

// caller
if out_path.is_file() && std::path::absolute(&out_path)? != src {
    ... remove_file ...
}
copy_with_progress(&work_src, &out_path, progress)?;
```
5. **Trigger:** `volte-cli build vendor.img --out vendor.img` (or any `--out` that resolves to the same file). `keep_out = !a.out.is_empty()` is always true when `--out` is given, so the guard in `resolve_output` is skipped.
6. **Observed behaviour / evidence:** External repro `a01_out_equals_src_truncates_the_source`:
   - `before=4096`, `after=0`
   - pipeline returns `Err` whose message contains `Copy vendor failed (disk/file is open).`
   - Source is already destroyed by `File::create` before the copy loop reads it.
7. **Expected behaviour:** Detect that the destination is the source **before** opening/truncating anything; abort with a clear error and leave the source untouched. If `--out` is the source, either refuse or write to `_VolLTE_out.img`.
8. **Impact:** Irreversible loss of the user's input image. This is the highest-impact defect in the project because it occurs through a documented flag with no warning.
9. **Fix recommendation:**
   - Canonicalize both paths (`std::fs::canonicalize` when they exist; on Windows compare case-insensitively) in `resolve_output` for **both** branches.
   - Refuse `out == src` (or auto-select `_VolLTE_out.img`) before `File::create`.
   - Add a regression test that asserts the source bytes are unchanged after the error.

---

### A01b — Case-only output alias deletes the source on Windows

1. **Severity:** Critical
2. **Status:** Reproduced
3. **Location:** `volte-core/src/pipeline.rs:317-325`
4. **Current code:**
```rust
if out_path.is_file() && std::path::absolute(&out_path)? != src {
    if let Err(e) = std::fs::remove_file(&out_path) { ... }
}
copy_with_progress(&work_src, &out_path, progress)?;
```
5. **Trigger:** On a case-insensitive filesystem, `--out VENDOR.IMG` while the source is `vendor.img`. `PathBuf` equality is case-sensitive, so the guard thinks they differ and calls `remove_file` on what is actually the source.
6. **Observed behaviour / evidence:** External repro `a01b_case_alias_out_deletes_the_source`:
   - `result=Err(...)`
   - `source_still_exists=false`
   - the subsequent open fails with `Io` OS error 2.
7. **Expected behaviour:** Same as A01: identity must be decided by the filesystem's own identity rules, not by `PathBuf` string equality.
8. **Impact:** Source deletion with no output produced. A user can lose the only copy even though the source filename differs in case.
9. **Fix recommendation:** Compare canonical paths and, on Windows, case-fold them; additionally use file identity (`GetFileInformationByHandle` / `same-file` crate) or open the source first and compare device/inode equivalents. Do not call `remove_file` on a path that may alias the source.

---

### A02 — Pre-AVB backup is unverified; restore failure only warns

1. **Severity:** High
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/pipeline.rs:495-522`
4. **Current code:**
```rust
let pre_avb_ok = std::fs::copy(&work, &pre_avb).is_ok();
...
if !boot_safe {
    if pre_avb_ok {
        if let Err(e) = std::fs::copy(&pre_avb, &work) {
            log.log(format!("[WARN] Restore pre-AVB: {}", ...));
        }
    }
    slot::force_slot_size(&work, slot_size, true, "BOOT-SAFE", log)?;
    skip_avb = true;
}
```
5. **Trigger:** Disk-full/IO error during the pre-AVB copy, or a truncated backup; later the AVB rebuild fails and the code restores from the damaged backup.
6. **Observed behaviour / evidence:** Static path: `.is_ok()` checks only that `copy` returned `Ok`, not that the byte count/`metadata().len()` equals the source or that a hash matches. A failed restore is logged at `[WARN]` and execution continues to `force_slot_size` and then returns `Ok`.
7. **Expected behaviour:** Verify backup size/hash before proceeding; if the backup cannot be verified, abort before touching the image. If restore fails, return `Err` and never emit an `[OK]` summary.
8. **Impact:** The output can contain a partially restored image (old footer + patched data, or truncated data) while the build reports success. A device may bootloop, or the user may flash a corrupt vendor partition.
9. **Fix recommendation:** Hash the backup, compare size, and fail closed. Wrap the restore in a checked transaction: write to a side file, `fsync`, rename over `work`, and propagate any error as fatal.

---

### A03 — Disk-space probe underestimates peak usage

1. **Severity:** Medium
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/pipeline.rs:151-164`, `246-263`
4. **Current code:**
```rust
fn probe_space(dir: &Path, needed: u64) -> std::io::Result<()> {
    ...
    let res = file.set_len(needed);
    ...
}
...
probe_space(&folder, orig_sz)
probe_space(&std::env::temp_dir(), orig_sz)
```
5. **Trigger:** A source image close to free space, especially a sparse image whose `orig_sz` is the compressed size but whose unsparsed output is much larger.
6. **Observed behaviour / evidence:** The probe checks `orig_sz` in the output folder and `orig_sz` on the temp drive independently. It does not account for: (a) the copy existing while the source still exists, (b) the unsparsed temp file, (c) AVB padding/hashtree growth, (d) FEC/hashtree scratch. Peak need is roughly 2–3× the source. Sparse sources make `orig_sz` an underestimate before `unsparse`.
7. **Expected behaviour:** Compute the post-unsparse size from the sparse header (`total_blks × blk_sz`), reserve space for copy + temp + AVB growth, and fail before starting.
8. **Impact:** The build runs for minutes and then fails mid-write, potentially after the output file has been partially created. With A02, a failed AVB restore can then produce a corrupt output.
9. **Fix recommendation:** Parse the sparse header before the probe, use the uncompressed size, and add an explicit headroom factor. Keep the probe best-effort only for the warning path; make the fatal decision conservative.

---

### A04 — `e2fsck` failure still returns `Ok`; summary starts with `[OK]`

1. **Severity:** High
2. **Status:** Reproduced
3. **Location:** `volte-core/src/pipeline.rs:480-492`, `552-571`; `volte-core/src/report.rs:72`; `volte-cli/src/main.rs:195-204`
4. **Current code:**
```rust
Err(e) => {
    fsck_status = "FAIL";
    log.log(format!("[WARN] e2fsck: {}", ...));
    log.log("e2fsck=FAIL");
}
...
let summary = report::Summary { e2fsck: fsck_status, ... };
for line in report::summary_lines(&summary) { log.log(line); }
...
Ok(work)
```
```rust
// report.rs:72 — unconditional
"[OK] EDL size matches - UnlockTool can flash it.".to_string(),
```
5. **Trigger:** `e2fsck -fn` returns non-zero (filesystem inconsistency, damaged image, or missing tool).
6. **Observed behaviour / evidence:** External repro `a04_fsck_failure_still_returns_ok` with `AlwaysFailFsck`:
   - `pipeline_ok=true`
   - `e2fsck_fail_logged=true`
   - `edl_ok_logged=true`
   - CLI maps `Ok` to exit code 0.
7. **Expected behaviour:** A failed fsck must make the build fail (or at minimum return a non-zero exit code and suppress `[OK]` lines). The report should say `e2fsck=FAIL` and not claim the image is flashable.
8. **Impact:** A user can flash a filesystem that e2fsck explicitly rejected. Automation sees exit 0 and publishes the artifact.
9. **Fix recommendation:** Make `Fsck` failure fatal by default; if the Python parity requires continuing, add an explicit `--allow-fsck-fail` opt-in and change the exit code/summary. Make `summary_lines` take the status into account instead of hard-coding `[OK]`.

---

### B01 — Sparse image ending in `DONT_CARE` loses the tail

1. **Severity:** High
2. **Status:** Reproduced
3. **Location:** `volte-core/src/sparse.rs:109-113`
4. **Current code:**
```rust
DONT_CARE => {
    let n = csize * blk_sz;
    out.seek(SeekFrom::Current(n as i64))?;
    written += n;
}
```
5. **Trigger:** A valid sparse image whose last chunk (or entire content) is `DONT_CARE`/`0xCAC3`.
6. **Observed behaviour / evidence:** External repro `b01_trailing_dont_care_loses_the_tail` with `total_blks=4`, one `DONT_CARE` chunk:
   - `written=16384`
   - `actual=0`
   - `assert_ne!(actual, written)` passes, proving the output is shorter than the declared size.
7. **Expected behaviour:** The unsparsed file must be exactly `total_blks × blk_sz` bytes. A trailing hole must extend the file (`set_len`/`seek` + write a zero at the end), not just move the cursor.
8. **Impact:** A sparse vendor image with a hole at the end produces a truncated output. The downstream ext4/AVB logic then operates on a short file; the final size checks may fail or, worse, pass with missing data.
9. **Fix recommendation:** After the chunk loop, call `out.get_mut().set_len(written)` (or write a zero byte at `written - 1`) and verify the file length equals `written`. Add a dedicated regression test.

---

### B02 — Sparse header and sizes are insufficiently validated

1. **Severity:** Medium
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/sparse.rs:56-80`, `109-113`
4. **Current code:**
```rust
let file_hdr_sz = ...;
let chunk_hdr_sz = ...;
let blk_sz = ...;
let total_chunks = ...;
if magic != SPARSE_MAGIC { return Ok(NotSparse); }
if file_hdr_sz > FILE_HEADER_LEN { seek(...) }
...
let csize = ...;            // no upper bound
match ctype { ... DONT_CARE => out.seek(... n as i64) ... }
```
5. **Trigger:** Malformed/malicious sparse images: `blk_sz = 0`, `chunk_hdr_sz < 12`, huge `csize`, `total_blks` inconsistent with chunk data, or `n > i64::MAX`.
6. **Observed behaviour / evidence:** Static: the sparse version (`major`/`minor`) is never checked; `total_blks` (header bytes 16–19) is read but unused; `blk_sz == 0` is accepted; `csize * blk_sz` is not bounded by `total_blks` or a sane maximum; `n as i64` wraps negative for huge `DONT_CARE`, and `SeekFrom::Current` then moves backwards.
7. **Expected behaviour:** Validate version, `blk_sz` range, header sizes, chunk count, and cumulative output against `total_blks × blk_sz`; reject before allocating or seeking.
8. **Impact:** Malformed input can produce a wrong-size image or an unexpected allocation. On 32-bit targets, `csize * blk_sz` can overflow `u64` math in the buffer loops.
9. **Fix recommendation:** Use `checked_mul`/`checked_add`, validate `blk_sz` against a supported set, reject `chunk_hdr_sz < 12`, and compare the final `written` to the header's total size. Treat a mismatch as a hard error.

---

### C01 — Default ext4 backend returns `Ok` after `rm` + failed `write`

1. **Severity:** Critical
2. **Status:** Reproduced
3. **Location:** `volte-core/src/rustfs_ext4.rs:244-271` (`run`), `106-167` (`execute`); consumers `volte-core/src/props.rs:137-146`, `volte-core/src/init_rc.rs:97-127`; `volte-cli/src/main.rs:180`
4. **Current code:**
```rust
for command in commands {
    if let Err(message) = session.execute(command, workdir) {
        out.push_str(&format!("debugfs: {command}: {message}\n"));
    }
}
session.fs.finish().map_err(map_fs)?;
Ok(out)                         // success even if every command failed
```
```rust
// props::write_back
let cmds = vec![
    format!("rm {vpath}"),
    format!("write {} {name}", forward_slashes(host)),
    ...
];
ops.run(image, &cmds, td, &mut quiet).map(|_| ())
```
5. **Trigger:** Any per-command failure after a destructive command, e.g. a host file disappears between `rm` and `write`, a path is too long, or an ext4 operation fails.
6. **Observed behaviour / evidence:** External repro `c01_rustfs_returns_ok_after_rm_plus_failed_write` on a real ext4 image:
   - seed `build.prop` exists
   - run `rm /build.prop` then `write <missing-host> build.prop`
   - `run` returns `Ok(out)`; `out` contains `debugfs:` error text
   - `build_prop_exists=false`
   - Data is gone, but the caller sees success.
7. **Expected behaviour:** A destructive batch must fail closed. Either execute atomically (write temp, then rename) or return `Err` if any command fails, and never report success after a partially applied batch.
8. **Impact:** Silent data loss inside the output image: `build.prop`, init RC files, overlay files, and props can be deleted without the pipeline noticing. `e2fsck` may still pass; the device loses VoLTE or boot configuration while the tool prints `[OK]`.
9. **Fix recommendation:**
   - Change `Ext4Ops::run` contract to return `Err` on any failed command (or add `run_strict`).
   - In `props::write_back`/`init_rc::link`, write to a new path and rename, or dump→modify→replace without an `rm` first.
   - Make the pipeline treat overlay/init/props write failures as fatal.
   - Align `DebugfsBackend::run` (`ext4.rs:180-183`) which currently only logs `[WARN] debugfs rc=` and returns `Ok`.

---

### C02 — `split_whitespace` truncates host paths containing spaces

1. **Severity:** High
2. **Status:** Reproduced
3. **Location:** `volte-core/src/rustfs_ext4.rs:111` (and every `parts.next()` consumer at 113-225)
4. **Current code:**
```rust
let mut parts = command.split_whitespace();
let verb = parts.next().unwrap_or("");
...
"write" => {
    let host = parts.next().ok_or("write: missing host file")?;
    let name = parts.next().ok_or("write: missing image name")?;
    ...
}
```
5. **Trigger:** Work directory or host file path contains a space, e.g. `C:\Users\Jane Doe\AppData\Local\Temp\...` or a project folder named `My Build`.
6. **Observed behaviour / evidence:** External repro `c02_space_in_workdir_silently_skips_props`:
   - spaced workdir: `changed=0`
   - control workdir: `changed=75`
   - The truncated host token is read; `read_host` fails; the error is only text; `upsert` logs `[WARN]` and returns `Ok(0)`.
7. **Expected behaviour:** Host paths must be passed without whitespace splitting: quote them in the command grammar, or better, replace the string command list with typed commands carrying `PathBuf` arguments.
8. **Impact:** On a machine whose temp/user path contains a space, VoLTE props, overlay files, or init RC edits are silently skipped. The build still exits 0 and reports a flashable image.
9. **Fix recommendation:** Introduce a typed `Ext4Command` enum (`Write { host: PathBuf, target: String }`, `Dump`, `Rm`, …). If the textual debugfs format must stay, use shell-style quoting and a parser that respects quotes. Add a test with a spaced temp dir to the pipeline, not just `props::upsert`.

---

### D01 — Failing AVB backend over an existing footer reports success

1. **Severity:** Critical
2. **Status:** Reproduced
3. **Location:** `volte-core/src/avb.rs:398-416`
4. **Current code:**
```rust
let last_err = backend.add_hashtree_footer(&req).err();
let after = avb_info_fast(image)?;
if after.has_footer {
    ...
    log.log("[AVB] Rebuild hashtree completed.");
    ...
    return Ok(Some(after));      // old footer is indistinguishable from a new one
}
if let Some(e) = last_err { log.log(format!("[WARN] {}", ...)); }
...
Ok(None)
```
5. **Trigger:** `avbtool.py` is missing, Python cannot be spawned, or `add_hashtree_footer` fails, while the input image already has an AVB footer.
6. **Observed behaviour / evidence:** External repro `d01_failing_backend_keeps_old_footer_and_reports_success` with a synthetic image carrying an old footer and `FailAvb`:
   - `result_has_footer=true`
   - `log_contains_completed=true`
   - The pipeline then sets `avb_ok=true`, `boot_safe=true`, and `AVB_HASHTREE=PASS`.
7. **Expected behaviour:** If `add_hashtree_footer` returns `Err`, the rebuild failed. The code must not infer success from the presence of a pre-existing footer. Either compare the footer/vbmeta before and after (hash/size/mtime), or require the backend to return `Ok` before `Ok(Some)`.
8. **Impact:** The tool reports a valid AVB footer while the image actually contains the old hashtree over modified data. The device may fail verified boot, bootloop, or silently run an image whose AVB metadata does not match its contents. This is a security-relevant false claim.
9. **Fix recommendation:** Track `last_err` first: if it is `Some`, return `Err` (or `Ok(None)` with a fatal pipeline decision). After a successful backend call, verify the footer is new by comparing `vbmeta_offset/size` and a hash of the vbmeta region before and after. Never log "completed" unconditionally.

---

### D02 — **False positive:** `--do_not_generate_fec` + `--fec_num_roots`

1. **Severity:** Low (residual: misleading log only)
2. **Status:** **False positive** (original claim that the combination is invalid does not hold)
3. **Location:** `volte-core/src/avb.rs:274-276`; log text at `avb.rs:498` / `pipeline.rs:498`
4. **Current code:**
```rust
"--do_not_generate_fec".to_string(),
"--fec_num_roots".to_string(),
"2".to_string(),
```
5. **Trigger:** Every AVB build (the arguments are always emitted).
6. **Observed behaviour / evidence:** The authoritative `avbtool.py` accepts both options together:
   - `avbtool.py:4493-4500` defines `--do_not_generate_fec` and `--fec_num_roots`.
   - `avbtool.py:4917-4921` passes `not do_not_generate_fec` as `generate_fec` and the roots separately; no mutual-exclusion error is raised.
   - The only real defect is the log line `[STATUS] Generating hashtree and FEC for the image...` (`pipeline.rs:498`) when FEC is explicitly disabled.
7. **Expected behaviour:** Argument behaviour is correct; the log should say "hashtree (FEC disabled)" for honesty.
8. **Impact:** None on the image. Misleading log can confuse support/debugging.
9. **Fix recommendation:** Keep the arguments; fix the message. No code-path change required.

---

### D03 — `fill_from_vbmeta` never recovers salt/FEC/props/partition name

1. **Severity:** High
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/avb.rs:122-148`
4. **Current code:**
```rust
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
```
5. **Trigger:** Every image with an existing AVB footer, especially non-`vendor` partitions or images with a non-default salt/FEC configuration.
6. **Observed behaviour / evidence:** Static: `AvbInfo.salt`, `fec_roots`, and `props` are never assigned in this function. `partition_name` is set only when the literal `vendor` appears anywhere in the first 64 KiB of vbmeta; otherwise it stays at the `AvbInfo::empty` default. The algorithm is chosen by a byte search, not by parsing the `AvbHashDescriptor`/`AvbVBMetaImageHeader` fields.
7. **Expected behaviour:** Parse the vbmeta header/descriptors structurally (offsets, lengths, big-endian fields) and populate salt, FEC roots, props, and the actual partition name.
8. **Impact:** The rebuild may:
   - reuse a wrong/empty salt (changing the hashtree unexpectedly),
   - emit the wrong `--partition_name`,
   - drop original AVB properties,
   - mis-handle FEC.
   All of these can make the rebuilt footer invalid for the device even when `add_hashtree_footer` succeeds.
9. **Fix recommendation:** Implement a real vbmeta parser (or call `avbtool.py info_image --image` and parse its JSON/text). Use the parsed descriptor fields as the source of truth. Add golden tests against `avbtool.py` output for a real vendor image.

---

### D04 — Salt generation is not equivalent to `os.urandom(32)`

1. **Severity:** Low
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/avb.rs:215-236`
4. **Current code:**
```rust
let state = RandomState::new();
let now = SystemTime::now()...as_nanos();
let mut out = [0u8; 32];
for (i, slot) in out.chunks_mut(8).enumerate() {
    let mut h = state.build_hasher();
    h.write_u128(now);
    h.write_usize(i);
    slot.copy_from_slice(&h.finish().to_be_bytes());
}
```
5. **Trigger:** Every rebuild without an existing salt.
6. **Observed behaviour / evidence:** Static: four 64-bit SipHash outputs derived from one `RandomState` and a timestamp. This is not a CSPRNG and not 256 bits of entropy. `RandomState::new()` uses OS-seeded keys, but the output is still hash-derived and time-correlated. The doc comment admits the salt is not a cryptographic secret.
7. **Expected behaviour:** `os.urandom(32)` equivalent — read 32 bytes from the OS CSPRNG (`getrandom`/`rand::rngs::OsRng`).
8. **Impact:** Low for a vendor image where the salt is public anyway, but it is a parity and hygiene defect. If two builds share a seed/time, salts can collide, weakening the "fresh salt per build" property.
9. **Fix recommendation:** Add `getrandom` (or `rand` with `OsRng`) and fill 32 bytes directly. Keep the current function only if a documented deterministic mode is needed, and gate it behind a test/flag.

---

### E01 — Stale payload files are reported as `Verified`

1. **Severity:** High
2. **Status:** Reproduced
3. **Location:** `volte-core/src/verify.rs:71-130`
4. **Current code:**
```rust
let dump_apk = td.join("verify_overlay.apk");
let dump_rc = td.join("verify_volte.rc");
for name in APK_CANDIDATES {
    if size_gt(&dump_apk, 1000) { break; }   // pre-existing file can satisfy this
    dump_batch(...);
}
...
let (apk_sha256, apk) = if size_gt(&dump_apk, 500) {
    (sha256::file_hex_upper(&dump_apk)?, Verdict::Verified)
} else { ... }
```
5. **Trigger:** A previous run (or an attacker) leaves `verify_overlay.apk` / `verify_volte.rc` in the temp work dir, and the current dump fails or writes nothing.
6. **Observed behaviour / evidence:** External repro `e01_stale_payload_is_reported_verified` with a backend whose `dump` is a no-op:
   - stale APK = 600 bytes, stale RC = 20 bytes
   - `apk=Verified`, `rc=Verified`
   - `stale_hash_matches=true`
   - log contains `[OK] Overlay APK + RC verified`
7. **Expected behaviour:** Delete/truncate the host files before the dump loop; require the dump command to report success and the file to be newly written; verify the content against the source payload hash, not just its size.
8. **Impact:** The tool can claim the VoLTE overlay is inside the image when the image has no overlay at all. The user flashes a non-working build; support sees `APK_SHA256`/`RC_SHA256` tokens and trusts them.
9. **Fix recommendation:** Remove the dump targets before verification. Track the mtime/size of each successful dump. Compare the dumped hash to the hash of the payload that was written (`payload::overlay_commands` source). Treat a stale/unwritten file as `NotVerified`.

---

### E02 — Duplicate writes and stale copies can mask image-side failures

1. **Severity:** Medium
2. **Status:** Confirmed (static, with nuance)
3. **Location:** `volte-core/src/payload.rs:114-145`; `volte-core/src/verify.rs:93-119`; `volte-core/src/props.rs:220`
4. **Current code:**
```rust
// APK is written to /overlay/HDVoLTE and then to /overlay again
format!("write {} {}", forward_slashes(&payload_apk), apk_name),
...
"cd /overlay".to_string(),
format!("write {} {}", forward_slashes(&payload_apk), apk_name),

// RC is written to /etc/init and /etc/init/hw
...
"cd /etc/init/hw".to_string(),
format!("write {} {}", forward_slashes(&payload_rc), rc_name),
```
5. **Trigger:** The second write fails (C01) or the verify dump reads the other copy.
6. **Observed behaviour / evidence:** Static + repro context: host-side `std::fs::copy` errors in `payload::overlay_commands` propagate (`?`), but image-side `write` failures are only text under C01. The APK is written twice and the RC to two directories, so verification can succeed from one copy while the other is missing/corrupt. `verify.rs` dumps each candidate into the same shared host path, so a later failed dump does not clear an earlier success.
7. **Expected behaviour:** Either write once to the canonical path, or verify both copies. Host-side and image-side failures must both be fatal at the point of injection.
8. **Impact:** Partial payload: the APK may exist in `/overlay` but not `/overlay/HDVoLTE`, or the RC may exist in one init dir only. Verification may still pass. The device may not load the overlay.
9. **Fix recommendation:** Remove duplicate writes unless the Python parity explicitly requires them; if kept, assert both exist after injection. Clear dump targets before verification (E01) and hash against source payloads.

---

### F01 — UTF-8 byte slice panics on a multibyte error message

1. **Severity:** Medium
2. **Status:** Reproduced
3. **Location:** `volte-core/src/props.rs:232`
4. **Current code:**
```rust
log.log(format!("[WARN] Write {vpath}: {}", &e.message()[..e.message().len().min(120)]));
```
5. **Trigger:** A write error whose message contains a non-ASCII character at byte offset 120 (very common in localized Windows/Python error text, e.g. Vietnamese paths or messages).
6. **Observed behaviour / evidence:** External repro `f01_utf8_error_message_panics_on_byte_slice`:
   - backend returns `Err` with 119 ASCII bytes + `ế` (3 UTF-8 bytes) + padding
   - `catch_unwind` observes a panic
   - panic text: `end byte index 120 is not a char boundary; it is inside 'ế' (bytes 119..122) of ...`
   - panic location `props.rs:232:73`
7. **Expected behaviour:** Truncate by `char`s (or use `char_indices`/`is_char_boundary`) and never panic on an error path.
8. **Impact:** A localized error turns a warning into a process abort. The pipeline unwinds; the user sees a crash instead of the real write error; partial output remains.
9. **Fix recommendation:** Replace with `e.message().chars().take(120).collect::<String>()` (already the pattern used elsewhere, e.g. `trunc` in `pipeline.rs`). Add a non-ASCII regression test.

---

### F02 — Prop rewrite changes encoding, line endings, duplicates, mode, and labels

1. **Severity:** Medium
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/props.rs:165-220`, `237-255`
4. **Current code:**
```rust
let text = String::from_utf8_lossy(&raw);
...
for ln in text.lines() { ... }            // normalizes CRLF -> LF
...
std::fs::write(&host, out.as_bytes())?;
...
format!("set_inode_field {name} mode 0x81A4"),
...
format!("ea_set -f {} {name} security.selinux", ...)
```
5. **Trigger:** Any `build.prop`/`default.prop` with CRLF, non-UTF-8 bytes, duplicate keys, non-default mode/uid/gid, or existing SELinux labels.
6. **Observed behaviour / evidence:** Static: `from_utf8_lossy` replaces invalid bytes with U+FFFD; `lines()` drops `\r`; duplicate keys are not deduplicated (`seen` is only used to decide additions); the write-back forces `mode 0x81A4` (0100644), `uid 0`, `gid 0`; `ea_set` is applied to every existing prop file, not only `build.prop`.
7. **Expected behaviour:** Preserve the original encoding/line endings unless a change is required; rewrite only the targeted lines; preserve duplicates unless the reference explicitly normalizes; apply the label only where required.
8. **Impact:** Subtle image differences from the Python reference, possible breakage of properties that relied on CRLF or unusual bytes, and lost metadata. On a read-only `/default.prop`, forcing the mode may matter.
9. **Fix recommendation:** Operate on bytes, replace only the matching key lines, preserve the original terminator style, and record the original mode/uid/gid. Restrict `ea_set` to `build.prop` if that matches the reference. Add a golden test with CRLF + duplicate keys + non-UTF-8 bytes.

---

## 8. New findings (not in the prior A01–F02 set)

### G01 — Root-level init file is deleted and rewritten under `/etc/init/hw`

1. **Severity:** High
2. **Status:** Reproduced (inherited from the Python reference)
3. **Location:** `volte-core/src/init_rc.rs:30-36`, `96-127`; Python reference `hd_vendor_build.py:1031`
4. **Current code:**
```rust
fn split_vpath(vpath: &str) -> (String, String) {
    match vpath.rsplit_once('/') {
        Some((dir, name)) if !dir.is_empty() => (dir.to_owned(), name.to_owned()),
        Some((_, name)) => ("/etc/init/hw".to_owned(), name.to_owned()),
        None => ("/etc/init/hw".to_owned(), vpath.to_owned()),
    }
}
...
format!("rm {found}"),
format!("cd {dest_dir}"),
format!("write {} {dest_name}", forward_slashes(&host)),
...
log.log(format!("[OK] Linked the init script into {found}"));
```
5. **Trigger:** The selected candidate is a root-level file such as `/init.target.rc` (no directory component). `rsplit_once('/')` returns `Some(("", "init.target.rc"))`, and the empty-directory arm maps it to `/etc/init/hw`.
6. **Observed behaviour / evidence:** External repro `g01_root_init_file_is_written_to_the_wrong_directory`:
   - `found=true`
   - commands contain `rm /init.target.rc`
   - commands contain `cd /etc/init/hw` (never `cd /`)
   - log contains `Linked the init script into /init.target.rc`
   - The original root file is deleted; the new file goes to `/etc/init/hw/init.target.rc`.
   - The same `or "/etc/init/hw"` fallback exists in `hd_vendor_build.py:1031`, so this is inherited, not a Rust-only regression.
7. **Expected behaviour:** The write must target the same directory as the file that was dumped. For `/init.target.rc`, that is `/`.
8. **Impact:** The root init file is removed and not replaced at the same path. Boot behavior can change, and the log falsely claims the link succeeded. On images where the root init file is the only one, VoLTE init may never run.
9. **Fix recommendation:** Fix `split_vpath` to return `("/", name)` for a leading-slash path with no further directory, and `(".", name)` only for a truly relative name. Do not silently substitute `/etc/init/hw`. Fix the Python reference too if parity is required, or document the divergence. Add a test for `/init.target.rc`.

---

### G02 — `same_path` is case-sensitive; case-only alias makes `std::fs::copy` fail

1. **Severity:** Low
2. **Status:** Reproduced
3. **Location:** `volte-core/src/flags.rs:124-129`, `140-142`
4. **Current code:**
```rust
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
...
if !same_path(src, dst) {
    std::fs::copy(src, dst)?;
}
```
5. **Trigger:** `src` and `dst` are the same file with different case on Windows.
6. **Observed behaviour / evidence:** External repro `g02_flags_case_alias_bypasses_same_path`:
   - `patch_flags_keep_size("img.bin", "IMG.BIN")` returns `Err` (OS error 32, sharing violation) and the source is preserved.
   - If a caller used aliases, the pipeline would log `[WARN] FLAGS` and continue with flags unpached (the current pipeline passes the same `Path`, so this is latent).
7. **Expected behaviour:** Case-insensitive comparison on Windows, or file-identity comparison, so the copy is skipped and the in-place patch proceeds.
8. **Impact:** Low/latent today, but it makes the API fragile and can silently disable flag patching if paths are constructed differently.
9. **Fix recommendation:** Canonicalize and case-fold on Windows (or use file identity). Reuse the same helper as A01/A01b.

---

### N01 — `PythonAvbtool` has no timeout

1. **Severity:** Medium
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/avb.rs:316-328`
4. **Current code:**
```rust
let out = cmd
    .output()
    .map_err(|e| Error::build(format!("cannot run avbtool.py: {e}")))?;
```
5. **Trigger:** `avbtool.py` hangs (large image, blocked I/O, Python prompt, antivirus).
6. **Observed behaviour / evidence:** Static: `cmd.output()` waits indefinitely. The ext4 tools use `capture(..., RUN_TIMEOUT)` with a deadline (`ext4.rs:81-126`), so the project already has the pattern; AVB does not.
7. **Expected behaviour:** Apply a bounded timeout, kill the child, and return a clear error.
8. **Impact:** The CLI can hang forever with no log output. On a build server this blocks the worker.
9. **Fix recommendation:** Reuse the `capture`/deadline approach (or `wait_timeout`) with a configurable AVB timeout (e.g. 10–30 min for large images).

---

### N02 — `dump` allocates from untrusted inode size

1. **Severity:** Medium
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/rustfs_ext4.rs:231`
4. **Current code:**
```rust
let mut buf = vec![0u8; inode.size as usize];
let read = self.fs.read_ino(ino, 0, &mut buf)...;
```
5. **Trigger:** A malicious/corrupt image reports a huge `inode.size`.
6. **Observed behaviour / evidence:** Static: the allocation is driven by image metadata before any sanity check. On a 32-bit build `as usize` truncates; on 64-bit it can attempt a multi-GB allocation and abort/OOM.
7. **Expected behaviour:** Bound the size (e.g. the ext4 file size limit or a configurable maximum), read in chunks, and return an error for implausible sizes.
8. **Impact:** Denial of service on untrusted images; process abort instead of a clean error.
9. **Fix recommendation:** Validate against a maximum (e.g. 64 MiB for prop/RC files, or the actual image size), stream the dump, and use `try_reserve` to convert OOM into an error.

---

### N03 — `probe_space` relies on `set_len` actually allocating

1. **Severity:** Low
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/pipeline.rs:144-164`
4. **Current code:**
```rust
let res = file.set_len(needed);
drop(file);
let _ = std::fs::remove_file(&path);
res
```
5. **Trigger:** Filesystems where `set_len` creates a sparse file without allocating clusters (some Linux filesystems, network shares), or where quota is enforced on write.
6. **Observed behaviour / evidence:** Static + doc comment: the code documents NTFS behavior ("measured: a 2 GiB set_len lowered E: free space by exactly 2 GiB") but generalizes to all platforms. `set_len` guarantees logical length, not allocated blocks.
7. **Expected behaviour:** If the probe is advisory, document it; if it is a gate, use a platform API (`GetDiskFreeSpaceExW`, `statvfs`) or write real bytes.
8. **Impact:** The disk-full guard can pass when the disk is actually full, leading to a mid-build failure.
9. **Fix recommendation:** Use `GetDiskFreeSpaceExW` on Windows and `statvfs` on Unix behind `cfg`, or keep the probe but label it best-effort and always handle `ENOSPC` at every write.

---

### N04 — Payload cleanup `cd` failure leaves the cwd at `/`

1. **Severity:** Low
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/payload.rs:113-145` (cleanup commands; see also the overlay build block)
4. **Current code (shape):**
```rust
"cd /overlay/HDVoLTE".to_string(),
...
"cd /".to_string(),
"rm -f ...".to_string(),
```
5. **Trigger:** The cleanup `cd` targets a directory that does not exist (or the earlier `mkdir` failed), so the session cwd remains `/`; the following `rm` resolves relative names against the root.
6. **Observed behaviour / evidence:** Static: commands are line-oriented and the new backend keeps `cwd` across commands; a failed `cd` is only text (C01), so the subsequent `rm` can target the wrong directory. The Python/debugfs reference has the same command-list shape.
7. **Expected behaviour:** Use absolute paths for every `rm`; never rely on `cd` state for destructive commands.
8. **Impact:** Low because the names are specific, but an `rm` at `/` is a dangerous pattern and can delete a root-level file if names collide.
9. **Fix recommendation:** Replace `cd X; rm name` with `rm /X/name` (absolute). Fail the batch if the `cd` failed.

---

### N05 — Unconditional success banners/logs

1. **Severity:** Low
2. **Status:** Confirmed (static)
3. **Location:** `volte-core/src/pipeline.rs:383-384`, `526`; `volte-core/src/report.rs:72`, `82-84`
4. **Current code:**
```rust
log.log("[STATUS] Erasing old AVB footer...");
log.log("[OK] Old AVB footer removed.");
...
log.log("[OK] AVB hashtree + EDL size match - safe to boot.");
...
"[OK] EDL size matches - UnlockTool can flash it.".to_string(),
```
5. **Trigger:** Failures earlier in the same path (D01, A04, C01) — the banners are printed regardless.
6. **Observed behaviour / evidence:** Static + repro: D01 logs "Rebuild hashtree completed." after a failed backend; A04 logs `[OK] EDL size matches` with `e2fsck=FAIL`; `"Old AVB footer removed."` is printed before `add_hashtree_footer` is called, so it is true only in the sense that old bytes may be overwritten later.
7. **Expected behaviour:** Every `[OK]` must be conditional on the corresponding verified state. No success token before the operation succeeds.
8. **Impact:** Users and automation trust the log. Combined with exit code 0, this is how silent corruption reaches a device.
9. **Fix recommendation:** Introduce a small state machine/result type for each stage (`Pending/Ok/Failed`) and render tokens from it. Never emit `[OK]` from a constant.

---

### N06 — Documentation drift

1. **Severity:** Informational
2. **Status:** Confirmed (static)
3. **Location:** `README.md`, `docs/PORT_STATUS.md`, `tests/README.md`, `fixtures/README.md`
4. **Current code / text:** README references `AGENTS.md` and `reports/PYTHON_TO_RUST_MAPPING.md` (neither exists); typo `volta-core`/`volta-cli`; README claims exit code 2 on error but validation failures exit 0; `docs/PORT_STATUS.md` claims 130+10 tests while the actual suite is 135+10 and references absent `reports/` evidence; `tests/README.md` claims `volte-cli\tests\` exists.
5. **Trigger:** Reading the docs.
6. **Observed behaviour / evidence:** Directory listing shows only READMEs in `profiles/`, `fixtures/`, `tests/`; no `reports/` before this audit; no `AGENTS.md`; `cargo test` reports 145.
7. **Expected behaviour:** Documentation should match the tree and the test output.
8. **Impact:** Low, but it undermines the audit trail and makes onboarding/verification harder.
9. **Fix recommendation:** Update the docs in the same change that adds tests/reports, or generate the counts from CI.

---

## 9. Test-quality assessment

### What the suite does well

- **135 core tests** cover SHA-256 vectors, sparse chunk types/truncation, flag scanning, verify thresholds, AVB footer parsing, props upsert, init-RC linking, report formatting, and pipeline happy path.
- **10 CLI tests** cover argument parsing, help/usage, and exit codes for parse errors.
- **A real ext4 image is used** in several `rustfs_ext4` tests (via `mke2fs`), not just mocks.
- **Boundary tests exist** for verify (`> 500` vs `> 1000`) and flag windows.
- **Timeouts are tested** for the debugfs tool (`a_hung_tool_is_killed_at_the_deadline`).

### What the suite does not prove

| Gap | Evidence | Risk |
|---|---|---|
| Pipeline integration uses `DebugfsBackend`, not the production `RustFsExt4Backend` | `pipeline.rs` tests import `DebugfsBackend`; CLI selects `RustFsExt4Backend` at `main.rs:180` | C01/C02 ship untested through the real pipeline |
| AVB is a no-op in pipeline tests | `NoopAvb`/`NoFooterAvb` test seams | D01/D03/D04 never exercised end-to-end |
| Payload verification is size-only | `verify.rs` threshold logic; fixture is 1200 `A` bytes | E01 stale/hash confusion undetected |
| Overlay content is not asserted | Tests check file presence/tokens, not bytes at `/overlay/...` | E02 duplicate/partial writes undetected |
| No test for `--out == src` or case aliases | Repro had to be external | A01/A01b regressions possible |
| No test for a sparse trailing `DONT_CARE` | Repro found it | B01 regressions possible |
| No test for fsck failure propagation | Repro found it | A04 regressions possible |
| No non-ASCII error-path test | Repro found the panic | F01 regressions possible |
| No test for spaced workdir in the pipeline | Only `props` unit test with a control | C02 remains production-relevant |
| `cargo fmt --check` is not a gate | 110 diff blocks | Formatting drift accumulates |

### Test count vs behaviour

The project advertises "130+10 tests" (docs) and now has 145. The count is healthy. The *behavioural coverage of the shipped backend and AVB path* is the weak point. A test suite that passes while C01/D01/A04 exist is evidence of a coverage gap, not of correctness.

### Recommended test additions (highest value first)

1. Pipeline test with `RustFsExt4Backend` + a failing `write` after `rm` (C01).
2. Pipeline test with `--out == src` and a case alias, asserting the source bytes are unchanged (A01/A01b).
3. Pipeline test with a failing `Fsck` that asserts `Err`/non-zero exit and no `[OK]` summary (A04).
4. AVB test with a real/failing backend over an existing footer, asserting `Err`/`None` (D01).
5. Sparse test with trailing `DONT_CARE`, asserting file length == `written` (B01).
6. Verify test with stale host files and a failing dump (E01).
7. Props test with a 120-byte error message containing a multibyte char (F01).
8. Init-RC test for `/init.target.rc` → `cd /` (G01).
9. Differential/golden test: run the Python reference and the Rust pipeline on the same synthetic image, compare the extracted file tree and AVB info.

---

## 10. Security and reliability analysis

### Data-loss surfaces

1. **Source identity confusion (A01/A01b/G02).** Output aliasing can delete/truncate the source. Fix by canonical/file-ID comparison before any destructive operation.
2. **Destructive batch with swallowed errors (C01).** `rm` then failed `write` returns `Ok`. Any caller that trusts `Ok` loses data.
3. **Non-atomic rewrite (A02, C01, props/init_rc).** Files are replaced in place; a failure mid-way leaves a missing or partial file. Prefer temp + rename.
4. **Stale verification (E01).** Verification can attest to files that were never written in this run.

### False-success surfaces

- `e2fsck=FAIL` + exit 0 (A04).
- Old AVB footer + "Rebuild hashtree completed." (D01).
- `[OK] EDL size matches` unconditionally (report.rs:72).
- `[OK] Linked the init script into /init.target.rc` after writing elsewhere (G01).
- `[OK] Overlay APK + RC verified` from stale host files (E01).

These are more dangerous than crashes because they defeat the user's ability to detect a bad build.

### Untrusted-input handling

- **Sparse parser (B02):** no version/block-size/size cross-check; `csize * blk_sz`; `n as i64`.
- **Ext4 dump (N02):** allocation from inode metadata.
- **AVB parser (D03):** byte search rather than structural parsing; wrong salt/props can be carried into a rebuild.
- **Command construction (C02):** whitespace splitting of host paths; no shell is invoked, so this is not classic command injection, but it is a parsing bug with silent-skip consequences.

### Process execution

- **Debugfs/e2fsck:** bounded by `capture` timeouts (`ext4.rs:81-126`). Good.
- **Python/avbtool:** unbounded `cmd.output()` (N01). Add a timeout.

### Cryptographic notes

- **Salt (D04):** not a CSPRNG. AVB salts are public, so impact is low, but parity with `os.urandom(32)` is not met.
- **AVB algorithm:** `--algorithm NONE` is emitted (`avb.rs:266-267`). That matches the Python reference for an unsigned vendor hashtree, but it means the output is not cryptographically signed. Do not describe the result as "signed"; the tool builds an AVB hashtree, not a verified-boot signature chain. This is an expectation/documentation issue, not a code bug.

### Reliability

- **Disk space (A03/N03):** probe underestimates peak and is filesystem-dependent.
- **Concurrency:** not tested. Two builds sharing the same `Logs/` or temp dir can collide (`hue_chuan_*.log` is timestamped, but `commands_last.txt`, `vendor_VolLTE_report.txt`, and fixed host names in the temp work dir are not).
- **Partial output:** `pipeline::run` documents that a failure leaves the partial output in place. Combined with A01/A02, this can leave a plausible-looking but corrupt image at the output path.

---

## 11. Reproduction harness results

**Location:** `C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro`
**Dependency:** path dependency on `E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core`; its own `[patch]` for `rust-fs-core`
**Command:** `cargo test --test repro -- --test-threads=1`
**Result:** **11 passed, 0 failed** (26.18 s)

| Test | Finding | Key assertion / observed value |
|---|---|---|
| `a01_out_equals_src_truncates_the_source` | A01 | `before=4096`, `after=0`, error contains `Copy vendor failed` |
| `a01b_case_alias_out_deletes_the_source` | A01b | `source_still_exists=false`, error present |
| `a04_fsck_failure_still_returns_ok` | A04 | `pipeline_ok=true`, `e2fsck_fail_logged=true`, `edl_ok_logged=true` |
| `b01_trailing_dont_care_loses_the_tail` | B01 | `written=16384`, `actual=0` |
| `c01_rustfs_returns_ok_after_rm_plus_failed_write` | C01 | `build_prop_exists=false`, `run` returned `Ok` |
| `c02_space_in_workdir_silently_skips_props` | C02 | `spaced=0`, `control=75` |
| `d01_failing_backend_keeps_old_footer_and_reports_success` | D01 | `result_has_footer=true`, `log_contains_completed=true` |
| `e01_stale_payload_is_reported_verified` | E01 | `apk=Verified`, `rc=Verified`, stale hash matches |
| `f01_utf8_error_message_panics_on_byte_slice` | F01 | `panicked=true` at `props.rs:232:73` |
| `g01_root_init_file_is_written_to_the_wrong_directory` | G01 | `rm /init.target.rc` + `cd /etc/init/hw`, log claims success |
| `g02_flags_case_alias_bypasses_same_path` | G02 | `Err` OS error 32, source preserved (latent) |

### Why the harness is trustworthy

- It depends on the audited crate by path, so it executes the current source, not a copy.
- It creates its own temp images with `mke2fs.exe` from the bundled tool directory.
- It does not write inside the repository (the tests' temp dirs are under the system temp).
- It uses `catch_unwind` for F01 and real ext4 images for C01/C02.
- The one non-obvious caveat: `a01_out_equals_src_truncates_the_source` uses a plain file (not ext4) and `NoopOps`/`NoopFsck`/`NoopAvb`, because the defect happens before any image-aware step. That is exactly the production ordering.

### Reproductions not attempted

| Finding | Reason |
|---|---|
| A02 restore failure | Requires forcing a mid-build copy failure; destructive to the output and hard to make deterministic without fault injection. Static evidence is conclusive. |
| A03 disk full | Requires filling a real volume; risky and environment-specific. |
| D03 real-image salt/props | Requires a genuine vendor image with AVB metadata; not available in the repo. |
| B02 malicious sparse image | Would exercise huge allocations; non-destructive fuzzing was out of scope. |
| N01 avbtool hang | Would require a hanging child process; static evidence is conclusive. |
| True concurrent builds | Shared-state collision is visible in code, but a deterministic repro needs timing control. |

---

## 12. Unverified areas and limitations

1. **No prior audit report.** `reports/` did not exist and the workspace is not a git repo, so the prior findings' original wording/line numbers could not be diffed. Each claim was re-derived from current code.
2. **No real vendor image.** The audit used synthetic sparse/AVB data and `mke2fs` images. Real OPPO/MTK/QC images may exercise different ext4 features (inline data, metadata checksums, 64-bit, encryption) and different vbmeta descriptors.
3. **No device/flash test.** The audit cannot confirm whether a given AVB footer actually boots. It only confirms whether the code reports success correctly.
4. **No dependency vulnerability scan.** `cargo-audit`/`cargo-deny` are absent; the git-pinned ext4 crates were not reviewed for advisories.
5. **No Miri/UB check.** Miri is unavailable on stable MSVC. The unsafe surface is small (mostly in dependencies), but this is not proven.
6. **Disk-full, concurrency, huge-image, and malicious-image resource exhaustion** were not executed. Severity for those is based on static analysis.
7. **Python parity for edge cases** was checked only for D02 and G01. Other divergences (e.g. exact prop rewrite semantics) are inferred from code, not from a differential run.
8. **Formatting drift** is reported but not attributed to a specific commit (no git history).

---

## 13. False positives and rejected claims

### D02 — rejected

**Claim (reconstructed):** `--do_not_generate_fec` combined with `--fec_num_roots` is invalid / makes `avbtool.py` fail.

**Verdict:** **False positive.**

**Evidence:**
- `E:\Build_app\Volte\VoLTE\vendor_patcher\tool\avbtool.py:4493-4500` defines both options on the same `add_hashtree_footer` parser.
- `avbtool.py:4917-4921` calls the FEC generator with `generate_fec = not args.do_not_generate_fec` and passes `args.fec_num_roots` independently.
- No mutual-exclusion check exists.

**Residual issue (Low):** the pipeline log says `Generating hashtree and FEC` even though FEC is disabled. That is a message defect, not an argument defect.

### Claims checked and not promoted

| Claim area | Result |
|---|---|
| `DebugfsBackend` vs `RustFsExt4Backend` parity | Both swallow per-command failures (`ext4.rs:180-183`, `rustfs_ext4.rs:265-270`); the finding is C01, not a separate backend-divergence bug |
| `cargo clippy` warnings | None with `-D warnings`; no finding |
| Duplicate dependencies | None (`cargo tree -d`) |
| `payload` host `std::fs::copy` errors | They propagate with `?`; not a bug. The image-side write is the bug (E02/C01) |
| `sparse` CRC chunk handling | CRC is read but not validated; that matches the Python reference's tolerance and is not treated as a defect here |

---

## 14. Recommendations and readiness

### P0 — must fix before any release (blocking)

1. **Input/output identity (A01/A01b/G02).** Canonicalize + case-fold/file-ID and refuse or rename before any destructive open. Regression tests must assert source bytes are unchanged.
2. **Fail-closed command execution (C01).** `Ext4Ops::run` must return `Err` if any command fails, or provide a strict variant used by props/init/overlay. Make the pipeline treat overlay/init/props failures as fatal. Align `DebugfsBackend`.
3. **Truthful success (A04/D01/N05).** Propagate fsck failure; require the AVB backend to succeed before `Ok(Some)`; make every `[OK]` token conditional. Non-zero exit on any failed stage.
4. **Sparse correctness (B01).** Extend the output file to `written` and verify the final length.

### P1 — high priority (next change)

5. **AVB metadata parsing (D03).** Parse vbmeta structurally or call `avbtool.py info_image`; preserve salt/FEC/props/partition name.
6. **Verification integrity (E01/E02).** Clear dump targets, require a successful dump, hash against the source payload, and verify every copy that matters.
7. **Host-path parsing (C02).** Typed commands or quoted parsing; test with a spaced temp dir through the pipeline.
8. **Init-RC destination (G01).** Write back to the original directory; fix `split_vpath`; test `/init.target.rc`.
9. **Backup/restore integrity (A02).** Hash + size verification; fatal restore failure.
10. **UTF-8 error-path panic (F01).** Truncate by chars; add a non-ASCII test.

### P2 — medium/low priority

11. **Sparse validation (B02).** Version/block-size/chunk-size/final-size checks; checked arithmetic.
12. **Disk probe (A03/N03).** Use the uncompressed size + headroom or a real free-space API.
13. **AVB timeout (N01).** Reuse the `capture` deadline.
14. **Dump allocation (N02).** Bound and stream.
15. **Salt (D04).** Use the OS CSPRNG.
16. **Payload cleanup (N04).** Absolute paths; no `cd` state for `rm`.
17. **Prop rewrite fidelity (F02).** Preserve encoding/line endings/metadata; restrict `ea_set`.
18. **Formatting gate.** Run `cargo fmt --all` and add `cargo fmt --check` to CI.
19. **Docs (N06).** Correct names, counts, exit codes, and missing-file references.

### Readiness gate

The project becomes **READY WITH WARNINGS** when:

- P0 items 1–4 are fixed and covered by tests that run the **default** backend end-to-end.
- `cargo fmt --check`, `cargo check`, `cargo test`, and `cargo clippy -D warnings` are all green.
- A real vendor image is built and the resulting AVB footer is validated with `avbtool.py verify_image` / a device boot test.
- A dependency audit (`cargo audit`/`cargo deny`) is run and clean.

Until then: **NOT READY**.

---

## Appendix A — Evidence index

| Evidence | Path |
|---|---|
| Workspace manifests | `Cargo.toml`, `volte-core/Cargo.toml`, `volte-cli/Cargo.toml` |
| Pipeline | `volte-core/src/pipeline.rs` |
| AVB | `volte-core/src/avb.rs` |
| Default ext4 backend | `volte-core/src/rustfs_ext4.rs` |
| Debugfs backend | `volte-core/src/ext4.rs` |
| Sparse | `volte-core/src/sparse.rs` |
| Verify | `volte-core/src/verify.rs` |
| Props | `volte-core/src/props.rs` |
| Init RC | `volte-core/src/init_rc.rs` |
| Flags | `volte-core/src/flags.rs` |
| Report | `volte-core/src/report.rs` |
| Payload | `volte-core/src/payload.rs` |
| CLI | `volte-cli/src/main.rs` |
| Repro harness | `C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro\tests\repro.rs` |
| Python reference | `E:\Build_app\Volte\VoLTE\vendor_patcher\hd_vendor_build.py` |
| Authoritative avbtool | `E:\Build_app\Volte\VoLTE\vendor_patcher\tool\avbtool.py` |

## Appendix B — Command transcript summary

```
cargo check --workspace --all-targets          -> exit 0
cargo clippy --workspace --all-targets -- -D warnings -> exit 0, no warnings
cargo test --workspace                         -> exit 0, 145 passed (10 + 135)
cargo fmt --all -- --check                     -> exit 1, 110 diff blocks
cargo tree -d                                  -> exit 0, no duplicates
cargo audit / cargo deny / cargo nextest       -> not installed
cargo miri --version                           -> exit 1, component unavailable (stable MSVC)
cargo test --test repro -- --test-threads=1    -> 11 passed, 0 failed
```

---

## Vietnamese summary (tóm tắt tiếng Việt)

**Kết luận: NOT READY.**

Đã kiểm tra độc lập, chỉ đọc, không sửa bất kỳ file nguồn nào. Báo cáo đầy đủ ở `reports/AI2_INDEPENDENT_AUDIT.md`.

**Số lượng phát hiện:** 25 tổng cộng — 11 tái hiện được (Reproduced), 13 xác nhận bằng bằng chứng tĩnh (Confirmed static), 0 cần kiểm chứng thêm (Needs verification), 1 báo động giả (False positive).

**Theo mức độ:** 4 Critical, 7 High, 7 Medium, 6 Low, 1 Informational.

**Critical/High cần xử lý ngay:**
- **A01/A01b (Critical):** `--out` trùng nguồn (hoặc khác hoa/thường trên Windows) làm mất/ghi đè ảnh gốc.
- **C01 (Critical):** backend ext4 mặc định trả `Ok` sau khi `rm` + `write` lỗi → mất file trong ảnh nhưng báo thành công.
- **D01 (Critical):** AVB backend lỗi nhưng footer cũ còn đó vẫn trả `Ok(Some)` và log “Rebuild hashtree completed.”.
- **A02, A04, B01, C02, D03, E01, G01 (High):** backup/restore không kiểm tra; e2fsck fail vẫn exit 0; sparse kết thúc bằng DONT_CARE bị thiếu đuôi; đường dẫn có dấu cách làm mất props; parser vbmeta không lấy salt/FEC/props; file stale được báo Verified; `/init.target.rc` bị ghi sai thư mục.

**Build/test/lint hiện tại:**
- `cargo check`: PASS
- `cargo clippy -D warnings`: PASS, 0 cảnh báo
- `cargo test`: PASS, 145 test (10 CLI + 135 core)
- `cargo fmt --check`: FAIL, 110 diff block (chỉ style)
- `cargo tree -d`: không có dependency trùng
- `cargo audit`/`cargo deny`/`cargo nextest`: chưa cài; Miri không khả dụng trên stable MSVC

**Điểm chưa kiểm chứng:** ảnh vendor thật, flash thiết bị thật, disk-full, chạy song song, ảnh độc hại gây OOM, lỗ hổng dependency. Không có báo cáo audit trước trong repo nên không đối chiếu được nguyên văn.

**Báo cáo:** `E:\Build_app\Volte\VoLTE-Vendor-Builder\reports\AI2_INDEPENDENT_AUDIT.md`
