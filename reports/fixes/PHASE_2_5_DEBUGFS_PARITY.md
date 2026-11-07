# PHASE 2.5 — XÓA DEBUGFS (PHẦN XÓA, SAU PARITY CHECK)

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline của phần xóa:** `3fb8d0d` (Đợt 2.5: báo cáo parity hai backend)
**Commit của phần xóa:** `c21f98b` — `Đợt 2.5: xóa DebugfsBackend và debugfs.exe khỏi sản phẩm`
**Báo cáo đo parity:** `reports/fixes/PHASE_2_5_PARITY.md`
**Trạng thái:** HOÀN THÀNH — dừng chờ người dùng xác nhận trước Đợt 3

---

## 1. Mục tiêu

Sau khi người dùng xác nhận parity đạt yêu cầu, xóa `DebugfsBackend` và `tool\debugfs.exe` khỏi
sản phẩm để V1.0 chỉ còn một engine EXT4 (in-process `rust-fs-ext4`), đồng thời chứng minh bằng
lệnh chạy thật rằng build/test vẫn PASS khi **không có** `tool\debugfs.exe`.

## 2. Phương án đã chọn và lý do

Chọn **xóa hẳn** `DebugfsBackend` (không giữ sau feature flag `legacy-debugfs`):

- Sản phẩm chỉ có một engine; giữ một backend lenient sau flag sẽ tạo đường thứ hai có thể được
  bật nhầm trong tương lai, trái với kết luận của Đợt 2 (C01: không được báo thành công giả).
- Flag không mặc định vẫn phải biên dịch trong `--all-features` (clippy/CI), kéo theo test cần
  `debugfs.exe` — mâu thuẫn với yêu cầu "test mặc định không phụ thuộc debugfs.exe".
- Ít code hơn, ít bảo trì hơn; hành vi lịch sử vẫn truy được qua git (`3fb8d0d:volte-core/src/ext4.rs`).
- Harness đo parity (`volte-core/examples/parity.rs`, commit `71bacac`) bị xóa khỏi cây vì nó
  tham chiếu backend đã xóa; vẫn khôi phục được từ lịch sử git khi cần đo lại.

## 3. Trạng thái trước khi xóa

- HEAD `3fb8d0d`; 165 test PASS (155 core + 10 CLI); clippy sạch; fmt 112 block drift baseline.
- Parity A3s/F11 đã đo xong: nội dung + metadata trùng khớp, e2fsck rc=0, AVB PASS/SKIP giống nhau,
  SELinux xattr giống nhau (xem `PHASE_2_5_PARITY.md` §6–§13).

## 4. Thay đổi code

**`volte-core/src/ext4.rs` → `volte-core/src/ext4_tools.rs`** (git rename, similarity 55%):

- Xóa: `DebugfsBackend`, `CMD_FILE`, `reject_non_ext4`, `RUN_TIMEOUT`/`STAT_TIMEOUT`, toàn bộ test
  chỉ chạy được với `debugfs.exe`.
- Giữ: `Ext4Ops` (kèm `run_best_effort`), `Fsck`, `E2fsck`, `capture`, `read_vendor_flag_bit`,
  `tail_chars` và test của `E2fsck`/timeout.
- `lib.rs`: `pub mod ext4;` → `pub mod ext4_tools;`.
- Cập nhật import ở `init_rc.rs`, `props.rs`, `verify.rs`, `rustfs_ext4.rs`, `pipeline.rs`,
  `volte-cli/src/main.rs`.
- `pipeline.rs` test: `missing_source_is_a_build_error` và
  `a_full_build_reaches_the_final_tokens_on_a_synthetic_image` chuyển từ `DebugfsBackend` sang
  `RustFsExt4Backend` (backend mặc định thật).
- `rustfs_ext4.rs`: tiền tố lỗi từng lệnh `debugfs:` → `ext4:`; doc không còn nhắc `DebugfsBackend`.
- `error.rs`: doc/test của `ErrorKind::BadMagic` đổi sang thông điệp `rust-fs-ext4: bad magic number`.

## 5. `tool_dir()` và assert trong test

