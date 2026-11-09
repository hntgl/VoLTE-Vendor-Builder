# VoLTE Vendor Builder (Rust port, V1.0)

Rust port of `..\VoLTE\vendor_patcher\hd_vendor_build.py` (V2.9.2 / V298). The Python file stays the
reference implementation and is never modified; this workspace reproduces its behaviour for
the closed set of 30 mapping entries in `..\reports\PYTHON_TO_RUST_MAPPING.md` (ID 31, the Tk GUI,
is out of scope for V1.0). Paths in this file are relative to the project root; `..\` is the
parent folder shared with the Python reference and the audit reports.

## Layout

```
VoLTE-Vendor-Builder/
├── Cargo.toml            workspace (volte-core + volte-cli)
├── volte-core/           pure logic, no console output
├── volte-cli/            thin CLI: parse args -> call volte-core
├── profiles/             per-chip documentation (Qualcomm / MediaTek)
├── fixtures/             generated test fixtures (no vendor data)
├── tests/                reserved for gate evidence (currently empty)
├── tool/                 bundled runtime: e2fsck.exe, mke2fs.exe,
│                         avbtool.py, Python 3.10, MSYS/Cygwin DLLs, etc\mke2fs.conf
└── payload/              OPPO_VoLTE_Overlay.apk, oppo_{mtk,qc}_volte.rc,
                          volte_features.xml, overlay.bin, config.bin
```

The tree is standalone for testing and running: `tool\` and `payload\` started as copies of
`..\VoLTE\vendor_patcher\tool\` and the payload files (`__pycache__` directories were
the only entries left out), except that `tool\debugfs.exe` was intentionally removed after
the Đợt 2.5 parity check. Neither the test suite nor the binary reads anything from the
original repository.

The two `.rc` payload files and the features XML were renamed on 2026-11-08, dropping the
`hd_` prefix the original Python tool used (`hd_oppo_mtk_volte.rc` → `oppo_mtk_volte.rc`,
`hd_oppo_qc_volte.rc` → `oppo_qc_volte.rc`, `hd_volte_features.xml` → `volte_features.xml`).
This is a deliberate, user-requested rename, not a port divergence: the new names are what
`volte-core` now writes into the vendor partition and imports from `init.target.rc`, and
`volte-core/src/config.rs` and `volte-core/src/profile.rs` record it. `config.bin` and
`overlay.bin` were intentionally left unrenamed — they are host-side SELinux-label source
files, never written into the vendor image under those names. The historical reports
under `..\reports\` describe the pre-rename state and were left as-is; they are an audit
trail, not current documentation.

## Build and test

```bat
cargo fmt --all -- --check
cargo check --workspace --all-targets --offline
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo tree -d
```

The workspace builds offline against the committed `Cargo.lock`. Test counts are not kept in
this file: run `cargo test --workspace --offline` and read the `test result:` line for each
target.

Target: `x86_64-pc-windows-msvc`, stable toolchain, no nightly, no `rust-toolchain.toml`.

## Run

`tool\` and `payload\` are resolved from the directory of the running executable
(`env::exe_root()`), so copy the binary next to them first:

```bat
cargo build --release
copy target\release\volte-cli.exe .
volte-cli.exe vendor <vendor.img> [--mtk | --qc] [--out <path>]
```

Exit code `0` on success and for `--help`; `2` for a usage error, the GUI divergence and any
pipeline error, matching the Python tool. A failed mandatory step fails the build: when
`e2fsck -fn` reports an error the CLI exits `2` without printing any flashable line. An AVB
backend failure is recoverable: the saved tail is restored and the image is still exported
with `AVB_HASHTREE=SKIP`; only a tail restore that cannot be verified is fatal (exit `2`,
no `[OUTPUT]`). See "AVB footer handling" below.
Logs, the command list and the report go to `.\Logs\`, which is created on demand.

## AVB footer handling

The vendor build classifies the source image right after copying it, before any ext4 work:

| Source state | Behavior |
|---|---|
| No `AVBf` footer at the last 64 bytes and none found by the backward scan | Skip AVB: `AVB_HASHTREE=SKIP`, image exported at the locked slot size. |
| Footer parsed from the last 64 bytes | Rebuild the hashtree. `AVB_HASHTREE=PASS` when the new footer fits the slot; if the backend fails or the hashtree grows past the slot, the tail is restored and the image is exported with `SKIP`. |
| Footer found only by the backward scan, not at the exact tail | Build error before debugfs, `e2fsck` or avbtool runs; no output is reported and the copied file stays on disk untouched. The source was padded or modified after signing - re-dump the original `vendor.img`. |

Notes:

- Only the tail from the footer's `original_image_size` to EOF is saved and restored; there is
  no whole-image `vendor_pre_avb.img` snapshot.
- avbtool only recognises a footer in the last 64 bytes, so a displaced footer (the last row
  above) must be rejected before it reaches avbtool.
- Every error after avbtool starts (backend error, unparsable footer, backend returning `Ok`
  without writing a footer, hashtree larger than the slot, padding failure) restores the saved
  tail. If the restore itself fails, the build exits `2` without reporting an output.
- The source image is never modified: the pipeline works on a copy, so a failed run can be
  retried from the original.
- Tests compare the whole-file SHA-256 before and after the AVB step, so a skipped rebuild
  leaves the copy byte-identical to its pre-AVB state.

## Modem (BUILD MODEM / BUILD FIX MODEM)

Port of `..\VoLTE\modem_patcher\tool\qc_modem_volte.py` plus the CLI half of
`build_all_in_one.py`, added as two more `volte-cli` subcommands (`volte-core/src/modem.rs`).
They patch a Qualcomm NON-HLOS modem partition image (FAT16) so the Viettel VoLTE MBN config
is first in `oem_sw.txt` / `mbn_sw.txt`, or inject that MBN from a donor image when the
destination modem does not have one yet. Image size never changes.

```bat
:: BUILD MODEM: modem already has a Viettel MBN, move it to the front
volte-cli.exe modem <modem.bin|modem.img> [--out <path>]

