# PHASE 2 — BACKEND EXT4 VÀ XỬ LÝ LỖI (C01 / C02)

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline của đợt:** `1075c60` (Đợt 1: báo cáo PHASE_1_SOURCE_PROTECTION)
**Commit của đợt:** `ac54ad33f4c2c5be3c5cf2e33574d2cef9ee0f60` (`ac54ad3`)
**Trạng thái đợt:** HOÀN THÀNH — dừng chờ người dùng xác nhận trước Đợt 2.5

---

## 1. Mục tiêu

`RustFsExt4Backend` (backend mặc định của CLI) không được báo thành công khi một lệnh chỉnh sửa
EXT4 thất bại (C01), và không được hỏng khi đường dẫn host có dấu cách hoặc ký tự Unicode (C02).
Mọi regression test của đợt này chạy qua chính `RustFsExt4Backend`, không qua `DebugfsBackend`.

## Bổ sung theo yêu cầu người dùng (chốt trước Đợt 2)

**2.1. Test chứng minh `--out` là đường dẫn khác bình thường, không alias với nguồn.**

Trong 157 test hiện có ở baseline `1075c60`, **không có test nào chỉ định một `--out` khác nguồn
một cách tường minh**:

- `pipeline::a_full_build_reaches_the_final_tokens_on_a_synthetic_image` dùng `out_path: ""`
  (đường dẫn mặc định cạnh nguồn), không phải `--out` do người dùng chỉ định.
- `pipeline::output_path_defaults_next_to_the_source` và `explicit_out_path_is_kept_when_keep_out`
  chỉ kiểm tra `resolve_output`, không chạy build.
- Các test còn lại của `pipeline`/`path_id`/`flags` đều là nhánh bị chặn (alias).

Vì vậy đã bổ sung test chạy **toàn bộ pipeline** qua `RustFsExt4Backend` với `--out` tường minh:
`pipeline::an_explicit_out_path_that_differs_from_the_source_is_accepted`
(`volte-core/src/pipeline.rs:997`). Test tạo ảnh EXT4 thật, seed payload, chạy build với
`out_path = <tmp>/output/vendor_volte_custom.img`, rồi khẳng định:

- `out` đúng bằng đường dẫn đã chỉ định;
- hash SHA-256 của nguồn không đổi;
- kích thước output bằng nguồn;
- log có `[OUTPUT]`, `e2fsck=PASS`;
- `APK_SHA256` khác `UNVERIFIED` (overlay được ghi và verify qua backend mặc định).

Test này vừa chứng minh guard của Đợt 1 không chặn nhầm trường hợp hợp lệ, vừa là regression test
end-to-end cho Đợt 2.

**2.2. Đổi tên test G02 trong repro harness.**

`g02_flags_case_alias_bypasses_same_path` → `g02_flags_case_alias_patches_in_place` trong
`C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro\tests\repro.rs:111`. Chỉ đổi tên hàm; nội dung
và assertion giữ nguyên. Harness chạy lại: test PASS.

## 2. Finding được xử lý

| ID | Mức độ | Mô tả |
|---|---|---|
| C01 | Critical | `RustFsExt4Backend::run` gom lỗi từng lệnh vào text rồi vẫn trả `Ok`, nên batch `rm` + `write` thất bại được báo là thành công (mất dữ liệu mà build không biết) |
| C02 | High | Parser `command.split_whitespace()` cắt đường dẫn host có dấu cách, khiến `write`/`dump`/`ea_set` trượt âm thầm (props injection trả 0 thay đổi) |

## 3. Trạng thái trước sửa (baseline `1075c60`)

- `volte-core/src/rustfs_ext4.rs:105-167` — `Session::execute` bắt đầu bằng
  `let mut parts = command.split_whitespace();`. Mọi host path có dấu cách bị cắt tại dấu cách đầu
  tiên; phần còn lại bị hiểu thành token khác.
- `volte-core/src/rustfs_ext4.rs:244-271` — vòng lặp của `run`:
  `if let Err(message) = session.execute(...) { out.push_str(&format!("debugfs: ...")) }`, sau đó
  `finish()` và `Ok(out)`. Không có đường nào trả `Err` cho một lệnh thất bại; caller chỉ thấy text.
- Hệ quả đo được trong repro harness trước sửa: `props::upsert` trên workdir có dấu cách trả
  `changed = 0` (im lặng), trong khi workdir không dấu cách trả `75`; batch `rm` + `write` hỏng trả
  `Ok` kèm text `debugfs:`.

## 4. Nguyên nhân gốc

