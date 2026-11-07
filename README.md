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
├── volta-core/           pure logic, no console output
├── volta-cli/            thin CLI: parse args -> call volte-core
├── docs/PORT_STATUS.md   port status of every mapping ID
├── profiles/             per-chip documentation (Qualcomm / MediaTek)
├── fixtures/             generated test fixtures (no vendor data)
├── tests/                cross-crate contract artefacts
├── tool/                 bundled runtime: e2fsck.exe, mke2fs.exe,
│                         avbtool.py, Python 3.10, MSYS/Cygwin DLLs, etc\mke2fs.conf
└── payload/              OPPO_VoLTE_Overlay.apk, hd_oppo_{mtk,qc}_volte.rc,
                          hd_volte_features.xml, overlay.bin, config.bin
```

The tree is standalone for testing and running: `tool\` and `payload\` are copies of
`..\VoLTE\vendor_patcher\tool\` and the payload files (`__pycache__` directories were
the only entries left out), except that `tool\debugfs.exe` was intentionally removed after
the Đợt 2.5 parity check. Neither the test suite nor the binary reads anything from the
original repository.

## Build and test

```bat
cargo check
cargo build --release
cargo clippy -- -D warnings
cargo test
```

Target: `x86_64-pc-windows-msvc`, stable toolchain, no nightly, no `rust-toolchain.toml`.

## Run

`tool\` and `payload\` are resolved from the directory of the running executable
(`env::exe_root()`), so copy the binary next to them first:

```bat
cargo build --release
copy target\release\volte-cli.exe .
volte-cli.exe <vendor.img> [--mtk | --qc] [--out <path>]
```

Exit code `0` on success, `2` on error, matching the Python tool. Logs, the command
list and the report go to `.\Logs\`, which is created on demand.

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
