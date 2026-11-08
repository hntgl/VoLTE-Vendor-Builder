# GIAI ĐOẠN 5 — A2: ĐỌC ĐỦ DESCRIPTOR TAG 2 / TAG 4 VÀ GHI NHẬN TAG LẠ

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Phạm vi source: `volte-core/src/avb.rs` (match tag trong `parse_vbmeta`, 2 hàm parse mới, `VbmetaMeta`, `AvbInfo`, test `a2_`) và `volte-core/src/pipeline.rs` (1 vòng cảnh báo tại call site đã có `log`).
- Không sửa `volte-core/src/report.rs`.
- Toolchain: `rustc 1.101.0-nightly (12433c307 2026-10-24)` / `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- Môi trường: Windows 10, Git Bash.
- Log tạm đối chiếu (ngoài repo): `C:\Users\HT\.qoder\tmp\phase5\`
- Snapshot cuối: `avb.rs` SHA-256 `c31cb5e3af3926ea0204c3614a040e951fe9ee79e59903ea7b346cb2a50834a2`, `pipeline.rs` SHA-256 `0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1`.

## 1. STATUS

**DONE.**

- Tag 2 (`AvbHashDescriptor`) được đọc: `partition_name`, `hash_algorithm`, `salt`; `digest_len == 0` được chấp nhận là persistent digest.
- Tag 4 (`AvbChainPartitionDescriptor`) được đọc: chỉ `partition_name`; không suy diễn `hash_algorithm`/`salt`/`fec_roots`.
- Tag khác 0/1/2/4 được ghi vào `VbmetaMeta.unsupported_tags` → `AvbInfo.unsupported_descriptor_tags`, vẫn parse `Ok`; pipeline in `[WARN] AVB vbmeta: unsupported descriptor tag {tag}`.
- 3 test mới `a2_` PASS; toàn bộ test AVB cũ và snapshot `avb_lines_snapshot_is_byte_for_byte` PASS y nguyên.
- `cargo test --workspace`: 30 + 246 PASS, exit 0.
- `cargo build --workspace`: exit 0.
- `cargo clippy --workspace --all-targets`: exit 0, không phát sinh warning mới (3 warning cũ trong `modem.rs`).
- `git diff --check`: exit 0.

**Mức độ nghiêm trọng:** đây là lỗi **hoàn thiện theo spec**, không phải lỗi chặn luồng chính. Trong luồng hiện tại, `avb_info_fast`/`parse_vbmeta` chỉ được gọi trên chính `vendor.img` đang build (footer AVB của một partition); `add_hashtree_footer` của một vendor image thông thường chỉ sinh tag 0 và tag 1. Tag 2/4 trên thực tế gần như không xuất hiện trong luồng chính của tool. Việc sửa đảm bảo nếu ảnh có descriptor hợp lệ khác thì thông tin được đọc đúng thay vì âm thầm bỏ qua.

Không tuyên bố đã chạy thật trên thiết bị; bằng chứng chỉ là test Rust + build.

## 2. BỐI CẢNH VÀ PHẠM VI

### 2.1. Hiện trạng trước khi sửa

`parse_vbmeta` chỉ có hai nhánh:

```rust
match tag {
    0 => { if let Some((key, value)) = parse_property(body, pos)? { meta.props.push((key, value)); } }
    1 => parse_hashtree(body, pos, &mut meta)?,
    _ => {}
}
```

Hệ quả:

- Tag 2 (hash descriptor) và tag 4 (chain partition descriptor) bị bỏ qua hoàn toàn, nên `AvbInfo` giữ mặc định `"vendor"` / `"sha256"` / salt rỗng dù vbmeta có thông tin.
- Tag lạ bị bỏ qua không dấu vết; người dùng không biết vbmeta chưa được đọc đầy đủ.

### 2.2. Đã thay đổi

- `volte-core/src/avb.rs`:
  - Thêm `pub unsupported_descriptor_tags: Vec<u64>` vào `AvbInfo` (dòng 58), mặc định rỗng trong `AvbInfo::empty` (dòng 77).
  - Gán `info.unsupported_descriptor_tags = meta.unsupported_tags;` trong `fill_from_vbmeta` (dòng 169).
  - Thêm hằng `HASH_DESCRIPTOR_LEN = 132` (dòng 182) và `CHAIN_PARTITION_DESCRIPTOR_LEN = 92` (dòng 184).
  - Thêm `unsupported_tags: Vec<u64>` vào `VbmetaMeta` (dòng 196).
  - `parse_vbmeta`: nhánh `2`, `4`, `_` tại dòng 296–298; cập nhật doc comment dòng 227–232.
  - Thêm `parse_hash_descriptor` (dòng 359–407) và `parse_chain_partition_descriptor` (dòng 412–448).
  - Test mới tại dòng 1467 (`a2_hash_...`), 1485 (`a2_chain_...`), 1497 (`a2_an_unknown_...`); fixture độc lập tại dòng 1371–1461.
- `volte-core/src/pipeline.rs`: vòng cảnh báo dòng 453–457, ngay sau `avb::avb_info_fast(&work)?`.

### 2.3. Không thay đổi

- `parse_hashtree`, `parse_property`, cách xử lý tag 0/1.
- Fallback `"vendor"` / `"sha256"` / salt rỗng của `AvbInfo::empty()`.
- `report.rs` / `avb_lines`.
- Không thêm `digest` / `public_key` vào `VbmetaMeta`/`AvbInfo`.
- Không thêm tham số `Logger` vào các hàm AVB thuần; cảnh báo đặt tại call site pipeline.
- Không sửa dependency, kiến trúc; không xóa/disable test.

## 3. ĐỐI CHIẾU SPEC

Nguồn tự tải và giải mã nguyên văn (không copy từ brief):

- `AvbHashDescriptor` — AOSP `external/avb`, tag `android-9.0.0_r31`:
  `https://android.googlesource.com/platform/external/avb/+/android-9.0.0_r31/libavb/avb_hash_descriptor.h?format=TEXT`
  SHA-256 file tải về: `61811399ac8e4ca0392a87039de9edfcaab72d165a346337e87b5fb5c3a4d602`
