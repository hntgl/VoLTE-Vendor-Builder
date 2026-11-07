# Đợt 5 — Hardening và hoàn thiện

## 1. Mục tiêu

Xử lý tám finding còn lại của audit độc lập (`reports/AI2_INDEPENDENT_AUDIT.md`): B02,
D04, N01, N02, A03/N03, N04, F02 và N06; sau đó chạy đầy đủ cổng định dạng, build, test,
clippy, kiểm tra dependency, đối chiếu repro harness và đồng bộ tài liệu.

Phạm vi: chỉ `volte-core`, tài liệu trong repo và harness độc lập ngoài repo. Không đụng
`CLAUDE.md`, không sửa file Python reference, không flash hay kết nối thiết bị thật.

## 2. Baseline, phạm vi, môi trường

- Nhánh: `ai2/fix-audit-findings`.
- Baseline trước Đợt 5: `4287f1c` (báo cáo Đợt 4).
- Toolchain: Rust stable, target `x86_64-pc-windows-msvc`, build offline.
- Mốc test trước Đợt 5 (Đợt 4): 176 test `volte-core` + 11 test `volte-cli`.
- Mốc test sau Đợt 5: 198 test `volte-core` + 11 test `volte-cli`, 0 failed, 0 ignored
  (`cargo test --workspace --offline`, log `E:\Build_app\Volte\_work\phase5_test_after_fmt.txt`).
- Tất cả thay đổi nằm trong working tree trên `4287f1c`; không thay đổi lịch sử Git.

## 3. B02 — Sparse header và kích thước chưa được kiểm tra đủ

**Audit:** Medium, `reports/AI2_INDEPENDENT_AUDIT.md:393`. Python `unsparse`
(`hd_vendor_build.py:452-523`) không kiểm tra header; một ảnh hỏng có thể gây cấp phát hoặc
ghi quá kích thước khai báo.

**Thay đổi (`volte-core/src/sparse.rs`):**

- `parse_header` (`:65`) kiểm tra: major = 1; `file_hdr_sz >= 28`; `chunk_hdr_sz >= 12`;
  `blk_sz` dương và chia hết cho 4; `total_blks > 0`; kích thước khai báo tính bằng
  `checked_mul` và không vượt 64 GiB (`MAX_IMAGE_SIZE`, `:24`, `:100`).
- Số chunk phải thỏa `total_chunks <= total_blks * 2 + 1` (`:110`), để header không thể khai
  báo hàng tỷ chunk trên một ảnh nhỏ.
- `check_chunk_size` (`:139`) buộc chunk CRC có `csize = 0`, `tsize = 4`; chunk dữ liệu có
  `csize > 0` và `tsize = csize * blk_sz` (checked).
- `write_image` (`:196`) kiểm tra từng chunk bằng `checked_add`; chunk vượt kích thước khai
  báo bị từ chối sớm, thiếu byte cũng là lỗi. Ảnh đích bị xóa nếu lỗi sau khi đã tạo.
- `output_size` (`:126`) dùng cùng bộ kiểm tra nên bước tính dung lượng không tin header hỏng.

**Test mới:** `an_unsupported_major_version_is_rejected`, `a_short_file_header_is_rejected`,
`a_short_chunk_header_is_rejected`, `a_zero_block_size_is_rejected`,
`a_block_size_not_multiple_of_four_is_rejected`, `an_image_declaring_no_blocks_is_rejected`,
`an_absurd_declared_size_is_rejected`, `a_chunk_count_that_cannot_fit_the_file_is_rejected`,
`a_chunk_payload_size_that_disagrees_with_its_blocks_is_rejected`,
`a_crc_chunk_that_declares_blocks_is_rejected`,
`chunks_that_cover_more_than_the_header_declares_are_rejected_early`,
`output_size_reads_the_declared_raw_size`.

**Bằng chứng:** 24 test `sparse::tests` PASS trong log
`E:\Build_app\Volte\_work\phase5_test_after_fmt.txt`; harness hardening
`b02_malformed_sparse_headers_are_rejected` PASS (5 header hỏng + chunk size lệch).

