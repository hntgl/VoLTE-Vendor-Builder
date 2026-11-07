# PHASE 4 — TÍNH TOÀN VẸN DỮ LIỆU VÀ TÍNH TƯƠNG THÍCH

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline của đợt:** `0ac8b47` (commit báo cáo Đợt 3)
**Commit của đợt:** `3aa7d4d` — `Đợt 4: sửa toàn vẹn dữ liệu AVB, payload, backup và đường dẫn init`
**Trạng thái:** HOÀN THÀNH — Đợt 4 không nằm trong checkpoint bắt buộc, được phép nối sang Đợt 5

---

## 1. Mục tiêu

Đảm bảo metadata AVB, payload, backup và file cấu hình trong vendor image không bị mất,
không bị xác minh nhầm, và mọi thất bại trên đường ghi/khôi phục đều là lỗi fatal thay vì
được báo thành công.

| Finding | Mức | Trạng thái audit | Kết quả đợt này |
|---|---|---|---|
| D03 | High | Confirmed | Đã sửa — parser vbmeta theo cấu trúc descriptor; golden test 704 byte |
| E01 | High | Reproduced | Đã sửa — xóa host cũ trước khi dump, hash theo payload nguồn |
| E02 | Medium | Confirmed (static) | Đã sửa phần che giấu lỗi — xác minh cả hai bản copy; giữ ghi trùng theo parity |
| A02 | High | Confirmed | Đã sửa — backup xác minh size + SHA-256; restore atomic, lỗi fatal |
| G01 | High | Reproduced | Đã sửa — `/init.target.rc` được ghi lại đúng `/`, không fallback |
| F01 | Medium | Reproduced | Đã sửa — cắt thông điệp lỗi theo ký tự, không panic UTF-8 |

Nguyên tắc xuyên suốt giữ nguyên từ Đợt 3: **truthful success** — không `[OK]`, không
`Verified`, không token hash nếu thao tác tương ứng chưa thành công và chưa được kiểm chứng.

## 2. Baseline, phạm vi, môi trường

- Baseline code: `0ac8b47` (HEAD sau Đợt 3), xác nhận bằng `git log --oneline -3` trước khi commit.
- Phạm vi code: `volte-core/src/{avb,verify,pipeline,init_rc,props}.rs` và `fixtures/avb/`.
- Không sửa `payload.rs`/`profile.rs`: hai đích ghi APK/RC và thứ tự candidate vẫn giữ nguyên
  theo Python reference; thay đổi nằm ở bước xác minh sau khi ghi.
- Không đụng `CLAUDE.md` (giữ nguyên từ `d444e43`).
- Không thêm dependency; chỉ dùng `std` và các module sẵn có (`crate::sha256`).
- Python reference đối chiếu trực tiếp:
  `E:\Build_app\Volte\VoLTE\vendor_patcher\hd_vendor_build.py`.
- Harness độc lập: `%TEMP%\volte_audit_repro` (không ghi vào repo).
- Mọi lệnh build/test/clippy chạy `--offline`; integration test của pipeline chạy qua backend
  mặc định `RustFsExt4Backend`.

## 3. D03 — parser AVB có cấu trúc

### 3.1 Hiện trạng baseline

Audit chỉ đúng `avb.rs:122-148`: `fill_from_vbmeta` chỉ đọc một vùng vbmeta và điền
`partition_name`/algorithm từ dữ liệu tìm được, không phục hồi được `salt`, FEC roots,
AVB properties; các trường thiếu bị thay bằng giá trị mặc định. Hệ quả: rebuild có thể
mang salt/props sai hoặc bỏ mất metadata mà không báo lỗi.

### 3.2 Cách sửa

Parser mới đọc vbmeta theo đúng cấu trúc mà `avbtool.py` ghi:

- `parse_footer` (`avb.rs:78`): 64 byte footer, kiểm magic `AVBf`, `original_image_size`,
  `vbmeta_offset`, `vbmeta_size` trong biên ảnh.