- `env::tool_dir` chọn `tool\` → `tools\` → root theo dấu hiệu **`avbtool.py` hoặc `e2fsck.exe`**
  (trước đây là `debugfs.exe` hoặc `avbtool.py`). Test `env` được viết lại theo marker mới.
- `pipeline.rs` test helper: assert `mke2fs.exe` (fixture cần) thay vì `debugfs.exe`.
- `ext4_tools.rs` test helper: assert `mke2fs.exe`.
- Không còn `assert!` nào yêu cầu `debugfs.exe` trong test mặc định (xem §11).

## 6. Log engine thật trong `pipeline.rs`

- Dòng giả `debugfs 1.44.5 (15-Dec-2018)` → `EXT4 engine: rust-fs-ext4 (in-process)`.
- Echo lệnh `debugfs: <cmd>` → `ext4: <cmd>`; cảnh báo `debugfs clean` → `ext4 clean`; lỗi
  `debugfs overlay injection failed` → `ext4 overlay injection failed`.
- Log không còn tuyên bố một tool không tồn tại; tên engine khớp backend CLI đang dùng
  (`RustFsExt4Backend`, `volte-cli/src/main.rs:180`).

## 7. Xóa binary và harness

- `tool/debugfs.exe` (181.779 B) đã xóa khỏi repo (`git rm`); blob vẫn nằm trong lịch sử git.
- Các DLL e2fsprogs dùng chung (`cyge2p`, `cygext2fs`, `cygcom_err`, …) **được giữ** vì
  `e2fsck.exe` và `mke2fs.exe` vẫn cần chúng.
- `volte-core/examples/parity.rs` xóa (`git rm`), khôi phục bằng
  `git show 71bacac:volte-core/examples/parity.rs`.

## 8. Tài liệu đã cập nhật

- `README.md`: layout `tool\` không còn `debugfs.exe`; mục External tools nói rõ một engine,
  `debugfs.exe` không được dùng/bundle; thêm Design notes (FEC, `s_mtime`); sửa câu "byte-identical
  copy" cho đúng sau khi xóa.
- `docs/PORT_STATUS.md`: ID 05/06/07/16 trỏ `ext4_tools`; số test 151 + 10; thêm mục
  **Design notes and known limitations** (một engine, FEC, superblock counters).
- `CLAUDE.md`: thêm §5 "Project notes" (một engine, FEC, known limitation).
- `tests/README.md`: mô tả artefact là "EXT4 command files (debugfs grammar)".
- `tool/README_RUNTIME.txt`: bỏ `debugfs` khỏi danh sách runtime cần khôi phục.

## 9. Ghi nhận FEC 2 → 0 là thiết kế có chủ đích

Đã bổ sung vào `PHASE_2_5_PARITY.md` §11 và `docs/PORT_STATUS.md`: `avb::HashtreeRequest::cli_args`
luôn truyền `--do_not_generate_fec` (bộ tham số đóng băng từ Python), nên footer rebuild ghi
`FEC num roots: 0` dù ảnh stock có FEC roots. Đây là hành vi thiết kế, không phải lỗi mới.

## 10. Ghi nhận known limitation `s_mtime` / `Lifetime writes`

Đã bổ sung vào `PHASE_2_5_PARITY.md` §14, `docs/PORT_STATUS.md` và `README.md`:
`rust-fs-ext4` không cập nhật `s_mtime` và bộ đếm `Lifetime writes` sau khi ghi (F11 978 MB vs
977 MB). e2fsck vẫn rc=0; pipeline không được mô tả là làm mới đầy đủ metadata superblock.

## 11. Grep `debugfs.exe` sau khi sửa

Lệnh: `grep -rn "debugfs\.exe" . --exclude-dir=target --exclude-dir=.git --exclude-dir=python`
(loại `reports/`). Còn đúng 4 dòng, **toàn bộ là tài liệu mô tả việc đã xóa**, không có mã:

| File:dòng | Nội dung | Vì sao giữ |
|---|---|---|
| `docs/PORT_STATUS.md:10` | "...**with `tool\debugfs.exe` absent**" | Ghi bằng chứng môi trường đo |
| `docs/PORT_STATUS.md:88` | "`DebugfsBackend`, `tool\debugfs.exe` ... were removed" | Ghi chú kiến trúc 1-engine |
| `README.md:28` | "`tool\debugfs.exe` was intentionally removed" | Giải thích `tool\` không còn byte-identical |
| `README.md:62` | "`debugfs.exe` is not used, not bundled" | Mô tả runtime hiện tại |

`DebugfsBackend` không còn xuất hiện trong bất kỳ file `.rs` nào (grep rc=1). Các từ `debugfs`
còn lại trong code là **tên ngữ pháp lệnh** mà engine Rust phải hiểu (ví dụ
`rustfs_ext4.rs:49,77`, `payload.rs:5,66`) — giữ vì chúng mô tả định dạng lệnh, không phải tool.

## 12. Bài test then chốt — không có `tool\debugfs.exe`

Bằng chứng lưu tại `E:\Build_app\Volte\_work\p25_parity\nodebugfs\`:

```text
preflight.txt : tool/debugfs.exe present? NO
                tool/e2fsck.exe present? YES
                git HEAD: 3fb8d0def5abc7b01bf9f5f017de0a7729dacc0e