**Khác Python:** Python ghi log rồi trả success với header lệch; bản Rust fail-closed. Đây là
khác biệt có chủ đích đã ghi ở `docs/PORT_STATUS.md:65`.

## 4. D04 — Salt không tương đương `os.urandom(32)`

**Audit:** Low, `reports/AI2_INDEPENDENT_AUDIT.md:569`. Salt cũ sinh từ hash thời gian, không
phải CSPRNG.

**Thay đổi (`volte-core/src/avb.rs`):**

- Thêm `getrandom = "0.3.4"` vào `volte-core/Cargo.toml`; `random_salt_hex` (`avb.rs:448`)
  gọi `getrandom::fill` và trả 32 byte dạng hex, đúng độ dài `os.urandom(32)`.
- `rebuild_hashtree` dùng `random_salt_hex()?` (`:576`); lỗi CSPRNG là lỗi build, không có
  salt dự phòng đoán được.

**Test:** `avb::tests::twenty_five_salts_are_all_distinct` PASS; harness
`d04_salts_are_random_hex` kiểm tra 64 ký tự hex và hai lần gọi khác nhau.

## 5. N01 — `PythonAvbtool` không có timeout

**Audit:** Medium, `reports/AI2_INDEPENDENT_AUDIT.md:761`.

**Thay đổi (`volte-core/src/avb.rs`, `volte-core/src/ext4_tools.rs`):**

- `AVB_TIMEOUT = 30 phút` (`avb.rs:27`); `PythonAvbtool::add_hashtree_footer` (`:528`) chạy
  qua `ext4_tools::capture` (`ext4_tools.rs:85`) — pipe stdout/stderr, kiểm tra deadline,
  kill process khi quá hạn và trả `tool timed out after N s`.
- Exit code khác 0 trả `Error::build("avbtool.py failed (code): stdout+stderr")`
  (`avb.rs:533-542`) thay vì chỉ ghi log rồi coi như thành công.
- `Captured`/`capture` được đưa về `pub(crate)` để tái sử dụng cho `e2fsck` và `avbtool.py`.

**Test:** `ext4_tools::tests::a_hung_tool_is_killed_at_the_deadline` PASS;
harness `n01_a_failing_avbtool_is_an_error_with_its_output` PASS với script Python giả exit 3
và in `boom from fake avbtool`, xác nhận thông điệp lỗi chứa cả code và stderr.

**Giới hạn kiểm chứng:** deadline 30 phút không chạy hết trong test; cơ chế deadline đã được
test bằng timeout ngắn trong `ext4_tools`.

## 6. N02 — `dump` cấp phát theo inode size không đáng tin

**Audit:** Medium, `reports/AI2_INDEPENDENT_AUDIT.md:780`.

**Thay đổi (`volte-core/src/rustfs_ext4.rs`):**

- `dump_buffer` (`:72`) từ chối khi `declared > image_len` với thông điệp
  `file claims N bytes, more than the M-byte image`; dùng `usize::try_from` và
  `try_reserve_exact` trước khi đọc, nên inode hỏng không thể ép cấp phát vô hạn.
- Nhánh `dump` lấy `metadata(image).len()` một lần khi mở session (`Session { image_len }`)
  và truyền vào `dump_buffer`.

**Test:** `rustfs_ext4::tests::dump_buffer_bounds_the_claim_by_the_image_size` PASS.

**Giới hạn kiểm chứng:** `dump_buffer` là hàm private; harness độc lập không gọi trực tiếp
được, nên finding này được chứng minh bằng unit test trong repo và đọc code, không có test
hardening tương ứng.

## 7. A03/N03 — Probe dung lượng đánh giá thấp đỉnh sử dụng

**Audit:** Medium/Low, `reports/AI2_INDEPENDENT_AUDIT.md:311` và `:798`.

**Thay đổi (`volte-core/src/pipeline.rs`, `volte-core/src/path_id.rs`):**

