# GIAI ĐOẠN 4B — SỬA ĐỆ QUY VÔ HẠN TRONG `Fat16Image::walk`

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Phạm vi source: `volte-core/src/modem.rs` — chỉ `mkdir` (đường ghi entry `"."`), `encode_83` (không sửa, chỉ dùng gián tiếp), `walk`.
- Toolchain: `rustc 1.101.0-nightly (12433c307 2026-10-24)` / `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- Môi trường: Windows 10, Git Bash; `exit 127` tương ứng `0xc00000fd` (`STATUS_STACK_OVERFLOW`).
- Log tạm đối chiếu (ngoài repo): `C:\Users\HT\AppData\Local\Temp\a4probe\logs\`

## 1. STATUS

**DONE.**

- Sửa gốc tại `mkdir`: entry `"."` được ghi bằng 8.3 literal `".          "` thay vì `encode_83(".")`.
- Thêm lớp phòng vệ chu trình trong `walk` bằng tập cluster đã thăm (`HashSet<u32>`), trả về kết quả đã thu thập thay vì panic.
- 2 test cũ từng treo + 5 test còn lại từng overflow đều PASS.
- `cargo test -p volte-core modem::tests -- --nocapture`: 34/34 PASS.
- `cargo test --workspace`: 30 + 243 PASS, exit 0.
- `cargo build --workspace`: exit 0 (1 warning dead-code có sẵn).
- `cargo clippy --workspace --all-targets`: exit 0 (3 warning style có sẵn, không đổi).
- `git diff --check`: exit 0.
- Không đổi hành vi `find_viettel_mbn`/`build_modem_fix_*` trên ảnh hợp lệ; toàn bộ test MBN/Viettel hiện có vẫn PASS.

Không tuyên bố VoLTE chạy thật trên thiết bị; phạm vi bằng chứng chỉ là test Rust + build.

## 2. KẾT QUẢ KIỂM TRA DEBUG

Không thay đổi gì liên quan Debug trong giai đoạn này. Kiểm tra lại để chắc chắn không bị mất do dịch dòng:

```bash
grep -n "derive(Debug)" volte-core/src/modem.rs
```

```text
288:#[derive(Debug)]
937:#[derive(Debug)]
```

- `Fat16Image`: dòng 288.
- `ModemReport`: dòng 937.
- Cả hai vẫn còn nguyên; không thêm trùng.

## 3. ĐIỀU TRA HAI TEST STACK OVERFLOW VÀ KẾT LUẬN NGUYÊN NHÂN

### 3.1. Trích nguyên văn kết luận đã có ở report Giai đoạn 4

Theo yêu cầu, không điều tra lại từ đầu. Trích mục 3.5 của `reports/phase-4-a4-fat16-truncated.md`:

> **Đã chứng minh:**
>
> - Nguyên nhân Stack Overflow là đệ quy vô hạn của `Fat16Image::walk` (`modem.rs:748`) khi gặp entry thư mục tên rỗng, cluster tự trỏ, do `encode_83(".")` tạo ra và `iter_dir` không lọc (`modem.rs:567`).
> - Không liên quan đến MBN size hay dữ liệu MBN: overflow xảy ra ở bước duyệt cây thư mục, không phải ở `inject_mbn` hay `read_file`.
> - Không liên quan đến A4: các test overflow dùng ảnh **không bị cắt cụt**; khi A4 được bật đầy đủ, overflow vẫn tái hiện. Ở chiều ngược lại, khi gỡ ba lớp bảo vệ A4, các test A4 fail bằng panic index/`Err` sai kiểu, không phải overflow.
>
> **Kết luận:** **LỖI ĐỘC LẬP với A4.** Theo yêu cầu, không tự sửa; ghi ở mục 9.

Bằng chứng thực nghiệm đã có ở mục 3.4 report Giai đoạn 4:

```text
PROBE_ENCODE_DOT=[32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
PROBE_DECODE_DOT=""
PROBE_ENCODE_DOTDOT=[46, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32]
PROBE_DECODE_DOTDOT="."
PROBE_ITER_DIR=[("", 2, true)]
```

Nghĩa là: `"."` → tên rỗng, cluster tự trỏ; `".."` → giải mã đúng `"."` nên đã bị lọc.

### 3.2. Xác nhận lại trên cây trước khi sửa 4b

Chạy lại toàn bộ `modem::tests` trước khi sửa bất cứ dòng nào:

```bash
cargo test -p volte-core modem::tests -- --nocapture
```

```text
running 32 tests
...
test modem::tests::build_modem_fix_from_mbn_rejects_a_too_small_blob ... ok
test modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err ... ok
test modem::tests::build_modem_fix_viettel_refuses_a_donor_without_viettel ... ok
test modem::tests::build_modem_volte_refuses_a_modem_without_viettel ... ok
test modem::tests::build_modem_volte_refuses_out_equal_to_source ... ok
test modem::tests::decode_83_handles_a_name_with_no_extension ... ok
test modem::tests::decode_83_round_trips_a_simple_name ... ok
test modem::tests::default_dst_appends_the_suffix_before_the_extension ... ok
test modem::tests::empty_oem_blob_still_gets_the_viettel_line ... ok
test modem::tests::fewer_than_three_data_clusters_is_rejected ... ok

thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (8116) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

`EXIT=127`; 17 test đã in `... ok` trước khi tiến trình bị abort. Kết quả này trùng với report Giai đoạn 4.

### 3.3. Kết luận

Nguyên nhân đã chứng minh không thay đổi:

1. `encode_83(".")` (`modem.rs:196` bản Giai đoạn 4) → mảng 8.3 toàn dấu cách → `decode_83` trả `""`.
2. `iter_dir` (`modem.rs:567` bản Giai đoạn 4) chỉ lọc tên đúng `"."`/`".."`, nên entry tên rỗng lọt qua.
3. `walk` đệ quy vô điều kiện vào entry thư mục, gặp lại chính cluster đó → đệ quy vô hạn.

Không có nguyên nhân mới. Không liên quan MBN, không liên quan A4.

## 4. DIFF THỰC TẾ TRƯỚC/SAU

`volte-core/src/modem.rs` là file untracked nên không có `git diff`. Baseline "trước 4b" là bản cuối Giai đoạn 4 (SHA-256 `f4cdc9d8049a454f05874541e86a9cfc1a0c57360e3f34e2661c3bcf195c9d0b`). Bản "sau 4b" có SHA-256 `b03e5ca8fc6bc9cfd34a6d62d606178786132b00dc4f029b26599d0bfacff20a`.

Lệnh và output thật (`diff -u modem.rs.final modem.rs.4b`, exit 1 vì có khác biệt):

```diff
--- modem.rs.final	2026-11-08 12:03:08.561526400 +0700
+++ modem.rs.4b	2026-11-08 13:25:19.097006400 +0700
@@ -37,6 +37,12 @@
 const ATTR_DIR: u8 = 0x10;
 const ATTR_LFN: u8 = 0x0F;
 
+/// 8.3 name of the "." directory entry: one dot padded with ten spaces.
+/// Routing it through `encode_83` would yield an all-space name, which
+/// `decode_83` turns into `""`; `iter_dir` only filters literal "."/"..",
+/// so that entry would look like a child directory pointing at itself.
+const DOT_ENTRY_NAME: [u8; 11] = *b".          ";
+
 /// Below this size an `mcfg_sw.mbn` is treated as missing/corrupt, matching
 /// the Python reference's `MBN_MIN_SIZE`.
 pub const MBN_MIN_SIZE: usize = 1000;
@@ -676,7 +682,7 @@
         let csz = self.cluster_size as usize;
         self.data[off..off + csz].fill(0);
         self.fat_set(cl, FAT16_EOF);
-        self.put_ent(off, &encode_83("."), ATTR_DIR, cl, 0)?;
+        self.put_ent(off, &DOT_ENTRY_NAME, ATTR_DIR, cl, 0)?;
         self.put_ent(off + 32, &encode_83(".."), ATTR_DIR, parent_cluster, 0)?;
         let idx = self.alloc_dirent(parent_cluster, 1)?;
         let aoff = self
@@ -746,12 +752,25 @@
     }
 
     fn walk(&self, start_cluster: u32, prefix: &[String]) -> Vec<(Entry, Vec<String>)> {
+        let mut visited = std::collections::HashSet::new();
+        self.walk_into(start_cluster, prefix, &mut visited)
+    }
+
+    fn walk_into(
+        &self,
+        start_cluster: u32,
+        prefix: &[String],
+        visited: &mut std::collections::HashSet<u32>,
+    ) -> Vec<(Entry, Vec<String>)> {
+        if !visited.insert(start_cluster) {
+            return Vec::new();
+        }
         let mut out = Vec::new();
         for it in self.iter_dir(start_cluster) {
             let mut parts = prefix.to_vec();
             parts.push(it.name.clone());
             if it.is_dir() {
-                out.extend(self.walk(it.cluster, &parts));
+                out.extend(self.walk_into(it.cluster, &parts, visited));
             } else {
                 out.push((it, parts));
             }
@@ -1351,6 +1370,41 @@
         );
     }
 
+    // -- 4b: walking must terminate on deep or cyclic directory trees -----
+
+    #[test]
+    fn a4b_deep_directories_are_walked_without_overflowing_the_stack() {
+        let mut fat = fresh_image();
+        let mut cur = 0u32;
+        for _ in 0..50 {
+            cur = fat.mkdir(cur, "d").expect("mkdir").cluster;
+        }
+        let idx = fat.alloc_dirent(cur, 1).unwrap();
+        let off = fat.slot_abs(cur, idx).unwrap();
+        fat.put_ent(off, &encode_83("leaf.bin"), ATTR_ARC, 0, 0)
+            .unwrap();
+
+        let found = fat.walk(0, &[]);
+        assert_eq!(found.len(), 1, "the walk must reach the leaf exactly once");
+        assert_eq!(found[0].1.len(), 51);
+        assert_eq!(found[0].1.last().unwrap(), "LEAF.BIN");
+    }
+
+    #[test]
+    fn a4b_cycle_guard_skips_a_self_referencing_directory() {
+        let mut fat = fresh_image();
+        let d = fat.mkdir(0, "cycle").expect("mkdir");
+        let off = fat.clus_off(d.cluster);
+        fat.put_ent(off, &[b' '; 11], ATTR_DIR, d.cluster, 0)
+            .unwrap();
+
+        let found = fat.walk(d.cluster, &[]);
+        assert!(
+            found.is_empty(),
+            "a self-referencing directory must be skipped, not followed forever"
+        );
+    }
+
     // -- 8.3 / LFN name codecs -----------------------------------------
 
     #[test]
```

Vị trí trong cây cuối:

- Hằng `DOT_ENTRY_NAME`: `modem.rs:40-44`.
- `mkdir` ghi entry `"."`: `modem.rs:685` (entry `".."` giữ nguyên `encode_83("..")` tại `modem.rs:686`, không sửa).
- `walk` (giữ nguyên chữ ký): `modem.rs:754-757`.
- `walk_into` + kiểm tra visited: `modem.rs:759-779`, chốt chặn ở `modem.rs:765-767`.
- Hai test mới: `modem.rs:1375-1406`.

Lựa chọn phòng vệ: **phương án (a) — tập cluster đã thăm**. Lý do: ít xâm lấn hơn, chặn đúng cả chu trình trực tiếp lẫn chu trình gián tiếp (A→B→A) mà không phụ thuộc vào một con số độ sâu ước lượng; FAT16 hợp lệ không có hai thư mục cùng cluster nên không tạo false positive. Không thêm logger: `walk` không có logger trong chữ ký, và việc thêm tham số logger sẽ lan ra `find_viettel_mbn`/call site, vượt phạm vi; khi phát hiện chu trình, hàm trả về kết quả đã thu thập (có thể rỗng), không panic.

## 5. CÁC REGRESSION TEST ĐÃ THÊM

### 5.1. `a4b_deep_directories_are_walked_without_overflowing_the_stack` — `modem.rs:1376`

Tạo 50 thư mục lồng nhau bằng `mkdir`, ghi một file `leaf.bin` ở cấp sâu nhất, gọi `walk(0, &[])`. Kỳ vọng: trả về đúng 1 entry lá, đường dẫn 51 thành phần (50 thư mục + file), tên cuối `LEAF.BIN`. Test này khóa hành vi entry `"."` sau khi sửa: nếu ai đổi `mkdir` quay lại `encode_83(".")`, entry rỗng tự trỏ sẽ khiến `walk` đệ quy vô hạn và test treo/overflow ngay.

### 5.2. `a4b_cycle_guard_skips_a_self_referencing_directory` — `modem.rs:1394`

Dựng thủ công một entry thư mục tên rỗng (`&[b' '; 11]`) với cluster trỏ về chính thư mục chứa nó — mô phỏng ảnh bị hỏng theo cách khác bug `mkdir`. Gọi `walk(d.cluster, &[])`. Kỳ vọng: trả về rỗng, không treo. Test này kiểm tra riêng lớp phòng vệ ở mục 2.2, độc lập với việc sửa `mkdir`.

Không dùng `#[should_panic]`. Không xóa/disable test nào.

## 6. KẾT QUẢ TEST TRƯỚC KHI SỬA 4B

### 6.1. Baseline: toàn bộ `modem::tests` (chưa sửa gì)

Xem output nguyên văn ở mục 3.2: `EXIT=127`, 17 test `... ok`, sau đó:

```text
thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (8116) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`
```

### 6.2. Sau khi chỉ sửa `mkdir` (2.1), chưa có guard (2.2)

Để chứng minh lớp phòng vệ ở `walk` là cần thiết chứ không thừa, chạy hai test mới khi mới chỉ áp dụng 2.1:

```bash
cargo test -p volte-core modem::tests::a4b_ -- --nocapture
```

```text
running 2 tests
test modem::tests::a4b_deep_directories_are_walked_without_overflowing_the_stack ... ok

thread 'modem::tests::a4b_cycle_guard_skips_a_self_referencing_directory' (13244) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe 'modem::tests::a4b_' --nocapture` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

`EXIT=127`. Ý nghĩa: sửa `mkdir` xử lý đúng cây 50 cấp; nhưng một entry tự trỏ kiểu khác vẫn làm `walk` treo, nên guard 2.2 là bắt buộc.

## 7. KẾT QUẢ TEST SAU KHI SỬA 4B

```bash
cargo test -p volte-core modem::tests -- --nocapture
```

```text
running 34 tests
...
test modem::tests::a4b_cycle_guard_skips_a_self_referencing_directory ... ok
test modem::tests::a4b_deep_directories_are_walked_without_overflowing_the_stack ... ok
test modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn ... ok
test modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image ... ok
test modem::tests::find_viettel_mbn_also_checks_the_row_carrier ... ok
test modem::tests::find_viettel_mbn_finds_a_large_enough_sea_entry ... ok
test modem::tests::find_viettel_mbn_rejects_an_entry_at_or_below_the_minimum_size ... ok
test modem::tests::build_modem_volte_moves_viettel_first_and_keeps_the_image_size ... ok
test modem::tests::scan_modem_profiles_reports_viettel_presence ... ok

test result: ok. 34 passed; 0 failed; 0 ignored; 0 measured; 209 filtered out; finished in 0.05s
```

`EXIT=0`. Toàn bộ 7 test từng overflow (2 test được nêu + 5 test ở mục 3.2 report cũ) đều PASS, cùng 2 test mới; không còn dòng `overflowed` nào.

## 8. CARGO TEST / BUILD / CLIPPY THỰC TẾ

### 8.1. `cargo test --workspace`

```text
     Running unittests src\main.rs (target\debug\build\volte-cli/d68a8014c9353575\out\volte_cli-d68a8014c9353575.exe)
running 30 tests
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)
running 243 tests
test result: ok. 243 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 104.86s
running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