build.txt     : cargo build --workspace
                Finished `dev` profile [unoptimized + debuginfo] target(s)
                build_rc=0

test.txt      : cargo test --workspace
                test result: ok. 10 passed; 0 failed; 0 ignored
                test result: ok. 151 passed; 0 failed; 0 ignored
                test result: ok. 0 passed; 0 failed; 0 ignored
                test_rc=0

clippy.txt    : cargo clippy --workspace --all-targets -- -D warnings
                Finished `dev` profile
                clippy_rc=0

grep_debugfs_exe.txt : 4 dòng tài liệu, 0 dòng code (xem §11)
```

Thời điểm đo là **sau khi `git rm tool/debugfs.exe`** và trước commit; file không tồn tại trên đĩa
trong suốt cả build và test.

## 13. Test thay đổi

- 165 → 161 test (155→151 core, 10 CLI giữ nguyên).
- Bỏ 4 test chỉ kiểm tra `DebugfsBackend`: `command_file_is_lf_only_and_commands_reach_the_image`,
  `command_file_joins_an_empty_batch_to_a_single_newline`, `path_exists_matches_the_stat_probe`,
  `bad_magic_is_detected_by_text_not_exit_code`. Hành vi tương ứng của engine thật đã có test trong
  `rustfs_ext4.rs` (strict run, bad magic, path_exists/dump).
- Giữ/viết lại 5 test của `ext4_tools`: `e2fsck_falls_back_to_the_bare_name`,
  `fsck_accepts_a_clean_image_and_rejects_errors_with_the_tail` (dùng superblock magic bị hỏng để
  tạo rc≠0), `vendor_flag_bit_reads_the_dumped_build_prop` (qua `RustFsExt4Backend`), `tail_chars`,
  `a_hung_tool_is_killed_at_the_deadline`.
- `pipeline` full-build test chạy qua `RustFsExt4Backend`, vẫn PASS với đầy đủ token
  (`e2fsck=PASS`, `AVB_HASHTREE=SKIP`, `VOLTE_FLAGS=0`, `FLASH=NO`).

## 14. Rủi ro còn lại

1. Không còn engine dự phòng: nếu gặp ảnh EXT4 mà `rust-fs-ext4` chưa hỗ trợ (feature lạ, nén…),
   pipeline fail-closed thay vì fallback debugfs — đúng chủ trương, nhưng cần theo dõi khi gặp ảnh mới.
2. `e2fsck.exe` vẫn là tool ngoài; test cần `mke2fs.exe`/`e2fsck.exe` trong `tool\`.
3. FEC tắt khi rebuild và `s_mtime`/`Lifetime writes` không cập nhật là hành vi/limitation đã ghi
   tài liệu, không phải lỗi mới.
4. Harness parity không còn trong cây; muốn đo lại phải khôi phục từ `71bacac` và dựng lại backend
   oracle từ `3fb8d0d:volte-core/src/ext4.rs`.

## 15. Git

- Phần xóa: `c21f98b` (18 file, +121/−480; xóa `tool/debugfs.exe`, xóa harness, rename
  `ext4.rs` → `ext4_tools.rs`).
- Báo cáo này được commit riêng sau `c21f98b`; hash xem `git log --oneline -2`.
- Không `--amend`, không force-push, không viết lại lịch sử.

## 16. Kết luận

Đợt 2.5 hoàn tất: parity đã được xác nhận, `DebugfsBackend` + `tool\debugfs.exe` đã bị xóa, và
`cargo build --workspace` / `cargo test --workspace` / `cargo clippy -D warnings` đều PASS trong
môi trường **không có** `debugfs.exe`. **Dừng chờ xác nhận trước khi sang Đợt 3.**