- `AVB_HEADROOM = 64 MiB` (`pipeline.rs:87`).
- `space_need` (`:94`): đích cần `raw * 2 + 64 MiB` (bản copy để restore + output AVB);
  thư mục tạm cần `raw` (hoặc `raw * 2` khi nguồn là sparse). Mọi phép tính dùng
  `saturating_*`.
- Khi thư mục đích và thư mục tạm cùng volume (`path_id::same_volume`, `path_id.rs:44`), hai
  nhu cầu được cộng lại và probe một lần (`pipeline.rs:353-357`); nếu khác volume, probe đích
  là fatal và probe tạm chỉ cảnh báo.
- `probe_space` (`:169`) giữ nguyên việc tạo file tạm và `set_len`; tài liệu của hàm nói rõ
  trên NTFS đây là reservation thật, trên filesystem chỉ advisory thì mọi lần ghi vẫn fatal
  khi `ENOSPC`.

**Test:** `pipeline::tests::space_need_counts_the_copy_the_restore_side_and_the_temp_backup`,
`space_need_saturates_instead_of_overflowing`, `path_id::tests::a_directory_and_its_child_share_a_volume`
PASS; harness `a03_a_directory_and_its_child_share_a_volume` PASS.

**Giới hạn:** `volte-core` có `#![forbid(unsafe_code)]`, nên không thể gọi
`GetDiskFreeSpaceExW`; probe không phải reservation nguyên tử trên mọi filesystem. Đây là
lựa chọn đã ghi trong tài liệu hàm, không phải PASS giả.

## 8. N04 — Cleanup payload phụ thuộc trạng thái `cd`

**Audit:** Low, `reports/AI2_INDEPENDENT_AUDIT.md:818`.

**Thay đổi (`volte-core/src/payload.rs:92`):**

- Danh sách clean còn 15 lệnh, tất cả dùng đường dẫn tuyệt đối:
  `rm /overlay/HDVoLTE/...`, `rm /overlay/...`, `rmdir /overlay/HDVoLTE`,
  `rm /etc/init/...`, `rm /etc/init/hw/...`, `mkdir /overlay`, `mkdir /etc`,
  `mkdir /etc/init`, `mkdir /etc/init/hw`.
- Không còn `cd`; một `cd` thất bại không thể làm lệnh `rm` kế tiếp trỏ sai thư mục.
- Số lệnh write giữ nguyên (38 cho QC, 44 cho MTK khi có features + QC RC).

**Test:** `payload::tests::cleanup_uses_absolute_paths_and_never_changes_directory` PASS;
các test đếm lệnh cũ được cập nhật từ 20 xuống 15 cho clean; harness
`n04_cleanup_uses_absolute_paths` PASS và kiểm tra mọi đích bắt đầu bằng `/`.

**Khác Python:** đường dẫn tuyệt đối là khác biệt có chủ đích, ghi ở
`docs/PORT_STATUS.md:77`.

## 9. F02 — Rewrite props làm đổi encoding, line ending, metadata, label

**Audit:** Medium, `reports/AI2_INDEPENDENT_AUDIT.md:671`.

**Thay đổi (`volte-core/src/props.rs`):**

- `split_lines` (`:153`) tách theo byte và giữ terminator gốc (`\n` hoặc `\r\n`); dòng cuối
  không có newline được giữ nguyên.
- `upsert` (`:199`) đọc/ghi `Vec<u8>`: chỉ thay dòng có key trùng, giữ byte không hợp lệ
  UTF-8, comment, dòng trống, thứ tự và mọi bản trùng; key trùng được sửa tất cả.
- `write_back` (`:178`) chỉ phát một lệnh `write <host> <vpath>`; không `rm`, không `cd`,
  không `set_inode_field`. Vì `rust-fs-ext4` thay nội dung tại chỗ, mode/uid/gid và
  `security.selinux` hiện có của inode được giữ.
- `ea_set` chỉ chạy cho `/build.prop` (`:303`), với label `vendor_file`; các prop file khác
  giữ label gốc của chúng.