1. **C01** — backend được viết theo mô phỏng `debugfs -f`: lỗi từng lệnh chỉ là output, không phải
   trạng thái. Hợp đồng đó đúng cho debugfs nhưng sai cho engine mặc định: pipeline tin `Ok` nghĩa là
   ảnh đã được sửa, nên một `write` thất bại sau `rm` bị coi là thành công.
2. **C02** — grammar `debugfs` không có quoting; parser dùng `split_whitespace` cho mọi token, trong
   khi host path là trường tự do duy nhất có thể chứa dấu cách. Caller (`props`, `init_rc`, `verify`,
   `payload`, `identify`) truyền host path tuyệt đối, nên workdir có dấu cách là đủ để hỏng.

## 5. Phương án sửa

1. **Fail-closed mặc định** (`Ext4Ops::run`): lệnh đầu tiên thất bại → `finish()` để trả mount sạch,
   rồi trả `Err(Build)` chứa nguyên văn lệnh và nguyên nhân. Batch dừng tại lỗi, không chạy tiếp các
   lệnh phụ thuộc.
2. **Giữ ngữ nghĩa probe một cách tường minh**: thêm method có default implementation
   `Ext4Ops::run_best_effort` (mặc định gọi `run`). `DebugfsBackend` giữ nguyên hành vi (debugfs `-f`
   vốn tiếp tục sau lỗi); `RustFsExt4Backend` override bằng vòng lặp bỏ qua lỗi và gom text — đúng
   cho các batch dò file không tồn tại.
3. **C02 — parser theo grammar thật của từng verb**, không dùng `split_whitespace` cho host path:
   - `write <host> <name>`: token cuối là tên file trong ảnh, phần còn lại là host path.
   - `ea_set -f <host> <name> <attr>`: bóc `attr` và `name` từ cuối, phần giữa là host path.
   - `dump [-p] <vpath> <host>`: `vpath` là token đầu, phần còn lại là host path.
   - `cd`/`mkdir`/`rmdir`/`rm`: dùng toàn bộ phần còn lại làm path.
   - `set_inode_field`: tách 3 token (tên/trường/giá trị đều không chứa dấu cách).
4. **`rm` file không tồn tại là idempotent**: batch overlay có `rm` file features tuỳ chọn; coi
   `NotFound` của `rm` là thành công (giống cleanup của Python/debugfs), tránh fail-closed chặn nhầm.
5. **Chỉ định rõ call site nào là probe**: clean overlay batch, dumps ứng viên của `init_rc`/`verify`,
   dump prop files/`identify`, dump flag bit, và `ea_set` nhãn SELinux sau khi ghi prop. Các batch
   ghi thật (`overlay_commands` write, `props::write_back`, `init_rc` write-back) giữ `run` strict.

## 6. File đã thay đổi

| File | Thay đổi |
|---|---|
| `volte-core/src/rustfs_ext4.rs` | parser theo verb; `run` fail-closed; override `run_best_effort`; `open_session`; `rm` idempotent; +7 test mới, 1 test cũ viết lại |
| `volte-core/src/ext4.rs` | thêm method default `run_best_effort` vào trait; `read_vendor_flag_bit` dùng probe |
| `volte-core/src/pipeline.rs` | clean batch dùng probe; +1 test end-to-end `--out` tường minh |
| `volte-core/src/props.rs` | 3 call site dump/`ea_set` chuyển sang probe |
| `volte-core/src/init_rc.rs` | dump ứng viên chuyển sang probe |
| `volte-core/src/verify.rs` | `dump_batch` chuyển sang probe |

`git show --stat ac54ad3`: 6 files changed, 352 insertions(+), 51 deletions(-).

## 7. Chi tiết thay đổi

- `volte-core/src/rustfs_ext4.rs:86` — `split_verb`; `:95` — `split_token` (giữ phần còn lại);
  `:104` — `split_last_token`.
