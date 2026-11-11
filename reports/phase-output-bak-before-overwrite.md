# BÁO CÁO — ĐỔI OUTPUT CŨ THÀNH `.bak` TRƯỚC KHI GHI ĐÈ

- Ngày: 2026-11-09
- Nhánh: `ai2/fix-audit-findings`, HEAD `81af79b`
- Vai trò: AI2 — Local Executor
- Loại nhiệm vụ: sửa một hành vi duy nhất trong `volte-core/src/pipeline.rs`
- Phạm vi: khối copy output trong `run()` + test của đúng khối này. Không đụng `avb.rs`, `report.rs`, `slot.rs`, guard Tag 4/Tag 2/FEC, `env.rs`, README.
- Trạng thái: **CODE DONE — test/toolchain pass, chưa commit**

---

## 1. STATUS

| Hạng mục | Kết quả |
|---|---|
| Lần build đầu, `out_path` chưa tồn tại | ✅ build bình thường, không tạo `.bak` |
| `out_path` tồn tại, chưa có `.bak` | ✅ `.bak` giữ đúng nội dung output cũ (byte-for-byte) |
| `out_path` và `.bak` đều tồn tại | ✅ `.bak` cũ bị thay bằng output của lần build trước; không có `.bak.1` |
| `--out` tùy chỉnh trỏ tới file đã tồn tại | ✅ hành vi giống output mặc định |
| Build fail sau khi đã rename | ✅ `.bak` còn nguyên, dùng được |
| Rename lỗi | ✅ chỉ `[WARN] Destination file is open/locked: ...`, build tiếp tục |
| `cargo fmt --all -- --check` | ✅ sạch |
| `cargo check --offline --all-targets` | ✅ sạch |
| `cargo clippy --offline --all-targets -- -D warnings` | ✅ sạch |
| `cargo test --workspace --offline` | ✅ 30 CLI + 285 core pass, 2 ignored, 0 fail |
| Checksum `pipeline.rs` trước/sau | ✅ ghi ở §4 |
| `git diff --stat` | ✅ chỉ `pipeline.rs` (+213/−1) |
| Commit/push | ❌ không thực hiện (chờ duyệt) |

---

## 2. THAY ĐỔI

Chỉ `volte-core/src/pipeline.rs`, trong `run()` — thay `remove_file` bằng rename sang
`{out_path}.bak`:

```rust
// Trước
if out_path.is_file() && !path_id::same_file(&out_path, &src) {
    if let Err(e) = std::fs::remove_file(&out_path) {
        log.log(format!(
            "[WARN] Destination file is open/locked: {}",
            trunc(&e.to_string(), 160)
        ));
    }
}

// Sau
if out_path.is_file() && !path_id::same_file(&out_path, &src) {
    // Keep the previous output as a single-generation backup so a failed
    // build cannot destroy the image the user is currently flashing.
    let bak = {
        let mut s = out_path.as_os_str().to_owned();
        s.push(".bak");
        PathBuf::from(s)
    };
    if bak.is_file() {
        let _ = std::fs::remove_file(&bak);
    }
    if let Err(e) = std::fs::rename(&out_path, &bak) {
        log.log(format!(
            "[WARN] Destination file is open/locked: {}",
            trunc(&e.to_string(), 160)
        ));
    }
}
```

Đúng spec:

1. Chỉ chạy khi `out_path` tồn tại và khác file nguồn — giữ nguyên điều kiện cũ.
2. `.bak` cũ bị xóa trước để chỉ còn một thế hệ backup.
3. Áp dụng cho cả output mặc định và `--out` (cùng một nhánh `run()`).
4. Rename lỗi chỉ log `[WARN]`, không trả `Err`.
5. Không đổi `resolve_output()`, AVB, report hay bất kỳ logic nào khác.

Test-only: thêm helper `run_full_build()` và `bak_path()`; không thay đổi test cũ.

---

## 3. TEST

6 test mới trong `pipeline::tests`:

| Test | Kịch bản |
|---|---|
| `a_first_build_creates_no_backup` | out chưa tồn tại → không có `.bak` |
| `an_existing_output_is_kept_as_a_single_generation_backup` | out cũ 15 byte → `.bak` đúng 15 byte, out là ảnh mới |
| `an_old_backup_is_replaced_by_the_previous_output` | out + `.bak` cũ → `.bak` mới = out trước build, không có `.bak.*` |
| `a_custom_out_path_gets_the_same_backup_treatment` | `--out custom_output.img` tồn tại → `.bak` đúng nội dung cũ |
| `a_failed_build_keeps_the_previous_output_backup` | e2fsck fail sau copy → `Err`, `.bak` còn nguyên, không có report |
| `a_rename_failure_only_warns_and_the_build_continues` | rename fail → `[WARN]`, build `Ok`, out là ảnh mới |

Kết quả chạy thật:

```text
cargo test --offline -p volte-core pipeline::tests
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 255 filtered out; finished in 159.35s

cargo test --offline -p volte-core pipeline::tests::a_failed_build_keeps_the_previous_output_backup -- --exact
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 286 filtered out; finished in 18.61s

cargo test --workspace --offline
volte-cli: test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
volte-core: test result: ok. 285 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 233.78s
doc-tests:  test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Lưu ý về test rename fail: trên Windows, khóa `out_path` bằng `share_mode(0)` sẽ làm
`rename` fail nhưng đồng thời `File::create` của bước copy cũng fail, nên không thể chứng
minh "build tiếp tục". Test dùng một **thư mục** tại đường dẫn `.bak`: `remove_file` bỏ qua
(vì `is_file()` false), `rename` fail, còn `out_path` vẫn ghi được — đúng hành vi cần kiểm
chứng (rename lỗi chỉ WARN). Đây là mô phỏng có chủ đích, đã ghi rõ trong comment test.

Clippy ban đầu báo `too_many_arguments` cho helper test 8 tham số; đã bỏ tham số `keep_out`
(suy ra `!out.is_empty()`) còn 7, sau đó clippy sạch.

---

## 4. CHECKSUM VÀ DIFF

| File | Trước | Sau |
|---|---|---|
| `volte-core/src/pipeline.rs` | `17adbce4ed3eeb3e664bb7bfd5b326ecbfa14f4da681e4fe81d24098db617bd3` | `704df4d9ca35516130acc5a066a459c77e054f49d1334babf6b3a3fb708e25d0` |

```text
git diff --stat
 volte-core/src/pipeline.rs | 214 ++++++++++++++++++++++++++++++++++++++++++++-
 1 file changed, 213 insertions(+), 1 deletion(-)
```

`git status --short` phần tracked: chỉ `M volte-core/src/pipeline.rs`.

---

## 5. GHI CHÚ

1. Checksum trước khớp đúng giá trị `17adbce4…` mà các phase Native AVB đã dùng làm bất biến.
2. `.bak` nằm cạnh output (`vendor_VolLTE.img.bak`), không phải trong `Logs/`.
3. Không xử lý riêng trường hợp `.bak` là thư mục: `remove_file` lỗi bị bỏ qua, `rename` lỗi
   chỉ WARN rồi copy tiếp — đúng hợp đồng "không fail vì bước backup".
4. Không flash/root/reboot thiết bị; không chạy build trên ảnh thật trong phase này vì thay
   đổi chỉ tác động nhánh ghi đè output và đã được phủ bằng test end-to-end với ext4 thật.
5. Release build: `target/release/volte-cli.exe`, 1.254.400 B,
   SHA-256 `dec4a3cc517245b3e77437384a648c71277856ac9414d367c8ac743ad4337ee2`.
6. Chưa commit/push.