**Test:** `props::tests::crlf_duplicates_and_invalid_bytes_survive_the_rewrite` (changed = 75,
CRLF và byte `0xFF/0xFE` còn nguyên), `duplicate_keys_are_all_corrected_and_kept`
(changed = 76, cả hai bản trùng được sửa), `the_rewrite_preserves_inode_metadata_instead_of_resetting_it`,
`only_build_prop_gets_the_vendor_file_label` PASS; test backend thật
`rustfs_ext4::tests::props_upsert_works_from_a_work_directory_with_spaces` PASS.

Harness `f02_rewrite_preserves_bytes_and_labels_only_build_prop` PASS: changed = 151 cho hai
file, lệnh chỉ có `write` + một `ea_set` cho `/build.prop`, nội dung giữ `\r\n` và `0xFF 0xFE`,
hai bản trùng `persist.radio.VT_ENABLE` đều thành `=1`.

**Khác Python:** Python xóa file rồi ghi lại, ép mode `0x81A4`, uid/gid 0 và gắn label cho cả
sáu file. Bản Rust cố ý không làm vậy (audit F02 yêu cầu); khác biệt ghi ở
`docs/PORT_STATUS.md:82`. Hệ quả: các file prop khác giữ label gốc thay vì bị đổi thành
`vendor_file`.

## 10. N06 — Tài liệu lệch mã nguồn

**Audit:** Informational, `reports/AI2_INDEPENDENT_AUDIT.md:860`.

Đã sửa:

- `README.md`: sửa `volta-core`/`volta-cli` thành `volte-core`/`volte-cli`; cổng build/test
  dùng `--workspace --all-targets --offline`; nói rõ cách lấy số test từ lệnh thay vì ghi số
  cố định; exit code `0` gồm `--help`, `2` cho usage/GUI/pipeline error; bổ sung ghi chú
  sparse validation, probe dung lượng, props rewrite tại chỗ và CSPRNG/timeout AVB;
  `tests/` được mô tả là thư mục dành cho evidence, hiện trống.
- `docs/PORT_STATUS.md`: bỏ số test cố định, hướng dẫn đọc `test result:`; cập nhật ID 09
  (validate header), ID 21 và ID 23 sang `diverges`, bổ sung đoạn ID 21, ID 23, CSPRNG/timeout
  ở ID 27 và probe dung lượng ở ID 29.