- `AvbChainPartitionDescriptor` — AOSP `external/avb`, nhánh `master`:
  `https://android.googlesource.com/platform/external/avb/+/master/libavb/avb_chain_partition_descriptor.h?format=TEXT`
  SHA-256 file tải về: `50d29e5c5ec59b50467cd8e3b372c92c3aff26c1f6fd0ea08e8c0176ae73bcf9`
- `AvbDescriptor` (header chung) — cùng tag `android-9.0.0_r31`:
  `https://android.googlesource.com/platform/external/avb/+/android-9.0.0_r31/libavb/avb_descriptor.h?format=TEXT`
  SHA-256 file tải về: `4ba2c2686c1fe657a51b7d3b683866885b8b038846256357b527e2c327f26797`

### 3.1. `AvbDescriptor` — 16 byte đầu của mọi descriptor

Trích nguyên văn `avb_descriptor.h` dòng 62–65:

```c
typedef struct AvbDescriptor {
  uint64_t tag;
  uint64_t num_bytes_following;
} AVB_ATTR_PACKED AvbDescriptor;
```

Đối chiếu code: `parse_vbmeta` đọc `tag = be_u64(&vb[pos..pos + 8])`, `nbf = be_u64(&vb[pos + 8..pos + 16])`, và truyền `body = &vb[pos..pos + 16 + nbf]`. Vì vậy trong `parse_hash_descriptor`/`parse_chain_partition_descriptor`, `body[0..16]` là `parent_descriptor`, khớp đúng struct trên.

### 3.2. `AvbHashDescriptor` (tag 2)

Trích nguyên văn `avb_hash_descriptor.h` dòng 48–73:

```c
/* A descriptor containing information about hash for an image.
 *
 * This descriptor is typically used for boot partitions to verify the
 * entire kernel+initramfs image before executing it.
 *
 * Following this struct are |partition_name_len| bytes of the
 * partition name (UTF-8 encoded), |salt_len| bytes of salt, and then
 * |digest_len| bytes of the digest.
 *
 * The |reserved| field is for future expansion and must be set to NUL
 * bytes.
 *
 * Changes in v1.1:
 *   - flags field is added which supports AVB_HASH_DESCRIPTOR_FLAGS_USE_AB
 *   - digest_len may be zero, which indicates the use of a persistent digest
 */
typedef struct AvbHashDescriptor {
  AvbDescriptor parent_descriptor;
  uint64_t image_size;
  uint8_t hash_algorithm[32];
  uint32_t partition_name_len;
  uint32_t salt_len;
  uint32_t digest_len;
  uint32_t flags;
  uint8_t reserved[60];
} AVB_ATTR_PACKED AvbHashDescriptor;
```

Đối chiếu offset từng field với code (`body` tính cả 16 byte header):

| Field (AOSP) | Offset trong `body` | Code đọc |
|---|---:|---|
| `parent_descriptor` (16 byte) | 0–16 | `parse_vbmeta`: tag/nbf |
| `image_size` (u64) | 16–24 | không đọc (không có trường dùng trong `VbmetaMeta`) |
| `hash_algorithm[32]` | 24–56 | `fixed_string(&body[24..56], "hash algorithm")` |
| `partition_name_len` (u32) | 56–60 | `be_u32(&body[56..60])` |
| `salt_len` (u32) | 60–64 | `be_u32(&body[60..64])` |
| `digest_len` (u32) | 64–68 | `be_u32(&body[64..68])` |
| `flags` (u32) | 68–72 | không đọc (không cần) |
| `reserved[60]` | 72–132 | không đọc |
| `partition_name` (UTF-8) | 132 … 132+name_len | `std::str::from_utf8` |
| `salt` | tiếp theo `salt_len` byte | `hex_lower` |
| `digest` | tiếp theo `digest_len` byte | **không đọc** (không có trường chứa) |

Kích thước cố định: 16 + 8 + 32 + 4 + 4 + 4 + 4 + 60 = **132 byte**. Công thức `num_bytes_following` của libavb là `round8(sizeof(AvbHashDescriptor) - sizeof(AvbDescriptor) + name_len + salt_len + digest_len)`, code dùng `dynamic = 132 + name_len + salt_len + digest_len`, `expected = round8(dynamic - 16)` — khớp.

Điều khoản persistent digest: trích dẫn nguyên văn ở trên — “`digest_len` may be zero, which indicates the use of a persistent digest”. Code không kiểm tra `digest_len != 0`, chỉ dùng nó trong phép tính độ dài; test `a2_hash_descriptor_sets_partition_name_and_algorithm` truyền `digest = &[]` để khóa điều khoản này.

Không set `fec_roots` từ tag 2: struct trên không có trường FEC; code giữ `meta.fec_roots` nguyên trạng.

### 3.3. `AvbChainPartitionDescriptor` (tag 4)

Trích nguyên văn `avb_chain_partition_descriptor.h` dòng 48–71:

```c
/* A descriptor containing a pointer to signed integrity data stored
 * on another partition. The descriptor contains the partition name in
 * question (without the A/B suffix), the public key used to sign the
 * integrity data, and rollback index location to use for rollback
 * protection.
 *
 * Following this struct are |partition_name_len| bytes of the
 * partition name (UTF-8 encoded) and |public_key_len| bytes of the
 * public key.
 *
 * The |reserved| field is for future expansion and must be set to NUL
 * bytes.
 *
 * Changes in v1.3:
 *   - flags field is added which supports AVB_CHAIN_PARTITION_DESCRIPTOR_FLAGS_DO_NOT_USE_AB
 */
typedef struct AvbChainPartitionDescriptor {
  AvbDescriptor parent_descriptor;
  uint32_t rollback_index_location;
  uint32_t partition_name_len;
  uint32_t public_key_len;
  uint32_t flags;
  uint8_t reserved[60];
} AVB_ATTR_PACKED AvbChainPartitionDescriptor;
```

Đối chiếu offset:

| Field (AOSP) | Offset trong `body` | Code đọc |
|---|---:|---|
| `parent_descriptor` (16 byte) | 0–16 | `parse_vbmeta`: tag/nbf |
| `rollback_index_location` (u32) | 16–20 | không đọc (không cần) |
| `partition_name_len` (u32) | 20–24 | `be_u32(&body[20..24])` |
| `public_key_len` (u32) | 24–28 | `be_u32(&body[24..28])` — chỉ dùng cho kiểm tra độ dài |
| `flags` (u32) | 28–32 | không đọc |
| `reserved[60]` | 32–92 | không đọc |
| `partition_name` (UTF-8) | 92 … 92+name_len | `std::str::from_utf8` |
| `public_key` | tiếp theo `public_key_len` byte | **không đọc/không lưu** |

Kích thước cố định: 16 + 4 + 4 + 4 + 4 + 60 = **92 byte**. Công thức: `dynamic = 92 + name_len + public_key_len`, `expected = round8(dynamic - 16)` — khớp cách libavb tính `num_bytes_following`.

Không suy diễn `hash_algorithm`/`salt`/`fec_roots` từ tag 4: struct trên không có các trường này. Test `a2_chain_partition_descriptor_sets_partition_name_only` khóa ranh giới: `partition_name == "boot"` nhưng `hash_algorithm == "sha256"` và `salt == ""`.

### 3.4. Ghi chú về “SIZE”

Hai header AOSP không định nghĩa macro `SIZE`; 132/92 là kích thước của struct packed theo đúng thứ tự field ở trên. Tên hằng trong code (`HASH_DESCRIPTOR_LEN`, `CHAIN_PARTITION_DESCRIPTOR_LEN`) theo quy ước đã có (`HASHTREE_DESCRIPTOR_LEN`, `PROPERTY_DESCRIPTOR_LEN`), giá trị được tính từ layout, không phải copy một macro không tồn tại.

## 4. THAY ĐỔI THỰC TẾ

`git diff -U3 -- volte-core/src/avb.rs` trên cây hiện tại (diff so với HEAD đúng bằng thay đổi Giai đoạn 5):

