# PHASE 1 — BẢO VỆ ẢNH NGUỒN (A01 / A01b / G02)

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline:** `e25c79c` (Import the Rust vendor patcher port V1.0)
**Commit của đợt:** `2978e1be77689a6907dc808e4b4d10d6ec536d88` (`2978e1b`)
**Trạng thái đợt:** HOÀN THÀNH — dừng chờ người dùng xác nhận trước Đợt 2

---

## 1. Mục tiêu

Loại bỏ mọi đường đi có thể xóa, truncate hoặc ghi đè ảnh đầu vào khi đường dẫn output trùng, alias
theo chữ hoa/thường, symlink hoặc hard link với nguồn.

## 2. Finding được xử lý

| ID | Mức độ | Mô tả |
|---|---|---|
| A01 | Critical | `--out` trùng nguồn làm nguồn bị truncate về 0 byte |
| A01b | Critical | Alias chỉ khác chữ hoa/thường trên Windows làm `remove_file` xóa nguồn |
| G02 | Low | `same_path` trong `flags.rs` so sánh case-sensitive, alias chữ hoa/thường đi vào nhánh `fs::copy` và lỗi |

## 3. Trạng thái trước sửa

Đối chiếu trực tiếp với mã nguồn tại baseline `e25c79c`:

- `pipeline.rs` (bản cũ dòng 317): `if out_path.is_file() && std::path::absolute(&out_path)? != src`
  so sánh `PathBuf`/absolute path theo kiểu phân biệt hoa thường. Trên Windows, `VENDOR.IMG` và
  `vendor.img` là cùng một file nhưng guard cho là khác, rồi gọi `remove_file(&out_path)` → xóa nguồn.
- `copy_with_progress` (bản cũ dòng 181–182) mở `File::create(dst)` trước khi đọc nguồn; khi
  `--out` chính là nguồn, file bị truncate về 0 byte trước khi vòng copy chạy.
- `resolve_output` (bản cũ dòng 138) chỉ so sánh `std::path::absolute(&path)? == src`, cũng
  case-sensitive và không nhận diện hard link/symlink.
- `flags::same_path` (bản cũ dòng 124–129) so sánh absolute path bằng `==`.

## 4. Nguyên nhân gốc

Danh tính file được quyết định bằng **chuỗi đường dẫn**, trong khi Windows là filesystem
case-insensitive và đường dẫn có thể là symlink/hard link. Không có bước xác minh danh tính file
(volume serial + file index trên Windows, device + inode trên Unix) trước thao tác phá hủy.

## 5. Phương án sửa

1. Thêm module `volte-core/src/path_id.rs` với `same_file(a, b)`:
   - Dùng `same_file::is_same_file` (so sánh danh tính file thật: volume/file index trên Windows,
     dev/inode trên Unix) → nhận diện cùng file, case alias, symlink, hard link.
   - Khi không mở được đường dẫn (thường vì đích chưa tồn tại) → fallback so sánh absolute path,
     case-insensitive trên Windows.
2. Chặn sớm trong `pipeline::run` ngay sau `resolve_output`, trước mọi thao tác tạo/xóa/truncate.
3. Thêm guard thứ hai ngay trong `copy_with_progress` — điểm truncate thực tế.
4. Thay guard `remove_file` và guard đặt tên mặc định bằng `path_id::same_file`.
5. Thay `flags::same_path` bằng `path_id::same_file` để patch tại chỗ thay vì copy đè.
6. Không xóa/thay đổi nguồn, trả `Err` với thông báo rõ ràng; không tạo kết quả thành công giả.