:: BUILD FIX MODEM: modem is missing the Viettel MBN entirely
volte-cli.exe fix-modem <modem.bin|modem.img> --donor <donor_modem.img> [--out <path>]
volte-cli.exe fix-modem <modem.bin|modem.img> --mbn <viettel_mcfg_sw.mbn> [--out <path>]
```

Default output is `<name>_volte<ext>` / `<name>_volte_fix<ext>` next to the source, like the
Python reference; `--out` overrides it. An `--out` that resolves to the same file as the
source modem is a build error (same A01-class guard as the vendor builder's
`path_id::same_file`), not a silent overwrite.

Intentionally not ported — GUI-only conveniences, not needed for a CLI, not covered by any
test here:
- the Tk GUI itself (`build_all_in_one.py`'s two tabs);
- zip-file input (a `.zip` containing `modem.bin`/`modem.img`);
- the `%APPDATA%\HoangDucTool` MBN cache and the sibling-folder auto-discovery of a donor
  modem (`find_mbn`) — pass `--donor` or `--mbn` explicitly instead;
- `_viettel_line_from_texts`, `_nl_join`, `_rel_to_parts`: dead code in the Python reference
  (defined, never called), not ported.

`volte-cli` always requires one of three subcommands first — `vendor`, `modem` or
`fix-modem` — so a vendor image can never be misread as the modem subcommands (or vice
versa) no matter what it is named. Earlier builds of this tool took the vendor image as a
bare positional argument with no subcommand (`volte-cli.exe <vendor.img> ...`); that form is
no longer accepted and now exits 2 with a usage error, specifically to remove the
name-collision risk this paragraph used to describe.

This module has not been run against a real OEM modem image or a real device — the tests in
`volte-core/src/modem.rs` build synthetic FAT16 images through the same public API the engine
uses; the vendor builder's own suite carries the same caveat.

## External tools

`e2fsck.exe` and `avbtool.py` are invoked as subprocesses from `tool\`; `avbtool.py`
runs under `tool\python\python.exe` when that bundled interpreter is present, otherwise
under `python.exe` from `PATH`. All ext4 reads and writes go through the in-process
`rust-fs-ext4` engine (`RustFsExt4Backend`); `debugfs.exe` is not used, not bundled and not
read by the build or the tests. Nothing is statically linked against `libext2fs`.

Design notes:

- The hashtree rebuild passes `--do_not_generate_fec`, so a rebuilt footer records
  `FEC num roots: 0` even though the stock image may have had FEC roots. This is the frozen
  parameter set of the Python reference, not a regression.
- Known limitation: `rust-fs-ext4` does not update the superblock `s_mtime` and
  `Lifetime writes` counters after writing. `e2fsck` reports the filesystem clean; the
  pipeline does not claim to refresh all superblock metadata.
- A sparse image is validated (version, header sizes, block size, chunk count, declared
  size) before conversion; a malformed or oversized header is an error, not a truncated
  output. Free space is checked against the raw size twice plus AVB headroom, and the
  temporary copy is charged to the same probe when it shares the destination volume.
- `build.prop` and the other property files are rewritten in place: untouched bytes, line
  endings, duplicate keys and inode metadata survive, and the vendor-file SELinux label is
  applied only to `/build.prop`.
- `avbtool.py` runs under a 30-minute deadline and its output is captured; the AVB salt is
  read from the OS CSPRNG.