- `fill_from_vbmeta` (`avb.rs:123`): đọc **đúng** `footer.vbmeta_size` byte; từ chối size 0
  hoặc lớn hơn `MAX_VBMETA = 64 KiB` (`avb.rs:167`). Không còn đọc tràn hay đoán vùng.
- `parse_vbmeta` (`avb.rs:221`): kiểm magic `AVB0`, kích thước header 256 byte
  (`avb.rs:169`), biên `auth`/`aux`, duyệt descriptor bằng `tag:u64 + num_bytes_following:u64`,
  bắt buộc `nbf` chia hết 8, mọi độ dài descriptor phải khớp công thức của chính nó.
  Tag lạ được bỏ qua giống `avbtool.py`; descriptor hỏng là `Err`.
- `parse_hashtree` (`avb.rs:291`): descriptor 180 byte; lấy `fec_roots`, `hash_algorithm`,
  `partition_name`, `salt`; kiểm `nbf == round8(164 + name_len + salt_len + digest_len)`.
- `parse_property` (`avb.rs:343`): descriptor 32 byte; kiểm
  `nbf == round8(16 + key_len + 1 + value_len + 1)`; giữ đủ cặp key/value.
- Helper `be_u32`/`be_u64`/`round8`/`fixed_string`/`hex_lower` (`avb.rs:187-215`) thay cho
  tìm kiếm chuỗi.

Không có fallback "đoán trường thiếu": vbmeta thiếu/hỏng là lỗi build, không phải metadata
mặc định.

### 3.3 Golden test và nguồn dữ liệu tham chiếu

- Fixture: `fixtures/avb/vbmeta_vendor_golden.bin`, **704 byte**, SHA-256
  `1C5338CD481F787AE627389681932A40BD38CF6F6CC284577819AD2197F1E81C`.
- Sinh bằng script `fixtures/avb/generate_vbmeta_golden.py`: ảnh 1 MiB toàn zero, gọi
  `tool/avbtool.py` 1.3.0 với partition `vendor`, `sha256`, salt cố định
  `00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`, hai prop
  `com.android.build.vendor=v2` và `com.android.build.vendor.extra=hello`, rồi trích đúng
  vùng `vbmeta_offset..vbmeta_offset+vbmeta_size`.
- Giá trị kỳ vọng lấy từ `avbtool.py info_image`, lưu tại
  `fixtures/avb/vbmeta_vendor_golden.info.txt` — **không** lấy từ output của parser.
- Test:
  - `the_golden_fixture_matches_its_reference_digest` (`avb.rs:821`): đúng 704 byte và đúng SHA-256.
  - `info_reads_the_golden_vbmeta_fields` (`avb.rs:830`): đọc đúng partition, algorithm, salt,
    FEC roots, props, offset/size.
  - `a_truncated_or_malformed_vbmeta_is_an_error` (`avb.rs:858`): magic sai, `vsz = 0`,
    `vsz = 65537`, `vsz = 100` (ngắn hơn header), `vsz = 300` (vượt biên aux) đều `Err`.

## 4. D03 — chứng minh footer mới và khôi phục nguyên byte

### 4.1 Vì sao thay cơ chế của Đợt 3

Đợt 3 chứng minh "footer mới" bằng cách so digest vùng vbmeta trước/sau. Cách đó không phân
biệt được một lần rebuild hợp lệ nhưng **byte-identical** (cùng salt/props) với việc backend
không làm gì. Đợt 4 thay bằng chứng cứ trực tiếp hơn: **xóa footer cũ trước khi gọi backend**.
Footer còn tồn tại sau lời gọi thì chỉ có thể do chính lần gọi này ghi; nếu backend lỗi, footer
đã lưu được đặt lại nguyên byte.

### 4.2 Cách sửa

- `rebuild_hashtree` (`avb.rs:567`) gọi `erase_footer` trước backend.
- `erase_footer` (`avb.rs:672`) chỉ xóa khi footer tại `partition_size - 64` **tự parse lại
  thành công**; footer không hợp lệ ⇒ `Err` "refusing to rebuild over an unverified footer",
  ảnh không bị chạm.
- Backend `Err` ⇒ `restore_or_report` (`avb.rs:705`) đặt footer cũ lại rồi trả `Err`
  (không bao giờ hạ cấp thành warning).
