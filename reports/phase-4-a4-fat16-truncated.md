# GIAI ĐOẠN 4 — A4: FAT16 PANIC KHI ẢNH BỊ CẮT CỤT

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Phạm vi source: `volte-core/src/modem.rs`
- Toolchain: `rustc 1.101.0-nightly (12433c307 2026-10-24)` / `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- Môi trường: Windows 10, Git Bash, test harness Windows (`exit 127` tương ứng `0xc00000fd`, `STATUS_STACK_OVERFLOW`)
- Log tạm dùng để đối chiếu nằm ngoài repo: `C:\Users\HT\AppData\Local\Temp\a4probe\logs\`

## 1. STATUS

**PARTIAL — A4 DONE; `cargo test --workspace` BLOCKED bởi lỗi ngoài phạm vi.**

- A4 (`Fat16Image::new`, `slot_abs`, `free_slot`) đạt đúng spec và có bằng chứng đỏ/xanh thật.
- 4/4 test A4 PASS sau khi bảo vệ được bật; 1/4 PASS khi ba lớp bảo vệ bị gỡ (3 test còn lại fail đúng như kỳ vọng).
- `cargo build --workspace`: PASS (exit 0, còn 1 warning dead-code có sẵn).
- `cargo clippy --workspace --all-targets`: PASS (exit 0, còn 3 warning style có sẵn).
- `cargo test --workspace`: **FAIL** tại `volte-core` với `STATUS_STACK_OVERFLOW` trong `modem::tests`.
  Lỗi này đã được điều tra và chứng minh là **độc lập với A4**; theo yêu cầu, không tự sửa.

Không tuyên bố workspace PASS. Không tuyên bố VoLTE chạy thật trên máy chỉ dựa vào `cargo build`.

## 2. KẾT QUẢ KIỂM TRA DEBUG

Lệnh:

```bash
grep -n "derive(Debug)" volte-core/src/modem.rs
```

Kết quả thật:

```text
282:#[derive(Debug)]
918:#[derive(Debug)]
```

- `Fat16Image`: đã có `#[derive(Debug)]` tại dòng 282.
- `ModemReport`: đã có `#[derive(Debug)]` tại dòng 918.
- Đã giữ nguyên, không thêm trùng, không sửa gì khác liên quan Debug.

Ghi nhận thêm: `Debug` của `Fat16Image` in toàn bộ `Vec<u8> data`, nên thông báo panic rất dài (quan sát được ở mục 6). Đây là hệ quả của derive hiện có, không phải thay đổi của giai đoạn này.

## 3. ĐIỀU TRA HAI TEST STACK OVERFLOW VÀ KẾT LUẬN

### 3.1. Chạy riêng từng test

Test 1:

```bash
cargo test -p volte-core modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn -- --nocapture
```

```text
running 1 test

thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (6844) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

`EXIT=127`.

Test 2:

```bash
cargo test -p volte-core modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image -- --nocapture
```

```text
running 1 test

thread 'modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image' (11904) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

`EXIT=127`. Cả hai test đều tái hiện Stack Overflow khi chạy riêng.

### 3.2. Phạm vi ảnh hưởng rộng hơn hai test được nêu

Các test sau cũng overflow với cùng mã lỗi:

```text
modem::tests::find_viettel_mbn_finds_a_large_enough_sea_entry
modem::tests::find_viettel_mbn_rejects_an_entry_at_or_below_the_minimum_size
modem::tests::find_viettel_mbn_also_checks_the_row_carrier
modem::tests::build_modem_volte_moves_viettel_first_and_keeps_the_image_size
modem::tests::scan_modem_profiles_reports_viettel_presence
```

Ví dụ lần chạy `modem::tests::find_viettel_mbn` (3 test):

