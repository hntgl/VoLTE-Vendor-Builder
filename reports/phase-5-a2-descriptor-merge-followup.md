# BÁO CÁO FOLLOW-UP GIAI ĐOẠN 5 (A2) — SỬA LOGIC MERGE AVB DESCRIPTOR THEO TỪNG TRƯỜNG

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Vai trò: AI2 (Local Executor)
- Phạm vi được phép sửa: `volte-core/src/avb.rs` (+ test trong module AVB) và report bổ sung này.
- Không sửa: `report.rs`, `pipeline.rs`, `sparse.rs`, `modem.rs`, `init_rc.rs`, các file nguồn khác, `Cargo.toml`, `Cargo.lock`, snapshot/golden test cũ.
- Không commit, không push, không reset/checkout/clean/stash.

---

## 8.1. TÌNH TRẠNG

**STATUS: DONE.**

### Edge case ban đầu

Trong Giai đoạn 5, cả ba hàm parse descriptor đều gói toàn bộ việc cập nhật metadata trong một điều kiện duy nhất:

```rust
if meta.partition_name.is_none() {
    // gán partition_name
    // gán hash_algorithm
    // gán salt
    // (tag 1) gán fec_roots
}
```

Hệ quả: descriptor đầu tiên có tên partition (ví dụ tag 4 `AvbChainPartitionDescriptor`) đã đặt `meta.partition_name`, nên mọi descriptor sau — kể cả tag 2/tag 1 **cùng partition** — bị chặn toàn bộ. Tag 2 không bổ sung được `hash_algorithm`/`salt`; tag 1 không bổ sung được `fec_roots`. Đây chính là Trường hợp A và C trong brief.

### Vì sao "first-non-None toàn cục" không an toàn

Cách sửa ngây thơ là bỏ điều kiện `partition_name.is_none()` và cho mỗi trường tự điền nếu đang `None`. Cách đó **sai** vì nó không kiểm tra descriptor có thuộc partition đang được chọn hay không:

- Tag 1 cho `vendor_dlkm` có thể điền `fec_roots` cho `boot` (Trường hợp E).
- Tag 2 cho `vendor_dlkm` có thể điền `hash_algorithm`/`salt` cho `boot` (Trường hợp B).

Kết quả là metadata bị ghép từ hai partition khác nhau, tạo ra tham số FEC/hash không tồn tại trong thực tế — nguy hiểm hơn cả việc để trống, vì `rebuild_hashtree` sẽ dùng các giá trị đó để dựng lại footer. Vì vậy quy tắc merge phải là **first-non-None theo từng trường, nhưng chỉ trong cùng một partition**.

### Quy tắc merge cuối cùng

1. **Chọn partition:** descriptor hợp lệ đầu tiên có tên partition sẽ đặt `meta.partition_name`. Tên đã chọn không bị ghi đè.
2. **Kiểm tra cùng partition:** mỗi descriptor tự giải mã `partition_name` của chính nó trước khi merge; chỉ khi `descriptor_partition_name == meta.partition_name` mới được đóng góp metadata.
3. **Merge từng trường độc lập:** `hash_algorithm` chỉ gán khi đang `None` và giá trị không rỗng; `salt` chỉ gán khi đang `None`; `fec_roots` chỉ gán khi đang `None`. Descriptor trước thắng.

Phạm vi từng tag (không suy diễn trường không tồn tại):

| Tag | Descriptor | Trường được cung cấp |
|---|---|---|
| 1 | `AvbHashtreeDescriptor` | partition_name, hash_algorithm, salt, fec_roots |
| 2 | `AvbHashDescriptor` | partition_name, hash_algorithm, salt |
| 4 | `AvbChainPartitionDescriptor` | partition_name |

---

## 8.2. THAY ĐỔI SOURCE

### File và hàm được sửa

Chỉ `volte-core/src/avb.rs` (source sản xuất + test trong module AVB):