- Backend `Ok` nhưng `avb_info_fast` không thấy footer ⇒ khôi phục footer cũ và trả `Err`
  "AVB backend returned Ok but no AVB footer exists after the rebuild".
- `Ok(None)` chỉ còn dùng cho trường hợp hashtree lớn hơn slot đã khóa; `Ok(Some(info))`
  chỉ khi footer mới đọc lại hợp lệ.
- Toàn bộ `vbmeta_digest`/`old_vbmeta`/`sha512` của cơ chế cũ đã được xóa; `grep` trong
  `avb.rs` không còn ký tự nào.

### 4.3 Test

- `a_failing_backend_over_an_old_footer_is_never_reported_as_success` (`avb.rs:1053`):
  backend `Err`, ảnh **byte-for-byte** như trước, log không có "completed"/`[OK] AVB`.
- `a_backend_that_returns_ok_without_writing_a_footer_is_an_error` (`avb.rs:1081`):
  backend `Ok` nhưng không ghi footer ⇒ `Err`, footer cũ được đặt lại nguyên byte.
- `a_footer_that_does_not_reparse_is_not_erased` (`avb.rs:1098`): footer bị sửa magic ⇒
  `Err` trước khi xóa, ảnh không đổi.
- `a_verified_rebuild_reports_the_new_footer` (`avb.rs:1123`): backend ghi salt mới; metadata
  đọc lại mang đúng salt mới, chứng minh footer hiện tại do lần gọi này tạo.
- `the_rebuild_request_carries_the_parsed_metadata` (`avb.rs:1164`): request gửi backend có
  đúng `partition_size` khóa slot, tên partition, algorithm, salt và props lấy từ parser.

## 5. E01/E02 — xác minh toàn vẹn payload

### 5.1 Python reference

`verify_payload` tại `hd_vendor_build.py:1208-1247`:

- Dùng chung hai file host `verify_overlay.apk` / `verify_volte.rc` cho mọi candidate.
- Vòng lặp dừng ngay khi file **đã tồn tại** và APK > 1000 byte / RC > 10 byte
  (`:1218`, `:1230`) — file cũ của lần chạy trước thỏa điều kiện này.
- Ghi cả `/overlay/<name>` và `/overlay/HDVoLTE/<name>` vào cùng một host, bản sau ghi đè.
- Kết luận chỉ dựa trên kích thước: APK > 500 byte, RC > 10 byte (`:1240-1243`).
- Khi không dump được gì, vẫn log `[OK] ... THÀNH CÔNG` và in literal
  `VERIFIED_APK_OK` / `VERIFIED_RC_OK` (`:1241-1246`).

### 5.2 Cách sửa

- `check_copy` (`verify.rs:89`): **xóa file host trước** khi dump; nếu không xóa được ⇒
  `Stale`. Dump bằng một lệnh strict `dump -p <vpath> <host>` qua `Ext4Ops::run`; lỗi ⇒
  `Failed`; không có file ⇒ `Missing`; có file nhưng hash khác nguồn ⇒ `Mismatch`; khớp ⇒
  `Verified`.
- `verify_payload` (`verify.rs:145`) xác minh **đúng hai đích mà `overlay_commands` ghi**:
  - APK: `/overlay/HDVoLTE/{apk_name}` và `/overlay/{apk_name}`
  - RC: `/etc/init/{rc_name}` và `/etc/init/hw/{rc_name}`
- Mỗi bản copy có host riêng (`verify_overlay.apk`, `verify_overlay_root.apk`,
  `verify_volte.rc`, `verify_volte_hw.rc`), nên lần dump thứ hai không thể thừa hưởng kết quả
  của lần trước.
- Hash nguồn lấy từ `td/payload.apk` và `td/payload.rc` — chính file đã được ghi vào ảnh.
- `Verdict::Verified` chỉ khi **cả hai** bản copy của payload đó `Verified`.
- Token `APK_SHA256`/`RC_SHA256` chỉ in hash nguồn khi verified; nếu không thì `UNVERIFIED`.
  Không còn literal `VERIFIED_APK_OK`/`VERIFIED_RC_OK`, không còn `[OK]` vô điều kiện.
