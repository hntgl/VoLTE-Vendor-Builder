# Port status

Status of the 30 ported mapping IDs (`..\reports\PYTHON_TO_RUST_MAPPING.md` §1 and §10). ID 31
(Tk GUI) is excluded by master prompt §8. Paths here are relative to the project root.

Status values: `pending` (not written yet), `ported` (code + unit test), `verified` (group test
green and Step C re-measurement done), `diverges` (intentional, documented difference).

Last measured: 2026-11-07 (`cargo test --workspace`: 151 passed in `volte-core`, 10 in `volte-cli`,
0 failed, 0 ignored) **with `tool\debugfs.exe` absent**. The Phase 4 Step C evidence in
`..\reports\` records the earlier count:
`_p4_c_test1/2.txt` (10 + 130 passed, 0 ignored), `_p4_c_check1/2.txt`, `_p4_c_build1/2.txt`,
`_p4_c_clippy1/2.txt`, `_p4_c_parity1/2.txt`, `_p4_c_stubs1/2.txt`,
`_p4_c_integrity1/2.txt`, `_p4_c_sha1/2.txt`, `_p4_c_sha_diff.txt`.

| ID | Python function | Rust symbol | Status |
| --- | --- | --- | --- |
| 01 | `tool_dir` | `env::tool_dir` | verified |
| 02 | `payload_dirs` / `payload_file` / `payload_required` / `first_payload` | `payload::{payload_dirs, payload_file, payload_required, first_payload}` | verified |
| 03 | `sha256_file` | `sha256::file_hex_upper` | verified |
| 04 | `BuildLog` / `_safe_print` / `_decode` | `logging::{Logger, FileSink, decode, fmt_time}` | diverges |
| 05 | `run_debugfs` | `ext4_tools::Ext4Ops::run` (`RustFsExt4Backend`) | diverges (in-process engine, strict errors) |
| 06 | `debugfs_exists` | `ext4_tools::Ext4Ops::path_exists` (`RustFsExt4Backend`) | verified |
| 07 | `run_e2fsck` | `ext4_tools::Fsck::check` (`E2fsck`) | verified |
| 08 | `is_sparse` | `sparse::is_sparse` | verified |
| 09 | `unsparse` | `sparse::unsparse` | verified |
| 10 | `_parse_avbf` | `avb::parse_footer` | verified |
| 11 | `avb_info_fast` | `avb::avb_info_fast` | verified |
| 12 | `fit_avb_part` | `avb::fit_avb_part` | verified |
| 13 | `force_slot_size` | `slot::force_slot_size` | verified |
| 14 | `_prop_boundary` | `flags::prop_boundary` | verified |
| 15 | `patch_flags_keep_size` | `flags::patch_flags_keep_size` | verified |
| 16 | `read_vendor_flag_bit` | `ext4_tools::read_vendor_flag_bit`, `flags::flag_bit_from_dumps` | verified |
| 17 | `volte_flag_note` | `report::flag_note` | verified |
| 18 | `detect_vendor_chip` (binary markers) | `chip_detect::detect_vendor_chip` | verified |
| 19 | `detect_vendor_chip` (path keywords) | `chip_detect::detect_vendor_chip` | verified |
| 20 | `refine_chip_from_props` | `chip_detect::refine_chip_from_props` | verified |
| 21 | `overlay_cmds` | `payload::overlay_commands` | verified |
| 22 | `link_init_target` | `init_rc::link` | verified |
| 23 | `inject_volte_show_props` | `props::upsert` | verified |
| 24 | `dump_device_props` | `props::identify` | verified |
| 25 | `verify_payload` | `verify::verify_payload` | diverges |
| 26 | `avb_module` | `avb::PythonAvbtool` | verified |
| 27 | `rebuild_hashtree` | `avb::rebuild_hashtree` | verified |
| 28 | `hue_avb_lines` | `report::avb_lines` | verified |
| 29 | `build` | `pipeline::run` | diverges |
| 30 | `main` | `volte_cli::{parse, build, run}` | diverges |

Test counts behind the table: 151 unit/integration tests in `volte-core` and 10 in `volte-cli`
(`cargo test`, 0 ignored); every module listed above carries at least four `#[test]` functions,
the smallest being `env` (4) and `slot` (5).

