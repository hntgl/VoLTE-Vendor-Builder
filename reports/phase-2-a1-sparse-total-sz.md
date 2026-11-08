# Giai đoạn 2 — A1: `total_sz` của sparse chunk sai theo từng loại chunk

STATUS: DONE

## 1. Thay đổi

- `volte-core/src/sparse.rs`
  - `check_chunk_size` không còn so `total_sz` với `csize * blk_sz` cho mọi loại chunk. Giờ nó tính `total_sz` kỳ vọng theo từng loại: RAW `12 + csize*blk_sz`, FILL `12 + 4`, DONT_CARE `12`, CRC32 `12 + 4`, và báo lỗi nêu rõ header + data khi lệch.
  - Helper `build_sparse` trong `#[cfg(test)] mod tests` ghi đúng `total_sz` cho cả 4 loại chunk (trước đây cũng sai cùng một kiểu, nên test cũ tự che bug).
  - Thêm 4 regression test tiền tố `a1_`.
- `reports/phase-2-a1-sparse-total-sz.md` (báo cáo này).

Không có thay đổi nào khác trong cây làm việc cuối cùng.

Ghi chú minh bạch: để test target biên dịch được (lỗi có sẵn `modem.rs` thiếu `Debug`, ngoài phạm vi A1), tôi đã **tạm thời** thêm `#[derive(Debug)]` vào `Fat16Image` và `ModemReport`, chạy test, rồi đã gỡ bỏ. Hai dòng đó **không còn** trong cây cuối cùng (`Select-String ... derive(Debug)` đếm được `0`); `modem.rs` không nằm trong phạm vi sửa của A1.

## 2. Căn cứ định dạng (trích nguyên văn)

Fix này dựa trên định nghĩa `sparse_header`/`chunk_header` của AOSP. Trích nguyên văn từ `libsparse/sparse_format.h`:

```c
  __le32 chunk_sz; /* in blocks in output image */
  __le32 total_sz; /* in bytes of chunk input file including chunk header and data */
```

```c
/* Following a Raw or Fill or CRC32 chunk is data.
 *  For a Raw chunk, it's the data in chunk_sz * blk_sz.
 *  For a Fill chunk, it's 4 bytes of the fill data.
 *  For a CRC32 chunk, it's 4 bytes of CRC32
 */
```

Nguồn: `https://android.googlesource.com/platform/system/core/+/refs/heads/main/libsparse/sparse_format.h?format=TEXT` (nội dung base64 đã giải mã, lưu tại `%TEMP%\opencode\sparse_format.h.txt`).

Suy ra: `total_sz` **luôn** gồm 12 byte chunk header + data thật của chunk; data thật phụ thuộc loại chunk. DONT_CARE không có data nên `total_sz = 12`. Vì vậy code cũ (`tsize != data_len`) sai với FILL, DONT_CARE và CRC32.

## 3. Diff

### 3.1. `git diff -- volte-core/src/sparse.rs` (nguyên văn)

Git in thêm một cảnh báo LF→CRLF cho file này; phần dưới là nội dung diff thật:

```diff
diff --git a/volte-core/src/sparse.rs b/volte-core/src/sparse.rs
index c3608ba..c6da63b 100644
--- a/volte-core/src/sparse.rs
+++ b/volte-core/src/sparse.rs
@@ -135,27 +135,48 @@ pub fn output_size(src: &Path) -> Result<Option<u64>> {
     Ok(Some(parse_header(&head)?.expected))
 }
 
-/// Rejects a chunk whose declared payload size disagrees with its block count.
+/// Rejects a chunk whose declared `total_sz` disagrees with the format.
+///
+/// `total_sz` counts the 12-byte chunk header plus the chunk's real data, and
+/// the data differs per type: RAW carries `csize * blk_sz` bytes, FILL carries
+/// the fixed 4-byte pattern, DONT_CARE carries nothing, and CRC32 carries its
+/// 4-byte checksum with no blocks of its own.
 fn check_chunk_size(ctype: u16, csize: u64, blk_sz: u64, tsize: u64) -> Result<u64> {
     if ctype == CRC32 {
-        if csize != 0 || tsize != 4 {
+        if csize != 0 || tsize != CHUNK_HEADER_LEN + 4 {
             return Err(Error::build(format!(
-                "sparse CRC chunk declares {csize} blocks and {tsize} bytes (expected 0 and 4)"
+                "sparse CRC chunk declares {csize} blocks and {tsize} bytes (expected 0 and {})",
+                CHUNK_HEADER_LEN + 4
             )));
         }
         return Ok(0);
     }
-    let data_len = csize
-        .checked_mul(blk_sz)
-        .ok_or_else(|| Error::build("sparse chunk size overflows"))?;
     if csize == 0 {
         return Err(Error::build(format!(
             "sparse chunk 0x{ctype:X} declares no blocks"
         )));
     }
-    if tsize != data_len {
+    let data_len = csize
+        .checked_mul(blk_sz)
+        .ok_or_else(|| Error::build("sparse chunk size overflows"))?;
+    let expected = match ctype {
+        RAW => CHUNK_HEADER_LEN
+            .checked_add(data_len)
+            .ok_or_else(|| Error::build("sparse chunk size overflows"))?,
+        FILL => CHUNK_HEADER_LEN + 4,
+        DONT_CARE => CHUNK_HEADER_LEN,
+        other => {
+            return Err(Error::build(format!(
+                "bad sparse chunk type: 0x{other:X}"
+            )))
+        }
+    };
+    if tsize != expected {
         return Err(Error::build(format!(
-            "sparse chunk 0x{ctype:X} declares {tsize} bytes for {csize} blocks x {blk_sz}"
+            "sparse chunk 0x{ctype:X} declares {tsize} bytes but the format requires {expected} \
+             ({} header + {} data)",
+            CHUNK_HEADER_LEN,
+            expected - CHUNK_HEADER_LEN
         )));
     }
     Ok(data_len)
@@ -332,27 +353,27 @@ mod tests {
                     out.extend_from_slice(&RAW.to_le_bytes());
                     out.extend_from_slice(&0u16.to_le_bytes());
                     out.extend_from_slice(&((d.len() as u32) / BLK).to_le_bytes());
-                    out.extend_from_slice(&(d.len() as u32).to_le_bytes());
+                    out.extend_from_slice(&(12 + d.len() as u32).to_le_bytes());
                     out.extend_from_slice(d);
                 }
                 Chunk::Fill(pat, n) => {
                     out.extend_from_slice(&FILL.to_le_bytes());
                     out.extend_from_slice(&0u16.to_le_bytes());
                     out.extend_from_slice(&n.to_le_bytes());
-                    out.extend_from_slice(&(n * BLK).to_le_bytes());
+                    out.extend_from_slice(&(12u32 + 4).to_le_bytes());
                     out.extend_from_slice(pat);
                 }
                 Chunk::DontCare(n) => {
                     out.extend_from_slice(&DONT_CARE.to_le_bytes());
                     out.extend_from_slice(&0u16.to_le_bytes());
                     out.extend_from_slice(&n.to_le_bytes());
-                    out.extend_from_slice(&(n * BLK).to_le_bytes());
+                    out.extend_from_slice(&12u32.to_le_bytes());
                 }
                 Chunk::Crc(v) => {
                     out.extend_from_slice(&CRC32.to_le_bytes());
                     out.extend_from_slice(&0u16.to_le_bytes());
                     out.extend_from_slice(&0u32.to_le_bytes());
-                    out.extend_from_slice(&4u32.to_le_bytes());
+                    out.extend_from_slice(&(12u32 + 4).to_le_bytes());
                     out.extend_from_slice(&v.to_le_bytes());
                 }
             }
@@ -741,4 +762,110 @@ mod tests {
         assert!(output_size(&bad_path).is_err());
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    fn total_sz_at(sp: &[u8], chunk_at: usize) -> u32 {
+        u32::from_le_bytes(sp[chunk_at + 8..chunk_at + 12].try_into().unwrap())
+    }
+
+    #[test]
+    fn a1_fill_chunk_total_size_is_header_plus_four_bytes() {
+        let sp = build_sparse(&[Chunk::Fill([0xAA, 0xBB, 0xCC, 0xDD], 4)]);
+        assert_eq!(
+            total_sz_at(&sp, 28),
+            12 + 4,
+            "FILL total_sz is the 12-byte chunk header plus the 4-byte pattern, \
+             independent of how many blocks the chunk covers"
+        );
+    }
+
+    #[test]
+    fn a1_dont_care_chunk_total_size_is_only_the_header() {
+        let sp = build_sparse(&[Chunk::DontCare(5)]);
+        assert_eq!(
+            total_sz_at(&sp, 28),
+            12,
+            "DONT_CARE carries no data, so total_sz is only the chunk header"
+        );
+    }
+
+    #[test]
+    fn a1_raw_chunk_total_size_is_data_plus_the_header() {
+        let sp = build_sparse(&[Chunk::Raw(vec![7u8; 2 * BLK as usize])]);
+        assert_eq!(
+            total_sz_at(&sp, 28),
+            12 + 2 * BLK,
+            "RAW total_sz is the chunk header plus chunk_sz * blk_sz"
+        );
+    }
+
+    #[test]
+    fn a1_img2simg_style_sparse_image_is_accepted() {
+        fn file_header(total_blks: u32, total_chunks: u32) -> Vec<u8> {
+            let mut out = Vec::new();
+            out.extend_from_slice(&SPARSE_MAGIC.to_le_bytes());
+            out.extend_from_slice(&1u16.to_le_bytes());
+            out.extend_from_slice(&0u16.to_le_bytes());
+            out.extend_from_slice(&28u16.to_le_bytes());
+            out.extend_from_slice(&12u16.to_le_bytes());
+            out.extend_from_slice(&BLK.to_le_bytes());
+            out.extend_from_slice(&total_blks.to_le_bytes());
+            out.extend_from_slice(&total_chunks.to_le_bytes());
+            out.extend_from_slice(&0u32.to_le_bytes());
+            out
+        }
+
+        fn chunk(ctype: u16, csize: u32, total_sz: u32, data: &[u8]) -> Vec<u8> {
+            let mut out = Vec::new();
+            out.extend_from_slice(&ctype.to_le_bytes());
+            out.extend_from_slice(&0u16.to_le_bytes());
+            out.extend_from_slice(&csize.to_le_bytes());
+            out.extend_from_slice(&total_sz.to_le_bytes());
+            out.extend_from_slice(data);
+            out
+        }
+
+        let dir = tmp("volte_sparse_a1_img2simg");
+        let src = dir.join("image.sparse");
+        let dst = dir.join("image.raw");
+
+        let mut raw_block = vec![0u8; BLK as usize];
+        raw_block[0] = 0x5A;
+        let pattern = [0x11u8, 0x22, 0x33, 0x44];
+        let mut sp = file_header(5, 4);
+        sp.extend_from_slice(&chunk(RAW, 1, 12 + BLK, &raw_block));
+        sp.extend_from_slice(&chunk(FILL, 2, 12 + 4, &pattern));
+        sp.extend_from_slice(&chunk(DONT_CARE, 2, 12, &[]));
+        sp.extend_from_slice(&chunk(
+            CRC32,
+            0,
+            12 + 4,
+            &0xDEAD_BEEFu32.to_le_bytes(),
+        ));
+        std::fs::write(&src, &sp).unwrap();
+
+        assert_eq!(
+            unsparse(&src, &dst).unwrap(),
+            Unsparse::Converted {
+                written: 5 * BLK as u64
+            },
+            "a real img2simg image uses total_sz = header + per-type data"
+        );
+        let raw = std::fs::read(&dst).unwrap();
+        assert_eq!(raw.len(), 5 * BLK as usize);
+        assert_eq!(&raw[..BLK as usize], &raw_block[..]);
+        for i in 1..=2 {
+            let off = BLK as usize * i;
+            assert!(
+                (0..BLK as usize).all(|j| raw[off + j] == pattern[j % 4]),
+                "fill block {i}"
+            );
+        }
+        assert!(
+            raw[3 * BLK as usize..5 * BLK as usize]
+                .iter()
+                .all(|&b| b == 0),
+            "the DONT_CARE tail must stay zero"
+        );
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

## 4. Test đã thêm

Tất cả trong `volte-core/src/sparse.rs`, `#[cfg(test)] mod tests`, tiền tố `a1_` để chạy gọn bằng `cargo test -p volte-core a1_`:

1. `a1_fill_chunk_total_size_is_header_plus_four_bytes`
   - `build_sparse` một `Chunk::Fill` phủ 4 block.
   - Khẳng định `total_sz` tại offset 28 là `12 + 4 = 16`, không phụ thuộc số block.
   - Trên code cũ: thực tế `16384` (`4 * 4096`) → FAIL.

2. `a1_dont_care_chunk_total_size_is_only_the_header`
   - `Chunk::DontCare(5)`.
   - Khẳng định `total_sz == 12` (chỉ header).
   - Trên code cũ: thực tế `20480` (`5 * 4096`) → FAIL.

3. `a1_raw_chunk_total_size_is_data_plus_the_header`
   - `Chunk::Raw(vec![7; 2*BLK])`.
   - Khẳng định `total_sz == 12 + 2*BLK = 8204`.
   - Trên code cũ: thực tế `8192` (thiếu 12 header) → FAIL.

4. `a1_img2simg_style_sparse_image_is_accepted`
   - Tự dựng file sparse đúng định dạng img2simg: header 28 byte + 4 chunk (RAW 1 block, FILL 2 block, DONT_CARE 2 block, CRC32), mỗi chunk dùng `total_sz = header + data` theo loại.
   - Chạy `unsparse` thật ra file raw, kiểm tra kích thước, nội dung block RAW, pattern FILL và vùng DONT_CARE phải bằng 0.
   - Trên code cũ: `unsparse` trả `Err(... "sparse chunk 0xCAC1 declares 4108 bytes for 1 blocks x 4096")` → FAIL.