```text
running 3 tests

thread 'modem::tests::find_viettel_mbn_rejects_an_entry_at_or_below_the_minimum_size' (6900) has overflowed its stack

thread 'modem::tests::find_viettel_mbn_finds_a_large_enough_sea_entry' (9988) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests::find_viettel_mbn' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

Điểm chung: mọi test overflow đều gọi `find_viettel_mbn()` trên ảnh có thư mục được tạo bằng `Fat16Image::mkdir`.

### 3.3. Call graph và code liên quan

Đường gọi thực tế:

1. `modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn`
   (`modem.rs:1550`) → `inject_mbn` tạo cây thư mục bằng `mkdir`
   → cuối test gọi `out_fat.find_viettel_mbn()` (`modem.rs:1583`).
2. `modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image`
   (`modem.rs:1597`) → `build_modem_fix_viettel` (`modem.rs:1148`)
   → `dfat.find_viettel_mbn()` (`modem.rs:1156`).
3. Các test `find_viettel_mbn_*`, `build_modem_volte_*`, `scan_modem_profiles_*`
   cũng đi qua `find_viettel_mbn()` (`modem.rs:788`).

`find_viettel_mbn` gọi đệ quy duy nhất của module:

```rust
// modem.rs:748
fn walk(&self, start_cluster: u32, prefix: &[String]) -> Vec<(Entry, Vec<String>)> {
    let mut out = Vec::new();
    for it in self.iter_dir(start_cluster) {
        let mut parts = prefix.to_vec();
        parts.push(it.name.clone());
        if it.is_dir() {
            out.extend(self.walk(it.cluster, &parts));   // modem.rs:754
        } else {
            out.push((it, parts));
        }
    }
    out
}
```

`walk` không có tập "đã thăm" và không có giới hạn độ sâu. Nó đệ quy vào **mọi** entry có `attr & ATTR_DIR != 0`, kể cả entry trỏ về chính cluster đang duyệt.

Nguồn của entry tự trỏ đó nằm ở `mkdir`:

```rust
// modem.rs:679
self.put_ent(off, &encode_83("."), ATTR_DIR, cl, 0)?;
// modem.rs:680
self.put_ent(off + 32, &encode_83(".."), ATTR_DIR, parent_cluster, 0)?;
```

`encode_83` xử lý dấu chấm theo kiểu `rfind('.')`:

```rust
// modem.rs:196
fn encode_83(name: &str) -> [u8; 11] {
    let upper = name.to_uppercase();
    let (base, ext) = match upper.rfind('.') {
        Some(pos) => (&upper[..pos], &upper[pos + 1..]),
        None => (upper.as_str(), ""),
    };
    ...
}
```

Với `"."`: `rfind('.') == Some(0)` → `base == ""`, `ext == ""` → tên 8.3 toàn dấu cách.
Với `".."`: `rfind('.') == Some(1)` → `base == "."` → tên 8.3 bắt đầu bằng `'.'`, nên vẫn được nhận là `"."`.

`decode_83` trim dấu cách nên `encode_83(".")` giải mã thành chuỗi rỗng. `iter_dir` chỉ bỏ qua tên đúng bằng `"."` hoặc `".."`:

```rust
// modem.rs:567
if name == "." || name == ".." {
    continue;
}
```

Vì tên giải mã là `""`, entry "." không bị bỏ qua; nó được trả về như một thư mục tên rỗng, cluster = chính nó. `walk` đệ quy vào entry đó và lặp vô hạn.

### 3.4. Bằng chứng thực nghiệm (probe tạm, đã gỡ)

Để chứng minh thay vì suy đoán, một test probe tạm được thêm vào module test, chạy, rồi gỡ. Nội dung probe chỉ đọc dữ liệu, không sửa thuật toán:

```text
running 1 test
PROBE_DIR_CLUSTER=2
PROBE_ENCODE_DOT=[32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
PROBE_DECODE_DOT=""
PROBE_ENCODE_DOTDOT=[46, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
PROBE_DECODE_DOTDOT="."
PROBE_ITER_DIR=[("", 2, true)]
test modem::tests::probe_dot_entry_encoding ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 240 filtered out; finished in 0.00s
```

Diễn giải:

- `PROBE_ENCODE_DOT` toàn `32` (dấu cách) → `ENCODE_83(".")` mất tên.
- `PROBE_DECODE_DOT=""` → tên rỗng.
- `PROBE_DECODE_DOTDOT="."` → `".."` vẫn được lọc đúng.
- `PROBE_ITER_DIR=[("", 2, true)]` → duyệt thư mục cluster 2 trả về **một** entry tên rỗng, là thư mục, cluster 2 (chính nó).

Sau khi chạy, probe đã được gỡ. File được phục hồi từ bản backup và kiểm tra hash:

```text
6bd79491c8878e3345821fff60dc0a5d414c1d8b6847f3a58345d8c9d06883eb *volte-core/src/modem.rs
6bd79491c8878e3345821fff60dc0a5d414c1d8b6847f3a58345d8c9d06883eb *C:/Users/HT/AppData/Local/Temp/a4probe/modem.rs.bak
```

`grep -c "probe_dot_entry_encoding"` sau khi gỡ: `0`.

### 3.5. Kết luận điều tra

**Đã chứng minh:**

- Nguyên nhân Stack Overflow là đệ quy vô hạn của `Fat16Image::walk` (`modem.rs:748`) khi gặp entry thư mục tên rỗng, cluster tự trỏ, do `encode_83(".")` tạo ra và `iter_dir` không lọc (`modem.rs:567`).
- Không liên quan đến MBN size hay dữ liệu MBN: overflow xảy ra ở bước duyệt cây thư mục, không phải ở `inject_mbn` hay `read_file`.
- Không liên quan đến A4: các test overflow dùng ảnh **không bị cắt cụt**; khi A4 được bật đầy đủ, overflow vẫn tái hiện. Ở chiều ngược lại, khi gỡ ba lớp bảo vệ A4, các test A4 fail bằng panic index/`Err` sai kiểu, không phải overflow.

**Giả thuyết chưa chứng minh (không dùng làm căn cứ sửa):**

- Ảnh thật do tool Python tham chiếu tạo ra có ghi entry "." đúng tên hay không. Nếu có, lỗi này chỉ ảnh hưởng ảnh do `mkdir` của Rust tạo; nếu không, ảnh thật cũng có thể kích hoạt. Chưa có mẫu ảnh thật để kiểm chứng.
- Số frame đệ quy chính xác trước khi tràn stack; không đo bằng debugger.

**Kết luận:** **LỖI ĐỘC LẬP với A4.** Theo yêu cầu, không tự sửa; ghi ở mục 9.

## 4. DIFF THỰC TẾ TRƯỚC/SAU

### 4.1. Trạng thái khi bắt đầu Giai đoạn 4

`volte-core/src/modem.rs` là file **untracked** (`?? volte-core/src/modem.rs`), nên không thể dùng `git diff` để lấy "trước". Khi bắt đầu giai đoạn này, file đã chứa sẵn ba lớp bảo vệ A4 (khả năng cao từ công việc chuẩn bị trước đó).

Để có bằng chứng đỏ/xanh thật, baseline "trước" được **tái lập có kiểm soát**: gỡ đúng ba lớp bảo vệ A4 khỏi bản sao, chạy test, rồi phục hồi file gốc byte-đồng-nhất. Hash của bản phục hồi trùng khớp bản trước khi gỡ (xem mục 3.4 và 7).

Diff thật giữa baseline "trước" và code "sau" (lệnh `diff -u before.rs modem.rs.final`, exit 1 vì có khác biệt):

```diff
--- before.rs	2026-11-08 12:04:27.885527400 +0700
+++ modem.rs.final	2026-11-08 12:03:08.561526400 +0700
@@ -344,6 +344,16 @@
         if data_clusters < 3 {
             return Err(Error::bad_magic(BAD_FAT_MSG));
         }
+        // A truncated image can still pass `data_off > n` (the boot sector,
+        // FATs and root directory survive) while its data region does not.
+        // Every cluster access assumes this range exists, so reject it here.
+        let data_end = (data_clusters as usize)
+            .checked_mul(cluster_size as usize)
+            .and_then(|bytes| data_off.checked_add(bytes));
+        match data_end {
+            Some(end) if end <= n => {}
+            _ => return Err(Error::bad_magic(BAD_FAT_MSG)),
+        }
         let max_cluster = data_clusters + 1;
         let mut oem = [0u8; 8];
         oem.copy_from_slice(&bs[3..11]);
@@ -514,6 +524,9 @@
             }
             self.clus_off(ch[ci]) + (index % per) * 32
         };