`EXIT=0`. Đây là lần đầu workspace xanh trong chuỗi Giai đoạn 3–4b; không dùng `--skip`.

### 8.2. `cargo build --workspace`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
    |
294 | pub struct Fat16Image {
    |            ---------- field in this struct
...
297 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib) generated 1 warning
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.58s
```

`EXIT=0`. Warning `spc` có sẵn từ trước, ngoài phạm vi 4b, không sửa.

### 8.3. `cargo clippy --workspace --all-targets`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
...
warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:390:31
...
warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:713:13
...
warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 suggestions)
warning: `volte-core` (lib test) generated 3 warnings (3 duplicates)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.91s
```

`EXIT=0`. Đúng 3 warning cũ, không phát sinh warning mới từ thay đổi 4b.

### 8.4. `git diff --check`

```text
EXIT=0
```

`modem.rs` vẫn untracked nên `git diff --check` không quét file này; kiểm tra bổ sung:

```text
$ echo "trailing_ws_hits=$(grep -cE ' +$' volte-core/src/modem.rs)"
trailing_ws_hits=0
```

## 9. PHÁT HIỆN NGOÀI PHẠM VI

1. **3 warning clippy/dead-code có sẵn** vẫn giữ nguyên: `spc` không đọc (`modem.rs:292`), `manual_pattern_char_comparison` (`modem.rs:390`), `manual_div_ceil` (`modem.rs:713`). Không sửa.
2. **Giới hạn của guard đã chọn:** tập visited chặn mọi chu trình cluster, nhưng không đặt trần độ sâu cho một cây **không có chu trình** nhưng cực sâu. Với FAT16, độ sâu bị giới hạn tự nhiên bởi số cluster (`max_cluster`), và dữ liệu thực tế rất nông; nếu muốn trần cứng, cần thêm tham số độ sâu — việc này ngoài phạm vi 4b.
3. **Không log cảnh báo khi phát hiện chu trình:** `walk` không có logger; thêm logger sẽ lan sang `find_viettel_mbn` và call site. Đã chọn trả kết quả một cách im lặng theo đúng cho phép của brief.
4. **Trạng thái cây làm việc:** ngoài `modem.rs` và report này, cây còn nhiều thay đổi untracked/deleted từ các giai đoạn trước. Không dọn dẹp, không commit.
5. **Không thay đổi:** thuật toán MBN/QC/MTK, dependency, kiến trúc; không xóa/disable test; không sửa lỗi ngoài phạm vi.