- `tests/README.md`: nói đúng thư mục hiện trống, không còn `volte-cli\tests\`; test CLI nằm
  trong unit test của `volte-cli\src\main.rs`.
- `fixtures/README.md`: fixture AVB sinh bằng `fixtures/avb/generate_vbmeta_golden.py`, các
  fixture khác sinh tại temp lúc chạy test, `cargo test` không ghi vào cây này.

**Bằng chứng:** `cargo test --workspace --offline` ngày 2026-11-07 trả 198 + 11 test; các tài
liệu trên không còn nhắc `DebugfsBackend` như backend khả dụng, không còn số 130/135/145/163.

## 11. Bảng thay đổi code

| File | Nội dung | Finding |
|---|---|---|
| `volte-core/src/sparse.rs` | validate header/chunk, checked arithmetic, giới hạn 64 GiB | B02 |
| `volte-core/Cargo.toml`, `Cargo.lock` | thêm `getrandom 0.3.4` | D04 |
| `volte-core/src/avb.rs` | CSPRNG salt, deadline 30 phút, lỗi kèm stdout/stderr | D04, N01 |
| `volte-core/src/ext4_tools.rs` | `Captured`/`capture` dùng chung | N01 |
| `volte-core/src/rustfs_ext4.rs` | `dump_buffer` giới hạn theo kích thước ảnh | N02 |
| `volte-core/src/path_id.rs` | `same_volume` | A03/N03 |
| `volte-core/src/pipeline.rs` | `space_need`, probe kết hợp cùng volume | A03/N03 |
| `volte-core/src/payload.rs` | cleanup bằng đường dẫn tuyệt đối | N04 |
| `volte-core/src/props.rs` | rewrite theo byte, giữ metadata, label chỉ `build.prop` | F02 |
| `README.md`, `docs/PORT_STATUS.md`, `tests/README.md`, `fixtures/README.md` | đồng bộ tài liệu | N06 |

## 12. Regression tests

- Tổng: **198** test `volte-core` + **11** test `volte-cli`, 0 failed, 0 ignored
  (`E:\Build_app\Volte\_work\phase5_test_after_fmt.txt`).
- Không xóa, không nới lỏng test cũ. Các test đếm lệnh clean được cập nhật từ 20 xuống 15 để
  khớp danh sách tuyệt đối mới; test cũ yêu cầu `cd` đã được thay bằng test cấm `cd`.
- Test mới theo finding: 12 test sparse (B02), salt distinct (D04), dump bound (N02),
  same-volume (A03/N03), space_need (N03), cleanup tuyệt đối (N04), bốn test props (F02).
- Test backend thật: `props_upsert_works_from_a_work_directory_with_spaces` chạy trên ảnh
  EXT4 tạo bằng `mke2fs`, xác nhận `write` tuyệt đối và thay nội dung tại chỗ hoạt động.

## 13. Đối chiếu repro harness

### 13.1 Harness gốc

`%TEMP%\volte_audit_repro`, lệnh:

```text
cargo test --offline --test repro -- --test-threads=1
```

Kết quả `E:\Build_app\Volte\_work\phase5_harness_after_fix.txt`:

```text
test result: FAILED. 1 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.29s
```

Trạng thái này **giống Đợt 4** (1 passed / 10 failed). Mỗi failure là harness assert hành vi
lỗi cũ; không test nào chuyển trạng thái trong Đợt 5, nên không có hồi quy. Ví dụ hiện tại:

```text
A01 before=4096 after=4096 result=Err("Refusing to build: the output path is the same file ...")
A01b result=Err("... same file ...") source_still_exists=true
A04 pipeline_ok=false e2fsck_fail_logged=true edl_ok_logged=false
C02 spaced_workdir_changed=75 control_changed=0
E01 apk=NotVerified rc=NotVerified stale_hash_matches=false
F01 panicked=false
```

### 13.2 Harness hardening (mới, ngoài repo)

Thêm `tests/hardening.rs` để đối chiếu trực tiếp các finding Đợt 5 qua public API:

```text
cargo test --offline --test hardening -- --test-threads=1
```

Kết quả `E:\Build_app\Volte\_work\phase5_harness_hardening.txt`:

```text
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s
```

Sáu test: `b02_malformed_sparse_headers_are_rejected`, `d04_salts_are_random_hex`,
`n01_a_failing_avbtool_is_an_error_with_its_output`,
`a03_a_directory_and_its_child_share_a_volume`, `n04_cleanup_uses_absolute_paths`,
`f02_rewrite_preserves_bytes_and_labels_only_build_prop`.

N02 (hàm private) và deadline 30 phút của N01 không có test harness tương ứng; đã ghi rõ ở §5
và §6.

## 14. Cổng build / test / lint / dependency

Log tại `E:\Build_app\Volte\_work\`:

| Lệnh | Kết quả |
|---|---|
| `cargo fmt --all -- --check` trước khi format | **FAIL** (rc=1, 2156 dòng diff) |
| `cargo fmt --all` + `cargo fmt --all -- --check` | **PASS** (rc=0) |
| `cargo check --workspace --all-targets --offline` | **PASS** (rc=0) |
| `cargo test --workspace --offline` | **PASS**: 11 + 198 passed, 0 failed |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | **PASS** (rc=0) sau khi sửa `manual_is_multiple_of` tại `sparse.rs:89` |
| `cargo tree -d` | **PASS**: `warning: nothing to print.` |
| `cargo audit` | **NOT RUN**: `error: no such command: audit` |
| `cargo deny` | **NOT RUN**: `error: no such command: deny` |

Clippy lần đầu trả rc=101:

```text
error: manual implementation of `.is_multiple_of()`
  --> volte-core\src\sparse.rs:89:23