- `volte-core/src/rustfs_ext4.rs:170` — `rm`: `FsError::NotFound` → `Ok(())`, kèm chú thích lý do.
- `volte-core/src/rustfs_ext4.rs:183` — `write`: host = phần trước token cuối.
- `volte-core/src/rustfs_ext4.rs:246` — `ea_set`: bóc `attr`, `name` từ cuối.
- `volte-core/src/rustfs_ext4.rs:266` — `dump`/`dump -p`: host = phần còn lại sau `vpath`.
- `volte-core/src/rustfs_ext4.rs:298` — `open_session` dùng chung cho hai chế độ.
- `volte-core/src/rustfs_ext4.rs:310` — `run` strict: lỗi đầu tiên → `finish()` → `Err(Build)`.
- `volte-core/src/rustfs_ext4.rs:334` — `run_best_effort`: gom `debugfs: <lệnh>: <lỗi>` và tiếp tục.
- `volte-core/src/ext4.rs:56` — `run_best_effort` default gọi `run` (giữ nguyên hành vi debugfs).
- `volte-core/src/pipeline.rs:394` — clean batch dùng probe (đây là batch xoá path có thể vắng).
- `volte-core/src/ext4.rs:271`, `props.rs:161`, `props.rs:255`, `props.rs:272`, `init_rc.rs:59`,
  `verify.rs:79` — các batch probe tường minh.

## 8. Regression tests mới (chạy qua `RustFsExt4Backend`)

Tổng: **+7 test** so với baseline (157 → 164).

| Test | Nội dung |
|---|---|
| `rustfs_ext4::a_failed_command_is_an_error_and_stops_the_batch` | lệnh lỗi → `Err(Build)` chứa lệnh + nguyên nhân; lệnh trước giữ, lệnh sau không chạy |
| `rustfs_ext4::a_failed_write_after_rm_is_reported_instead_of_succeeding` | đúng hình dạng C01: `rm` xong `write` hỏng → `Err`, không còn "thành công giả" |
| `rustfs_ext4::a_failed_replacement_keeps_the_existing_file_intact` | thay nội dung thất bại → file cũ còn nguyên byte-for-byte |
| `rustfs_ext4::removing_an_absent_path_is_idempotent` | `rm` path vắng không chặn batch (cleanup hợp lệ) |
| `rustfs_ext4::best_effort_reports_failures_and_keeps_going` | probe batch báo lỗi trong output và chạy tiếp (viết lại từ test cũ) |
| `rustfs_ext4::host_paths_with_spaces_and_non_ascii_survive_write_and_dump` | workdir `audit dir\thư mục làm việc`, host `seed file.prop` → write + dump đúng nội dung |
| `rustfs_ext4::props_upsert_works_from_a_work_directory_with_spaces` | `props::upsert` trên workdir có dấu cách trả đủ 75 thay đổi và prop nằm trong ảnh |
| `pipeline::an_explicit_out_path_that_differs_from_the_source_is_accepted` | build đầy đủ qua backend mặc định với `--out` tường minh (yêu cầu 2.1) |

Test cũ `a_failed_command_is_reported_and_the_batch_continues` được **viết lại**, không bị xoá:
phần "tiếp tục sau lỗi" chuyển sang `best_effort_reports_failures_and_keeps_going`, phần "không được
báo thành công" là `a_failed_command_is_an_error_and_stops_the_batch`.

## 9. Kết quả đối chiếu repro harness gốc

Harness: `C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro\tests\repro.rs` (chỉ đổi tên test G02).
Lệnh: `cargo test --offline --test repro -- --test-threads=1`.

Kết quả: **7 passed, 4 failed**. Bốn "failed" đều là test cũ khẳng định hành vi lỗi, nay không còn
tái hiện:

```
C01: ops.run(...).unwrap() giờ trả Err:
debugfs: write ...\missing-host.prop build.prop: ... The system cannot find the file specified.
     (os error 2)
     → test cũ panic vì nó kỳ vọng Ok + text "debugfs:".

C02: spaced_workdir_changed=75 control_changed=0
     → test cũ panic ở assert spaced == 0 (bug đã hết). Control = 0 vì lần chạy có dấu cách
       đã ghi đủ 75 prop vào ảnh; lần control chạy sau đó là idempotent (không còn gì để đổi).

A01/A01b: vẫn "failed" đúng như báo cáo Đợt 1 (test cũ kỳ vọng nguồn bị phá huỷ).
G02: g02_flags_case_alias_patches_in_place ... PASS (tên mới).
```

Không sửa nội dung assertion của C01/C02 trong harness: đây là bằng chứng gốc, việc chúng fail sau
khi sửa là dấu hiệu bug không còn tái hiện.

## 10. Lệnh kiểm thử đã chạy

```
cargo check --offline --workspace --all-targets                      → PASS
cargo clippy --offline --workspace --all-targets -- -D warnings     → PASS, 0 cảnh báo
cargo test --offline --workspace                                     → 10 CLI + 154 core = 164 PASS, 0 FAIL
cargo test --offline -p volte-core rustfs_ext4::                     → 11 PASS
cargo test --offline -p volte-core an_explicit_out_path_...          → 1 PASS
cargo fmt --check                                                    → 112 diff block (xem §12)
cargo test --offline --test repro -- --test-threads=1 (ngoài repo)   → 7 PASS, 4 FAIL (kỳ vọng)
```

