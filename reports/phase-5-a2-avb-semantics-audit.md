# BÁO CÁO AUDIT — GIAI ĐOẠN 5 (A2) FOLLOW-UP: NGỮ NGHĨA AVB VÀ TÍNH TOÀN VẸN KHI REBUILD

- Ngày: 2026-11-08
- Nhánh: `ai2/fix-audit-findings`
- Vai trò: AI2 — Local Executor
- Loại nhiệm vụ: audit trước, chỉ sửa source khi có bằng chứng (Phần III của brief)
- Phạm vi: Giai đoạn 5 (A2), không mở sang A8/A5/A10/A11/nhóm B.

---

## 1. MỤC TIÊU

Xác định tính đúng đắn theo AOSP và theo kiến trúc thực tế của tool cho ba vấn đề:

- **A.** Phân biệt partition được mô tả bởi Tag 1/2 và partition được tham chiếu bởi Tag 4.
- **B.** Rebuild có làm mất thông tin AVB gốc hay không, đặc biệt với Chain Partition Descriptor và Hash Descriptor.
- **C.** Metadata FEC được parse có thực sự được dùng khi rebuild hay chỉ được đọc rồi bỏ qua.

Kết luận cuối cùng của audit nằm ở mục 7; đề xuất sửa tối thiểu ở mục 8.

---

## 2. WORKING TREE VÀ CHECKSUM TRƯỚC KHI KHẢO SÁT

### 2.1. Lệnh đã chạy

```text
$ git branch --show-current
ai2/fix-audit-findings

$ sha256sum volte-core/src/avb.rs volte-core/src/pipeline.rs volte-core/src/report.rs
6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7 *volte-core/src/avb.rs
0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1 *volte-core/src/pipeline.rs
c292c89cd88f27bbd872b5b9b8db973c37241d7d3750920eb177cbb3249e0361 *volte-core/src/report.rs
```

Cả ba checksum khớp report đã cung cấp:

| File | Checksum thực tế | Khớp report |
|---|---|---|
| `avb.rs` sau follow-up | `6e70a30715eca12e0e904fbce9ba5ce47c72d5b5d5d59d0601b503f7bd1840e7` | Có |
| `pipeline.rs` | `0f9cb07c8329628f013877a7d59a73070645b8ee01fab5134379469c1989e5f1` | Có |
| `report.rs` | `c292c89cd88f27bbd872b5b9b8db973c37241d7d3750920eb177cbb3249e0361` | Không có trong report, ghi nhận mới |

Kết luận: `avb.rs` **thực sự** đã có follow-up merge (`select_partition`, `merge_hash_fields`, 5 test `a2_`); working tree khớp report. Không có khác biệt bất ngờ trong hai file AVB.

### 2.2. `git status --short` (trước khảo sát)

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
?? reports/phase-5-a2-descriptor-merge-followup.md
?? volte-cli/src/modem_cli.rs
?? volte-core/src/modem.rs
```

Các thay đổi ngoài `avb.rs`/`pipeline.rs` thuộc các giai đoạn trước; không đụng tới, không reset/checkout/clean/stash.

### 2.3. Nguồn AOSP dùng để đối chiếu

Repo: `platform/external/avb`, commit `761178607206f4cb2af79ed9eec52d8cbd814adb` (`refs/heads/main`, lấy ngày 2026-11-08 bằng `git ls-remote`).

| File | URL | SHA-256 tải về |
|---|---|---|
| `libavb/avb_chain_partition_descriptor.h` | `https://android.googlesource.com/platform/external/avb/+/761178607206f4cb2af79ed9eec52d8cbd814adb/libavb/avb_chain_partition_descriptor.h` | `50d29e5c5ec59b50467cd8e3b372c92c3aff26c1f6fd0ea08e8c0176ae73bcf9` |
| `libavb/avb_hashtree_descriptor.h` | `.../libavb/avb_hashtree_descriptor.h` | `9c99a743473108bc12fe7ed1bcc413f138bdeb77ab8d42a7885a813c16f39f2c` |
| `libavb/avb_hash_descriptor.h` | `.../libavb/avb_hash_descriptor.h` | `61811399ac8e4ca0392a87039de9edfcaab72d165a346337e87b5fb5c3a4d602` |
| `libavb/avb_descriptor.h` | `.../libavb/avb_descriptor.h` | `4ba2c2686c1fe657a51b7d3b683866885b8b038846256357b527e2c327f26797` |
| `libavb/avb_vbmeta_image.h` | `.../libavb/avb_vbmeta_image.h` | `0979deac8902cbcbd0ec1d2f94af6692ab954fe698b1bd2e8b193b7eec3a3607` |
| `avbtool.py` | `https://android.googlesource.com/platform/external/avb/+/761178607206f4cb2af79ed9eec52d8cbd814adb/avbtool.py` | `e5a664a38db623da00f080219bc0ee60a640a9dc4a872803616fae4938ac749b` |