| Vị trí hiện tại | Thay đổi |
|---|---|
| `VbmetaMeta` (doc, dòng 186–190) | Ghi rõ ngữ nghĩa: descriptor đầu tiên đặt tên partition, descriptor cùng partition sau đó chỉ điền trường còn `None`. |
| `select_partition()` (dòng 232) | **Mới.** Đặt tên partition nếu chưa có, trả `bool` cho biết descriptor có thuộc partition đã chọn không. |
| `merge_hash_fields()` (dòng 242) | **Mới.** First-non-None cho `hash_algorithm` (bỏ qua rỗng) và `salt`. |
| `parse_hashtree()` (dòng 328) | Giải mã tên trước; nếu cùng partition thì merge hash và điền `fec_roots` nếu còn `None`. |
| `parse_hash_descriptor()` (dòng 378) | Giải mã tên trước; nếu cùng partition thì merge hash. |
| `parse_chain_partition_descriptor()` (dòng 425) | Luôn giải mã tên và gọi `select_partition`; không suy diễn hash/salt/fec. |
| Test module | Thêm fixture `a2_hashtree_descriptor()` (dòng 1434) và 5 test mới (dòng 1551–1636). |

### Cách ngăn ghép metadata giữa hai partition

`select_partition` trả về `false` ngay khi tên descriptor khác tên đã chọn, nên toàn bộ khối merge nằm trong `if select_partition(...)`. Một descriptor khác partition không thể chạm vào `hash_algorithm`, `salt` hay `fec_roots` — kể cả khi các trường đó đang `None`.

Các phần **không** thay đổi:
- Công thức kiểm tra `num_bytes_following`/kích thước descriptor (giữ nguyên `round8`, `checked_add`, các thông báo lỗi).
- Xử lý vbmeta malformed/truncated (vẫn trả `Err`).
- Tag 0 (`AvbPropertyDescriptor`) và tag lạ (`unsupported_tags.push`).
- Fallback của `AvbInfo::empty()` (`"vendor"`, `"sha256"`, salt rỗng, `fec_roots = 0`).
- Ngữ nghĩa chuỗi rỗng: hash algorithm rỗng vẫn không được chọn; salt rỗng vẫn được ghi nhận là `Some("")` cho descriptor đầu tiên, đúng như code cũ.

### Diff follow-up thực tế

Diff chỉ chứa thay đổi của nhiệm vụ follow-up (tách khỏi thay đổi Giai đoạn 5 gốc), lưu tại `C:\Users\HT\.qoder\tmp\phase5\followup-only.diff` (7 hunk, 260 dòng):

