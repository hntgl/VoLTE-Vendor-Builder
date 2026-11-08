# PHASE 5 — A2 AVB SEMANTICS FIX

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Phạm vi: follow-up Giai đoạn 5 (A2) — chỉ sửa lỗi Tag 4 đã được audit chứng minh.
- Report audit tương ứng: `reports/phase-5-a2-avb-semantics-audit.md`.
- Không commit, không push, không reset/checkout/clean/stash.
- Không sửa `volte-core/src/pipeline.rs`, `volte-core/src/report.rs`, `sparse.rs`, `modem.rs`, `init_rc.rs`, payload RC, `Cargo.toml`, `Cargo.lock`.

**STATUS: DONE**

---

## 1. LỖI ĐÃ XÁC NHẬN

### 1.1. Tag 4 (`AvbChainPartitionDescriptor`) chiếm quyền chọn tên image partition

`parse_chain_partition_descriptor()` gọi `select_partition(meta, name)`. Vì helper này chọn descriptor đầu tiên có tên partition, một Tag 4 đứng trước Tag 1/2 sẽ đặt `meta.partition_name` bằng **partition được chain tới** (ví dụ `boot`), không phải image đang rebuild.

Hệ quả thực tế:

```text
Tag 4: partition_name = "boot"
Tag 1: partition_name = "vendor", hash_algorithm = "sha512", salt = ...

=> AvbInfo.partition_name = "boot"
=> rebuild_hashtree gọi:
   avbtool.py add_hashtree_footer --partition_name boot ...
   cho vendor.img
=> salt/algorithm/FEC roots của vendor bị bỏ qua vì khối metadata bị chặn.
```

Đây là **BUG** logic, không phải chỉ là giới hạn đọc metadata.

### 1.2. Bằng chứng AOSP (nguyên văn, chép từ audit)

Repo: `platform/external/avb`, commit `761178607206f4cb2af79ed9eec52d8cbd814adb`.

`libavb/avb_chain_partition_descriptor.h` dòng 44-51:

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
 */
```

`avbtool.py`, `AvbChainPartitionDescriptor.verify()` dòng 1905-1930:

```python
value = expected_chain_partitions_map.get(self.partition_name)
if not value:
  sys.stderr.write('No expected chain partition for partition {}. Use '
                   '--expected_chain_partition to specify expected '
                   'contents or --follow_chain_partitions.\n'.
                   format(self.partition_name))
  return False
```

`expected_chain_partitions_map` được xây từ `--expected_chain_partition <partition>:<rollback_location>:<key>` (dòng 2533-2544). Vậy `partition_name` của Tag 4 là **partition được chain tới**, không phải image chứa descriptor.

Trong khi đó, `add_hashtree_footer` (dòng 3795-3846) đặt `ht_desc.partition_name = partition_name`, tức là tên image đang được đóng gói — nguồn đúng cho rebuild. `add_hash_footer` (dòng 3529-3551) tương tự.

---

## 2. FILE ĐƯỢC SỬA

| File | Thay đổi |
|---|---|
| `volte-core/src/avb.rs` | Đổi tên helper `select_partition` → `select_image_partition`; chỉ Tag 1/2 gọi helper; Tag 4 không chọn partition; cập nhật doc; thêm/đổi regression test. |
| `volte-core/src/pipeline.rs` | Không sửa. Không cần cảnh báo/fail-safe mới: sau khi sửa, `info.partition_name` lấy từ Tag 1/2 đúng như pipeline mong đợi. |
| `volte-core/src/report.rs` | Không sửa. |

Vị trí sau sửa:

- `select_image_partition`: `volte-core/src/avb.rs:235`
- `parse_chain_partition_descriptor`: `volte-core/src/avb.rs:428`
- Test mới/đổi tên: `volte-core/src/avb.rs:1531`, `:1578`, `:1604`, `:1625`

Không còn lời gọi `select_partition` trong file (grep trả về exit 1).

---

## 3. DIFF THỰC TẾ

Diff dưới đây được tạo bằng `git diff --no-index` giữa bản tái tạo pre-fix và file hiện tại. Bản tái tạo khớp **chính xác** SHA-256 trước sửa của file (`6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7`), nên đây là toàn bộ thay đổi của nhiệm vụ này, không lẫn các thay đổi follow-up chưa commit trước đó.

```diff
diff --git a/volte-core/src/avb.rs b/volte-core/src/avb.rs
index 332fe43..33c8828 100644
--- a/volte-core/src/avb.rs
+++ b/volte-core/src/avb.rs
@@ -185,8 +185,10 @@ const CHAIN_PARTITION_DESCRIPTOR_LEN: usize = 92;
 
 /// Fields the rebuild needs from the vbmeta descriptors. `None` means the
 /// image has no descriptor that carries the field, not that the field was