Tool trong repo dùng bản bundled `tool/avbtool.py` (`sha256 393e9317d519d7e5700c26d96c7cb60b07420d55be23f82e25b91726735be615`, release string `avbtool 1.3.0`). Các đoạn logic liên quan dưới đây được trích từ bản bundled (bản thực thi) và đối chiếu với AOSP main; hai bản giống nhau ở các đoạn này.

---

## 3. ĐƯỜNG ĐI THỰC TẾ CỦA AVB TRONG TOOL

### 3.1. `pipeline.rs`

| Bước | Vị trí | Hành vi |
|---|---|---|
| Copy source → output | `pipeline.rs:432-449` | Làm việc trên bản copy `out_path` (`work`), không sửa file gốc. |
| Đọc AVB | `pipeline.rs:451-452` | `avb::avb_info_fast(&work)` — đọc footer **của chính vendor image đang build**. |
| Cảnh báo tag lạ | `pipeline.rs:453-457` | `[WARN] AVB vbmeta: unsupported descriptor tag ...` |
| In AVB gốc | `pipeline.rs:458-462` | `report::avb_lines(&avb_info)` |
| Khóa slot | `pipeline.rs:467-476` | `orig_sz =` kích thước file; `avb_info.partition_size` bị đặt lại bằng `orig_sz`. |
| Snapshot trước AVB | `pipeline.rs:630-633` | `take_pre_avb_backup` + xác minh size/SHA-256. |
| Rebuild | `pipeline.rs:636-642` | `avb::rebuild_hashtree(&work, &avb_info, orig_sz, deps.avb, log)`; lỗi chỉ log `[WARN]`, không fail build. |
| Kiểm tra footer mới | `pipeline.rs:643-647` | `avb_info_fast(&work).has_footer` và `cur == slot_size` → `boot_safe`. |
| Fallback | `pipeline.rs:649-658` | Nếu không `boot_safe`: khôi phục pre-AVB snapshot, ép size slot, `skip_avb = true`. |
| Token | `pipeline.rs:683` | `AVB_HASHTREE=PASS` hoặc `SKIP`. |

Kết luận: parser đọc **vbmeta trong footer của vendor.img** (Tag 1/2/4 của footer này), không đọc vbmeta tổng hợp (top-level vbmeta) của thiết bị.

### 3.2. `avb.rs` — parser và metadata

- `avb_info_fast()` dòng 496: tìm footer 64 byte ở cuối (hoặc quét lùi tối đa 32 MiB), rồi `fill_from_vbmeta`.
- `fill_from_vbmeta()` dòng 129: khởi tạo `AvbInfo::empty` (fallback `"vendor"`, `"sha256"`, salt rỗng, `fec_roots = 0`), rồi chỉ ghi đè khi `VbmetaMeta` có `Some`.
- `parse_vbmeta()` dòng 256: duyệt descriptor theo tag: `0` property, `1` hashtree, `2` hash, `4` chain, còn lại `unsupported_tags`.
- `parse_hashtree()` dòng 328: đọc `fec_roots` (`[52..56]`), `hash_algorithm` (`[72..104]`), name/salt; gọi `select_partition`.
- `parse_hash_descriptor()` dòng 378: đọc `hash_algorithm` (`[24..56]`), name/salt; gọi `select_partition`.
- `parse_chain_partition_descriptor()` dòng 425: đọc name/public_key; hiện **gọi `select_partition`**.
- `select_partition()` dòng 232: chọn tên partition đầu tiên, trả `bool` cùng partition.
- `merge_hash_fields()` dòng 242: first-non-None cho algorithm/salt.

### 3.3. `HashtreeRequest` và `rebuild_hashtree`

`HashtreeRequest` (dòng 577) chỉ có: `image`, `partition_size`, `partition_name`, `hash_algorithm`, `salt`, `props`. **Không có** `fec_roots`, `chain_partition`, loại descriptor (hash vs hashtree), `digest`, `public_key`, `rollback_index_location`.

`HashtreeRequest::cli_args()` (dòng 588) luôn tạo:

```text
add_hashtree_footer
--image ...
--partition_size ...
--partition_name <info.partition_name>
--hash_algorithm <info.hash_algorithm>
--block_size 4096
--salt <info.salt>
--algorithm NONE
--rollback_index 0
--rollback_index_location 0
--flags 0
--do_not_generate_fec
--fec_num_roots 2
--prop <k>:<v> ...
```

`rebuild_hashtree()` (dòng 672):

- `part_size` lấy `info.partition_size` (đã bị pipeline đặt bằng kích thước file).
- `salt` = salt gốc nếu khác rỗng, ngược lại random.
- `partition_name` = `info.partition_name` (fallback `"vendor"`).
- `hash_algorithm` = `info.hash_algorithm` (fallback `"sha256"`).
- `props` = `info.props`.
- Gọi `erase_footer` (chỉ **zero 64 byte footer**, không truncate), rồi `backend.add_hashtree_footer(&req)` — luôn là `add_hashtree_footer`, không bao giờ `add_hash_footer`.
- Sau đó đọc lại footer; nếu ảnh lớn hơn slot → `Ok(None)` để pipeline khôi phục pre-AVB.

---

## 4. ĐỐI CHIẾU AOSP THEO TAG 1/2/4

### 4.1. Tag 4 — `AvbChainPartitionDescriptor`

Nguồn: `avb_chain_partition_descriptor.h` (commit trên), dòng 44-51, nguyên văn:

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

`avbtool.py` (commit trên), `AvbChainPartitionDescriptor.verify()` dòng 1905-1930:

```python
value = expected_chain_partitions_map.get(self.partition_name)
if not value:
  sys.stderr.write('No expected chain partition for partition {}. Use '
                   '--expected_chain_partition to specify expected '
                   'contents or --follow_chain_partitions.\n'.
                   format(self.partition_name))
  return False
```

`expected_chain_partitions_map` được xây từ `--expected_chain_partition <partition>:<rollback_location>:<key>` (dòng 2533-2544). Nghĩa là `partition_name` của Tag 4 là **partition được chain tới**, không phải image chứa descriptor.

Kết luận Tag 4: `partition_name` là **tham chiếu**, không phải tên image chứa descriptor. Code hiện tại cho Tag 4 gọi `select_partition` là **sai ngữ nghĩa AOSP**.

### 4.2. Tag 1 — `AvbHashtreeDescriptor`

Nguồn: `avb_hashtree_descriptor.h` dòng 46-51:

```c
/* A descriptor containing information about a dm-verity hashtree.
 *
 * Hash-trees are used to verify large partitions typically containing
 * file systems. ...
 *
 * Following this struct are |partition_name_len| bytes of the
 * partition name (UTF-8 encoded), |salt_len| bytes of salt, and then
 * |root_digest_len| bytes of the root digest.
 */
```

`avbtool.py`, `AvbHashtreeDescriptor` doc dòng 1356-1376:

```python
fec_num_roots: Number of roots used for FEC (0 if FEC is not used).
fec_offset: Offset of FEC data (0 if FEC is not used).
fec_size: Size of FEC data (0 if FEC is not used).
```

`add_hashtree_footer` (dòng 3795-3846) đặt `ht_desc.partition_name = partition_name` — chính là `--partition_name` của image đang đóng gói; `ht_desc.fec_num_roots` chỉ được gán khi `generate_fec` true.

Kết luận Tag 1: `partition_name` là **tên image/partition được mô tả** bởi descriptor; đây là nguồn đúng cho tên partition dùng khi rebuild.

### 4.3. Tag 2 — `AvbHashDescriptor`

Nguồn: `avb_hash_descriptor.h` dòng 45-54:

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
 * Changes in v1.1:
 *   - flags field is added which supports AVB_HASH_DESCRIPTOR_FLAGS_USE_AB
 *   - digest_len may be zero, which indicates the use of a persistent digest
 */