```diff
--- a/volte-core/src/avb.rs (pre-follow-up)
+++ b/volte-core/src/avb.rs (follow-up)
@@ -185,7 +185,8 @@
 
 /// Fields the rebuild needs from the vbmeta descriptors. `None` means the
 /// image has no descriptor that carries the field, not that the field was
-/// guessed.
+/// guessed. The first descriptor that names a partition picks it; later
+/// descriptors of that same partition fill only the fields still `None`.
 #[derive(Debug, Default, PartialEq, Eq)]
 struct VbmetaMeta {
     partition_name: Option<String>,
@@ -225,6 +226,28 @@
     out
 }
 
+/// Selects the first descriptor's partition name and reports whether this
+/// descriptor belongs to the selected partition. A descriptor from any other
+/// partition may not contribute metadata.
+fn select_partition(meta: &mut VbmetaMeta, name: &str) -> bool {
+    if let Some(chosen) = meta.partition_name.as_deref() {
+        return chosen == name;
+    }
+    meta.partition_name = Some(name.to_owned());
+    true
+}
+
+/// First non-empty algorithm and first salt win. Only descriptors of the
+/// selected partition reach this helper.
+fn merge_hash_fields(meta: &mut VbmetaMeta, algorithm: String, salt: String) {
+    if meta.hash_algorithm.is_none() && !algorithm.is_empty() {
+        meta.hash_algorithm = Some(algorithm);
+    }
+    if meta.salt.is_none() {
+        meta.salt = Some(salt);
+    }
+}
+
 /// Parses `AvbVBMetaImageHeader` plus its descriptor block. Tags are read from
 /// the descriptor header, never searched for as text; tags this build does not
 /// understand are recorded in `unsupported_tags` rather than dropped silently.
@@ -338,18 +361,14 @@
     let name_at = HASHTREE_DESCRIPTOR_LEN;
     let salt_at = name_at + name_len;
     let salt = &body[salt_at..salt_at + salt_len];
+    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
 
-    if meta.partition_name.is_none() {
-        let name = std::str::from_utf8(&body[name_at..name_at + name_len])
-            .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
-        meta.partition_name = Some(name.to_owned());
-        meta.hash_algorithm = if algorithm.is_empty() {
-            None
-        } else {
-            Some(algorithm)
-        };
-        meta.salt = Some(hex_lower(salt));
-        meta.fec_roots = Some(fec_roots);
+    if select_partition(meta, name) {
+        merge_hash_fields(meta, algorithm, hex_lower(salt));
+        if meta.fec_roots.is_none() {
+            meta.fec_roots = Some(fec_roots);
+        }
     }
     Ok(())
 }
@@ -391,17 +410,11 @@
     let name_at = HASH_DESCRIPTOR_LEN;
     let salt_at = name_at + name_len;
     let salt = &body[salt_at..salt_at + salt_len];
+    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
 
-    if meta.partition_name.is_none() {
-        let name = std::str::from_utf8(&body[name_at..name_at + name_len])
-            .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
-        meta.partition_name = Some(name.to_owned());
-        meta.hash_algorithm = if algorithm.is_empty() {
-            None
-        } else {
-            Some(algorithm)
-        };
-        meta.salt = Some(hex_lower(salt));
+    if select_partition(meta, name) {
+        merge_hash_fields(meta, algorithm, hex_lower(salt));
     }
     Ok(())
 }
@@ -438,12 +451,10 @@
             "chain partition descriptor at {pos} declares {nbf} following bytes, expected {expected}"
         ));
     }
-    if meta.partition_name.is_none() {
-        let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
-        let name = std::str::from_utf8(&body[name_at..name_at + name_len])
-            .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
-        meta.partition_name = Some(name.to_owned());
-    }
+    let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
+    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
+    select_partition(meta, name);
     Ok(())
 }
 
@@ -1417,6 +1428,39 @@
         d
     }
 
+    /// `AvbHashtreeDescriptor` from the AOSP field order: 16-byte header, then
+    /// 164 fixed bytes (`fec_num_roots` at 52, `hash_algorithm` at 72), then
+    /// partition name, salt and root digest.
+    fn a2_hashtree_descriptor(
+        image_size: u64,
+        algorithm: &str,
+        name: &str,
+        salt: &[u8],
+        digest: &[u8],
+        fec_roots: u32,
+    ) -> Vec<u8> {
+        let mut d = vec![0u8; 180];
+        d[0..8].copy_from_slice(&1u64.to_be_bytes());
+        d[16..20].copy_from_slice(&1u32.to_be_bytes()); // dm_verity_version
+        d[20..28].copy_from_slice(&image_size.to_be_bytes());
+        d[44..48].copy_from_slice(&4096u32.to_be_bytes()); // data_block_size
+        d[48..52].copy_from_slice(&4096u32.to_be_bytes()); // hash_block_size
+        d[52..56].copy_from_slice(&fec_roots.to_be_bytes());
+        let alg = algorithm.as_bytes();
+        assert!(alg.len() < 32, "hash_algorithm must fit in char[32]");
+        d[72..72 + alg.len()].copy_from_slice(alg);
+        d[104..108].copy_from_slice(&(name.len() as u32).to_be_bytes());
+        d[108..112].copy_from_slice(&(salt.len() as u32).to_be_bytes());
+        d[112..116].copy_from_slice(&(digest.len() as u32).to_be_bytes());
+        d.extend_from_slice(name.as_bytes());
+        d.extend_from_slice(salt);
+        d.extend_from_slice(digest);
+        let nbf = a2_pad8(d.len() - 16);
+        d[8..16].copy_from_slice(&(nbf as u64).to_be_bytes());
+        d.resize(16 + nbf, 0);
+        d
+    }
+
     /// Descriptor with an arbitrary tag and payload, used to prove unknown
     /// tags are recorded instead of silently dropped.
     fn a2_unknown_descriptor(tag: u64, payload: &[u8]) -> Vec<u8> {
@@ -1502,4 +1546,113 @@
         assert_eq!(info.unsupported_descriptor_tags, vec![99]);
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    #[test]
+    fn a2_chain_before_hash_same_partition_merges_fields() {
+        let dir = tmp("volte_avb_a2_merge_chain_hash");
+        let vb = a2_vbmeta_with(&[
+            a2_chain_descriptor("vendor_dlkm", &[0x01, 0x02, 0x03], 1),
+            a2_hash_descriptor(
+                4096,
+                "sha512",
+                "vendor_dlkm",
+                &[0xaa, 0xbb, 0xcc, 0xdd],
+                &[],
+            ),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "vendor_dlkm");
+        assert_eq!(info.hash_algorithm, "sha512");
+        assert_eq!(info.salt, "aabbccdd");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_descriptors_for_different_partitions_do_not_mix_fields() {
+        let dir = tmp("volte_avb_a2_merge_mixed_partitions");
+        let vb = a2_vbmeta_with(&[
+            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
+            a2_hash_descriptor(4096, "sha1", "vendor_dlkm", &[0x11, 0x22, 0x33, 0x44], &[]),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "boot");
+        assert_eq!(
+            info.hash_algorithm, "sha256",
+            "hash must not come from the other partition"
+        );
+        assert_eq!(info.salt, "", "salt must not come from the other partition");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_hash_before_hashtree_same_partition_fills_fec_only() {
+        let dir = tmp("volte_avb_a2_merge_hash_tree");
+        let vb = a2_vbmeta_with(&[
+            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
+            a2_hashtree_descriptor(
+                4096,
+                "sha256",
+                "vendor",
+                &[0x11, 0x22, 0x33, 0x44],
+                &[0x55; 32],
+                2,
+            ),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "vendor");
+        assert_eq!(info.hash_algorithm, "sha512", "first algorithm wins");
+        assert_eq!(info.salt, "aabbccdd", "first salt wins");
+        assert_eq!(info.fec_roots, 2, "hashtree still fills the missing FEC");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_hashtree_before_hash_same_partition_keeps_first_values() {
+        let dir = tmp("volte_avb_a2_merge_tree_hash");
+        let vb = a2_vbmeta_with(&[
+            a2_hashtree_descriptor(
+                4096,
+                "sha256",
+                "vendor",
+                &[0x11, 0x22, 0x33, 0x44],
+                &[0x55; 32],
+                2,
+            ),
+            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "vendor");
+        assert_eq!(info.hash_algorithm, "sha256");
+        assert_eq!(info.salt, "11223344");
+        assert_eq!(info.fec_roots, 2);
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_hashtree_for_another_partition_does_not_fill_missing_fields() {
+        let dir = tmp("volte_avb_a2_merge_tree_other");
+        let vb = a2_vbmeta_with(&[
+            a2_hash_descriptor(4096, "sha512", "vendor", &[0xaa, 0xbb, 0xcc, 0xdd], &[]),
+            a2_hashtree_descriptor(
+                4096,
+                "sha256",
+                "vendor_dlkm",
+                &[0x11, 0x22, 0x33, 0x44],
+                &[0x55; 32],
+                4,
+            ),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "vendor");
+        assert_eq!(
+            info.fec_roots, 0,
+            "FEC from another partition must be ignored"
+        );
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

Ghi chú về cách tạo diff trên: để tách riêng thay đổi follow-up khỏi thay đổi Giai đoạn 5 gốc, baseline trước follow-up được dựng lại từ `git show HEAD:volte-core/src/avb.rs` + patch Giai đoạn 5 đã lưu, kiểm tra SHA-256 khớp giá trị đã ghi nhận trước follow-up (`c31cb5e3af3926ea0204c3614a040e951fe9ee79e59903ea7b346cb2a50834a2`), rồi diff với file hiện tại. Diff tích lũy so với HEAD nằm ở `C:\Users\HT\.qoder\tmp\phase5\followup-full-avb-diff.patch` (501 dòng, gồm cả thay đổi Giai đoạn 5).

### Fixture độc lập với code sản xuất

- `a2_pad8(n) = n.div_ceil(8) * 8` dùng `std`, **không** gọi `round8` của code đang kiểm thử.
- `a2_hashtree_descriptor` dựng 180 byte theo đúng thứ tự trường AOSP (`avb_hashtree_descriptor.h`): `fec_num_roots` tại `[52..56]`, `hash_algorithm[32]` tại `[72..104]`, ba length tại `[104..116]`, phần biến thiên nối sau. Test không dùng lại hằng số offset của production.
- Thứ tự descriptor trong `a2_vbmeta_with(&[...])` chính là thứ tự thực tế trong vbmeta (descriptor block nối tiếp sau header 256 byte).

---

## 8.3. REGRESSION TEST

### Danh sách test mới

| Test | Kịch bản | Assertion quan trọng |
|---|---|---|
| `a2_chain_before_hash_same_partition_merges_fields` (dòng 1551) | Tag 4 `vendor_dlkm` → Tag 2 `vendor_dlkm` (sha512, salt `aabbccdd`, digest rỗng) | `partition_name == "vendor_dlkm"`; `hash_algorithm == "sha512"`; `salt == "aabbccdd"` |
| `a2_descriptors_for_different_partitions_do_not_mix_fields` (dòng 1572) | Tag 4 `boot` → Tag 2 `vendor_dlkm` (sha1, salt `11223344`) | `partition_name == "boot"`; `hash_algorithm == "sha256"` (fallback); `salt == ""` |
| `a2_hash_before_hashtree_same_partition_fills_fec_only` (dòng 1590) | Tag 2 `vendor` (sha512, salt `aabbccdd`) → Tag 1 `vendor` (sha256, salt `11223344`, fec 2) | `hash_algorithm == "sha512"`; `salt == "aabbccdd"`; `fec_roots == 2` |
| `a2_hashtree_before_hash_same_partition_keeps_first_values` (dòng 1613) | Tag 1 `vendor` (sha256, salt `11223344`, fec 2) → Tag 2 `vendor` (sha512, salt `aabbccdd`) | `hash_algorithm == "sha256"`; `salt == "11223344"`; `fec_roots == 2` |
| `a2_hashtree_for_another_partition_does_not_fill_missing_fields` (dòng 1636) | Tag 2 `vendor` (sha512) → Tag 1 `vendor_dlkm` (fec 4) | `fec_roots == 0` — FEC partition khác bị bỏ qua dù đang `None` |

Ba test `a2_` cũ của Giai đoạn 5 giữ nguyên: `a2_hash_descriptor_sets_partition_name_and_algorithm`, `a2_chain_partition_descriptor_sets_partition_name_only`, `a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped`.

### Kết quả TRƯỚC khi sửa (code cũ, test mới)

Lệnh `cargo test -p volte-core a2_ -- --nocapture`, log `C:\Users\HT\.qoder\tmp\phase5\followup-red.log`, `EXIT=101`:

```text
running 8 tests