+        if off.checked_add(32)? > self.data.len() {
+            return None;
+        }
         Some(off)
     }
 
@@ -598,7 +611,7 @@
         let mut start = 0usize;
         for i in 0..nslots {
             let off = self.slot_abs(dir_cluster, i)?;
-            let b = self.data[off];
+            let b = *self.data.get(off)?;
             if b == 0x00 || b == 0xE5 {
                 if run == 0 {
                     start = i;
```

Vị trí trong cây hiện tại:

- `Fat16Image::new`: kiểm tra vùng data tại `modem.rs:347-356`, dùng `checked_mul` + `checked_add`; chấp nhận biên `end == n`, từ chối `end > n` hoặc tràn số bằng `Error::bad_magic(BAD_FAT_MSG)`.
- `slot_abs`: trả `Option<usize>`; `None` khi `off + 32 > data.len()` với `checked_add`, tại `modem.rs:527-529`.
- `free_slot`: dùng `self.data.get(off)?` tại `modem.rs:614`; `None` được lan truyền an toàn, không panic.

Ghi chú về "bảo vệ overflow" ở `slot_abs`: phép tính `off` phía trước được giới hạn bởi dữ liệu thực tế (`rootent` kiểu `u16`, `chain` chỉ chứa cluster `u16`, `cluster_size` tối đa 512 KiB), nên không thể tràn `usize` trên nền 64-bit; phép cộng cuối `off + 32` mới là chỗ có rủi ro và đã dùng `checked_add`.

### 4.2. Thay đổi source duy nhất của giai đoạn này

Diff giữa cây lúc bắt đầu Giai đoạn 4 và cây cuối (chỉ thêm một test biên):

```diff
--- modem.rs.bak	2026-11-08 11:58:50.292168800 +0700
+++ modem.rs.final	2026-11-08 12:03:08.561526400 +0700
@@ -1288,6 +1288,19 @@
     // -- A4: a truncated image must fail cleanly, never panic ----------
 
     #[test]
+    fn a4_a_valid_image_is_still_accepted() {
+        let data = blank_image(64);
+        let len = data.len();
+        let fat = Fat16Image::new(data).expect("a well-formed image must still parse");
+        assert_eq!(fat.cluster_size(), 512);
+        assert_eq!(
+            fat.data.len(),
+            len,
+            "the data-region bound must accept an image whose data ends exactly at EOF"
+        );
+    }
+
+    #[test]
     fn a4_a_truncated_data_region_is_rejected_by_new() {
         let fat = fresh_image();
         let full = fat.data.len();
```

Không có thay đổi nào khác ngoài `modem.rs` và report này.

## 5. CÁC REGRESSION TEST ĐÃ THÊM

### 5.1. Test thêm mới

`a4_a_valid_image_is_still_accepted` — `modem.rs:1290-1301`.

Mục đích: khóa biên trên của kiểm tra mới. Ảnh `blank_image(64)` có vùng data kết thúc **đúng** tại EOF (`data_off + data_clusters * cluster_size == n`), nên test này bảo đảm lớp bảo vệ không từ chối nhầm ảnh hợp lệ.

### 5.2. Test đã có sẵn, được xác minh lại

- `a4_a_truncated_data_region_is_rejected_by_new` — `modem.rs:1303`: cắt ảnh hợp lệ còn 50%, `Fat16Image::new` phải trả `BadMagic`.
- `a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking` — `modem.rs:1317`: parse ảnh hợp lệ, tạo 50 thư mục lồng nhau, cắt 50%, sau đó `free_slot` phải trả `None`; test khẳng định trước rằng cluster hiện tại đã nằm ngoài buffer, nên nó thực sự đi qua lớp bảo vệ của `slot_abs`/`.get(off)`.
- `a4_fix_modem_from_mbn_on_a_truncated_image_returns_err` — `modem.rs:1337`: đường xử lý fix-modem trên ảnh cắt cụt phải trả `BadMagic`, không panic.

Không dùng `#[should_panic]` làm tiêu chí PASS. Không xóa, không disable test nào.

## 6. KẾT QUẢ TEST TRƯỚC KHI SỬA A4

Baseline "trước" được tái lập bằng cách gỡ đúng ba lớp bảo vệ (mục 4.1), sau đó chạy:

```bash
cargo test -p volte-core modem::tests::a4_ -- --nocapture
```

Output thật (`EXIT=101`):

```text
running 4 tests
test modem::tests::a4_a_valid_image_is_still_accepted ... ok

thread 'modem::tests::a4_a_truncated_data_region_is_rejected_by_new' (11804) panicked at volte-core\src\modem.rs:1296:41:
called `Result::unwrap_err()` on an `Ok` value: Fat16Image { data: [0, 0, 0, 84, 69, 83, 84, 79, 69, 77, 49, 0, 2, 1, 1, 0, 1, 16, 0, 67, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, ... }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test modem::tests::a4_a_truncated_data_region_is_rejected_by_new ... FAILED

thread 'modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking' (11288) panicked at volte-core\src\modem.rs:601:30:
index out of bounds: the len is 17152 but the index is 26624
test modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking ... FAILED

thread 'modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err' (11304) panicked at volte-core\src\modem.rs:1334:9:
assertion `left == right` failed: fix-modem must refuse a truncated image before touching it: oem_sw.txt not found in the target modem.
  left: Build
 right: BadMagic
test modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err ... FAILED

failures:

failures:
    modem::tests::a4_a_truncated_data_region_is_rejected_by_new
    modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err
    modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 237 filtered out; finished in 0.01s
```

Ghi chú: dòng panic đầu tiên in toàn bộ `Fat16Image { data: [...] }`; phần `data` ở trên được lược bớt vì dài khoảng 50 KB. Nội dung được giữ nguyên văn ở đoạn đầu/cuối; log đầy đủ ở `C:\Users\HT\AppData\Local\Temp\a4probe\logs\before4.txt`.

Ý nghĩa từng failure:

- `a4_a_truncated_data_region_is_rejected_by_new`: không có kiểm tra vùng data, `new` trả `Ok` → `unwrap_err()` panic.
- `a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking`: `self.data[off]` panic `index out of bounds` (len 17152, index 26624).
- `a4_fix_modem_from_mbn_on_a_truncated_image_returns_err`: đường fix-modem trả `Build` ("oem_sw.txt not found") thay vì `BadMagic`, tức ảnh cắt cụt không bị từ chối ngay ở `Fat16Image::new`.

## 7. KẾT QUẢ TEST SAU KHI SỬA A4

Phục hồi nguyên trạng file (hash trùng bản trước khi gỡ), chạy lại đúng lệnh:

```bash
cargo test -p volte-core modem::tests::a4_ -- --nocapture
```

Output thật (`EXIT=0`):

```text
running 4 tests
test modem::tests::a4_a_truncated_data_region_is_rejected_by_new ... ok
test modem::tests::a4_a_valid_image_is_still_accepted ... ok
test modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking ... ok
test modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 237 filtered out; finished in 0.02s
```

## 8. CARGO TEST / BUILD / CLIPPY THỰC TẾ

Tất cả lệnh dưới đây chạy trên cây cuối (đã có test mới), Windows/Git Bash.

### 8.1. `cargo test -p volte-core fat16 -- --nocapture`

```text
running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 241 filtered out; finished in 0.00s
```

`EXIT=0`. Lưu ý trung thực: trên cây hiện tại **không có tên test nào chứa chuỗi `fat16`**, nên filter này không chạy test nào. Nó không phải bằng chứng A4 PASS; bằng chứng A4 là lệnh `modem::tests::a4_` ở mục 7.

### 8.2. `cargo test -p volte-core modem::tests -- --nocapture`

```text
running 32 tests
...
test modem::tests::decode_83_round_trips_a_simple_name ... ok
test modem::tests::default_dst_appends_the_suffix_before_the_extension ... ok
test modem::tests::empty_oem_blob_still_gets_the_viettel_line ... ok
test modem::tests::fewer_than_three_data_clusters_is_rejected ... ok

thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (10040) has overflowed its stack

thread 'modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image' (12124) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

`EXIT=127`. Trước khi tiến trình bị abort, 17 dòng `... ok` đã in ra (đếm bằng `grep -c`), trong đó có các test A4; không có dòng `FAILED` nào trước khi abort. Nguyên nhân abort là lỗi độc lập ở mục 3, không phải A4.

### 8.3. `cargo test --workspace`

```text
     Running unittests src\main.rs (target\debug\build\volte-cli/d68a8014c9353575\out\volte_cli-d68a8014c9353575.exe)
running 30 tests
...
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)
running 241 tests
...
test modem::tests::a4_a_valid_image_is_still_accepted ... ok
test modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err ... ok
test modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking ... ok
test modem::tests::a_blank_image_parses_and_reports_its_oem ... ok
test modem::tests::a_non_power_of_two_sectors_per_cluster_is_rejected ... ok
test modem::tests::a_too_short_buffer_is_rejected ... ok
test modem::tests::a_zero_bytes_per_sector_is_rejected ... ok

thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (11584) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
note: test exited abnormally; to see the full output pass --no-capture to the harness.
```

`EXIT=127`. **Workspace KHÔNG PASS** vì lỗi ngoài phạm vi. 30/30 test `volte-cli` PASS; `volte-core` lib bị abort giữa chừng (114 dòng `... ok` đã in, bao gồm 4 test A4; 0 dòng `FAILED` tường minh). Không dùng `--skip` để che lỗi này.

### 8.4. `cargo build --workspace`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib) generated 1 warning
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.82s
```

`EXIT=0`. Warning `spc` là dead-code có sẵn từ trước, không phải lỗi build; không sửa theo nguyên tắc "surgical changes".

### 8.5. `cargo clippy --workspace --all-targets`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
...
warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:384:31
    |
384 |             .trim_end_matches(|c: char| c == '\u{0}' || c == ' ')
...
warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:707:13
    |
707 |             (blob.len() + self.cluster_size as usize - 1) / self.cluster_size as usize
...
warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 suggestions)
warning: `volte-core` (lib test) generated 3 warnings (3 duplicates)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.91s
```

`EXIT=0`. Ba warning đều là style/dead-code có sẵn, nằm ngoài phạm vi A4; giữ nguyên.

### 8.6. `git diff --check`

```text
EXIT=0
```

Không có whitespace error ở các file tracked. Lưu ý: `volte-core/src/modem.rs` đang **untracked**, nên `git diff --check` không kiểm tra file này. Kiểm tra bổ sung trực tiếp:

```text
$ grep -nE " +$" volte-core/src/modem.rs | head -5
$ echo "trailing_ws_hits=$(grep -cE ' +$' volte-core/src/modem.rs)"
trailing_ws_hits=0
```

## 9. PHÁT HIỆN NGOÀI PHẠM VI

### 9.1. Stack Overflow trong `walk` do entry "." tên rỗng (đã chứng minh)

- Vị trí: `encode_83` (`volte-core/src/modem.rs:196`), `mkdir` (`modem.rs:668`), `iter_dir` (`modem.rs:533`, điều kiện lọc ở `modem.rs:567`), `walk` (`modem.rs:748`, đệ quy ở `modem.rs:754`).
- Hệ quả: `find_viettel_mbn` (`modem.rs:788`) gọi `walk` và treo vô hạn trên ảnh có thư mục do `mkdir` tạo.
- Test bị ảnh hưởng: 2 test được nêu + 3 test `find_viettel_mbn_*` + `build_modem_volte_moves_viettel_first_and_keeps_the_image_size` + `scan_modem_profiles_reports_viettel_presence` (danh sách đầy đủ ở mục 3.2). Đây là lý do `cargo test --workspace` không thể xanh.
- **Không sửa** theo yêu cầu "nếu lỗi độc lập, không tự sửa; ghi vào mục phát hiện ngoài phạm vi".

Hướng sửa tiềm năng (chỉ ghi nhận, chưa thực hiện, cần một giai đoạn/phạm vi riêng):

1. Viết entry "." bằng byte 8.3 literal `b".          "` thay vì `encode_83(".")`, và/hoặc
2. Cho `iter_dir` bỏ qua entry tên rỗng có `ATTR_DIR` và cluster tự trỏ, và/hoặc
3. Thêm tập "đã thăm" hoặc giới hạn độ sâu cho `walk`.

Cả ba đều thay đổi hành vi FAT16/MBN, nằm ngoài phạm vi A4, nên không tự ý làm.

### 9.2. Warning clippy/dead-code có sẵn

`spc` không đọc (`modem.rs:286`), `manual_pattern_char_comparison` (`modem.rs:384`), `manual_div_ceil` (`modem.rs:707`). Không sửa vì ngoài phạm vi và không ảnh hưởng build/test.

### 9.3. Trạng thái cây làm việc

Ngoài `modem.rs` và report này, cây còn nhiều thay đổi untracked/deleted từ các giai đoạn trước (ví dụ `VoLTE-Vendor-Builder.zip`, các report phase 1–3, các file payload mới). Không dọn dẹp, không commit, không đụng tới.

### 9.4. Những gì KHÔNG thay đổi

- Không sửa thuật toán MBN, QC, MTK.
- Không refactor.
- Không cập nhật dependency.
- Không xóa/disable test.
- Không sửa lỗi ngoài phạm vi.

## 10. CÁC VẤN ĐỀ CÒN TỒN TẠI CẦN NGƯỜI DÙNG QUYẾT ĐỊNH

1. **Chấp nhận cách chứng minh A4:** ba lớp bảo vệ A4 đã có sẵn trong cây khi Giai đoạn 4 bắt đầu. Tôi không viết lại chúng; tôi xác minh đúng spec, tái lập baseline "trước" bằng cách gỡ đúng ba lớp đó để chứng minh đỏ, rồi phục hồi byte-đồng-nhất để chứng minh xanh. Anh có chấp nhận A4 là "đã có sẵn + được xác minh" thay vì "mới viết trong giai đoạn này" không?
2. **Test biên thêm mới:** `a4_a_valid_image_is_still_accepted` là thay đổi source duy nhất của tôi. Anh có muốn giữ không?
3. **Lỗi Stack Overflow độc lập:** cần một giai đoạn riêng để sửa (gợi ý ở mục 9.1). Chưa thực hiện. Anh chọn hướng nào (sửa `encode_83`/`mkdir`, lọc trong `iter_dir`, hay thêm cycle guard trong `walk`)?
4. **Filter `cargo test -p volte-core fat16`:** hiện khớp 0 test. Có cần đổi quy ước đặt tên test (ví dụ thêm tiền tố `fat16_`) trong giai đoạn sau, hay giữ nguyên và dùng filter `a4_`/`modem::tests`?
5. **Trạng thái PARTIAL:** A4 xong nhưng `cargo test --workspace` vẫn đỏ vì lỗi ngoài phạm vi. Anh xác nhận coi đây là PARTIAL (không phải DONE toàn workspace) chứ?
6. **Chưa kiểm chứng trên thiết bị:** báo cáo này chỉ dựa trên test Rust và build; không có bằng chứng VoLTE chạy thật trên máy.

Đã xong Giai đoạn 4, report tại `reports/phase-4-a4-fat16-truncated.md`. Chờ xác nhận để làm Giai đoạn 5.