```

Đã đổi thành `!blk_sz.is_multiple_of(4)`; các cổng sau đó đều xanh. Không cài thêm
`cargo-audit`/`cargo-deny` vì môi trường build offline và không có yêu cầu cài đặt; hai công
cụ này **không được báo PASS**.

## 15. Khác biệt có chủ đích và rủi ro còn lại

1. **Sparse fail-closed (B02).** Python chấp nhận header lệch; bản Rust từ chối. Ảnh sparse
   do OEM tạo đúng chuẩn vẫn chuyển đổi bình thường; ảnh hỏng không còn tạo output ngắn.
2. **Probe dung lượng (A03/N03).** Không có FFI nên không thể reservation nguyên tử trên mọi
   filesystem; trên NTFS là reservation thật, nơi khác chỉ advisory. Mọi lỗi ghi `ENOSPC`
   vẫn là lỗi build.
3. **Props rewrite (F02).** Khác Python: không xóa file, không ép mode/uid/gid, chỉ gắn label
   cho `/build.prop`. Các file prop khác giữ label gốc; đây là thay đổi có chủ đích và đã ghi
   trong `docs/PORT_STATUS.md`.
4. **Deadline AVB (N01).** Test dùng deadline ngắn của `capture`; nhánh 30 phút chưa được chạy
   hết trong CI. Nhánh exit code khác 0 đã được test bằng script giả.
5. **N02.** Giới hạn dựa trên độ dài thật của file ảnh; nếu chính file ảnh bị khai báo quá lớn
   ở tầng filesystem thì không có thêm biện pháp nào trong `volte-core` (không dùng unsafe).
6. **N06.** Số test không còn là hằng số trong tài liệu; nếu chạy trên toolchain khác, đọc
   trực tiếp `test result:` thay vì tin báo cáo này.

## 16. Điều kiện hoàn thành, Git và kết luận

- [x] B02 — header/chunk được validate trước khi xử lý; 12 test mới; harness hardening PASS.
- [x] D04 — salt từ `getrandom::fill`; test 25 salt khác nhau; harness PASS.
- [x] N01 — `avbtool.py` có deadline 30 phút, thu stdout/stderr, exit code khác 0 là lỗi;
      harness PASS cho nhánh lỗi.
- [x] N02 — `dump_buffer` giới hạn theo kích thước ảnh; unit test PASS.
- [x] A03/N03 — `space_need` tính đỉnh và probe kết hợp cùng volume; test + harness PASS.
- [x] N04 — cleanup tuyệt đối, không `cd`; test + harness PASS.
- [x] F02 — rewrite theo byte, giữ CRLF/byte lạ/trùng khóa/metadata; label chỉ `build.prop`;
      test + harness PASS.
- [x] N06 — README, PORT_STATUS, tests/README, fixtures/README khớp mã và số test lấy từ lệnh.
- [x] `cargo fmt --check`, `cargo check --all-targets`, `cargo test --workspace`,
      `cargo clippy -D warnings`, `cargo tree -d` đều chạy và PASS.
- [x] `cargo audit`/`cargo deny` ghi rõ **NOT RUN** vì chưa cài, không báo PASS giả.
- [x] Không `--amend`, không force-push, không viết lại lịch sử; không đụng `CLAUDE.md`.

**Git:**

- Baseline: `4287f1c` (báo cáo Đợt 4).
- Commit code + tài liệu Đợt 5: `56fe46d` —
  `Đợt 5: hardening sparse, AVB, props và tài liệu` (14 file, +875/−213).
- Commit định dạng riêng: `dcd3c6c` —
  `Đợt 5: chuẩn hóa định dạng toàn workspace bằng cargo fmt` (19 file, +975/−282).
- Báo cáo này được commit riêng sau `dcd3c6c`; hash xem `git log --oneline -3`.

**Kết luận:** Đợt 5 hoàn tất. Tám finding B02/D04/N01/N02/A03/N03/N04/F02/N06 đã được sửa
hoặc đồng bộ tài liệu, có test trong repo, có đối chiếu harness độc lập (6/6 hardening PASS;
harness gốc giữ nguyên trạng thái 1 passed / 10 failed như Đợt 4, không hồi quy), và các cổng
định dạng/build/test/clippy/dependency đều xanh.