thread 'avb::tests::a2_chain_before_hash_same_partition_merges_fields' (14100) panicked at volte-core\src\avb.rs:1555:9:
assertion `left == right` failed
  left: "sha256"
 right: "sha512"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... FAILED
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_descriptors_for_different_partitions_do_not_mix_fields ... ok
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok

thread 'avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only' (1808) panicked at volte-core\src\avb.rs:1606:9:
assertion `left == right` failed: hashtree still fills the missing FEC
  left: 0
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... FAILED
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok

failures:

failures:
    avb::tests::a2_chain_before_hash_same_partition_merges_fields
    avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only

test result: FAILED. 6 passed; 2 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.03s

error: test failed, to rerun pass `-p volte-core --lib`
```

Diễn giải: đúng hai lỗi cần tái hiện, FAIL tại đúng assertion metadata (`hash_algorithm` và `fec_roots`), không có panic fixture, không có lỗi biên. Các test B/D/E và 3 test cũ PASS trên code cũ vì đó là test bảo vệ hành vi (guard), không phải test tái hiện lỗi.

### Kết quả SAU khi sửa

Lệnh `cargo test -p volte-core a2_ -- --nocapture`, log `C:\Users\HT\.qoder\tmp\phase5\followup-green-a2-final.log`, `EXIT=0`:

```text
running 8 tests
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_descriptors_for_different_partitions_do_not_mix_fields ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.05s
```

---

## 8.4. KẾT QUẢ KIỂM THỬ

Tất cả log dưới đây được tạo trong phiên làm việc này. `volte-core/src/avb.rs` có mtime `2026-11-08 14:08:51`; các log cuối được tạo lúc `14:09–14:13`, nên log phản ánh đúng nội dung file hiện tại (SHA-256 `6e70a307…`, xem 8.6). Không sửa file sau các lần chạy.

### 8.4.1. `cargo test -p volte-core a2_ -- --nocapture`

Log `followup-green-a2-final.log`, `EXIT=0`:

```text
   Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
    |