```diff
diff --git a/volte-core/src/avb.rs b/volte-core/src/avb.rs
index a4300ee..2259ceb 100644
--- a/volte-core/src/avb.rs
+++ b/volte-core/src/avb.rs
@@ -53,6 +53,9 @@ pub struct AvbInfo {
     pub footer_major: u32,
     pub footer_minor: u32,
     pub props: Vec<(String, String)>,
+    /// Descriptor tags this build does not know how to read. Recorded so the
+    /// pipeline can warn instead of pretending the vbmeta was fully parsed.
+    pub unsupported_descriptor_tags: Vec<u64>,
 }
@@ -71,6 +74,7 @@ impl AvbInfo {
             footer_major: 0,
             footer_minor: 0,
             props: Vec::new(),
+            unsupported_descriptor_tags: Vec::new(),
         }
     }
 }
@@ -162,6 +166,7 @@ fn fill_from_vbmeta(path: &Path, footer: &AvbFooter) -> Result<AvbInfo> {
         info.fec_roots = roots;
     }
     info.props = meta.props;
+    info.unsupported_descriptor_tags = meta.unsupported_tags;
     Ok(info)
 }
@@ -173,6 +178,10 @@ const VBMETA_HEADER_LEN: usize = 256;
 const HASHTREE_DESCRIPTOR_LEN: usize = 180;
 /// `AvbPropertyDescriptor.SIZE`.
 const PROPERTY_DESCRIPTOR_LEN: usize = 32;
+/// `AvbHashDescriptor.SIZE` (16-byte header + 116 byte fixed body).
+const HASH_DESCRIPTOR_LEN: usize = 132;
+/// `AvbChainPartitionDescriptor.SIZE` (16-byte header + 76 byte fixed body).
+const CHAIN_PARTITION_DESCRIPTOR_LEN: usize = 92;
 
@@ -184,6 +193,7 @@ struct VbmetaMeta {
     salt: Option<String>,
     fec_roots: Option<u32>,
     props: Vec<(String, String)>,
+    unsupported_tags: Vec<u64>,
 }
@@ -216,10 +226,10 @@ fn hex_lower(bytes: &[u8]) -> String {
 /// Parses `AvbVBMetaImageHeader` plus its descriptor block. Tags are read from
-/// the descriptor header, never searched for as text; unknown tags are skipped
-/// the way `avbtool.py` skips them. Any length that does not match the
-/// descriptor's own size formula is an error, so a truncated image can never
-/// be read as metadata.
+/// the descriptor header, never searched for as text; tags this build does not
+/// understand are recorded in `unsupported_tags` rather than dropped silently.
+/// Any length that does not match the descriptor's own size formula is an
+/// error, so a truncated image can never be read as metadata.
@@ -283,7 +293,9 @@ fn parse_vbmeta(vb: &[u8]) -> std::result::Result<VbmetaMeta, String> {
             1 => parse_hashtree(body, pos, &mut meta)?,
-            _ => {}
+            2 => parse_hash_descriptor(body, pos, &mut meta)?,
+            4 => parse_chain_partition_descriptor(body, pos, &mut meta)?,
+            _ => meta.unsupported_tags.push(tag),
         }
@@ -342,6 +354,99 @@ fn parse_hashtree(
+/// Tag 2, `AvbHashDescriptor`: a whole-partition hash. `digest_len == 0` is a
+/// persistent digest and therefore valid.
+fn parse_hash_descriptor(
+    body: &[u8],
+    pos: usize,
+    meta: &mut VbmetaMeta,
+) -> std::result::Result<(), String> {
+    if body.len() < HASH_DESCRIPTOR_LEN {
+        return Err(format!(
+            "hash descriptor at {pos} is shorter than {HASH_DESCRIPTOR_LEN} bytes"
+        ));
+    }
+    let nbf = be_u64(&body[8..16]) as usize;
+    let algorithm = fixed_string(&body[24..56], "hash algorithm")?;
+    let name_len = be_u32(&body[56..60]) as usize;
+    let salt_len = be_u32(&body[60..64]) as usize;
+    let digest_len = be_u32(&body[64..68]) as usize;
+    let dynamic = HASH_DESCRIPTOR_LEN
+        .checked_add(name_len)
+        .and_then(|n| n.checked_add(salt_len))
+        .and_then(|n| n.checked_add(digest_len))
+        .ok_or("hash descriptor lengths overflow")?;
+    if dynamic > body.len() {
+        return Err(format!(
+            "hash descriptor at {pos} declares {dynamic} bytes but carries {}",
+            body.len()
+        ));
+    }
+    let expected = round8(dynamic.saturating_sub(16));
+    if nbf != expected {
+        return Err(format!(
+            "hash descriptor at {pos} declares {nbf} following bytes, expected {expected}"
+        ));
+    }
+    let name_at = HASH_DESCRIPTOR_LEN;
+    let salt_at = name_at + name_len;
+    let salt = &body[salt_at..salt_at + salt_len];
+
+    if meta.partition_name.is_none() {
+        let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+            .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
+        meta.partition_name = Some(name.to_owned());
+        meta.hash_algorithm = if algorithm.is_empty() {
+            None
+        } else {
+            Some(algorithm)
+        };
+        meta.salt = Some(hex_lower(salt));
+    }
+    Ok(())
+}
+
+/// Tag 4, `AvbChainPartitionDescriptor`: names another partition signed by a
+/// public key. It carries no hash algorithm, salt or FEC roots, so none of
+/// those are derived from it.
+fn parse_chain_partition_descriptor(
+    body: &[u8],
+    pos: usize,
+    meta: &mut VbmetaMeta,
+) -> std::result::Result<(), String> {
+    if body.len() < CHAIN_PARTITION_DESCRIPTOR_LEN {
+        return Err(format!(
+            "chain partition descriptor at {pos} is shorter than {CHAIN_PARTITION_DESCRIPTOR_LEN} bytes"
+        ));
+    }
+    let nbf = be_u64(&body[8..16]) as usize;
+    let name_len = be_u32(&body[20..24]) as usize;
+    let public_key_len = be_u32(&body[24..28]) as usize;
+    let dynamic = CHAIN_PARTITION_DESCRIPTOR_LEN
+        .checked_add(name_len)
+        .and_then(|n| n.checked_add(public_key_len))
+        .ok_or("chain partition descriptor lengths overflow")?;
+    if dynamic > body.len() {
+        return Err(format!(
+            "chain partition descriptor at {pos} declares {dynamic} bytes but carries {}",
+            body.len()
+        ));
+    }
+    let expected = round8(dynamic.saturating_sub(16));
+    if nbf != expected {
+        return Err(format!(
+            "chain partition descriptor at {pos} declares {nbf} following bytes, expected {expected}"
+        ));
+    }
+    if meta.partition_name.is_none() {
+        let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
+        let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+            .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
+        meta.partition_name = Some(name.to_owned());
+    }
+    Ok(())
+}
```