Dependency: thêm `same-file = "1.0.6"` (đã có sẵn trong cache registry cùng `winapi-util`).
Lý do: API danh tính file của std trên Windows (`file_index`, `volume_serial_number`) vẫn là
unstable (`windows_by_handle`, issue #63010); std không đáp ứng được nhu cầu này.

## 6. File đã thay đổi

| File | Thay đổi |
|---|---|
| `volte-core/Cargo.toml` | + `same-file = "1.0.6"` |
| `Cargo.lock` | + `same-file`, `winapi-util`, `windows-sys`, `windows-link` (khóa phiên bản) |
| `volte-core/src/lib.rs` | + `pub mod path_id;` |
| `volte-core/src/path_id.rs` | **mới** — helper danh tính file + 5 test |
| `volte-core/src/pipeline.rs` | guard sớm, guard trong copy, thay 2 so sánh path, + 5 test |
| `volte-core/src/flags.rs` | dùng `path_id::same_file`, + 2 test |

`git show --stat 2978e1b`: 6 files changed, 368 insertions(+), 10 deletions(-).

## 7. Chi tiết thay đổi

- `volte-core/src/path_id.rs:14` — `same_file` dùng `is_same_file`, fallback `absolute_eq`
  (`path_id.rs:22`), case-fold trên Windows (`path_id.rs:30`).
- `volte-core/src/pipeline.rs:250` — chặn `out_path` trùng nguồn trước khi tạo thư mục/copy:
  trả `Err("Refusing to build: the output path is the same file as the source image ...")`.
- `volte-core/src/pipeline.rs:182` — guard bổ sung trong `copy_with_progress` trước `File::create`.
- `volte-core/src/pipeline.rs:139` — nhánh đặt tên mặc định `_VolLTE_out.img` dùng `same_file`.
- `volte-core/src/pipeline.rs:331` — guard `remove_file` dùng `same_file`.
- `volte-core/src/flags.rs:134` — `patch_flags_keep_size` bỏ copy khi src/dst là cùng file.

## 8. Regression tests mới

Tổng: **12 test** (5 `path_id`, 5 `pipeline`, 2 `flags`).

| Test | Nội dung |
|---|---|
| `path_id::identical_paths_are_the_same_file` | cùng path = true; path khác = false |
| `path_id::relative_and_absolute_paths_are_the_same_file` | path tương đối vs tuyệt đối |
| `path_id::case_only_alias_is_the_same_file_on_windows` | `img.bin` vs `IMG.BIN` |
| `path_id::symlink_is_the_same_file_when_the_platform_allows_it` | symlink (bỏ qua nếu OS từ chối) |
| `path_id::hard_link_is_the_same_file_when_the_platform_allows_it` | hard link |
| `pipeline::output_equal_to_the_source_is_refused_and_the_source_is_untouched` | `--out == src`: Err + hash/độ dài nguồn không đổi |
| `pipeline::relative_source_with_absolute_output_is_refused` | src tương đối, out tuyệt đối |
| `pipeline::case_only_output_alias_is_refused_and_the_source_is_untouched` | alias hoa/thường (Windows) |
| `pipeline::symlinked_output_is_refused_when_the_platform_allows_it` | out là symlink |
| `pipeline::hard_linked_output_is_refused_when_the_platform_allows_it` | out là hard link |
| `flags::case_alias_patches_in_place_instead_of_copying` | G02: patch tại chỗ, nguồn = `volte_support=1` |
| `flags::hard_link_alias_patches_in_place_when_the_platform_allows_it` | G02 với hard link |

Mọi test alias đều kiểm tra hash SHA-256 và/hoặc độ dài nguồn trước/sau, không chỉ kiểm tra
`Result`. Một số test hard link/symlink tự bỏ qua khi nền tảng không tạo được link.

## 9. Kết quả đối chiếu repro harness gốc

Harness: `C:\Users\HNTCPR\AppData\Local\Temp\volte_audit_repro\tests\repro.rs` (không sửa).
Lệnh: `cargo test --offline --test repro -- --test-threads=1`.

Kết quả: **9 passed, 2 failed** — hai "failed" là hai test cũ khẳng định hành vi phá hủy, nay
không còn tái hiện:

```
A01 before=4096 after=4096 result=Err(... "Refusing to build: the output path is the same file
as the source image (...\vendor.img). Choose another --out path; the source was not modified.")
panic: assertion failed: result.unwrap_err().message().contains("Copy vendor failed")

A01b result=Err(... "Refusing to build: the output path is the same file as the source image
(...\VENDOR.IMG). Choose another --out path; the source was not modified.") source_still_exists=true
panic: repro: the case-alias destination removed the source
```

Diễn giải: `after=4096` (nguồn còn nguyên) và `source_still_exists=true` chứng minh A01/A01b đã
được sửa; test cũ fail vì nó kỳ vọng nguồn bị truncate/xóa. `g02_flags_case_alias_bypasses_same_path`
**PASS** (patch tại chỗ, nguồn `volte_support=1`). 8 test còn lại (A04, B01, C01, C02, D01, E01, F01,
G01) vẫn tái hiện đúng như audit — thuộc các đợt sau.

## 10. Lệnh kiểm thử đã chạy

```
cargo check --offline --workspace --all-targets      → PASS
cargo clippy --offline --workspace --all-targets -- -D warnings → PASS, 0 cảnh báo
cargo test --offline --workspace                     → 10 CLI + 147 core = 157 PASS, 0 FAIL
cargo test --offline -p volte-core --lib -- --nocapture output → 7 PASS (5 test mới + 2 test resolve_output cũ)
cargo fmt --all -- --check                           → 113 diff block (xem §11/§12)
cargo test --offline --test repro -- --test-threads=1 (harness ngoài repo) → 9 PASS, 2 FAIL (kỳ vọng)
```

## 11. Kết quả thực tế

- `cargo check`: PASS.
- `cargo clippy -D warnings`: PASS, 0 cảnh báo.
- Test: **157 PASS / 0 FAIL** (trước Đợt 1: 145 → +12 test).
- Các test bảo vệ nguồn chạy thực tế (không bị lọc):
  `relative_source_with_absolute_output_is_refused`, `output_equal_to_the_source_is_refused...`,
  `case_only_output_alias_is_refused...`, `symlinked_output...`, `hard_linked_output...` đều PASS.
- Hard link: `fsutil hardlink create` trên volume này thành công → nhánh assertion hard link thực sự chạy.
- Symlink: `symlink_file` trả lỗi OS 1314 ("A required privilege is not held by the client") trên máy
  này → hai test symlink thoát sớm (không assertion). Ghi nhận là giới hạn môi trường, không phải PASS giả.
- `cargo fmt --check`: **113 diff block**. Đo baseline `e25c79c` bằng `git worktree` riêng: cũng **113**.
  Tức Đợt 1 không thêm drift định dạng. Không chạy `cargo fmt --all` (sẽ tạo diff toàn dự án, tách
  sang Đợt 5 theo quy định).
- `Cargo.lock` thay đổi chỉ do thêm `same-file` và dependency bắc cầu; không nâng phiên bản dependency cũ.

## 12. Test chưa chạy và lý do

- Test symlink trên Windows: bị OS từ chối tạo symlink (privilege 1314) → thân test không chạy.
  Đường code vẫn được kiểm chứng gián tiếp qua hard link (cùng hàm `same_file`) và case alias.
- Không chạy build thật trên ảnh vendor: Đợt 1 chỉ thay đổi guard trước khi chạm ảnh; kiểm chứng
  bằng ảnh vendor thật thuộc Đợt 2.5 theo kế hoạch.

## 13. Rủi ro còn lại

- Nếu file đích đang bị khóa đến mức không mở được để đọc, `is_same_file` trả lỗi và fallback chỉ so
  sánh absolute path (case-insensitive). Khi đó alias hard link/symlink hiếm gặp có thể không bị phát
  hiện; trường hợp `--out == src` và case alias vẫn bị chặn bởi fallback.
- Hành vi thay đổi có chủ đích: `--out` trùng nguồn giờ trả lỗi thay vì phá hủy nguồn. Caller nào
  trước đây dựa vào việc "build vẫn chạy" với `--out == src` phải đổi cách gọi.
- Chưa xử lý A02 (backup/restore) và các copy khác của pipeline — thuộc Đợt 4.

## 14. Ảnh hưởng đến Python reference

Python reference (`hd_vendor_build.py`) không có guard danh tính file và sẽ truncate/xóa nguồn trong
cùng tình huống. Đợt 1 **cố ý phân kỳ** khỏi Python để bảo vệ dữ liệu, theo yêu cầu P0 của audit và
prompt. Không thay đổi thuật toán build, tên output hợp lệ, hay nội dung ảnh.

## 15. Git commit hash của đợt

- Code + test: `2978e1be77689a6907dc808e4b4d10d6ec536d88` (`2978e1b`)
  `Đợt 1: chặn output trùng nguồn (A01/A01b/G02)`
- Baseline: `e25c79c`

## 16. Kết luận đợt

A01, A01b, G02 đã **FIXED** với regression test chứng minh nguồn không bị thay đổi (hash/độ dài) và
repro gốc không còn tái hiện hành vi phá hủy. Không còn đường nào trong pipeline mở file đích ở chế
độ phá hủy khi đích trùng nguồn.

**DỪNG TẠI ĐÂY — chờ người dùng xác nhận "tiếp tục" trước khi sang Đợt 2.**