289 | pub struct Fat16Image {
    |            ---------- field in this struct
...
292 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.00s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 8 tests
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_descriptors_for_different_partitions_do_not_mix_fields ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.05s
```

### 8.4.2. `cargo test -p volte-core avb -- --nocapture`

Log `followup-regression-avb-final.log`, `EXIT=0`:

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 35 tests
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_chain_partition_descriptor_sets_partition_name_only ... ok
test avb::tests::a2_descriptors_for_different_partitions_do_not_mix_fields ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a_footer_that_does_not_reparse_is_not_erased ... ok
test avb::tests::a_failing_backend_over_an_old_footer_is_never_reported_as_success ... ok
test avb::tests::a_failing_backend_is_an_error_and_keeps_the_image_untouched ... ok
test avb::tests::cli_args_carry_the_frozen_parameters ... ok
test avb::tests::fit_partition_size_properties ... ok
test avb::tests::info_defaults_without_footer ... ok
test avb::tests::a_verified_rebuild_reports_the_new_footer ... ok
test avb::tests::info_reads_the_golden_vbmeta_fields ... ok
test avb::tests::missing_avbtool_names_the_exact_path ... ok
test avb::tests::parse_accepts_valid_footer ... ok
test avb::tests::parse_rejects_magic_size_and_offsets ... ok
test avb::tests::random_salt_is_64_lower_hex_chars_and_changes ... ok
test avb::tests::info_scans_back_past_padding ... ok
test avb::tests::a_backend_that_returns_ok_without_writing_a_footer_is_an_error ... ok
test avb::tests::a_truncated_or_malformed_vbmeta_is_an_error ... ok
test avb::tests::the_golden_fixture_matches_its_reference_digest ... ok
test avb::tests::tree_size_uses_32_byte_digests ... ok
test avb::tests::twenty_five_salts_are_all_distinct ... ok
test config::tests::sparse_and_avb_magics ... ok
test pipeline::tests::a_pre_avb_backup_is_verified_by_size_and_hash ... ok
test avb::tests::the_rebuild_request_carries_the_parsed_metadata ... ok
test avb::tests::rebuild_detects_footer_and_keeps_the_locked_size ... ok
test report::tests::avb_lines_fall_back_to_vendor_and_sha256 ... ok
test report::tests::avb_lines_snapshot_is_byte_for_byte ... ok
test avb::tests::rebuild_refuses_to_shrink_a_grown_image ... ok
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb ... ok
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree has been running for over 60 seconds
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree ... ok

test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 216 filtered out; finished in 93.35s
```

### 8.4.3. `cargo test --workspace`

Log `followup-workspace-final.log`, `EXIT=0`. Trích nguyên văn các dòng `Running`/`test result`:

```text
     Running unittests src\main.rs (target\debug\build\volte-cli/d68a8014c9353575\out\volte_cli-d68a8014c9353575.exe)
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)
test result: ok. 251 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 102.70s
   Doc-tests volte_core
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

251 = 246 test của Giai đoạn 5 + 5 test follow-up. Không dùng `--skip`. Toàn bộ output (gồm danh sách test) nằm trong log đã dẫn.

### 8.4.4. `cargo build --workspace`

Log `followup-build-final.log`, `EXIT=0`:

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
    |
289 | pub struct Fat16Image {
    |            ---------- field in this struct
...
292 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib) generated 1 warning
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.85s
```

### 8.4.5. `cargo clippy --workspace --all-targets`

Log `followup-clippy-final.log`, `EXIT=0`:

```text
    Checking volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
warning: field `spc` is never read
   --> volte-core\src\modem.rs:292:5
    |
289 | pub struct Fat16Image {
    |            ---------- field in this struct
...
292 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:390:31
    |
390 |             .trim_end_matches(|c: char| c == '\u{0}' || c == ' ')
    |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using an array of `char`: `['\u{0}', ' ']`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/main/index.html#manual_pattern_char_comparison
    = note: `#[warn(clippy::manual_pattern_char_comparison)]` on by default

warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:713:13
    |
713 |             (blob.len() + self.cluster_size as usize - 1) / self.cluster_size as usize
    |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using `.div_ceil()`: `blob.len().div_ceil(self.cluster_size as usize)`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/main/index.html#manual_div_ceil
    = note: `#[warn(clippy::manual_div_ceil)]` on by default

warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 suggestions)
    Checking volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