```

`avbtool.py`, `add_hash_footer` (dòng 3529-3551) tạo `AvbHashDescriptor` với `h_desc.partition_name = partition_name`, `h_desc.digest = digest`, rồi gọi `_generate_vbmeta_blob(..., [h_desc], ...)`.

Kết luận Tag 2: `partition_name` cũng là **tên image được mô tả**, nhưng integrity scheme là hash toàn image, không phải dm-verity. Đây là nguồn hợp lệ thứ hai cho tên image.

### 4.4. Thứ tự descriptor do avbtool tạo

`avbtool.py`, `_generate_vbmeta_blob`:

- `make_vbmeta_image` gọi `_generate_vbmeta_blob(..., descriptors=[], ...)` (dòng 2969-2978).
- `add_hash_footer` gọi `_generate_vbmeta_blob(..., [h_desc], ...)` (dòng 3548-3556).
- `add_hashtree_footer` gọi `_generate_vbmeta_blob(..., [ht_desc], ...)` (dòng 3850-3858).
- Trong `_generate_vbmeta_blob`, chain descriptor được `descriptors.append(desc)` **sau** các descriptor được truyền vào, rồi mới encode theo thứ tự list:

```python
    # Insert chained partition descriptors, if any
    ...
        descriptors.append(desc)

    # Descriptors.
    encoded_descriptors = bytearray()
    for desc in descriptors:
      encoded_descriptors.extend(desc.encode())
```

- Descriptor từ `--include_descriptors_from_image` lại được encode **sau** props/kernel cmdline, tức **sau** chain descriptor.

Hệ quả:
- Với `add_hashtree_footer`/`add_hash_footer`: thứ tự là `[tag 1 hoặc tag 2]` rồi `[tag 4...]` — Tag 4 **không** đứng trước trong output của hai lệnh này.
- Với `make_vbmeta_image --chain_partition ... --include_descriptors_from_image ...`: Tag 4 đứng **trước** descriptor được include (có thể là Tag 1/2). Đây là trường hợp **có thể tái hiện bằng avbtool thật** (xem 4.5).
- Format AVB không quy định thứ tự descriptor (`avb_descriptor_foreach` duyệt tuần tự, không yêu cầu tag nào trước), nên parser không được giả định Tag 1/2 luôn đứng trước Tag 4.

### 4.5. Thực nghiệm với avbtool bundled (không đụng repo)

Thư mục tạm: `C:\Users\HT\.qoder\tmp\phase5-audit\lab4`, dùng `tool/python/python.exe` + `tool/avbtool.py`.

Lệnh:

```text
avbtool.py add_hashtree_footer --image vendor.img --partition_size 4194304 \
  --partition_name vendor --hash_algorithm sha256 --block_size 4096 \
  --salt 0011...eeff --algorithm NONE --rollback_index 0 \
  --rollback_index_location 0 --flags 0 --do_not_generate_fec \
  --fec_num_roots 2 --chain_partition boot:1:fakekey.bin
```

`info_image` (nguyên văn, rút phần descriptors):

```text
Descriptors:
    Hashtree descriptor:
      ...
      Partition Name:        vendor
      ...
    Chain Partition descriptor:
      Partition Name:          boot
      Rollback Index Location: 1
```

Sau đó tạo vbmeta tổng hợp:

```text
avbtool.py make_vbmeta_image --output vb.img --algorithm NONE --rollback_index 0 \
  --flags 0 --chain_partition boot:1:fakekey.bin \
  --include_descriptors_from_image vendor.img
```

`info_image` (nguyên văn):

```text
Descriptors:
    Chain Partition descriptor:
      Partition Name:          boot
      Rollback Index Location: 1
      Public key (sha1):       cbf4d7fb248f319ffe031ebbf87ed795ce3b0009
      Flags:                   0
    Chain Partition descriptor:
      Partition Name:          boot
      Rollback Index Location: 1
      Public key (sha1):       cbf4d7fb248f319ffe031ebbf87ed795ce3b0009
      Flags:                   0
    Hashtree descriptor:
      ...
      Partition Name:        vendor
      ...
```

Kết luận thực nghiệm:
1. Tag 4 **có thể** đứng trước Tag 1 khi vbmeta được tạo bằng `make_vbmeta_image` + `--include_descriptors_from_image`; parser phải xử lý đúng thứ tự này.
2. Một vbmeta **có thể chứa nhiều Tag 4** (thực nghiệm có hai descriptor chain).
3. Trong cùng một vbmeta, Tag 4 mang tên `boot` trong khi Tag 1 mang tên `vendor` — đúng kịch bản lỗi của vấn đề A.

---

## 5. PHÂN TÍCH VẤN ĐỀ A — TAG 4 VÀ PARTITION ĐƯỢC REBUILD

### 5.1. Code hiện tại

`parse_chain_partition_descriptor()` (dòng 425-459) kết thúc bằng:

```rust
    let name_at = CHAIN_PARTITION_DESCRIPTOR_LEN;
    let name = std::str::from_utf8(&body[name_at..name_at + name_len])
        .map_err(|_| format!("partition name at {pos} is not UTF-8"))?;
    select_partition(meta, name);