`pipeline.rs` (hunk duy nhất của Giai đoạn 5, trích từ `git diff -U3`):

```diff
@@ -450,6 +450,11 @@ pub fn run(
     progress(30, "AVB info");
     let mut avb_info = avb::avb_info_fast(&work)?;
+    for tag in &avb_info.unsupported_descriptor_tags {
+        log.log(format!(
+            "[WARN] AVB vbmeta: unsupported descriptor tag {tag}"
+        ));
+    }
     if avb_info.has_footer {
```

Test mới và fixture độc lập: xem diff đầy đủ trong `git diff -U3 -- volte-core/src/avb.rs` hunk `@@ -1262,4 +1367,139 @@ mod tests {` (6 helper + 3 test). Fixture dựng vbmeta từ offset literal của spec, dùng `a2_pad8` của test (không dùng `round8` của code) để tránh test tự lừa.

## 5. BẰNG CHỨNG ĐỎ TRƯỚC KHI SỬA

### 5.1. Thử nghiệm đầu tiên — fixture sai, chưa dùng làm bằng chứng

Lần chạy đầu, fixture đặt `aux_size = 0` trong khi descriptor block nằm trong auxiliary data block, nên `parse_vbmeta` báo lỗi biên trước khi tới phần đọc tag:

```text
thread 'avb::tests::a2_chain_partition_descriptor_sets_partition_name_only' (12320) panicked at volte-core\src\avb.rs:1373:38:
called `Result::unwrap()` on an `Err` value: Error { kind: Build, message: "AVB vbmeta at offset 1048576: descriptors end at 360, past the auxiliary block end 256" }
thread 'avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm' (14080) panicked at volte-core\src\avb.rs:1357:38:
called `Result::unwrap()` on an `Err` value: Error { kind: Build, message: "AVB vbmeta at offset 1048576: descriptors end at 408, past the auxiliary block end 256" }
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.02s
```

Sửa fixture: ghi `aux block size = desc_size` vào header `[20..28]` (đúng semantics `AvbVBMetaImageHeader`). Không liên quan code sản xuất.

### 5.2. Đỏ thực sự — test 1 và 2 trên code chưa sửa

Lệnh: `cargo test -p volte-core avb::tests::a2_ -- --nocapture` (log `red-stage-a.log`), `EXIT=101`:

```text
running 2 tests

thread 'avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm' (10560) panicked at volte-core\src\avb.rs:1361:9:
assertion `left == right` failed
  left: "vendor"
 right: "vendor_dlkm"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... FAILED

thread 'avb::tests::a2_chain_partition_descriptor_sets_partition_name_only' (13344) panicked at volte-core\src\avb.rs:1377:9:
assertion `left == right` failed
  left: "vendor"
 right: "boot"
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... FAILED

failures:
    avb::tests::a2_chain_partition_descriptor_sets_partition_name_only
    avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.02s
```

Đúng như dự kiến: tag 2/4 bị bỏ qua nên `AvbInfo` vẫn là mặc định `"vendor"` (và nếu vượt qua assert đầu, các assert `hash_algorithm`/`salt` cũng sẽ fail với `"sha256"`/`""`).