warning: `volte-core` (lib test) generated 3 warnings (3 duplicates)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.21s
```

3 warning đều có sẵn trong `modem.rs` (ngoài phạm vi). Không có warning từ `avb.rs`.

### 8.4.6. `git diff --check`

`EXIT=0`. Output (stderr) chỉ gồm cảnh báo autocrlf, không có lỗi whitespace:

```text
warning: in the working copy of 'README.md', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'profiles/mediatek/README.md', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'profiles/qualcomm/README.md', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-cli/src/main.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/avb.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/config.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/error.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/ext4_tools.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/flags.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/init_rc.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/lib.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/payload.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/pipeline.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/profile.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/props.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/sparse.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/verify.rs', LF will be replaced by CRLF the next time Git touches it
```

### 8.4.7. Kiểm tra định dạng (bổ sung)

`cargo fmt --all -- --check` → `EXIT=1`. Toàn bộ 27 diff còn lại thuộc các file **không sửa trong nhiệm vụ này**:

`volte-cli/src/main.rs:28`, `volte-cli/src/modem_cli.rs:297`, `volte-core/src/modem.rs` (nhiều vị trí), `volte-core/src/pipeline.rs:56`, `volte-core/src/profile.rs:132`, `volte-core/src/sparse.rs:165`, `volte-core/src/sparse.rs:835`.

`avb.rs` **không xuất hiện** trong danh sách (`grep -c "avb.rs"` trên log = 0) sau khi định dạng tay phần code mới. Không chạy `cargo fmt` toàn repo.

---

## 8.5. KIỂM TRA REGRESSION CŨ

Từ log 8.4.2 (`cargo test -p volte-core avb -- --nocapture`, 35/35 PASS):

- 3 test A2 Giai đoạn 5 gốc vẫn PASS:
  - `a2_hash_descriptor_sets_partition_name_and_algorithm` — ok
  - `a2_chain_partition_descriptor_sets_partition_name_only` — ok
  - `a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped` — ok
- `info_reads_the_golden_vbmeta_fields` — ok (golden vbmeta 704 byte, tag 0/1 đọc y nguyên).
- `avb_lines_snapshot_is_byte_for_byte` — ok (`report.rs` không bị sửa; SHA-256 xem 8.6).
- `info_defaults_without_footer` — ok (fallback `"vendor"`/`"sha256"`/salt rỗng/`fec_roots = 0` giữ nguyên).
- `a_truncated_or_malformed_vbmeta_is_an_error` — ok; `parse_rejects_magic_size_and_offsets` — ok. Không nới lỏng validation nào: các nhánh kiểm tra độ dài `num_bytes_following`, overflow `checked_add`, `dynamic > body.len()` và UTF-8 name vẫn nguyên.
- Không thay đổi `report.rs` (SHA-256 dưới đây trùng giá trị trước follow-up).
- Không xóa/disable/sửa test cũ.

Workspace: 30 (volte-cli) + 251 (volte-core) + 0 doc-test, tất cả PASS.

---

## 8.6. GIT VÀ CHECKSUM

### `git status --short` trước và sau

Lưu tại `C:\Users\HT\.qoder\tmp\phase5\followup-git-status-before.txt` và `followup-git-status-after.txt`. Hai file **giống hệt nhau** (`diff` exit 0). Nội dung trước khi sửa (sau khi sửa y nguyên):

```text
 M README.md
 D docs/PORT_STATUS.md
 D payload/hd_oppo_mtk_volte.rc
 D payload/hd_oppo_qc_volte.rc
 M profiles/mediatek/README.md
 M profiles/qualcomm/README.md
 D reports/AI2_INDEPENDENT_AUDIT.md
 D reports/fixes/PHASE_1_SOURCE_PROTECTION.md
 D reports/fixes/PHASE_2_5_DEBUGFS_PARITY.md
 D reports/fixes/PHASE_2_5_PARITY.md
 D reports/fixes/PHASE_2_EXT4_BACKEND.md
 D reports/fixes/PHASE_3_AVB_SPARSE_PIPELINE.md
 D reports/fixes/PHASE_4_DATA_INTEGRITY.md
 D reports/fixes/PHASE_5_HARDENING.md
 M volte-cli/src/main.rs
 M volte-core/src/avb.rs
 M volte-core/src/config.rs
 M volte-core/src/error.rs
 M volte-core/src/ext4_tools.rs
 M volte-core/src/flags.rs
 M volte-core/src/init_rc.rs
 M volte-core/src/lib.rs
 M volte-core/src/payload.rs
 M volte-core/src/pipeline.rs
 M volte-core/src/profile.rs
 M volte-core/src/props.rs
 M volte-core/src/sparse.rs
 M volte-core/src/verify.rs