- Pipeline chỉ log `[STEP] Overlay APK + RC are inside the image` khi cả APK và RC đều
  verified (`pipeline.rs:553-561`); ngược lại log `[WARN] Overlay payload is not fully
  verified - still exporting the file.`

### 5.3 Định nghĩa `Verified` so với Python

Định nghĩa được đổi **có chủ đích** sau khi đối chiếu Python: Python coi "có file host lớn hơn
ngưỡng" là verified; port coi "cả hai bản copy trong ảnh vừa được dump và byte-identical với
payload nguồn" là verified. Lý do: ngưỡng kích thước không chứng minh nội dung, và file host
cũ không chứng minh lần chạy hiện tại đã ghi gì. Đây là thay đổi hành vi có tài liệu, không
phải trôi dạt ngẫu nhiên.

### 5.4 E02 — ghi trùng và bản copy cũ

- Ghi trùng được **giữ nguyên** vì `overlay_commands` của Python ghi cả hai đích; bỏ một đích
  sẽ phá parity và có thể làm thiết bị không nạp overlay.
- Lỗi che giấu của E02 được xử lý ở bước xác minh: cả hai bản copy đều phải tồn tại và khớp
  nguồn, nên một bản thiếu/hỏng không thể được bù bằng bản còn lại.
- Lỗi phía ghi ảnh đã được xử lý từ Đợt 2 (C01: `run` strict, lỗi đầu tiên là `Err`).

### 5.5 Test

| Test | Vị trí | Điều được chứng minh |
|---|---|---|
| `both_canonical_copies_must_match_the_payload` | `verify.rs:334` | 2 APK + 2 RC đều dump, hash khớp, 4 host riêng, `[OK]` đúng lúc |
| `the_mtk_rc_name_is_verified_in_both_init_directories` | `verify.rs:365` | RC MTK được kiểm ở cả `/etc/init` và `/etc/init/hw` |
| `a_stale_host_file_is_deleted_before_the_dump_and_never_hashed` | `verify.rs:385` | File cũ 6000 byte bị xóa; dump không ghi gì ⇒ `Missing`, `NotVerified` |
| `a_host_file_that_cannot_be_replaced_is_stale` | `verify.rs:404` | Không xóa được host ⇒ `Stale`, không hash, không `[OK]` |
| `a_failed_dump_is_reported_as_failed` | `verify.rs:424` | Dump lỗi ⇒ `Failed`, token `UNVERIFIED` |
| `a_copy_with_different_content_is_a_mismatch` | `verify.rs:442` | Một bản copy lệch nội dung ⇒ `Mismatch`, payload `NotVerified` |
| `a_missing_copy_is_not_ignored` | `verify.rs:464` | Thiếu bản copy thứ hai ⇒ `NotVerified`, không được bù bằng bản đầu |
| `without_a_source_payload_nothing_is_verified` | `verify.rs:481` | Không có payload nguồn + host cũ ⇒ `NotVerified`, `UNVERIFIED` |

## 6. A02 — backup và restore có xác minh

- `take_pre_avb_backup` (`pipeline.rs:213`): ghi size + SHA-256 của ảnh, copy sang backup,
  kiểm số byte đã copy và SHA-256 của bản backup. Bất kỳ sai lệch nào ⇒ `Err`
  "... refusing to run AVB without a verified restore point".
- `restore_pre_avb` (`pipeline.rs:246`): kiểm backup còn đúng size + hash; copy sang file phụ
  `<work>.restore.tmp`; kiểm lại size + hash của file phụ; `std::fs::rename` thay thế nguyên tử;
  kiểm lại ảnh sau rename. Mọi lỗi ⇒ `Err`, file phụ bị xóa, ảnh chính không bị ghi dở.
- Điểm gọi: backup trước AVB (`pipeline.rs:583-584`), restore trong nhánh `!boot_safe`
  (`pipeline.rs:602-603`). Không còn nhánh "restore lỗi nhưng tiếp tục xuất ảnh".