## 10. CÁC VẤN ĐỀ CÒN TỒN TẠI CẦN NGƯỜI DÙNG QUYẾT ĐỊNH

1. **Phương án phòng vệ:** đã chọn visited-set (a) thay vì giới hạn độ sâu (b). Anh xác nhận lựa chọn này, hay muốn bổ sung thêm trần độ sâu cứng trong một thay đổi riêng?
2. **Không có logger cảnh báo:** chấp nhận việc `walk` trả kết quả im lặng khi gặp chu trình, hay muốn một giai đoạn sau truyền logger xuống để ghi cảnh báo?
3. **Hai test mới:** giữ cả `a4b_deep_directories_are_walked_without_overflowing_the_stack` và `a4b_cycle_guard_skips_a_self_referencing_directory` chứ?
4. **Chuyển Giai đoạn 5 (A2 AVB):** workspace hiện đã xanh; có thể chuyển tiếp khi anh duyệt.
5. **Chưa kiểm chứng thiết bị:** không có bằng chứng VoLTE chạy thật; mới dừng ở test Rust + build.

## ĐỐI CHIẾU VỚI REPORT GIAI ĐOẠN 4

Trích mục **3.5** của `reports/phase-4-a4-fat16-truncated.md`:

> **Đã chứng minh:**
>
> - Nguyên nhân Stack Overflow là đệ quy vô hạn của `Fat16Image::walk` (`modem.rs:748`) khi gặp entry thư mục tên rỗng, cluster tự trỏ, do `encode_83(".")` tạo ra và `iter_dir` không lọc (`modem.rs:567`).
> - Không liên quan đến MBN size hay dữ liệu MBN: overflow xảy ra ở bước duyệt cây thư mục, không phải ở `inject_mbn` hay `read_file`.
> - Không liên quan đến A4: các test overflow dùng ảnh **không bị cắt cụt**; khi A4 được bật đầy đủ, overflow vẫn tái hiện.
>
> **Kết luận:** **LỖI ĐỘC LẬP với A4.** Theo yêu cầu, không tự sửa; ghi ở mục 9.