```

`select_partition()` (dòng 232-238) đặt `meta.partition_name` nếu chưa có.

### 5.2. Hệ quả với vbmeta có Tag 4 trước Tag 1/2

Kịch bản thực nghiệm 4.5 (Tag 4 `boot` trước, Tag 1 `vendor`):

1. Tag 4 → `select_partition(meta, "boot")` → `partition_name = Some("boot")`.
2. Tag 1 `vendor` → `select_partition` trả `false` (khác tên) → `hash_algorithm`, `salt`, `fec_roots` của `vendor` **bị bỏ**.
3. `AvbInfo.partition_name = "boot"`; `hash_algorithm`/`salt`/`fec_roots` giữ fallback (`"sha256"`, rỗng, 0).
4. `rebuild_hashtree` gọi `add_hashtree_footer --partition_name boot` cho **vendor.img**.
5. Nếu salt rỗng, tool sinh salt ngẫu nhiên; hash algorithm thành `sha256`.
6. Footer mới mô tả `vendor` với tên `boot` → sai định danh partition; dm-verity/Android tra cứu descriptor theo tên partition (`vendor`) sẽ không khớp.

Đây là **BUG logic**, không phải giới hạn đọc metadata.

### 5.3. Đánh giá phương án trong brief

- "Tag 1/2 mới có quyền chọn `meta.partition_name` dùng cho rebuild" — **đúng** với AOSP: Tag 1/2 mô tả image; Tag 4 tham chiếu partition khác.
- "Tag 4 không được ghi đè hoặc chiếm quyền chọn tên image partition" — **đúng**.
- "Nếu chỉ có Tag 4 mà không có Tag 1/2, giữ fallback hiện tại" — **đúng**: `AvbInfo::empty()` đã cho `"vendor"`; không dùng tên chain làm fallback.
- "Tag 1/2 cùng partition merge từng field; khác partition không ghép" — giữ nguyên hành vi follow-up trước, chỉ đổi tập descriptor được chọn tên.

### 5.4. Ảnh hưởng

- Ảnh hưởng thực tế khi vbmeta footer có Tag 4 đứng trước Tag 1/2. Với avbtool `add_hashtree_footer`, Tag 4 thường đứng sau (4.4), nên luồng phổ biến không lộ lỗi; nhưng vbmeta tổng hợp/gia công có thể có thứ tự ngược (4.5), và parser là thành phần nhận dữ liệu bên ngoài nên không được giả định thứ tự.
- Mức độ: **BUG** (sai tên partition + mất metadata hash/FEC của image).

---

## 6. PHÂN TÍCH VẤN ĐỀ B VÀ C

### 6.1. Vấn đề B — Tag 2 và chiến lược rebuild

#### Bằng chứng code

- Parser đọc Tag 2: `parse_hash_descriptor` (dòng 378) lấy `partition_name`, `hash_algorithm`, `salt`; **không** lưu `image_size`, `digest`.
- `rebuild_hashtree` chỉ có một backend: `Avb::add_hashtree_footer` (dòng 625, 728). `HashtreeRequest::cli_args()` luôn bắt đầu bằng `"add_hashtree_footer"` (dòng 590).
- `erase_footer` chỉ zero 64 byte footer (dòng 774-798); không truncate, không phân biệt loại descriptor cũ.
- Không có nhánh nào kiểm tra "Tag 2 hay Tag 1" trước khi rebuild; `VbmetaMeta` không lưu loại descriptor.

#### Bằng chứng AOSP

- `AvbHashDescriptor`: "typically used for boot partitions to verify the entire kernel+initramfs image" (`avb_hash_descriptor.h`).
- `AvbHashtreeDescriptor`: "used to verify large partitions typically containing file systems" (`avb_hashtree_descriptor.h`).
- `avbtool.py add_hash_footer` tạo `AvbHashDescriptor` + hash toàn image; `add_hashtree_footer` tạo `AvbHashtreeDescriptor` + dm-verity tree. Hai lệnh tạo hai loại descriptor khác nhau, không tự chuyển đổi.
- `HashtreeRequest` không có tham số nào để yêu cầu `add_hash_footer`, nên tool **không thể** rebuild Hash Descriptor đúng loại.

#### Phân tích luồng

- Tool chỉ build `vendor.img` (VoLTE Vendor Builder). Vendor là phân vùng filesystem, AOSP dùng hashtree cho phân vùng lớn chứa filesystem. Hash Descriptor dùng cho boot/initramfs.
- Nếu đầu vào là vendor.img có Tag 2 (không có Tag 1), tool vẫn:
  1. parse được name/algorithm/salt;
  2. `erase_footer`;
  3. gọi `add_hashtree_footer` → tạo **Hashtree Descriptor**, thêm hashtree + vbmeta mới.
- Đây là **chuyển đổi integrity scheme** (hash toàn image → dm-verity hashtree), không phải rebuild tương đương. Nó thay đổi cách Android xác minh image và làm thay đổi layout/kích thước.
- Tuy nhiên đầu vào vendor.img có Tag 2 là **ngoài luồng hỗ trợ thực tế** (vendor dùng hashtree). Không có bằng chứng tool cần hỗ trợ Hash Descriptor; brief cũng yêu cầu không mở rộng chức năng AVB khi chưa chứng minh cần.

#### Phân loại

- **LIMITATION** (giới hạn chức năng có chủ đích/ngầm định): tool chỉ rebuild hashtree, không hỗ trợ Hash Descriptor.
- **Tiềm ẩn DATA LOSS/semantics change** nếu đầu vào vendor.img có Tag 2: footer mới là hashtree thay vì hash, digest/image_size cũ bị bỏ. Chưa có bằng chứng đầu vào này thuộc phạm vi hỗ trợ; **không sửa** trong nhiệm vụ này. Đề xuất: nếu muốn an toàn, phát hiện Tag 2-only và WARN/fail — cần một nhiệm vụ riêng vì phải thêm trạng thái descriptor vào `AvbInfo` và test tích hợp avbtool.

### 6.2. Vấn đề C — FEC roots và rebuild

#### Bằng chứng code

- `parse_hashtree` đọc `fec_roots` (dòng 339) → `meta.fec_roots` → `AvbInfo.fec_roots` (dòng 165-167).
- `report::avb_lines` in `FEC num roots: {info.fec_roots}` (`report.rs:40`), và snapshot có dòng `FEC num roots: 0` (`report.rs:197`).
- `HashtreeRequest` **không có** trường `fec_roots` (dòng 577-584); `cli_args` hardcode:
  ```text
  --do_not_generate_fec
  --fec_num_roots 2
  ```
  (dòng 611-613).
- `rebuild_hashtree` không truyền `info.fec_roots` vào request (dòng 696-711).

#### Bằng chứng AOSP/avbtool

- `fec_num_roots`: "Number of roots used for FEC (0 if FEC is not used)" (`avbtool.py` dòng 1368).
- `add_hashtree_footer` CLI (dòng 4911-4935):
  ```python
  self.avb.add_hashtree_footer(
      args.image.name if args.image else None,
      args.partition_size,
      args.partition_name,
      not args.do_not_generate_fec, args.fec_num_roots,
      ...
  ```
- Trong `add_hashtree_footer` (dòng 3825-3846): chỉ khi `generate_fec` true mới gọi `generate_fec_data(..., fec_num_roots)`, gán `fec_offset`/`fec_size`, và đặt `ht_desc.fec_num_roots`.
- Với `--do_not_generate_fec`, `generate_fec = False` → **không có FEC data**, `fec_num_roots` trong descriptor = 0.

#### Thực nghiệm

Lab `lab2` (hashtree, `--do_not_generate_fec`), `info_image` nguyên văn:

```text
      FEC num roots:         0
      FEC offset:            0
      FEC size:              0 bytes
```

Khi thử bỏ `--do_not_generate_fec`, avbtool gọi binary `fec` và thất bại trong môi trường này:

```text
  File "...\tool\avbtool.py", line 4106, in calc_fec_data_size
    p = subprocess.Popen(
FileNotFoundError: [WinError 2] The system cannot find the file specified
```

Repo `tool/` không có `fec.exe`; `command -v fec` trả `NO_SYSTEM_FEC`. Vì vậy tạo FEC thực sự **không khả thi** trong môi trường hiện tại.

#### Phân loại

- **LIMITATION** (có chủ đích, kế thừa): tool luôn tắt FEC; `fec_roots` chỉ được đọc/ hiển thị, không ảnh hưởng pipeline.
- Không được tuyên bố "bảo toàn FEC gốc": footer mới luôn có `fec_num_roots = 0` và không có FEC data.
- Nếu đầu vào có FEC, rebuild làm **mất khả năng sửa lỗi FEC** (error-correction redundancy), nhưng không làm sai dm-verity root hash.
- Log pipeline dòng 635 (`[STATUS] Generating hashtree and FEC for the image...`) **không khớp** với `--do_not_generate_fec`; đây là lỗi câu chữ, không phải semantics — ghi nhận, không sửa trong nhiệm vụ này.

### 6.3. Phát hiện bổ sung ngoài A/B/C: rebuild với footer đã tồn tại ở full slot

Trong quá trình kiểm chứng C, thực nghiệm lab2 phát hiện một vấn đề ảnh hưởng trực tiếp tới "tính toàn vẹn khi rebuild":

1. Tạo `vendor.img` 2 MiB, `add_hashtree_footer --partition_size 4194304` → file 4 MiB (full slot), footer ở cuối, `Original image size: 2097152`.
2. Mô phỏng đúng `erase_footer` của tool: zero 64 byte cuối (không truncate).
3. Chạy lại `add_hashtree_footer --partition_size 4194304`:
   ```text
   avbtool.py: Adding hashtree_footer failed: Image size of 4194304 exceeds maximum image size of 4087808 in order to fit in a partition size of 4194304..
   ```
4. Nếu truncate về `original_image_size` (2097152) trước khi chạy lại thì thành công (`TRUNC_RERUN_EXIT=0`).

Nguyên nhân: `avbtool` khi thấy footer sẽ tự truncate về `original_image_size` ("This way 'avbtool add_hashtree_footer' is idempotent", dòng 3725-3734). Tool zero footer trước khi gọi avbtool, nên avbtool coi toàn bộ file (đã bằng `partition_size`) là image → vượt `max_image_size` → lỗi.

Hệ quả: với ảnh có footer thật ở full slot, `rebuild_hashtree` trả lỗi; pipeline chỉ WARN rồi **DROP AVB** (restore pre-AVB, `AVB_HASHTREE=SKIP`). Đây là hành vi đã được pipeline dự phòng (`pipeline.rs:649-658`), nhưng nghĩa là AVB **không được rebuild** cho phần lớn ảnh vendor có footer.

Phân loại: **DATA LOSS/LIMITATION** (mất AVB trên output; không phải crash). Cần thay đổi thiết kế (truncate về `original_image_size` sau khi lưu footer, hoặc để avbtool tự truncate) và test tích hợp avbtool thật — **ngoài phạm vi sửa của nhiệm vụ này**, ghi thành đề xuất riêng (mục 8.4).

---

## 7. KẾT LUẬN AUDIT

| Vấn đề | Kết luận | Bằng chứng | Rủi ro |
|---|---|---|---|
| Tag 4 chọn partition | **BUG** — Tag 4 là tham chiếu partition khác; `select_partition` từ Tag 4 làm sai `meta.partition_name` và chặn metadata Tag 1/2 | `avb_chain_partition_descriptor.h` (pointer to another partition) + `avbtool.py:1905-1930` (`expected_chain_partitions_map`) + thực nghiệm 4.5 | Footer mới mang tên partition sai (`boot` cho vendor), mất salt/algorithm/FEC của image; dm-verity tra cứu sai |
| Chain descriptor khi rebuild | **DATA LOSS/LIMITATION** — rebuild không truyền `--chain_partition`, nên Tag 4 trong footer cũ bị bỏ khỏi footer mới | `HashtreeRequest` không có chain; `cli_args` không có `--chain_partition`; `avbtool` chỉ tạo chain khi được yêu cầu | Mất mục chain-of-trust; ngoài luồng vendor thông thường; cần nhiệm vụ riêng nếu muốn bảo toàn |
| Tag 2 khi rebuild | **LIMITATION** (hash descriptor ngoài luồng vendor) — nếu gặp Tag 2-only, tool chuyển thành hashtree footer, đổi integrity scheme | `AvbHashDescriptor` "typically boot"; `HashtreeRequest` chỉ `add_hashtree_footer`; `rebuild_hashtree` không phân biệt tag | Semantics AVB thay đổi với đầu vào ngoài luồng hỗ trợ; không sửa khi chưa có bằng chứng cần hỗ trợ |
| FEC khi rebuild | **LIMITATION** — `fec_roots` chỉ đọc/hiển thị; rebuild luôn `--do_not_generate_fec`, footer mới FEC = 0 | `HashtreeRequest` không có FEC; `cli_args` dòng 611-613; `avbtool.py:3825-3846`; lab2 `FEC num roots: 0`; thiếu binary `fec` | Mất FEC redundancy của ảnh gốc; không ảnh hưởng root hash; không được tuyên bố bảo toàn FEC |
| Rebuild với footer full slot | **DATA LOSS/LIMITATION** (phát hiện ngoài A/B/C) — zero footer không truncate làm avbtool từ chối, pipeline drop AVB | Lab2: `Image size ... exceeds maximum image size`; truncate về `original_image_size` thì thành công | AVB không được rebuild cho ảnh có footer; output chạy không AVB (unlocked/EDL) |

---

## 8. ĐỀ XUẤT GIẢI PHÁP TỐI THIỂU

### 8.1. Sửa Tag 4 (thuộc phạm vi nhiệm vụ, có bằng chứng)

- Chỉ Tag 1/2 được chọn `meta.partition_name`; đổi tên helper thành `select_image_partition` để chặn dùng nhầm.
- `parse_chain_partition_descriptor` vẫn giải mã/validate name (không nới lỏng kiểm tra UTF-8), nhưng **không** gọi `select_image_partition`.
- Giữ nguyên: kiểm tra kích thước, `checked_add`, `round8`, xử lý malformed, tag 0/tag lạ, fallback `AvbInfo::empty()`, merge từng field trong cùng partition.
- Test bắt buộc: Tag 4 trước Tag 1; Tag 4 trước Tag 2; chỉ Tag 4; nhiều Tag 4; Tag 1/2 cùng partition; Tag 1/2 khác partition.

### 8.2. Không sửa Tag 2 / FEC / chain preservation trong nhiệm vụ này

- Tag 2: giới hạn đọc metadata; nếu sau này cần hỗ trợ Hash Descriptor, phải thêm backend `add_hash_footer`, lưu `digest`/`image_size`, và test tích hợp avbtool. Không tự mở rộng.
- FEC: giữ chính sách không tạo FEC (phù hợp môi trường không có binary `fec`); ghi rõ limitation; không tuyên bố bảo toàn FEC.
- Chain preservation: cần trích `partition_name`/`rollback_index_location`/`public_key` và truyền `--chain_partition`; thêm trường vào `AvbInfo` — đề xuất nhiệm vụ riêng.

### 8.3. Đề xuất sửa log FEC (ngoài phạm vi sửa)

Đổi `[STATUS] Generating hashtree and FEC for the image...` thành câu phản ánh đúng (`...hashtree (FEC disabled)...`) trong một nhiệm vụ riêng, kèm cập nhật test pipeline đang tìm chuỗi `[STATUS] Generating hashtree`.

### 8.4. Đề xuất sửa rebuild full slot (ngoài phạm vi sửa)

Cân nhắc truncate về `info.original_image_size` sau khi lưu footer cũ (hoặc để avbtool tự truncate như thiết kế idempotent của nó), đồng thời điều chỉnh cơ chế restore khi backend lỗi. Cần test tích hợp với `tool/avbtool.py` thật và xác minh trên ảnh vendor thật. Không tự làm trong nhiệm vụ này.

---

## 9. CÁC VẤN ĐỀ CHƯA THỂ KẾT LUẬN

1. **Tần suất thực tế của Tag 4 trong footer vendor.img**: không có ảnh vendor thật trong workspace để kiểm tra. AOSP cho phép `--chain_partition`, nhưng không có dữ liệu thiết bị để khẳng định phổ biến hay không. → NEEDS DEVICE VALIDATION.
2. **Hành vi của tool Python gốc `hd_vendor_build.py`**: không có trong repo/zip; không thể xác nhận bản port có bỏ bước truncate hay không. → NEEDS ORIGINAL SOURCE.
3. **Có cần bảo toàn FEC/chain cho thiết bị cụ thể hay không**: phụ thuộc yêu cầu unlock/EDL và cơ chế verified boot của thiết bị. → NEEDS DEVICE VALIDATION.
4. **Tag 2-only trong vendor.img có tồn tại trên thiết bị thực tế hay không**: chưa có mẫu. → NEEDS DEVICE VALIDATION.

---

## 10. TÀI LIỆU/LOG ĐÍNH KÈM (ngoài repo)

- Source AOSP tải về: `C:\Users\HT\.qoder\tmp\phase5-audit\` (`*.h`, `avbtool.py`, kèm SHA-256 ở mục 2.3).
- Thực nghiệm avbtool: `C:\Users\HT\.qoder\tmp\phase5-audit\lab2`, `lab3`, `lab4`.
- Checksum/working tree trước khảo sát: mục 2.1-2.2.