- Test: `a_pre_avb_backup_is_verified_by_size_and_hash` (`pipeline.rs:860`),
  `restore_replaces_the_image_with_the_verified_snapshot` (`:873`),
  `a_truncated_backup_makes_restore_fatal_and_leaves_the_image_untouched` (`:887`),
  `a_backup_with_the_wrong_hash_makes_restore_fatal` (`:902`),
  `a_failed_atomic_replace_is_fatal_and_removes_the_side_file` (`:916`).

## 7. G01 — init file ở thư mục gốc

- Baseline: virtual path `/init.target.rc` bị tách sai và ghi vào `/etc/init/hw`, trong khi
  log vẫn báo "Linked the init script into /init.target.rc".
- `split_vpath` (`init_rc.rs:35`): `/name` ⇒ `("/", name)`; `a/b/c` ⇒ `("a/b", "c")`;
  chuỗi rỗng, `/`, hoặc kết thúc bằng `/` ⇒ `None`. Không còn fallback im lặng.
- `link` dùng kết quả này để phát lệnh `cd <dir>` đúng nơi file được dump ra; nếu không tách
  được thì không ghi và không báo thành công.
- Test: `split_vpath_keeps_the_directory_the_file_came_from` (`init_rc.rs:331`) phủ
  `/init.target.rc`, `/etc/init/hw/init.target.rc`, nested hợp lệ, `"init.rc"`, và ba dạng
  không hợp lệ; `a_root_init_file_is_written_back_to_the_root` (`:354`) yêu cầu `rm
  /init.target.rc`, có `cd /`, **không** có `cd /etc/init/hw`;
  `a_nested_init_file_is_written_back_to_its_own_directory` (`:381`) yêu cầu `cd /etc/init/hw`
  và không có `cd /`.

## 8. F01 — cắt thông điệp lỗi theo ký tự

- `props.rs:232` đổi từ cắt byte tại 120 sang `e.message().chars().take(120).collect()`,
  nên không thể panic khi ký tự thứ 120 nằm giữa một codepoint UTF-8.
- Test `a_multibyte_error_message_is_truncated_by_char_without_panicking` (`props.rs:515`):
  thông điệp `119 × 'x' + 'ế' + 10 × 'y'`; kết quả giữ đúng 120 ký tự, giữ nguyên `ế`, không panic.
- Rà soát các chỗ cắt chuỗi khác: `pipeline::trunc` (`pipeline.rs:78`) đã dùng `chars()`;
  `avb.rs` cắt stderr bằng `chars().take(240)`; không còn byte-slice trên `str` trong đường
  thông điệp lỗi.

## 9. Bảng thay đổi code

| File | Vùng | Thay đổi |
|---|---|---|
| `volte-core/src/avb.rs` | 78-215, 123-370, 560-720, 800-1184 | Parser footer/vbmeta/descriptor theo cấu trúc; xóa footer trước rebuild, khôi phục khi lỗi; golden test |
| `volte-core/src/verify.rs` | 1-229, 231-496 | `CopyState`/`CopyCheck`; xóa host, dump strict, hash theo nguồn; xác minh hai bản copy |
| `volte-core/src/pipeline.rs` | 53-63, 211-278, 553-561, 583-584, 602-603, 853-928 | Backup/restore xác minh; chỉ `[STEP]` khi cả APK và RC verified |
| `volte-core/src/init_rc.rs` | 30-53, 330-391 | `split_vpath` trả `Option`, root là `/`; test root/nested/invalid |
| `volte-core/src/props.rs` | 232, 514-540 | Cắt lỗi theo ký tự; test multibyte |
| `fixtures/avb/` | toàn bộ | Script sinh fixture, vbmeta 704 byte, output `info_image`, README |

Diff: `9 files changed, 1275 insertions(+), 303 deletions(-)` (5 file nguồn: +1153/−303;
fixture: 18 + 71 + 33 dòng text và 1 file nhị phân 704 byte).

## 10. Regression tests