### 5.3. Đỏ thực sự — test 3 (field chưa tồn tại)

Sau khi thêm test 3 vào cùng module, code chưa sửa **không biên dịch được**; toàn bộ test binary không chạy. Đây là dạng đỏ trung thực của test 3 (không phải run-FAIL):

```text
error[E0609]: no field `unsupported_descriptor_tags` on type `avb::AvbInfo`
    --> volte-core\src\avb.rs:1401:25
     |
1401 |         assert_eq!(info.unsupported_descriptor_tags, vec![99]);
     |                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^ unknown field
     |
     = note: available fields are: `has_footer`, `partition_name`, `hash_algorithm`, `salt`, `fec_roots` ... and 8 others

For more information about this error, try `rustc --explain E0609`.
error: could not compile `volte-core` (lib test) due to 1 previous error
```

`EXIT=101`. Số dòng 1357/1361/1377/1401 là vị trí trước khi thêm code sản xuất; sau khi thêm, các test nằm tại 1467/1485/1497.

## 6. KẾT QUẢ XANH SAU KHI SỬA

Lệnh: `cargo test -p volte-core avb::tests::a2_ -- --nocapture` (log `green-a2.log`), `EXIT=0`:

```text
running 3 tests
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.02s
```

## 7. REGRESSION AVB VÀ SNAPSHOT BÁO CÁO

Lệnh: `cargo test -p volte-core avb -- --nocapture` (log `regression-avb.log`), `EXIT=0`:

```text
running 30 tests
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::info_reads_the_golden_vbmeta_fields ... ok
test avb::tests::a_truncated_or_malformed_vbmeta_is_an_error ... ok
test avb::tests::info_defaults_without_footer ... ok
test report::tests::avb_lines_snapshot_is_byte_for_byte ... ok
test report::tests::avb_lines_fall_back_to_vendor_and_sha256 ... ok
...
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 216 filtered out; finished in 70.64s
```

Ghi chú: filter `avb` khớp thêm vài test `config`/`pipeline` có nhắc AVB, nên tổng là 30 chứ không phải riêng `avb::tests`. Các test trọng yếu:

- `info_reads_the_golden_vbmeta_fields` — PASS, golden 704-byte vbmeta (tag 0/1) đọc y nguyên.
- `avb_lines_snapshot_is_byte_for_byte` — PASS, `report.rs` không bị sửa.
- `info_defaults_without_footer` — PASS, fallback `"vendor"`/`"sha256"` giữ nguyên khi ảnh không có footer.
- `a_truncated_or_malformed_vbmeta_is_an_error` — PASS, các kiểm tra lỗi cũ không bị nới lỏng.

## 8. CARGO TEST / BUILD / CLIPPY / DIFF-CHECK

### 8.1. `cargo test --workspace`

```text
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 246 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 105.95s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

`EXIT=0`. 246 = 243 test cũ + 3 test `a2_`. Không dùng `--skip`.

### 8.2. `cargo build --workspace`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
...
warning: `volte-core` (lib) generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.81s
```

`EXIT=0`. Warning `spc` có sẵn, ngoài phạm vi.