Test 4 chính là bằng chứng "ảnh do img2simg/AOSP tạo ra bị tool từ chối" — đúng triệu chứng người dùng gặp.

Test helper `build_sparse` được sửa cùng lúc vì nếu chỉ sửa production code, các test cũ dùng helper sai sẽ không còn phản ánh định dạng thật.

## 5. Kết quả chạy thật

Môi trường:

- `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- `rustc 1.101.0-nightly (12433c307 2026-10-24)`
- Windows, workspace `E:\Build_app\Volte\VoLTE-Vendor-Builder`

### 5.1. Trước khi sửa — chứng minh bug tồn tại

Lệnh:

```text
cargo test -p volte-core a1_ -- --nocapture
```

Kết quả: `FAILED. 0 passed; 4 failed` — cả 4 test mới đều fail đúng lý do cần chứng minh.

```text
running 4 tests
test sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header ... FAILED
test sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes ... FAILED
test sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header ... FAILED
test sparse::tests::a1_img2simg_style_sparse_image_is_accepted ... FAILED

thread 'sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header' (8228) panicked at 
volte-core\src\sparse.rs:773:9:
assertion `left == right` failed: RAW total_sz is the chunk header plus chunk_sz * blk_sz
  left: 8192
 right: 8204

thread 'sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes' (6400) panicked at 
volte-core\src\sparse.rs:752:9:
assertion `left == right` failed: FILL total_sz is the 12-byte chunk header plus the 4-byte pattern, independent of 
how many blocks the chunk covers
  left: 16384
 right: 16

thread 'sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header' (10020) panicked at 
volte-core\src\sparse.rs:763:9:
assertion `left == right` failed: DONT_CARE carries no data, so total_sz is only the chunk header
  left: 20480
 right: 12