| Finding | Test chính | Vị trí | Chạy qua |
|---|---|---|---|
| D03 | golden digest, đọc metadata, vbmeta hỏng | `avb.rs:821`, `:830`, `:858` | seam `Avb`/file fixture |
| D03 | footer không parse lại, backend im lặng, rebuild thật, request | `avb.rs:1053-1184` | seam `Avb` |
| E01/E02 | 8 test verify | `verify.rs:334-495` | seam `Ext4Ops` (`FakeFs`) |
| A02 | 5 test backup/restore | `pipeline.rs:860-928` | file thật trong temp dir |
| G01 | 3 test split/link | `init_rc.rs:331-391` | seam `Ext4Ops` (`FakeOps`) |
| F01 | test multibyte | `props.rs:515` | seam `Ext4Ops` (`FakeOps`) |

Integration test end-to-end `a_full_build_reaches_the_final_tokens_on_a_synthetic_image`
(`pipeline.rs:1086`) và `an_avb_backend_failure_never_claims_a_passed_hashtree`
(`pipeline.rs:1264`) dùng backend mặc định thật `RustFsExt4Backend::new()`
(`pipeline.rs:1094`, `:1271`). Các unit test dùng seam là để kiểm tra hợp đồng của từng hàm
(parser, dump, backup) mà không cần dựng ảnh EXT4; đường build đầy đủ vẫn đi qua backend thật.

Số test sau đợt:

- `volte-core`: **176** (avb 20, verify 8, pipeline 19, props 11, init_rc 9, phần còn lại giữ nguyên).
- `volte-cli`: **11**.
- Không xóa/không nới lỏng test cũ. Các test cũ dựa trên parser tìm chuỗi (`sha512\0vendor`,
  `vsz = 4096`) được thay bằng fixture vàng vì chúng mô tả hành vi sai của D03.

## 11. Đối chiếu repro harness gốc

Harness tại `%TEMP%\volte_audit_repro`, chạy:

```text
cargo test --offline --test repro -- --test-threads=1
```

Kết quả lưu tại `E:\Build_app\Volte\_work\phase4_harness_after_fix.txt`:

```text
test result: FAILED. 1 passed; 10 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.48s
```

Mỗi failure là bằng chứng fix đang có hiệu lực, vì harness assert **hành vi lỗi cũ**:

| Test | Dòng quan sát | Ý nghĩa |
|---|---|---|
| `a01_out_equals_src_truncates_the_source` | `before=4096 after=4096`, lỗi "output path is the same file" | Đợt 1: nguồn không bị cắt; harness đòi lỗi "Copy vendor failed" |
| `a01b_case_alias_out_deletes_the_source` | `source_still_exists=true` | Đợt 1: source còn nguyên |
| `a04_fsck_failure_still_returns_ok` | `pipeline_ok=false` | Đợt 3: fsck lỗi ⇒ `Err` |
| `b01_trailing_dont_care_loses_the_tail` | `written=16384 actual=16384` | Đợt 3: đuôi DONT_CARE được mở rộng; harness đòi `actual == 0` |
| `c01_rustfs_returns_ok_after_rm_plus_failed_write` | `Err ... The system cannot find the file specified` | Đợt 2: write lỗi ⇒ `Err` |
| `c02_space_in_workdir_silently_skips_props` | `spaced_workdir_changed=75` | Đợt 2: workdir có dấu cách vẫn inject đủ prop |
| `d01_failing_backend_keeps_old_footer_and_reports_success` | `AVB vbmeta at offset 1048576: bad vbmeta magic (expected AVB0)` | **D03**: parser fail-closed trước khi rebuild; xem ghi chú bên dưới |
| `e01_stale_payload_is_reported_verified` | `apk=NotVerified rc=NotVerified stale_hash_matches=false` | E01: file cũ không được dùng |
| `f01_utf8_error_message_panics_on_byte_slice` | `panicked=false` | F01: không còn panic |
| `g01_root_init_file_is_written_to_the_wrong_directory` | `rm /init.target.rc`, `cd /`, write vào `init.target.rc` | G01: ghi đúng root; harness đòi `cd /etc/init/hw` |
| `g02_flags_case_alias_patches_in_place` | `ok` | Không đổi từ Đợt 1 |