?? VoLTE-Vendor-Builder.zip
?? payload/oppo_mtk_volte.rc
?? payload/oppo_qc_volte.rc
?? payload/volte_features.xml
?? reports/phase-1-a3-verify-fail.md
?? reports/phase-2-a1-sparse-total-sz.md
?? reports/phase-3-a7-a6-initrc-flags.md
?? reports/phase-4-a4-fat16-truncated.md
?? reports/phase-4b-walk-cycle-fix.md
?? reports/phase-5-a2-avb-descriptors.md
?? volte-cli/src/modem_cli.rs
?? volte-core/src/modem.rs
```

Ghi chú: `git status` ở trên được chụp trước khi tạo report này, nên chưa liệt kê `reports/phase-5-a2-descriptor-merge-followup.md`. Đây là file duy nhất được thêm sau đó, ngoài `avb.rs`.

Các thay đổi/xóa khác trong cây là từ các giai đoạn trước; không dọn dẹp, không reset, không stash, không commit, không push.

### Diff được phép

- Diff follow-up của `volte-core/src/avb.rs`: `C:\Users\HT\.qoder\tmp\phase5\followup-only.diff` (260 dòng, 7 hunk; nội dung đầy đủ ở 8.2).
- Diff tích lũy `avb.rs` so với HEAD (gồm Giai đoạn 5 + follow-up): `C:\Users\HT\.qoder\tmp\phase5\followup-full-avb-diff.patch` (501 dòng).
- `pipeline.rs`, `report.rs`: không có thay đổi mới.

### SHA-256

| File | SHA-256 | Ý nghĩa |
|---|---|---|
| `volte-core/src/avb.rs` (sau follow-up) | `6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7` | File nguồn duy nhất được sửa |
| `volte-core/src/avb.rs` (baseline trước follow-up) | `c31cb5e3af3926ea0204c3614a040e951fe9ee79e59903ea7b346cb2a50834a2` | Dùng để tách diff follow-up |
| `volte-core/src/pipeline.rs` | `0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1` | Không đổi trong follow-up |
| `volte-core/src/report.rs` | `c292c89cd88f27bbd872b5b9b8db973c37241d7d3750920eb177cbb3249e0361` | Không đổi |

### Report được tạo

- `reports/phase-5-a2-descriptor-merge-followup.md` (file này). Report Giai đoạn 5 `reports/phase-5-a2-avb-descriptors.md` giữ nguyên, không ghi đè.

---

## 8.7. PHÁT HIỆN NGOÀI PHẠM VI

1. **3 warning clippy cũ** trong `modem.rs`: `spc` không đọc (dòng 292), `manual_pattern_char_comparison` (dòng 390), `manual_div_ceil` (dòng 713). Có từ trước, không sửa.
2. **`cargo fmt --check` đỏ do file cũ** (27 diff) trong `main.rs`, `modem_cli.rs`, `modem.rs`, `pipeline.rs`, `profile.rs`, `sparse.rs`. Không sửa vì ngoài phạm vi; không chạy `cargo fmt` toàn repo.
3. **Warning `spc` của `volte-core` (lib)** lặp lại trong build và test; đây là dead-code có sẵn trong `modem.rs`.
4. **Cây làm việc còn nhiều thay đổi/xóa từ các giai đoạn trước** (README, docs, payload, profiles, nhiều file `.rs`, report cũ bị xóa, zip). Không xác định được phần nào thuộc nhiệm vụ hiện tại ngoài `avb.rs`; không đụng tới.
5. **Không kiểm chứng trên thiết bị thật.** Chỉ chạy test, build, clippy; chưa flash/chạy ảnh vendor trên máy.
6. **Ngữ nghĩa `salt` rỗng:** descriptor cùng partition đầu tiên vẫn ghi `Some("")` (giữ nguyên code cũ). Nếu sau này cần coi salt rỗng là "không có" để descriptor sau điền, phải sửa có chủ đích — hiện không thay đổi.
7. **Câu hỏi còn treo từ report Giai đoạn 5** (mục 10.2: "descriptor đầu tiên thắng" hay "first non-None từng trường") đã được trả lời bằng nhiệm vụ này: chọn first-non-None từng trường **trong cùng partition**.

---

Đã hoàn thành follow-up Giai đoạn 5 (A2), report tại reports/phase-5-a2-descriptor-merge-followup.md. Chờ xác nhận để tiếp tục.