-/// guessed. The first descriptor that names a partition picks it; later
-/// descriptors of that same partition fill only the fields still `None`.
+/// guessed. Only hashtree/hash descriptors (tags 1/2) name the image being
+/// rebuilt: the first one picks its partition and later descriptors of that
+/// same partition fill only the fields still `None`. Chain descriptors (tag 4)
+/// reference another partition and never select the image partition.
 #[derive(Debug, Default, PartialEq, Eq)]
 struct VbmetaMeta {
     partition_name: Option<String>,
@@ -226,10 +228,11 @@ fn hex_lower(bytes: &[u8]) -> String {
     out
 }
 
-/// Selects the first descriptor's partition name and reports whether this
-/// descriptor belongs to the selected partition. A descriptor from any other
-/// partition may not contribute metadata.
-fn select_partition(meta: &mut VbmetaMeta, name: &str) -> bool {
+/// Selects the first image descriptor's partition name and reports whether
+/// this descriptor belongs to the selected partition. Only tags 1/2 may call
+/// this: their `partition_name` is the image they describe. A descriptor from
+/// any other partition may not contribute metadata.
+fn select_image_partition(meta: &mut VbmetaMeta, name: &str) -> bool {
     if let Some(chosen) = meta.partition_name.as_deref() {
         return chosen == name;
     }
@@ -364,7 +367,7 @@ fn parse_hashtree(
     let name = std::str::from_utf8(&body[name_at..name_at + name_len])
         .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
 
-    if select_partition(meta, name) {
+    if select_image_partition(meta, name) {
         merge_hash_fields(meta, algorithm, hex_lower(salt));
         if meta.fec_roots.is_none() {
             meta.fec_roots = Some(fec_roots);
@@ -413,19 +416,19 @@ fn parse_hash_descriptor(
     let name = std::str::from_utf8(&body[name_at..name_at + name_len])
         .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
 
-    if select_partition(meta, name) {
+    if select_image_partition(meta, name) {
         merge_hash_fields(meta, algorithm, hex_lower(salt));
     }
     Ok(())
 }
 
 /// Tag 4, `AvbChainPartitionDescriptor`: names another partition signed by a
-/// public key. It carries no hash algorithm, salt or FEC roots, so none of
-/// those are derived from it.
+/// public key. It carries no hash algorithm, salt or FEC roots, and its
+/// `partition_name` is not the image being rebuilt, so it selects nothing.
 fn parse_chain_partition_descriptor(
     body: &[u8],
     pos: usize,
-    meta: &mut VbmetaMeta,
+    _meta: &mut VbmetaMeta,
 ) -> std::result::Result<(), String> {
     if body.len() < CHAIN_PARTITION_DESCRIPTOR_LEN {
         return Err(format!(
@@ -452,9 +455,8 @@ fn parse_chain_partition_descriptor(
         ));
     }
     let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
-    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
+    let _name = std::str::from_utf8(&body[name_at..name_at + name_len])
         .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
-    select_partition(meta, name);
     Ok(())
 }
 
@@ -1526,14 +1528,18 @@ mod tests {
     }
 
     #[test]
-    fn a2_chain_partition_descriptor_sets_partition_name_only() {
+    fn a2_chain_only_does_not_override_image_partition_fallback() {
         let dir = tmp("volte_avb_a2_chain");
         let vb = a2_vbmeta_with(&[a2_chain_descriptor("boot", &[0x11, 0x22, 0x33, 0x44], 1)]);
         let p = a2_image_with_vbmeta(&dir, &vb);
         let info = avb_info_fast(&p).unwrap();
-        assert_eq!(info.partition_name, "boot");
+        assert_eq!(
+            info.partition_name, "vendor",
+            "a chain reference names another partition, not the image being rebuilt"
+        );
         assert_eq!(info.hash_algorithm, "sha256");
         assert_eq!(info.salt, "");
+        assert_eq!(info.fec_roots, 0);
         let _ = std::fs::remove_dir_all(&dir);
     }
 
@@ -1569,7 +1575,33 @@ mod tests {
     }
 
     #[test]
-    fn a2_descriptors_for_different_partitions_do_not_mix_fields() {
+    fn a2_chain_before_hashtree_does_not_select_image_partition() {
+        let dir = tmp("volte_avb_a2_chain_before_tree");
+        let vb = a2_vbmeta_with(&[
+            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
+            a2_hashtree_descriptor(
+                4096,
+                "sha512",
+                "vendor",
+                &[0xaa, 0xbb, 0xcc, 0xdd],
+                &[0x55; 32],
+                2,
+            ),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(
+            info.partition_name, "vendor",
+            "the hashtree descriptor names the image partition"
+        );
+        assert_eq!(info.hash_algorithm, "sha512");
+        assert_eq!(info.salt, "aabbccdd");
+        assert_eq!(info.fec_roots, 2);
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_chain_reference_does_not_mix_hash_metadata() {
         let dir = tmp("volte_avb_a2_merge_mixed_partitions");
         let vb = a2_vbmeta_with(&[
             a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
@@ -1577,12 +1609,39 @@ mod tests {
         ]);
         let p = a2_image_with_vbmeta(&dir, &vb);
         let info = avb_info_fast(&p).unwrap();
-        assert_eq!(info.partition_name, "boot");
         assert_eq!(
-            info.hash_algorithm, "sha256",
-            "hash must not come from the other partition"
+            info.partition_name, "vendor_dlkm",
+            "the chain reference must not take over the image partition name"
         );
-        assert_eq!(info.salt, "", "salt must not come from the other partition");
+        assert_eq!(
+            info.hash_algorithm, "sha1",
+            "hash must come from the descriptor that names the image partition"
+        );
+        assert_eq!(info.salt, "11223344");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a2_multiple_chain_descriptors_are_handled_correctly() {
+        let dir = tmp("volte_avb_a2_chain_multiple");
+        let vb = a2_vbmeta_with(&[
+            a2_chain_descriptor("boot", &[0x01, 0x02, 0x03], 1),
+            a2_chain_descriptor("system", &[0x04, 0x05, 0x06], 2),
+            a2_hashtree_descriptor(
+                4096,
+                "sha256",
+                "vendor",
+                &[0xde, 0xad, 0xbe, 0xef],
+                &[0x77; 32],
+                1,
+            ),
+        ]);
+        let p = a2_image_with_vbmeta(&dir, &vb);
+        let info = avb_info_fast(&p).unwrap();
+        assert_eq!(info.partition_name, "vendor");
+        assert_eq!(info.hash_algorithm, "sha256");
+        assert_eq!(info.salt, "deadbeef");
+        assert_eq!(info.fec_roots, 1);
         let _ = std::fs::remove_dir_all(&dir);
     }
 
```

---

## 4. TEST-FIRST — ĐỎ TRƯỚC KHI SỬA

Lệnh: `cargo test -p volte-core a2_ -- --nocapture` trên code **chưa sửa**. Output nguyên văn (phần test; `tail -60`):

```text
running 10 tests

thread 'avb::tests::a2_chain_before_hashtree_does_not_select_image_partition' (4888) panicked at volte-core\src\avb.rs:1591:9:
assertion `left == right` failed: the hashtree descriptor names the image partition
  left: "boot"
 right: "vendor"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test avb::tests::a2_chain_before_hashtree_does_not_select_image_partition ... FAILED
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok

thread 'avb::tests::a2_chain_only_does_not_override_image_partition_fallback' (13600) panicked at volte-core\src\avb.rs:1534:9:
assertion `left == right` failed: a chain reference names another partition, not the image being rebuilt
  left: "boot"
 right: "vendor"
test avb::tests::a2_chain_only_does_not_override_image_partition_fallback ... FAILED
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok

thread 'avb::tests::a2_chain_reference_does_not_mix_hash_metadata' (8676) panicked at volte-core\src\avb.rs:1610:9:
assertion `left == right` failed: the chain reference must not take over the image partition name
  left: "boot"
 right: "vendor_dlkm"
test avb::tests::a2_chain_reference_does_not_mix_hash_metadata ... FAILED
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok

thread 'avb::tests::a2_multiple_chain_descriptors_are_handled_correctly' (14216) panicked at volte-core\src\avb.rs:1639:9:
assertion `left == right` failed
  left: "boot"
 right: "vendor"
test avb::tests::a2_multiple_chain_descriptors_are_handled_correctly ... FAILED
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok

failures:

failures:
    avb::tests::a2_chain_before_hashtree_does_not_select_image_partition
    avb::tests::a2_chain_only_does_not_override_image_partition_fallback
    avb::tests::a2_chain_reference_does_not_mix_hash_metadata
    avb::tests::a2_multiple_chain_descriptors_are_handled_correctly

test result: FAILED. 6 passed; 4 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.04s
```

`EXIT=101`. Cả 4 test đỏ đúng tại assertion tên partition, đều cho `left: "boot"` — đúng cơ chế Tag 4 chiếm tên.

---

## 5. XANH SAU KHI SỬA

### 5.1. `cargo test -p volte-core a2_ -- --nocapture`

Output nguyên văn (phần test; `tail -30`), `EXIT=0`:

```text
running 10 tests
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_chain_only_does_not_override_image_partition_fallback ... ok
test avb::tests::a2_chain_before_hashtree_does_not_select_image_partition ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok
test avb::tests::a2_chain_reference_does_not_mix_hash_metadata ... ok
test avb::tests::a2_multiple_chain_descriptors_are_handled_correctly ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 243 filtered out; finished in 0.08s
```

### 5.2. `cargo test -p volte-core avb -- --nocapture`

Output nguyên văn (phần test; `tail -45`), `EXIT=0`:

```text
running 37 tests
test avb::tests::a2_chain_only_does_not_override_image_partition_fallback ... ok
test avb::tests::a2_chain_before_hashtree_does_not_select_image_partition ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_chain_reference_does_not_mix_hash_metadata ... ok
test avb::tests::a2_multiple_chain_descriptors_are_handled_correctly ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok
test avb::tests::a_footer_that_does_not_reparse_is_not_erased ... ok
test avb::tests::a_failing_backend_over_an_old_footer_is_never_reported_as_success ... ok
test avb::tests::a_failing_backend_is_an_error_and_keeps_the_image_untouched ... ok
test avb::tests::cli_args_carry_the_frozen_parameters ... ok
test avb::tests::fit_partition_size_properties ... ok
test avb::tests::info_defaults_without_footer ... ok
test avb::tests::a_verified_rebuild_reports_the_new_footer ... ok
test avb::tests::info_reads_the_golden_vbmeta_fields ... ok
test avb::tests::info_scans_back_past_padding ... ok
test avb::tests::parse_accepts_valid_footer ... ok
test avb::tests::missing_avbtool_names_the_exact_path ... ok
test avb::tests::parse_rejects_magic_size_and_offsets ... ok
test avb::tests::random_salt_is_64_lower_hex_chars_and_changes ... ok
test avb::tests::a_backend_that_returns_ok_without_writing_a_footer_is_an_error ... ok
test avb::tests::the_golden_fixture_matches_its_reference_digest ... ok
test avb::tests::a_truncated_or_malformed_vbmeta_is_an_error ... ok
test avb::tests::tree_size_uses_32_byte_digests ... ok
test avb::tests::twenty_five_salts_are_all_distinct ... ok
test config::tests::sparse_and_avb_magics ... ok
test pipeline::tests::a_pre_avb_backup_is_verified_by_size_and_hash ... ok
test avb::tests::the_rebuild_request_carries_the_parsed_metadata ... ok
test avb::tests::rebuild_refuses_to_shrink_a_grown_image ... ok
test report::tests::avb_lines_fall_back_to_vendor_and_sha256 ... ok
test report::tests::avb_lines_snapshot_is_byte_for_byte ... ok
test avb::tests::rebuild_detects_footer_and_keeps_the_locked_size ... ok
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb ... ok
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree ... ok

test result: ok. 37 passed; 0 failed; 0 ignored; 0 measured; 216 filtered out; finished in 87.14s
```

### 5.3. `cargo test --workspace`

Output nguyên văn (phần kết quả; `tail -20`), `EXIT=0`:

```text
test verify::tests::a_host_file_that_cannot_be_replaced_is_stale ... ok
test verify::tests::a_missing_copy_is_not_ignored ... ok
test verify::tests::a_stale_host_file_is_deleted_before_the_dump_and_never_hashed ... ok
test verify::tests::both_canonical_copies_must_match_the_payload ... ok
test verify::tests::the_mtk_rc_name_is_verified_in_both_init_directories ... ok
test verify::tests::without_a_source_payload_nothing_is_verified ... ok
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree has been running for over 60 seconds
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted has been running for over 60 seconds
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image ... ok
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree ... ok
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted ... ok

test result: ok. 253 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 120.42s

   Doc-tests volte_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

253 = 243 test cũ + 10 test `a2_` (8 cũ, 2 mới; 2 test cũ được đổi tên/cập nhật assertion — xem mục 7). Không dùng `--skip`.

### 5.4. `cargo build --workspace`

Output nguyên văn (`tail -12`), `EXIT=0`:

```text
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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.84s
```

Warning `spc` có sẵn trong `modem.rs`, ngoài phạm vi.

### 5.5. `cargo clippy --workspace --all-targets`

Output nguyên văn (`tail -25`), `EXIT=0`:

```text
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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.24s
```

Không có warning nào từ `avb.rs`. 3 warning còn lại đều ở `modem.rs`, có trước nhiệm vụ này.

### 5.6. `git diff --check`

`EXIT=0`. Output nguyên văn chỉ gồm cảnh báo autocrlf, không có lỗi whitespace:

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

### 5.7. `cargo fmt --all -- --check`

`EXIT=1`. Danh sách file còn lỗi format (output nguyên văn, đã `sort -u`):

```text
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli\src\main.rs:28:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli\src\modem_cli.rs:297:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1200:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1234:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1255:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1438:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1448:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1481:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1509:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1520:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1538:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1611:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1620:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1659:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1669:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1676:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:1706:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:53:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:656:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:901:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:909:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:978:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\modem.rs:992:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\pipeline.rs:56:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\profile.rs:132:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\sparse.rs:165:
Diff in \\?\E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core\src\sparse.rs:835:
```

**`volte-core/src/avb.rs` không có trong danh sách.** Toàn bộ file lỗi format là file có sẵn từ các giai đoạn trước, ngoài phạm vi; không chạy `cargo fmt` toàn repo.

---

## 6. ĐỐI CHIẾU SPEC VÀ PHẠM VI THAY ĐỔI

Quy tắc đã áp dụng, đúng như audit đề xuất:

1. Chỉ Tag 1 (`AvbHashtreeDescriptor`) và Tag 2 (`AvbHashDescriptor`) được chọn `meta.partition_name`, vì `add_hashtree_footer`/`add_hash_footer` gán chính `--partition_name` của image đang đóng gói vào descriptor.
2. Tag 4 không chọn partition, không ghi đè, không cung cấp fallback. Nếu chỉ có Tag 4, giữ fallback hiện có của tool (`"vendor"`, `"sha256"`, salt rỗng, FEC 0).
3. Tag 4 vẫn được parse đầy đủ về biên (độ dài, alignment, UTF-8 của tên) để descriptor hỏng vẫn là lỗi; không thêm trường chain vào `AvbInfo` vì rebuild hiện tại không dùng.
4. Không đổi thuật toán rebuild, không đổi `HashtreeRequest`, không đổi `cli_args`, không đổi FEC policy, không đổi `report.rs`.

---

## 7. TEST CŨ ĐƯỢC GIỮ / ĐỔI TÊN

Không xóa, không disable, không `--skip` test nào.

| Test trước | Trạng thái sau | Lý do |
|---|---|---|
| `a2_hash_descriptor_sets_partition_name_and_algorithm` | Giữ nguyên | Vẫn đúng: Tag 2 là image descriptor. |
| `a2_chain_partition_descriptor_sets_partition_name_only` | **Đổi tên** thành `a2_chain_only_does_not_override_image_partition_fallback`, cập nhật assertion | Test cũ khóa hành vi sai đã được AOSP chứng minh (`partition_name = "boot"` từ Tag 4). Không thể giữ assertion cũ mà vẫn sửa lỗi. |
| `a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped` | Giữ nguyên | Không liên quan. |
| `a2_chain_before_hash_same_partition_merges_fields` | Giữ nguyên | Vẫn PASS: Hash Descriptor tự chọn `vendor_dlkm` và cung cấp `sha512`/salt. Lưu ý: sau sửa, test này không còn chứng minh "chain merge" — nó chứng minh Tag 2 cung cấp metadata. |
| `a2_descriptors_for_different_partitions_do_not_mix_fields` | **Đổi tên** thành `a2_chain_reference_does_not_mix_hash_metadata`, cập nhật assertion | Test cũ khóa hành vi sai (`"boot"`). Test mới kiểm tra đúng: tên và metadata lấy từ Tag 2. |
| `a2_hash_before_hashtree_same_partition_fills_fec_only` | Giữ nguyên | PASS. |
| `a2_hashtree_before_hash_same_partition_keeps_first_values` | Giữ nguyên | PASS. |
| `a2_hashtree_for_another_partition_does_not_fill_missing_fields` | Giữ nguyên | PASS. |
| — | **Mới:** `a2_chain_before_hashtree_does_not_select_image_partition` | Tag 4 trước Tag 1 cùng vbmeta. |
| — | **Mới:** `a2_multiple_chain_descriptors_are_handled_correctly` | Nhiều Tag 4 trước Tag 1. |

Các test trọng yếu khác vẫn PASS: `info_reads_the_golden_vbmeta_fields`, `info_defaults_without_footer`, `a_truncated_or_malformed_vbmeta_is_an_error`, `avb_lines_snapshot_is_byte_for_byte`.

Việc đổi tên/cập nhật assertion hai test trên là cần thiết vì assertion cũ mã hóa đúng hành vi lỗi mà audit đã chứng minh; nội dung kiểm tra không bị nới lỏng — test mới còn kiểm tra thêm `fec_roots` và metadata của partition đúng.

---

## 8. GIỚI HẠN CÒN LẠI (KHÔNG SỬA TRONG NHIỆM VỤ NÀY)

Theo audit:

1. **Tag 2 (Hash Descriptor) khi rebuild — LIMITATION.** Tool luôn gọi `add_hashtree_footer`; nếu đầu vào là Hash Descriptor, footer ra sẽ là Hashtree Descriptor. Đây vẫn là giới hạn ngoài phạm vi vendor.img; không thêm khả năng rebuild Hash Descriptor khi chưa có bằng chứng và test avbtool tương ứng.
2. **FEC — LIMITATION.** `HashtreeRequest::cli_args()` luôn có `--do_not_generate_fec`, nên footer mới không có FEC. `fec_roots` chỉ được đọc/hiển thị; việc đọc được giá trị này không có nghĩa FEC được bảo toàn. Môi trường không có binary `fec` để tạo FEC.
3. **Chain descriptor bị mất khi rebuild — DATA LOSS/LIMITATION.** Rebuild không truyền `--chain_partition`; Tag 4 trong footer cũ không được tái tạo. Cần nhiệm vụ riêng nếu muốn bảo toàn chain.
4. **Full-slot image + footer zeroed không truncate — ngoài phạm vi.** `avbtool` từ chối ảnh full-slot nếu chưa cắt về `original_image_size`; pipeline hiện rơi vào nhánh bỏ AVB. Đã ghi trong audit, không sửa ở đây.
5. **Chuỗi log pipeline `[STATUS] Generating hashtree and FEC for the image...`** vẫn đề cập FEC dù FEC bị tắt có chủ đích. Không sửa vì nằm ngoài phạm vi tối thiểu và có test pipeline phụ thuộc chuỗi này.

---

## 9. GIT STATUS VÀ CHECKSUM

### 9.1. Trước khi sửa (đầu phiên)

```text
6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7 *volte-core/src/avb.rs
0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1 *volte-core/src/pipeline.rs
c292c89cd88f27bbd872b5b9b8db973c37241d7d3750920eb177cbb3249e0361 *volte-core/src/report.rs
```

`git status --short` đầu phiên: `volte-core/src/avb.rs`, `volte-core/src/pipeline.rs` và nhiều file khác đã ở trạng thái `M` từ các giai đoạn trước; có các file `D`/`??` có sẵn. Không có thay đổi nào bị reset/khôi phục/xóa bởi nhiệm vụ này.

### 9.2. Sau khi sửa

```text
2fe954bec97e7ffce33e3c7ad2eaee34c2fd304c3afb5fceb8e8d8db31633fd4 *volte-core/src/avb.rs
0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1 *volte-core/src/pipeline.rs
c292c89cd88f27bbd872b5b9b8db973c37241d7d3750920eb177cbb3249e0361 *volte-core/src/report.rs
```

`pipeline.rs` và `report.rs` không đổi. `avb.rs` chỉ đổi đúng các dòng trong diff mục 3.

`git status --short` sau khi sửa (trích):

```text
 M volte-core/src/avb.rs
 M volte-core/src/pipeline.rs
?? reports/phase-5-a2-avb-semantics-audit.md
?? reports/phase-5-a2-avb-semantics-fix.md
```

Ngoài ra còn các thay đổi/xóa/untracked có sẵn từ trước (README, docs, payload, profiles, các `src/*.rs` khác, report cũ, `tool/python/**/__pycache__/` do test gọi Python sinh ra). Không commit, không push, không dọn dẹp.

### 9.3. Bản tái tạo pre-fix

File tạm `C:/Users/HT/.qoder/tmp/phase5-audit/avb_prefix.rs` (dựng bằng cách đảo đúng các sửa đổi của nhiệm vụ này) có SHA-256:

```text
6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7
```

Khớp checksum `avb.rs` trước sửa, xác nhận diff mục 3 không bỏ sót/thêm thay đổi nào khác.

---

## 10. KẾT LUẬN

- Đã tái hiện đúng lỗi Tag 4 chiếm tên image partition bằng 4 test đỏ.
- Đã sửa tối thiểu trong `avb.rs`: chỉ Tag 1/2 được chọn partition; Tag 4 chỉ được parse/validate.
- 10/10 test `a2_`, 37/37 test filter `avb`, 253/253 test workspace PASS.
- Build và clippy PASS; clippy không có warning mới từ `avb.rs`.
- `cargo fmt --check` chỉ đỏ ở các file cũ ngoài phạm vi; `avb.rs` sạch.
- Không có thay đổi nào ngoài `avb.rs` và hai report mới của nhiệm vụ.

Đã hoàn thành AVB semantics follow-up Giai đoạn 5, report tại reports/phase-5-a2-avb-semantics-audit.md và reports/phase-5-a2-avb-semantics-fix.md. Chờ xác nhận để tiếp tục.