So với Đợt 3 (`4 passed / 7 failed`), đúng **3 test** `e01`, `f01`, `g01` chuyển từ `ok`
sang `FAILED`; không test nào khác đổi trạng thái. Đó là ba finding của Đợt 4 đã được sửa,
không phải hồi quy.

**Ghi chú D01:** harness dựng footer trỏ tới vbmeta giả `sha512\0vendor` tại offset 1 MiB.
Parser mới từ chối vbmeta này ngay trong `avb_info_fast` (`bad vbmeta magic`) thay vì đi tiếp
đến backend như Đợt 3. Đây là hệ quả có chủ đích của D03: không rebuild trên metadata không
đọc được. Với footer do `avbtool.py` tạo, parser đọc thành công — chứng minh bằng golden fixture
ở §3.3.

## 12. Cổng build / test / clippy

```text
$ cargo build --workspace --offline
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s

$ cargo test --workspace --offline
running 11 tests
test result: ok. 11 passed; 0 failed; 0 ignored; ...
running 176 tests
test result: ok. 176 passed; 0 failed; 0 ignored; ...
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; ...

$ cargo clippy --workspace --all-targets --offline -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.01s
```

Test theo module sau đợt: `avb::` **20 passed**, `verify::` **8 passed**, `pipeline::`
**19 passed** (82.94s, gồm 3 integration build), `props::` **11 passed**, `init_rc::`
**9 passed**. Không có warning clippy.

## 13. Khác biệt có chủ đích so với Python reference

| ID | Python | Port | Lý do |
|---|---|---|---|
| `verify_payload` | Host dùng chung, dừng khi file cũ đủ lớn, chỉ so kích thước, `[OK]` vô điều kiện và literal `VERIFIED_*_OK` (`hd_vendor_build.py:1208-1247`) | Xóa host trước dump, strict dump, hash theo payload nguồn, xác minh cả hai bản copy, token `UNVERIFIED` khi chưa chứng minh | E01/E02: không xác nhận file cũ và không bỏ qua bản copy thứ hai |
| `rebuild_hashtree` | Backend lỗi vẫn có thể trả thành công nếu footer cũ còn (`hd_vendor_build.py:1271+`) | Xóa footer trước backend; backend lỗi/không ghi footer ⇒ `Err`, footer cũ được đặt lại | D01/D03: truthful success |
| AVB metadata | Điền mặc định khi thiếu trường | Parser cấu trúc; thiếu/hỏng ⇒ `Err`, không đoán | D03: metadata sai nguy hiểm hơn lỗi rõ ràng |
| Init RC path | Fallback về `/etc/init/hw` | `split_vpath` trả `None` cho đường dẫn không hợp lệ; root ghi đúng `/` | G01: không ghi sai thư mục rồi báo thành công |

Các khác biệt này được ghi trong module doc của `verify.rs` và `avb.rs`.

## 14. Fixture, tài liệu và giới hạn kiểm chứng

Đã thêm:

- `fixtures/avb/generate_vbmeta_golden.py` — sinh lại fixture, xóa ảnh tạm sau khi chạy.
- `fixtures/avb/vbmeta_vendor_golden.bin` — 704 byte, SHA-256 như §3.3.
- `fixtures/avb/vbmeta_vendor_golden.info.txt` — output `avbtool.py info_image` dùng làm tham chiếu.
- `fixtures/avb/README.md` — nguồn gốc, lệnh sinh, giá trị tham chiếu.

Không sửa `README.md`, `docs/PORT_STATUS.md`, `CLAUDE.md` trong đợt này.

**Giới hạn:** chưa có vendor image thật trong repo. Fixture là ảnh tổng hợp 1 MiB do
`avbtool.py` 1.3.0 tạo, nên parser mới được kiểm chứng với descriptor hashtree + property
chuẩn của avbtool, chưa kiểm chứng với descriptor đặc thù của từng hãng. Parser bỏ qua tag lạ
theo đúng hành vi `avbtool.py`, và từ chối mọi descriptor có độ dài không khớp công thức;
không có nhánh nào suy đoán giá trị thiếu.