### 8.3. `cargo clippy --workspace --all-targets`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:390:31
warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:713:13
warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 suggestions)
warning: `volte-core` (lib test) generated 3 warnings (3 duplicates)
```

`EXIT=0`. Không có warning nào từ `avb.rs`/`pipeline.rs`. Lần chạy đầu, helper test `a2_pad8` dùng `(n + 7) / 8 * 8` sinh thêm `manual_div_ceil`; đã đổi sang `n.div_ceil(8) * 8` (vẫn không dùng `round8` của code) và chạy lại — warning mới biến mất.

### 8.4. `git diff --check`

`EXIT=0`. Git chỉ in cảnh báo `LF will be replaced by CRLF` cho nhiều file (do cấu hình autocrlf), không phải lỗi whitespace.

### 8.5. Kiểm tra bổ sung `cargo fmt --all -- --check`

`EXIT=1` — nhưng toàn bộ diff còn lại thuộc các file **không sửa trong Giai đoạn 5**: `volte-cli/src/main.rs:28`, `volte-cli/src/modem_cli.rs:297`, `volte-core/src/modem.rs` (nhiều vị trí), `volte-core/src/profile.rs:132`, `volte-core/src/sparse.rs:165`, `volte-core/src/sparse.rs:835`, `volte-core/src/pipeline.rs:56`. Riêng `avb.rs` **không còn** trong danh sách sau khi định dạng tay 2 chỗ mới; `pipeline.rs` chỉ còn diff tại dòng 56 (code cũ, không phải vòng cảnh báo mới ở 453–457). Không chạy `cargo fmt` toàn repo để tránh đụng file ngoài phạm vi.

## 9. PHÁT HIỆN NGOÀI PHẠM VI

1. **3 warning clippy cũ** trong `modem.rs` (`spc` không đọc dòng 292, `manual_pattern_char_comparison` dòng 390, `manual_div_ceil` dòng 713) vẫn còn; không sửa.
2. **`cargo fmt --check` đỏ do các file cũ** nêu ở 8.5; không sửa vì ngoài phạm vi và sẽ làm diff loãng.
3. **`image_size` của `AvbHashDescriptor` và `rollback_index_location`/`public_key` của `AvbChainPartitionDescriptor` không được lưu.** Đã kiểm tra `pipeline.rs` và `report.rs`: không nơi nào cần các giá trị này. Nếu sau này cần hiển thị/đối chiếu, phải thêm trường mới có chủ đích.
4. **Không hiển thị tag lạ trong báo cáo tóm tắt AVB.** Theo brief, `report.rs`/`avb_lines` có golden snapshot nên không đụng; cảnh báo chỉ lên log pipeline. Hệ quả: nếu chạy ở chế độ không xem log, tag lạ vẫn im lặng ở phần báo cáo tóm tắt.
5. **Tag 2/4 hiếm trong luồng chính** (như mục 1): giá trị thực tế của thay đổi này chủ yếu là tính đúng đắn khi gặp vbmeta tổng hợp hoặc ảnh đặc biệt, không phải sửa một lỗi đang chặn build vendor.
6. **Trạng thái cây làm việc:** ngoài `avb.rs`, `pipeline.rs` và report này, cây còn nhiều thay đổi/xóa từ các giai đoạn trước. Không dọn dẹp, không commit, không push.

## 10. CÁC VẤN ĐỀ CÒN TỒN TẠI CẦN NGƯỜI DÙNG QUYẾT ĐỊNH

1. **Vị trí cảnh báo tag lạ:** hiện chỉ ở log pipeline (đúng brief, không đụng snapshot). Anh có muốn một giai đoạn sau đưa tag lạ vào `avb_lines`/báo cáo tóm tắt (kèm cập nhật snapshot) không?
2. **Quy ước “descriptor đầu tiên thắng”:** code giữ nguyên kiểu của `parse_hashtree` — nếu tag 4 xuất hiện trước tag 2, `partition_name` lấy từ tag 4 và tag 2 không ghi đè `hash_algorithm`/`salt` vì cả block bị chặn bởi `partition_name.is_none()`. Anh xác nhận đây là hành vi mong muốn, hay muốn mỗi trường độc lập “first non-None wins”?
3. **`digest`/`public_key`:** hiện bỏ qua có chủ đích. Nếu cần kiểm tra toàn vẹn ở giai đoạn sau, đây sẽ là thay đổi riêng có test riêng.
4. **Dọn định dạng:** có nên có một commit `cargo fmt` riêng cho các file cũ đang lệch format không?
5. **Chuyển mục tiếp theo:** workspace xanh; chờ xác nhận để làm A8/A5/A10/A11/nhóm B.
6. **Chưa kiểm chứng thiết bị:** không có bằng chứng chạy thật trên máy; mới dừng ở test Rust + build.

Đã xong Giai đoạn 5, report tại `reports/phase-5-a2-avb-descriptors.md`. Chờ xác nhận để làm các mục còn lại (A8/A5/A10/A11/nhóm B).