Trích mục **9.1** cùng report:

> - Vị trí: `encode_83` (`volte-core/src/modem.rs:196`), `mkdir` (`modem.rs:668`), `iter_dir` (`modem.rs:533`, điều kiện lọc ở `modem.rs:567`), `walk` (`modem.rs:748`, đệ quy ở `modem.rs:754`).
> - Hệ quả: `find_viettel_mbn` (`modem.rs:788`) gọi `walk` và treo vô hạn trên ảnh có thư mục do `mkdir` tạo.
> - **Không sửa** theo yêu cầu "nếu lỗi độc lập, không tự sửa; ghi vào mục phát hiện ngoài phạm vi".

Đối chiếu:

- Nguyên nhân sửa ở 4b **khớp chính xác** với nguyên nhân đã chứng minh: entry `"."` bị `encode_83` biến thành tên rỗng, lọt qua bộ lọc của `iter_dir`, khiến `walk` đệ quy vô hạn.
- Các dòng trong trích dẫn là số dòng tại thời điểm Giai đoạn 4; sau khi thêm hằng `DOT_ENTRY_NAME` (+6 dòng) và helper `walk_into`, vị trí hiện tại là: `iter_dir` `modem.rs:539` (lọc ở `modem.rs:573`), `mkdir` `modem.rs:674`, `walk` `modem.rs:754`, đệ quy `modem.rs:773`, `find_viettel_mbn` `modem.rs:807`. Bản chất không đổi.
- `encode_83` **không bị sửa**; chỉ `mkdir` đổi cách ghi entry `"."`. `encode_83("..")` giữ nguyên vì đã giải mã đúng `"."` và được lọc đúng.
- Lớp phòng vệ mới ở `walk` là bổ sung đúng như khuyến nghị "phòng vệ chống đệ quy" trong brief 4b, không phải nguyên nhân mới phát sinh.
- Không có lỗi mới ngoài dự kiến trong quá trình sửa. Mọi test MBN/Viettel từng PASS trước 4b vẫn PASS (thể hiện ở 34/34 và 243/243).

Đã xong Giai đoạn 4b, report tại `reports/phase-4b-walk-cycle-fix.md`. Chờ xác nhận để làm Giai đoạn 5.