## 15. Rủi ro còn lại

1. **D03 chưa có ảnh vendor thật.** Fixture tổng hợp chứng minh parser đúng với avbtool 1.3.0,
   không chứng minh mọi biến thể descriptor của OEM. Nếu gặp descriptor lạ, parser bỏ qua
   (giống avbtool) hoặc báo lỗi nếu độ dài sai — không âm thầm đoán.
2. **Verify yêu cầu cả hai bản copy.** Nếu một ảnh hợp lệ cố ý chỉ có một đích (không giống
   `overlay_commands`), kết quả sẽ là `NotVerified` (an toàn, không false success). Muốn thay
   đổi định nghĩa này phải đối chiếu lại `overlay_commands` và Python reference trước.
3. **Ghi trùng vẫn tồn tại.** E02 được xử lý ở bước xác minh, không ở bước ghi, vì parity.
   Nếu sau này bỏ một đích, phải bỏ đồng thời kiểm tra tương ứng.
4. **Restore dùng `std::fs::rename`.** Trên Windows, rename thay thế file đích; nếu đích là
   thư mục hoặc bị khóa, lỗi được trả fatal và file phụ bị xóa (đã test). Trường hợp ổ đĩa đầy
   giữa lúc copy file phụ cũng trả `Err`, không đụng ảnh chính.
5. **F01 chỉ nói về thông điệp lỗi.** Các chuỗi khác trong log không cắt theo byte; nếu thêm
   chỗ cắt mới, phải dùng `chars()` như `pipeline::trunc`.

## 16. Điều kiện hoàn thành, Git và kết luận

- [x] Không xác minh nhầm file cũ — host bị xóa trước dump; `Stale`/`Missing`/`Failed` không
      bao giờ `Verified` (`verify.rs:89-125`; test `verify.rs:385`, `:404`, `:424`).
- [x] Payload có kiểm tra nội dung — SHA-256 của cả hai bản copy so với payload nguồn
      (`verify.rs:145-229`; test `verify.rs:334`, `:442`).
- [x] Backup/restore được xác minh — size + SHA-256 trước và sau, side-file + rename nguyên tử,
      lỗi fatal (`pipeline.rs:213-278`; test `pipeline.rs:860-928`).
- [x] AVB metadata được phân tích đúng theo dữ liệu kiểm chứng được — parser cấu trúc, golden
      fixture từ avbtool 1.3.0 (`avb.rs:78-370`, `:821-889`); giới hạn ghi ở §14.
- [x] Init RC đúng đường dẫn — `/init.target.rc` ghi tại `/`, nested ghi tại thư mục gốc của nó
      (`init_rc.rs:35-53`; test `init_rc.rs:331-391`).
- [x] Không còn UTF-8 panic — `props.rs:232` dùng `chars()`; test `props.rs:515`.
- [x] `git commit` cho đợt đã thực hiện — `3aa7d4d`.
- [x] Báo cáo này được commit riêng sau `3aa7d4d`; hash xem `git log --oneline -2`.
- [x] Không `--amend`, không force-push, không viết lại lịch sử; không đụng `CLAUDE.md`.

**Git:**

- Baseline: `0ac8b47`.
- Commit code Đợt 4: `3aa7d4d` — `Đợt 4: sửa toàn vẹn dữ liệu AVB, payload, backup và đường dẫn init`
  (9 file, +1275/−303, gồm `fixtures/avb/`).

**Kết luận:** Đợt 4 hoàn tất. Sáu finding D03/E01/E02/A02/G01/F01 đã được sửa và chứng minh
bằng test trong repo, bằng golden fixture sinh từ `avbtool.py`, và bằng harness độc lập
(10 failure đều là harness assert hành vi lỗi cũ, 3 test chuyển trạng thái đúng như dự kiến).
`cargo build`, `cargo test` (176 core + 11 CLI, 0 failed) và `cargo clippy -D warnings` đều PASS.
Không phát hiện BLOCKED hay rủi ro mất dữ liệu/không thể đảo ngược, nên được phép nối sang Đợt 5
theo brief.