## Intentional divergences

Each item was requested or accepted before the code was written; none of them weakens a §4
invariant of `..\VoLTE\vendor_patcher\AGENTS.md` (size lock, untouched source, EXT4 only, flag-patch byte
neutrality, SELinux labels, honest AVB result, V2 payload state).

**ID 04 — logging.** Python `_safe_print` (`:329`) re-encodes every line to the console's
current code page with `errors="replace"`, and `_decode` (`:264`) reverses cp1258/cp1252 for
tool output. The port writes UTF-8 bytes + LF and flushes per line, because V1.0 targets a
UTF-8 console and the log text is English; `decode` keeps the cp1258/cp1252 fallback chain.
The log file is still appended UTF-8 with LF, like `BuildLog` (`:346`).

**ID 25 — `verify_payload`.** Python treats a failed dump as success and then prints
`[OK] ĐÃ XÁC NHẬN BẬT NÚT VOLTE THÀNH CÔNG` (`:1246`) with the `VERIFIED_APK_OK` /
`VERIFIED_RC_OK` fallbacks (`:1240-1243`) and `VERIFIED_OK` in the exception path
(`:1646-1649`). Those tokens cannot be told apart from a real verification, so the port never
emits them: a failed dump yields `UNVERIFIED` plus a `[WARN]` line (verify.rs:8) and is never
counted as PASS evidence. See AGENTS.md §8.

**ID 29 — `build`.** Three differences. (a) The log file name is stamped in UTC
(`pipeline::log_stamp`, `SystemTime` + civil-from-days) whereas Python uses local time
(`time.strftime` at `:1410`); the format `hue_chuan_%Y%m%d_%H%M%S` is identical, the offset is
not, so a log filename taken from the two tools can differ by the UTC offset (UTC+7 on this
machine). std has no local-time formatter without FFI, and keeping std-only was the approved
constraint. (b) Redundant `mkdir` commands are skipped through `Ext4Ops::path_exists`
(the user-requested divergence of 2026-11-04, AGENTS.md §4.6), so the `mkdir` orphan-inode
collision cannot occur. (c) Console and report text is English, per master prompt item 14.

**ID 30 — `main`.** `--gui`, or no image argument, exits with code 2 and
`[ERROR] GUI is not part of V1.0 - pass a vendor.img path (see --help).` instead of starting
Tkinter. The budget flags (`--qc`/`--mtk`/`--out`) and the QC-wins tie-break match Python.

## Design notes and known limitations

- **One EXT4 engine.** V1.0 uses only the in-process `rust-fs-ext4` engine
  (`RustFsExt4Backend`). `DebugfsBackend`, `tool\debugfs.exe` and the parity harness were
  removed after the Đợt 2.5 parity check (content/metadata/e2fsck/AVB/SELinux xattr identical
  on OPPO A3s and F11). `ext4_tools` keeps only the `Ext4Ops`/`Fsck` traits and the external
  `e2fsck` runner.
- **FEC is intentionally disabled on rebuild.** `avb::HashtreeRequest::cli_args` always passes
  `--do_not_generate_fec` (frozen from the Python reference), so a rebuilt footer records
  `FEC num roots: 0` even if the stock footer had FEC roots. This is design, not a bug.
- **Known limitation — superblock counters.** `rust-fs-ext4` does not refresh `s_mtime` or the
  `Lifetime writes` counter after writing. `e2fsck` stays clean (rc=0) and file content,
  metadata and SELinux labels match the debugfs oracle, but the pipeline does not claim to
  update every superblock field.