thread 'sparse::tests::a1_img2simg_style_sparse_image_is_accepted' (10096) panicked at volte-core\src\sparse.rs:826:34:
called `Result::unwrap()` on an `Err` value: Error { kind: Build, message: "sparse chunk 0xCAC1 declares 4108 bytes 
for 1 blocks x 4096" }

failures:
    sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header
    sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes
    sparse::tests::a1_img2simg_style_sparse_image_is_accepted
    sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header

test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 227 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p volte-core --lib`
```

Phân loại: FAIL do assertion/parse đúng theo định dạng sparse, không phải lỗi compile/fixture/môi trường.

### 5.2. Sau khi sửa

Lệnh:

```text
cargo test -p volte-core a1_ -- --nocapture
```

```text
running 4 tests
test sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes ... ok
test sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header ... ok
test sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header ... ok
test sparse::tests::a1_img2simg_style_sparse_image_is_accepted ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 227 filtered out; finished in 0.03s
```

### 5.3. Toàn bộ test module `sparse::` (không hồi quy)

Lệnh:

```text
cargo test -p volte-core sparse:: -- --nocapture
```

```text
running 28 tests
test sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header ... ok
test sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes ... ok
test sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header ... ok
test sparse::tests::a_block_size_not_multiple_of_four_is_rejected ... ok
test sparse::tests::a_chunk_count_that_cannot_fit_the_file_is_rejected ... ok
test sparse::tests::a_chunk_payload_size_that_disagrees_with_its_blocks_is_rejected ... ok
test sparse::tests::a_crc_chunk_that_declares_blocks_is_rejected ... ok
test sparse::tests::a_header_size_that_the_chunks_do_not_cover_is_an_error ... ok
test sparse::tests::a_short_chunk_header_is_rejected ... ok
test sparse::tests::a_short_file_header_is_rejected ... ok
test sparse::tests::a1_img2simg_style_sparse_image_is_accepted ... ok
test sparse::tests::a_trailing_dont_care_chunk_extends_the_output ... ok
test sparse::tests::a_zero_block_size_is_rejected ... ok
test sparse::tests::an_absurd_declared_size_is_rejected ... ok
test sparse::tests::an_image_declaring_no_blocks_is_rejected ... ok
test sparse::tests::an_unsupported_major_version_is_rejected ... ok
test sparse::tests::chunks_that_cover_more_than_the_header_declares_are_rejected_early ... ok
test sparse::tests::an_all_dont_care_image_is_zero_filled_to_the_declared_size ... ok
test sparse::tests::non_sparse_file_leaves_destination_absent ... ok
test sparse::tests::magic_detection ... ok
test sparse::tests::output_size_reads_the_declared_raw_size ... ok
test sparse::tests::an_image_without_dont_care_keeps_the_exact_length ... ok
test sparse::tests::truncated_chunk_header_is_an_error ... ok
test sparse::tests::raw_fill_dont_care_crc_round_trip ... ok
test sparse::tests::oversized_headers_are_skipped ... ok
test sparse::tests::truncated_fill_pattern_is_an_error ... ok
test sparse::tests::truncated_raw_chunk_is_an_error ... ok
test sparse::tests::unknown_chunk_type_is_an_error ... ok

test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 203 filtered out; finished in 0.13s
```

Đáng chú ý: test cũ `a_chunk_payload_size_that_disagrees_with_its_blocks_is_rejected` vẫn PASS — test đó dùng RAW và vẫn bị từ chối khi `total_sz` lệch, nên hành vi chặn ảnh hỏng được giữ nguyên.

### 5.4. Cargo build workspace

Lệnh:

```text
cargo build --workspace
```

Kết quả: PASS (chỉ còn warning dead-code có sẵn trong `modem.rs`).

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:285:5
    |
282 | pub struct Fat16Image {
    |            ---------- field in this struct
...
285 |     spc: u32,
    |     ^^^
    |
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib) generated 1 warning
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.39s
```

### 5.5. Cargo test workspace

Lệnh:

```text
cargo test --workspace
```

Kết quả trên cây cuối cùng (đã gỡ bản vá tạm ở `modem.rs`): **không biên dịch được test target** vì lỗi có sẵn trong `modem.rs` (ngoài phạm vi A1), đúng như Giai đoạn 1 đã ghi nhận.

```text
error[E0277]: `modem::Fat16Image` doesn't implement `Debug`
1240 |         let e = Fat16Image::new(vec![0u8; 100]).unwrap_err();
     |                                                 ^^^^^^^^^^ the trait `Debug` is not implemented for 
`modem::Fat16Image`
help: consider annotating `modem::Fat16Image` with `#[derive(Debug)]`

error[E0277]: `ModemReport` doesn't implement `Debug`  (×4, các dòng modem.rs:1453/1462/1509/1551)
help: consider annotating `ModemReport` with `#[derive(Debug)]`

error: could not compile `volte-core` (lib test) due to 5 previous errors
```

Để lấy bằng chứng rộng hơn, tôi đã chạy lại toàn bộ test module sparse khi còn bản vá tạm `Debug` (mục 5.3), rồi gỡ bản vá. Không có test sparse nào khác bị ảnh hưởng bởi A1.

### 5.6. Cargo clippy workspace

Lệnh:

```text
cargo clippy --workspace --all-targets
```

Kết quả: FAIL ở bước biên dịch `--all-targets` do cùng lỗi `Debug` có sẵn trong `modem.rs`; warning clippy có sẵn cũng nằm trong `modem.rs`, **không có warning nào ở `sparse.rs`**.

```text
warning: field `spc` is never read
warning: this manual char comparison can be written more succinctly
warning: manually reimplementing `div_ceil`
warning: `volte-core` (lib) generated 3 warnings
error[E0277]: `modem::Fat16Image` doesn't implement `std::fmt::Debug`
error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`  (×4)
error: could not compile `volte-core` (lib test) due to 5 previous errors
```

### 5.7. Git diff --check

Lệnh:

```text
git diff --check
```

Exit code: `0`. Chỉ có cảnh báo LF/CRLF của Git cho các file đang có sẵn thay đổi; không có whitespace error.

## 6. Đánh giá kết quả

- **Đã chứng minh được bug cũ:** Có. 4/4 test mới FAIL trên code cũ với đúng giá trị `total_sz` sai theo từng loại chunk, và test dựng ảnh kiểu img2simg bị từ chối.
- **Đã sửa code:** Có. `check_chunk_size` và helper `build_sparse` giờ dùng `total_sz = 12 + data` theo loại chunk đúng AOSP `sparse_format.h`.
- **Đã xác nhận regression test:** Có. Sau khi sửa, `4 passed; 0 failed`; toàn module `sparse::` `28 passed; 0 failed`.
- **Đã kiểm tra workspace:** một phần.
  - `cargo build --workspace`: PASS.
  - `cargo test --workspace`: FAIL do lỗi biên dịch test có sẵn trong `modem.rs` (thiếu `Debug`), không liên quan A1.
  - `cargo clippy --workspace --all-targets`: FAIL vì cùng lỗi biên dịch test của `modem.rs`.
- **Hạng mục chưa thể xác minh:** Không thể xác nhận toàn bộ workspace test/clippy PASS trên cây hiện tại vì các lỗi có sẵn ngoài phạm vi A1. Không có bằng chứng nào cho thấy A1 gây ra các lỗi này.
- Không lấy test PASS làm bằng chứng image thật của thiết bị flash được. Đây là kiểm chứng parser/writer sparse theo đúng định dạng AOSP trên fixture dựng tay.

## 7. Phát hiện ngoài phạm vi

Không tự sửa các mục sau:

1. `volte-core/src/modem.rs` (untracked) không biên dịch được test target: `Fat16Image`/`ModemReport` thiếu `Debug` (modem.rs:1240, 1453, 1462, 1509, 1551). Theo chỉ đạo của người dùng, việc này sẽ gộp vào đầu Giai đoạn 4 (A4).
2. Khi tạm thêm `Debug`, workspace test gặp `STATUS_STACK_OVERFLOW` tại `modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn`. Theo chỉ đạo của người dùng, cần điều tra kỹ ngay khi bắt đầu Giai đoạn 4 (trước khi sửa A4) để xác định có cùng gốc với A4 hay là lỗi thứ hai.
3. `cargo clippy` báo warning có sẵn trong `modem.rs`: `spc` never read, `manual_pattern_char_comparison`, `manual_div_ceil`.
4. Cây làm việc có nhiều thay đổi/xóa file chưa commit từ trước (README, profiles, reports cũ, `modem.rs`, `modem_cli.rs`, zip...). Không đụng tới.

## 8. File đã thay đổi

- `volte-core/src/sparse.rs`
- `reports/phase-2-a1-sparse-total-sz.md` (mới)

`volte-core/src/modem.rs` đã được sửa tạm thời để chạy test rồi gỡ bỏ; nội dung cuối cùng không thay đổi so với trước khi thực hiện A1.

## 9. Kết luận

A1 đã được tái hiện bằng 4 test thật (bao gồm một ảnh sparse dựng đúng kiểu img2simg bị tool từ chối), sửa đúng một hàm kiểm tra `total_sz` và helper test trong `sparse.rs`, và regression test đã PASS sau khi sửa. Parser giờ chấp nhận ảnh sparse hợp lệ do AOSP/img2simg tạo và vẫn từ chối ảnh khai báo `total_sz` sai.

STATUS: DONE cho phạm vi A1. Các kiểm tra workspace rộng hơn đang bị chặn bởi lỗi có sẵn trong `modem.rs` (ngoài phạm vi), đã ghi nhận và không tự sửa.