## 11. Kết quả thực tế

- Test: **164 PASS / 0 FAIL** (157 → 164, +7).
- `an_explicit_out_path_that_differs_from_the_source_is_accepted` chạy thật toàn pipeline qua
  `RustFsExt4Backend` + `e2fsck.exe` thật: PASS (44 s), chứng minh strict `run` không chặn build hợp lệ.
- `cargo fmt --check`: **112 diff block**. Baseline `1075c60` đo bằng `git worktree` riêng: **113**.
  Không thêm drift; giảm 1 vì nhánh `dump` được viết lại đúng format. Không chạy `cargo fmt --all`
  (sẽ tạo diff toàn dự án; để Đợt 5 theo quy định).
- Không thêm dependency, không đổi `Cargo.toml`/`Cargo.lock`.

## 12. Test chưa chạy và lý do

- Không có test nào bị bỏ qua có chủ ý trong đợt này.
- Chưa chạy parity đối chiếu từng byte với `debugfs.exe` — thuộc Đợt 2.5 theo kế hoạch; `DebugfsBackend`
  vẫn được giữ nguyên làm oracle.
- Chưa chạy trên ảnh vendor thật/thiết bị thật — ngoài phạm vi và bị cấm bởi quy tắc an toàn.

## 13. Rủi ro còn lại

- **Batch không phải transaction**: lệnh trước lỗi vẫn đã commit. Fail-closed chỉ đảm bảo không báo
  thành công, không rollback. `props::write_back` vẫn `rm` trước rồi `write` (giữ tương thích debugfs);
  nếu `write` hỏng, file cũ đã mất nhưng build nhận `Err` thay vì thành công giả. Việc bỏ `rm`/chuyển
  sang replace-in-place cần đối chiếu 2.5 rồi mới đổi.
- `run_best_effort` là lựa chọn tường minh của caller; nếu caller mới dùng sai cho batch ghi thật thì
  lỗi sẽ bị bỏ qua. Hiện chỉ 6 call site probe được liệt kê ở §8.
- Pipeline vẫn chỉ `[WARN]` khi `init_rc::link`/`props::upsert` thất bại (B01/A04) — thuộc Đợt 3/4.
- Parser vẫn giả định tên file trong ảnh và `vpath` không chứa dấu cách (đúng với mọi lệnh pipeline
  sinh ra); chỉ host path là trường tự do.
- `DebugfsBackend` vẫn lenient (đúng vai trò oracle); sau 2.5 khi gỡ `debugfs.exe` cần rà lại mọi
  caller còn dùng `run_best_effort`.

## 14. Ảnh hưởng đến Python reference

- **C01 — phân kỳ có chủ đích**: Python/debugfs tiếp tục sau lỗi và coi build là xong; Rust giờ trả
  lỗi khi lệnh sửa ảnh thất bại. Đây là yêu cầu P0 của audit, không phải thay đổi thuật toán.
- **C02 — sửa lỗi thuần tuý**: Python cũng hỏng với workdir có dấu cách (giống `split_whitespace`);
  bản Rust giờ xử lý đúng. Không đổi cú pháp lệnh, không đổi nội dung ảnh với workdir thông thường.
- Nhánh `rm` file vắng giữ đúng hành vi Python (bỏ qua), nên không tạo khác biệt mới.

## 15. Git commit hash của đợt

- Code + test: `ac54ad33f4c2c5be3c5cf2e33574d2cef9ee0f60` (`ac54ad3`)
  `Đợt 2: backend EXT4 báo lỗi thật và xử lý host path có dấu cách (C01/C02)`
- Baseline: `1075c60`

## 16. Kết luận đợt

C01 và C02 đã **FIXED** trên backend mặc định `RustFsExt4Backend`: không còn batch sửa ảnh nào trả
`Ok` khi một lệnh thất bại, và host path có dấu cách/Unicode đi qua `write`, `ea_set`, `dump` nguyên
vẹn. Regression test chạy qua chính backend mặc định, gồm một build end-to-end với `--out` tường minh
để chứng minh guard Đợt 1 không chặn nhầm. `DebugfsBackend` không bị xóa/tái cấu trúc, vẫn đủ điều
kiện làm oracle cho Đợt 2.5.

**DỪNG TẠI ĐÂY — chờ người dùng xác nhận "tiếp tục" trước khi sang Đợt 2.5.**
