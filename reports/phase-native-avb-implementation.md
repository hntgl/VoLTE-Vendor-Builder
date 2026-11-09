# BÁO CÁO — TRIỂN KHAI `NativeAvb` THEO KẾ HOẠCH ĐÃ DUYỆT

- Ngày: 2026-11-09
- Nhánh: `ai2/fix-audit-findings`, HEAD `5886208` (không commit trong phiên này)
- Vai trò: AI2 — Local Executor
- Kế hoạch nguồn: `reports/plan-native-avb-implementation.md` (bản có dòng bổ sung §4.3c)
- Tài liệu đã đọc lại trước khi code: kế hoạch trên, `addendum-native-avb-survey-review.md`,
  `reports/avbtool-footer-reuse-investigation.md`, `reports/phase-avb-tail-backup.md`

**STATUS: IMPLEMENTED — GATE 1, 2, 3, 6, 7 PASS. GATE 4 FAIL: ảnh gốc A3s dùng
`sha1`, NativeAvb v1 chỉ hỗ trợ `sha256`; GATE 5 DỪNG theo quy tắc phạm vi.
CHƯA COMMIT. BACKEND MẶC ĐỊNH VẪN LÀ `python`. CHỜ DUYỆT HƯỚNG XỬ LÝ.**

---

## 1. STATUS

| Hạng mục | Kết quả |
|---|---|
| P0 tự đọc 64 B cuối + cắt về `original_image_size` | ✅ `original_image_size` (`avb.rs:721`), có test T4 |
| Validate trước, ghi sau (12 bước §4.2) | ✅ không ghi byte nào trước bước 11 |
| Property descriptor dùng công thức parser | ✅ `round8(PROPERTY_DESCRIPTOR_LEN - 16 + k + 1 + v + 1)` tại `avb.rs:889` |
| Hash algorithm v1 | ✅ chỉ `sha256`, khác → `Err` trước khi ghi |
| `VOLTE_AVB_BACKEND` | ✅ `python`/`native` không phân biệt hoa thường, mặc định `python`, sai → exit 2 |
| Test T1–T9 | ✅ 10 test native pass (T1–T9 + golden root) |
| Harness parity với avbtool bundled | ✅ 4 input, toàn file IDENTICAL, `info_image` MATCH, `verify_image` exit 0, idempotent |
| `cargo fmt/check/clippy/test` | ✅ exit 0 (30 CLI + 268 core, 1 ignored) |
| `cargo build --release --offline` | ✅ exit 0, exe SHA-256 `f33bfab6add60a2d58c3b16f1500ac173a2f2f243ea52a6841c5fb9f8f424513` |
| `pipeline.rs` giữ checksum | ✅ `17adbce4…` không đổi |
| Gate ảnh vendor đã ký thật | ❌ DỪNG — `E:\ROM\a3s volte\vendor.img` dùng `sha1` (kèm FEC 2 root); NativeAvb v1 hard-fail `sha1` (xem §10) |
| Commit/push | ❌ không thực hiện (chưa được yêu cầu) |

---

## 2. PHẠM VI VÀ FILE THAY ĐỔI

Đúng 2 file như kế hoạch §1.1:

| File | Thay đổi |
|---|---|
| `volte-core/src/avb.rs` | `NativeAvb` + helper serialize + 10 test + harness `#[ignore]` |
| `volte-cli/src/main.rs` | `Backend` + `VOLTE_AVB_BACKEND` + log `[AVB] backend=...` |

Không đổi `pipeline.rs`, `report.rs`, `slot.rs`, `env.rs`, `sha256.rs`, `Cargo.toml`,
`volte-core/Cargo.toml`, fixture, `tool/avbtool.py`. Không thêm dependency.
Không mở rộng RSA/chained vbmeta/FEC/verify CLI.

---

## 3. CHECKSUM TRƯỚC / SAU

"Trước" lấy từ kế hoạch §2.2 (đo tại HEAD `5886208` trước khi sửa). "Sau" đo lại
sau khi hoàn tất toàn bộ code/test tại cây làm việc hiện tại.

| File | Trước | Sau |
|---|---|---|
| `volte-core/src/avb.rs` | `bfc095efd3d1149b672cb04fe481d23ac28bbe576610473da0577a1e764d923f` | `0c87d71aa2bacdc8e73aaa19af5e41e8bed6ecb6cb583e1cb34e1f36a2467b10` |
| `volte-cli/src/main.rs` | `25295986f3cc9e59248405827763c170d2bbd989617dd7813e2f4592743375cc` | `aded2f7d0b2c7309683055094f133ff2bf732025ff65407e9a5b535583d7af11` |

Các file ngoài phạm vi phải giữ nguyên — output thật:

```text
0c87d71aa2bacdc8e73aaa19af5e41e8bed6ecb6cb583e1cb34e1f36a2467b10 *volte-core/src/avb.rs
17adbce4ed3eeb3e664bb7bfd5b326ecbfa14f4da681e4fe81d24098db617bd3 *volte-core/src/pipeline.rs
1ec1b5816182fa6baf0c6f311c172af860b2d4a80a04f121ab5ad9405f7a464b *volte-core/src/env.rs
12fa0d49c0f7dfe1d59f4c77c1a3b32a05a96aaca473324d802e08b9d01091a8 *volte-core/src/sha256.rs
aded2f7d0b2c7309683055094f133ff2bf732025ff65407e9a5b535583d7af11 *volte-cli/src/main.rs
de67ba20cfa53bdb7d07a78922f66530754989e97468cf9b388519c190b9eb8c *Cargo.toml
56f92ce2ffdc85d8a7ab5f9f2bf516ed0f351f57e6c47c1f42b9b333eac39316 *volte-core/Cargo.toml
---EXE---
f33bfab6add60a2d58c3b16f1500ac173a2f2f243ea52a6841c5fb9f8f424513 *target/release/volte-cli.exe
1267200 target/release/volte-cli.exe
```

`pipeline.rs` vẫn đúng `17adbce4ed3eeb3e664bb7bfd5b326ecbfa14f4da681e4fe81d24098db617bd3`
như kế hoạch §2.2 và §10.1 gate 6. `env.rs`, `sha256.rs`, `Cargo.toml`,
`volte-core/Cargo.toml` trùng khớp baseline.

Diff và trạng thái cây làm việc (output thật):

```text
warning: in the working copy of 'volte-cli/src/main.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/avb.rs', LF will be replaced by CRLF the next time Git touches it
 volte-cli/src/main.rs |  47 +++-
 volte-core/src/avb.rs | 691 ++++++++++++++++++++++++++++++++++++++++++++++++++
 2 files changed, 735 insertions(+), 3 deletions(-)
---STATUS---
 M volte-cli/src/main.rs
 M volte-core/src/avb.rs
?? AI2_PROMPT_AUDIT_CLEANUP_VoLTE-Vendor-Builder.md
?? Logs/
?? VoLTE-Vendor-Builder.zip
?? VoLTE-Vendor-Builder_bak.zip
?? addendum-native-avb-survey-review.md
?? hd_vendor_build.py
?? reports/avbtool-footer-reuse-investigation.md
?? reports/f11_avbtool_verification_20261108.md
?? reports/f11_build_20261108_193953.log
?? reports/f11_build_avb_skip_20261108.log
?? reports/f11_build_tail_backup_20261109.log
?? reports/f11_tool_20261108_193953.log
?? reports/f11_tool_avb_skip_20261109.log
?? reports/phase-avb-noop-skip-on-failure.md
?? reports/phase-avb-tail-backup.diff
?? reports/phase-avb-tail-backup.md
?? reports/plan-native-avb-implementation.md
?? task-prompt-output-bak-before-overwrite.md
```

Hai file `??` mới so với baseline chỉ là `reports/plan-native-avb-implementation.md`
(kế hoạch đã duyệt, chưa commit) và các file vốn đã untracked từ trước. Không có file
tạm nào do phiên này tạo trong repo; `__pycache__` sinh ra khi harness chạy Python
đã được xoá (`find tool/python -type d -name __pycache__ -prune -exec rm -rf {} +`).

---

## 4. TRIỂN KHAI CHI TIẾT

### 4.1. P0 — tự phát hiện footer và cắt nội dung

`fn original_image_size` (`avb.rs:721`) chỉ đọc **đúng 64 byte cuối** file, gọi
`parse_footer(&blob, size)` hiện có; parse được thì trả `footer.original_image_size`,
không thì trả nguyên `size`. Không quét lùi, không mở rộng `HashtreeRequest`.
`native_add_hashtree_footer` (`avb.rs:950`) kiểm tra `original > size` trước khi làm
bất cứ việc gì (`avb.rs:966`).

### 4.2. Thứ tự validate trước — ghi sau

Trong `native_add_hashtree_footer`:

1. Mở file, `size = metadata.len()`.
2. Chặn sparse magic `3A FF 26 ED` (`avb.rs:954-963`).
3. P0 → `original`; từ chối footer nói quá kích thước file.
4. `partition_size` phải dương và bội 4096.
5. `rounded = ceil(original / 4096) * 4096`.
6. `max_image_size = partition_size - tree_size(partition_size) - 64 KiB - 4096`;
   `rounded > max_image_size` → `Err` với thông điệp `Image size of … exceeds maximum
   image size of …` (giữ tinh thần avbtool).
7. Hash algorithm: chỉ `sha256` (không phân biệt hoa thường), khác → `Err`.
8. Decode salt (rỗng → 32 byte từ CSPRNG; lẻ/không hex → `Err`).
9. Dựng Merkle tree trong RAM.
10. Dựng hashtree descriptor + property descriptor + vbmeta; kiểm tra
    `vbmeta <= 64 KiB` và `vbmeta_offset + round4096(vbmeta) + 4096 <= partition_size`.
11. Chỉ lúc này mới `set_len(original)`, ghi padding, tree, vbmeta, zero tới
    `partition_size - 4096`, rồi 4032 byte 0 + footer 64 byte.
12. `sync_all()`.

Mọi lỗi bước 1–10 trả `Err` và file giữ nguyên size + SHA-256 (test T6/T7/T9 xác nhận).
Không tự rollback trong backend khi lỗi I/O ở bước 11 — `rebuild_hashtree` phục hồi
tail như hợp đồng đã có ở `00079b3`.

### 4.3. Layout byte-exact và vị trí code

| Thành phần | Hàm |
|---|---|
| Level sizes Merkle (digest 32 B) | `hash_level_sizes` (`avb.rs:764`) |
| Merkle tree, level 0 nằm cuối vùng tree | `build_merkle` (`avb.rs:786`) |
| Hashtree descriptor (Tag 1, 180 B + name/salt/root) | `encode_hashtree_descriptor` (`avb.rs:850`) |
| Property descriptor (Tag 0) | `encode_property` (`avb.rs:886`) |
| VBMeta header `AVB0` + aux, release `avbtool 1.3.0`, algorithm NONE | `encode_vbmeta` (`avb.rs:912`) |
| Footer `AVBf` 64 B | `encode_footer` (`avb.rs:929`) |
| Ghép/ghi file | `native_add_hashtree_footer` (`avb.rs:950`) |

Property descriptor dùng đúng biểu thức tái dùng từ parser, không viết lại hằng số 32:

```rust
let nbf = round8(PROPERTY_DESCRIPTOR_LEN - 16 + k.len() + 1 + v.len() + 1);
let total = 16 + nbf;
```

### 4.4. Hash algorithm

`req.hash_algorithm.eq_ignore_ascii_case("sha256")` là điều kiện duy nhất; mọi giá trị
khác trả `Err("unsupported AVB hash algorithm \"…\" (supported: sha256)")` **trước**
khi ghi byte nào. Không fallback, không tự thêm sha1/sha512/blake2b. Thực nghiệm
T7 lặp lại oracle §0.4 của kế hoạch (avbtool exit 1, file không đổi).

### 4.5. CLI

- `Backend::from_env` (`main.rs:176`): `python` | `native`, không phân biệt hoa
  thường; không set → `python`; giá trị khác → `Err` trước khi chạy pipeline.
- `LazyAvb` (`main.rs:162`) giữ lazy-load Python; nhánh `Native` gọi trực tiếp
  `NativeAvb` và không kiểm tra `avbtool.py` tồn tại.
- `build` (`main.rs:225`) log `[AVB] backend=…` (`main.rs:254`).
- Mặc định không đổi; `env::tool_dir` vẫn giữ điều kiện `avbtool.py || e2fsck.exe`
  (thuộc phase gỡ Python, ngoài phạm vi).

---

## 5. ĐỐI CHIẾU TEST T1–T9 (kế hoạch §7.1)

Tất cả gọi **trực tiếp** `NativeAvb::add_hashtree_footer` với salt cố định
`001122…ddeeff`, không đi qua `rebuild_hashtree` (tránh salt ngẫu nhiên).

| # | Test thật | Oracle / assert | Kết quả |
|---|---|---|---|
| T1 | `native_vbmeta_matches_the_golden_fixture` (`avb.rs:2195`) | `assert_eq!(vbmeta, GOLDEN_VBMETA)` 704 B | ok |
| T2 | `native_full_image_matches_the_recorded_oracle` (`avb.rs:2231`) | size 16 MiB, SHA `4443FE9D…D66E7B`, `original=12582912`, `vbmeta_offset=12685312`, `vbmeta_size=512` | ok |
| T3 | `native_second_call_is_idempotent` (`avb.rs:2259`) | lần 2 byte-identical, size 2 MiB, footer cuối | ok |
| T4 | `native_reads_the_original_size_only_from_the_last_64_bytes` (`avb.rs:2277`) | `original=1 MiB` (không phải 2 MiB), `vbmeta_offset=1048576+12288`; footer lệch 4096 B → trả nguyên size | ok |
| T5 | `native_adds_a_footer_to_an_image_without_one` (`avb.rs:2309`) | không footer → tạo footer mới, `original=size ban đầu=1 MiB`, file 2 MiB | ok |
| T6 | `native_rejects_a_partition_that_cannot_hold_the_metadata` (`avb.rs:2324`) | `Err` chứa `12410880` + `exceeds maximum image size`; SHA file không đổi | ok |
| T7 | `native_rejects_an_unsupported_hash_algorithm_before_writing` (`avb.rs:2340`) | `Err` chứa `unsupported AVB hash algorithm` + `sha999`; SHA file không đổi | ok |
| T8 | `native_keeps_the_unrounded_original_size` (`avb.rs:2356`) | `original=1049810`, `vbmeta_offset=1052672+16384` (padding đúng) | ok |
| T9 | `native_does_not_hide_a_displaced_footer` (`avb.rs:2370`) | thêm 4096 B 0 → `Err` `exceeds maximum image size`, SHA không đổi | ok |
| — | `native_merkle_root_matches_the_golden_fixture` (`avb.rs:2211`) | root `ed31474c…d0ca`, tree 12288 B | ok |

Trích log thật:

```text
test avb::tests::native_adds_a_footer_to_an_image_without_one ... ok
test avb::tests::native_keeps_the_unrounded_original_size ... ok
test avb::tests::native_reads_the_original_size_only_from_the_last_64_bytes ... ok
test avb::tests::native_merkle_root_matches_the_golden_fixture ... ok
test avb::tests::native_rejects_an_unsupported_hash_algorithm_before_writing ... ok
test avb::tests::native_second_call_is_idempotent ... ok
test avb::tests::native_vbmeta_matches_the_golden_fixture ... ok
test avb::tests::native_rejects_a_partition_that_cannot_hold_the_metadata ... ok
test avb::tests::native_does_not_hide_a_displaced_footer ... ok
test avb::tests::native_full_image_matches_the_recorded_oracle ... ok
```

### 5.1. Ba kịch bản điều tra + kịch bản hash (kế hoạch §7.3)

| Kịch bản | Bằng chứng |
|---|---|
| Gọi 2 lần → idempotent | T3 + T2 + harness bước 7/8 |
| `partition_size` quá nhỏ | T6 |
| Không footer → pipeline SKIP, backend không được gọi | test cũ `pipeline::tests::a_no_footer_image_that_fills_the_slot_skips_avb_and_exports` vẫn pass, assert `avb.calls == 0` |
| Hash algorithm không hỗ trợ | T7 (oracle §0.4) |

---

## 6. HARNESS PARITY VỚI AVBTOOL BUNDLED (kế hoạch §7.2)

Lệnh thật:

```text
VOLTE_AVB_PARITY_PYTHON=E:/Build_app/Volte/VoLTE-Vendor-Builder/tool/python/python.exe \
  cargo test -p volte-core --offline avb_parity -- --ignored --nocapture
```

Harness (`avb.rs:2410`) tạo 4 input, chạy `avbtool.py add_hashtree_footer` với cùng
`HashtreeRequest` (cùng salt) lên `ref/vendor.img`, chạy `NativeAvb` lên
`native/vendor.img`, rồi:

1. so **toàn bộ byte** hai file (`assert_eq!` trên `Vec<u8>`, tương đương `cmp`);
2. so text `avbtool.py info_image` của hai file;
3. `avbtool.py verify_image --image native/vendor.img` phải exit 0;
4. gọi `NativeAvb` lần 2 và so với output lần 1.

Output thật (`--nocapture`):

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 1 test
pattern: size=16777216 sha256=4443FE9D0CD72B1D02AEE5E7D39D454B4EAD30A511CDC47AF8109B99C4D66E7B cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
unaligned: size=2097152 sha256=4E22342E0D46D60FB43CF0134722FD8AB3D874EB0916B920338A8D3FD5833F14 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
zeros: size=2097152 sha256=251E4EF4A679A7C0749206EBD1BB40CC9F7804EDB14F061349F0C972019E5CEB cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
deep: size=100663296 sha256=B7CFEAF0C469C0D9A958B1F100424B7AFBEA3541775C2D099F4383B0FE241420 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
test avb::tests::avb_parity_with_the_bundled_avbtool ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 268 filtered out; finished in 38.66s
```

Bốn input: `pattern` 12 MiB (nhiều level), `unaligned` 1 MiB + 1234 B,
`zeros` 1 MiB + 2 property descriptor, `deep` 70 MiB (3 tầng hash). Cả 4 đều
`cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL`.

Sau khi chạy, `__pycache__` dưới `tool/python` đã được xoá; temp dir harness
(`%TEMP%/volte_avb_native_parity`) tự xoá ở cuối test.

---

## 7. TOOLCHAIN — OUTPUT THẬT

Chạy tuần tự trên cây làm việc cuối (sau khi bổ sung T3/T5 và dòng `eprintln!`
của harness), mỗi lệnh ghi riêng file log ngoài repo.

### 7.1. `cargo fmt --all -- --check`

```text
fmt_exit=0
```

(không có output — sạch.)

### 7.2. `cargo check --offline --all-targets`

```text
    Checking volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
    Checking volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.86s
```

### 7.3. `cargo clippy --offline --all-targets -- -D warnings`

```text
    Checking volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
    Checking volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.12s
```

### 7.4. `cargo test --workspace --offline`

```text
   Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.48s
     Running unittests src\main.rs (target\debug\build\volte-cli/d68a8014c9353575\out\volte_cli-d68a8014c9353575.exe)

running 30 tests
test modem_cli::tests::fix_modem_parses_donor ... ok
test modem_cli::tests::fix_modem_parses_mbn ... ok
test modem_cli::tests::fix_modem_rejects_both_donor_and_mbn ... ok
test modem_cli::tests::fix_modem_accepts_equals_forms ... ok
test modem_cli::tests::fix_modem_requires_a_source ... ok
test modem_cli::tests::fix_modem_unknown_option_is_a_usage_error ... ok
test modem_cli::tests::modem_help_prints_and_returns_none ... ok
test modem_cli::tests::modem_accepts_out_equals_form ... ok
test modem_cli::tests::modem_parses_image_and_out ... ok
test modem_cli::tests::modem_rejects_a_second_positional ... ok
test modem_cli::tests::modem_requires_an_image ... ok
test tests::a_bare_image_with_no_subcommand_is_rejected ... ok
test modem_cli::tests::out_path_turns_empty_string_into_none ... ok
test tests::a_pipeline_error_exits_non_zero ... ok
test tests::a_second_positional_is_a_usage_error ... ok
test tests::defaults_are_empty_like_argparse ... ok
test tests::double_dash_ends_option_parsing ... ok
test tests::gui_flag_alone_selects_the_divergence_not_a_build ... ok
test tests::help_wins_from_any_position ... ok
test tests::missing_out_value_is_a_usage_error ... ok
test tests::modem_and_fix_modem_subcommands_are_unaffected ... ok
test tests::no_subcommand_is_a_usage_error ... ok
test tests::out_accepts_both_forms ... ok
test tests::parse_accepts_the_five_python_options ... ok
test tests::qc_wins_over_mtk_like_python ... ok
test tests::top_level_help_is_not_a_subcommand ... ok
test tests::unknown_option_is_a_usage_error ... ok
test tests::unknown_subcommand_is_a_usage_error ... ok
test tests::vendor_subcommand_help_still_works ... ok
test tests::vendor_subcommand_with_no_image_hits_the_gui_divergence ... ok

test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 269 tests
test avb::tests::a2_chain_before_hash_same_partition_merges_fields ... ok
test avb::tests::a2_chain_descriptor_is_flagged_in_avb_info ... ok
test avb::tests::a2_chain_before_hashtree_does_not_select_image_partition ... ok
test avb::tests::a2_an_unknown_descriptor_tag_is_recorded_not_silently_dropped ... ok
test avb::tests::a2_chain_reference_does_not_mix_hash_metadata ... ok
test avb::tests::a2_chain_only_does_not_override_image_partition_fallback ... ok
test avb::tests::a2_hash_before_hashtree_same_partition_fills_fec_only ... ok
test avb::tests::a2_hash_descriptor_sets_partition_name_and_algorithm ... ok
test avb::tests::a2_hashtree_before_hash_same_partition_keeps_first_values ... ok
test avb::tests::a2_descriptor_kind_distinguishes_hash_from_hashtree ... ok
test avb::tests::a2_hashtree_for_another_partition_does_not_fill_missing_fields ... ok
test avb::tests::a2_multiple_chain_descriptors_are_handled_correctly ... ok
test avb::tests::a_failing_backend_is_an_error_and_keeps_the_image_untouched ... ok
test avb::tests::a_truncated_or_malformed_vbmeta_is_an_error ... ok
test avb::tests::a_verified_rebuild_reports_the_new_footer ... ok
test avb::tests::avb_parity_with_the_bundled_avbtool ... ignored, needs the bundled Python and avbtool.py; byte-compares whole files
test avb::tests::cli_args_carry_the_frozen_parameters ... ok
test avb::tests::fit_partition_size_properties ... ok
test avb::tests::info_defaults_without_footer ... ok
test avb::tests::info_reads_the_golden_vbmeta_fields ... ok
test avb::tests::info_scans_back_past_padding ... ok
test avb::tests::missing_avbtool_names_the_exact_path ... ok
test avb::tests::a_failing_backend_over_an_old_footer_is_never_reported_as_success ... ok
test avb::tests::a_backend_that_writes_nothing_leaves_the_existing_footer_in_place ... ok
test avb::tests::a_hashtree_that_grows_past_the_slot_restores_the_tail ... ok
test avb::tests::native_adds_a_footer_to_an_image_without_one ... ok
test avb::tests::native_keeps_the_unrounded_original_size ... ok
test avb::tests::native_reads_the_original_size_only_from_the_last_64_bytes ... ok
test avb::tests::native_merkle_root_matches_the_golden_fixture ... ok
test avb::tests::native_rejects_an_unsupported_hash_algorithm_before_writing ... ok
test avb::tests::native_second_call_is_idempotent ... ok
test avb::tests::native_vbmeta_matches_the_golden_fixture ... ok
test avb::tests::original_size_larger_than_the_file_is_refused_before_the_backend ... ok
test avb::tests::parse_accepts_valid_footer ... ok
test avb::tests::parse_rejects_magic_size_and_offsets ... ok
test avb::tests::random_salt_is_64_lower_hex_chars_and_changes ... ok
test avb::tests::rebuild_detects_footer_and_keeps_the_locked_size ... ok
test avb::tests::the_golden_fixture_matches_its_reference_digest ... ok
test avb::tests::the_rebuild_request_carries_the_parsed_metadata ... ok
test avb::tests::tree_size_uses_32_byte_digests ... ok
test avb::tests::twenty_five_salts_are_all_distinct ... ok
test chip_detect::tests::binary_markers_win_and_qc_is_checked_first ... ok
test chip_detect::tests::clash_prefers_the_basename_then_qc ... ok
test chip_detect::tests::markers_beyond_16_mib_are_not_read ... ok
test chip_detect::tests::path_keywords_follow_the_python_order ... ok
test chip_detect::tests::refine_reads_the_vendor_keys_case_insensitively ... ok
test chip_detect::tests::tables_match_the_python_lengths ... ok
test config::tests::block_and_window_sizes ... ok
test config::tests::machine_tokens_spelling ... ok
test config::tests::one_apk_serves_both_branches ... ok
test config::tests::sparse_and_avb_magics ... ok
test config::tests::version_matches_python_source ... ok
test env::tests::exe_root_points_at_the_running_directory ... ok
test env::tests::forward_slashes_replaces_backslashes ... ok
test env::tests::tool_dir_falls_back_to_tools_then_root ... ok
test env::tests::tool_dir_prefers_tool_over_tools_and_root ... ok
test error::tests::bad_magic_kind_is_distinct ... ok
test error::tests::build_error_display_is_the_message ... ok
test error::tests::io_error_maps_to_io_kind ... ok
test error::tests::missing_names_the_exact_path ... ok
test ext4_tools::tests::a_hung_tool_is_killed_at_the_deadline ... ok
test ext4_tools::tests::a_missing_e2fsck_executable_is_an_error ... ok
test ext4_tools::tests::e2fsck_falls_back_to_the_bare_name ... ok
test avb::tests::native_rejects_a_partition_that_cannot_hold_the_metadata ... ok
test ext4_tools::tests::tail_chars_counts_characters_not_bytes ... ok
test ext4_tools::tests::vendor_flag_bit_reads_the_dumped_build_prop ... ok
test flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit ... ok
test flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit ... ok
test flags::tests::a6_no_pair_found_is_still_ok_zero ... ok
test flags::tests::boundary_uses_ascii_names_only ... ok
test flags::tests::case_alias_patches_in_place_instead_of_copying ... ok
test flags::tests::every_pair_is_applied_once_and_size_is_kept ... ok
test flags::tests::flag_bit_reads_both_dumps_and_separates_them ... ok
test flags::tests::flag_pairs_are_65_length_equal_and_non_empty ... ok
test flags::tests::hard_link_alias_patches_in_place_when_the_platform_allows_it ... ok
test flags::tests::limit_restricts_the_scanned_span ... ok
test ext4_tools::tests::fsck_accepts_a_clean_image_and_rejects_errors_with_the_tail ... ok
test flags::tests::patches_only_whole_property_names ... ok
test flags::tests::same_path_patches_in_place_and_empty_file_returns_zero ... ok
test avb::tests::native_does_not_hide_a_displaced_footer ... ok
test init_rc::tests::a7_a_commented_import_does_not_count_as_present ... ok
test init_rc::tests::a7_a_real_import_plus_comments_stays_idempotent ... ok
test init_rc::tests::a_dump_error_skips_the_candidate_instead_of_aborting ... ok
test init_rc::tests::a_nested_init_file_is_written_back_to_its_own_directory ... ok
test init_rc::tests::a_root_init_file_is_written_back_to_the_root ... ok
test init_rc::tests::candidate_counts_are_21_for_mtk_and_8_for_qc ... ok
test init_rc::tests::existing_directories_drop_their_mkdir_commands ... ok
test init_rc::tests::existing_imports_are_idempotent_and_skip_the_write_back ... ok
test init_rc::tests::inserts_the_missing_imports_and_keeps_the_file_lf_terminated ... ok
test init_rc::tests::split_vpath_keeps_the_directory_the_file_came_from ... ok
test init_rc::tests::undumpable_init_file_returns_false_without_error ... ok
test logging::tests::cp1252_bytes_decode_after_utf8_fails ... ok
test logging::tests::cp1258_bytes_win_before_cp1252 ... ok
test logging::tests::file_sink_appends_lines_with_lf ... ok
test logging::tests::fmt_time_matches_python ... ok
test logging::tests::latin1_is_the_last_resort ... ok
test logging::tests::logger_fans_out_to_every_sink_in_order ... ok
test logging::tests::memory_sink_collects_lines ... ok
test logging::tests::utf8_wins_when_valid ... ok
test modem::tests::a4_a_truncated_data_region_is_rejected_by_new ... ok
test modem::tests::a4_a_valid_image_is_still_accepted ... ok
test modem::tests::a4_fix_modem_from_mbn_on_a_truncated_image_returns_err ... ok
test modem::tests::a4_free_slot_on_a_truncated_buffer_returns_none_instead_of_panicking ... ok
test modem::tests::a4b_cycle_guard_skips_a_self_referencing_directory ... ok
test modem::tests::a4b_deep_directories_are_walked_without_overflowing_the_stack ... ok
test modem::tests::a_blank_image_parses_and_reports_its_oem ... ok
test modem::tests::a_non_power_of_two_sectors_per_cluster_is_rejected ... ok
test modem::tests::a_too_short_buffer_is_rejected ... ok
test modem::tests::a_zero_bytes_per_sector_is_rejected ... ok
test modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn ... ok
test modem::tests::build_modem_fix_from_mbn_rejects_a_too_small_blob ... ok
test modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image ... ok
test modem::tests::build_modem_fix_viettel_refuses_a_donor_without_viettel ... ok
test modem::tests::build_modem_volte_moves_viettel_first_and_keeps_the_image_size ... ok
test modem::tests::build_modem_volte_refuses_a_modem_without_viettel ... ok
test modem::tests::build_modem_volte_refuses_out_equal_to_source ... ok
test modem::tests::decode_83_handles_a_name_with_no_extension ... ok
test modem::tests::decode_83_round_trips_a_simple_name ... ok
test modem::tests::default_dst_appends_the_suffix_before_the_extension ... ok
test modem::tests::empty_oem_blob_still_gets_the_viettel_line ... ok
test modem::tests::fewer_than_three_data_clusters_is_rejected ... ok
test modem::tests::find_viettel_mbn_also_checks_the_row_carrier ... ok
test modem::tests::find_viettel_mbn_finds_a_large_enough_sea_entry ... ok
test modem::tests::find_viettel_mbn_rejects_an_entry_at_or_below_the_minimum_size ... ok
test modem::tests::image_size_never_changes_across_writes ... ok
test modem::tests::list_sea_carriers_lists_only_directories ... ok
test modem::tests::looks_like_qc_modem_is_true_only_for_a_parsable_image ... ok
test modem::tests::mkdir_is_idempotent_and_find_resolves_the_path ... ok
test modem::tests::part_match_is_case_insensitive_and_exact ... ok
test modem::tests::scan_modem_profiles_reports_viettel_presence ... ok
test modem::tests::viettel_line_is_added_when_absent_and_lf_is_preserved ... ok
test modem::tests::viettel_line_moves_to_the_front_and_dedups ... ok
test modem::tests::write_file_then_read_file_round_trips ... ok
test path_id::tests::a_directory_and_its_child_share_a_volume ... ok
test path_id::tests::case_only_alias_is_the_same_file_on_windows ... ok
test path_id::tests::hard_link_is_the_same_file_when_the_platform_allows_it ... ok
test path_id::tests::identical_paths_are_the_same_file ... ok
test path_id::tests::relative_and_absolute_paths_are_the_same_file ... ok
test path_id::tests::symlink_is_the_same_file_when_the_platform_allows_it ... ok
test payload::tests::cleanup_uses_absolute_paths_and_never_changes_directory ... ok
test payload::tests::command_details_match_python_values ... ok
test payload::tests::copies_land_in_the_work_directory ... ok
test payload::tests::first_directory_wins ... ok
test payload::tests::missing_features_xml_drops_only_its_block ... ok
test payload::tests::missing_payload_is_a_typed_error ... ok
test payload::tests::mtk_command_counts_match_python ... ok
test payload::tests::payload_dirs_order_and_dedupe ... ok
test payload::tests::payload_dirs_skips_missing_and_duplicates ... ok
test payload::tests::qc_command_counts_match_python ... ok
test flags::tests::pair_straddling_a_chunk_boundary_is_still_patched ... ok
test flags::tests::window_start_accepts_a_match_python_also_accepts ... ok
test avb::tests::native_full_image_matches_the_recorded_oracle ... ok
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... ok
test pipeline::tests::a3_payload_mismatch_after_injection_aborts_build ... ok
test pipeline::tests::a_footer_that_is_not_at_the_tail_stops_the_build_before_processing ... ok
test pipeline::tests::a_hash_only_input_warns_but_still_builds ... ok
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image has been running for over 60 seconds
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image ... ok
test pipeline::tests::a_no_footer_image_that_fills_the_slot_skips_avb_and_exports ... ok
test pipeline::tests::a_hashtree_that_grows_past_the_slot_skips_avb_and_exports_the_image ... ok
test pipeline::tests::a_successful_avb_rebuild_exports_the_image has been running for over 60 seconds
test pipeline::tests::a_successful_avb_rebuild_exports_the_image ... ok
test pipeline::tests::an_image_with_a_chain_descriptor_is_refused_before_avb ... ok
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb ... ok
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted ... ok
test pipeline::tests::case_only_output_alias_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::hard_linked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::log_stamp_has_the_python_shape ... ok
test pipeline::tests::missing_source_is_a_build_error ... ok
test pipeline::tests::output_equal_to_the_source_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::output_path_defaults_next_to_the_source ... ok
test pipeline::tests::output_path_honours_keep_out_and_renames_a_src_collision ... ok
test pipeline::tests::relative_source_with_absolute_output_is_refused ... ok
test pipeline::tests::space_need_counts_the_copy_and_the_temp_headroom ... ok
test pipeline::tests::space_need_saturates_instead_of_overflowing ... ok
test pipeline::tests::symlinked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::the_space_probe_allocates_and_cleans_up ... ok
test profile::tests::candidate_chains_are_canonical_names_only ... ok
test profile::tests::import_lines_match_the_renamed_rc_files ... ok
test profile::tests::init_vpath_lists_have_measured_lengths ... ok
test profile::tests::qc_rc_chain_falls_back_to_mtk_name ... ok
test profile::tests::selector_helpers_follow_the_branch ... ok
test props::tests::a_multibyte_error_message_is_truncated_by_char_without_panicking ... ok
test props::tests::a_second_run_changes_nothing ... ok
test props::tests::a_write_failure_is_logged_and_does_not_stop_the_other_files ... ok
test props::tests::crlf_duplicates_and_invalid_bytes_survive_the_rewrite ... ok
test props::tests::duplicate_keys_are_all_corrected_and_kept ... ok
test props::tests::every_prop_is_appended_once_and_comments_survive ... ok
test props::tests::existing_wrong_values_are_replaced_and_counted_as_dirty ... ok
test props::tests::identify_logs_at_most_six_allowlisted_lines ... ok
test props::tests::identify_rejects_a_short_blob ... ok
test props::tests::identify_returns_empty_when_the_dump_fails ... ok
test props::tests::keys_match_case_insensitively_but_values_stay_verbatim ... ok
test props::tests::missing_and_short_files_are_skipped ... ok
test props::tests::only_build_prop_gets_the_vendor_file_label ... ok
test props::tests::show_props_has_75_unique_keys_and_python_values ... ok
test props::tests::the_rewrite_preserves_inode_metadata_instead_of_resetting_it ... ok
test report::tests::a_failed_fsck_is_never_rendered_as_flashable ... ok
test report::tests::a_never_run_fsck_is_never_rendered_as_flashable ... ok
test report::tests::avb_lines_fall_back_to_vendor_and_sha256 ... ok
test report::tests::avb_lines_snapshot_is_byte_for_byte ... ok
test report::tests::flag_note_maps_to_zero_or_one ... ok
test report::tests::missing_hashes_omit_the_token_like_python_falsy_values ... ok
test report::tests::report_body_is_the_six_line_python_file ... ok
test report::tests::summary_lines_match_python_order_and_tokens ... ok
test report::tests::the_skip_note_replaces_the_pass_note ... ok
test rustfs_ext4::tests::a_failed_command_is_an_error_and_stops_the_batch ... ok
test rustfs_ext4::tests::a_failed_replacement_keeps_the_existing_file_intact ... ok
test rustfs_ext4::tests::a_failed_write_after_rm_is_reported_instead_of_succeeding ... ok
test rustfs_ext4::tests::a_non_ext4_file_is_refused_with_bad_magic ... ok
test rustfs_ext4::tests::best_effort_reports_failures_and_keeps_going ... ok
test rustfs_ext4::tests::dump_buffer_bounds_the_claim_by_the_image_size ... ok
test rustfs_ext4::tests::ea_set_accepts_a_host_path_with_spaces ... ok
test rustfs_ext4::tests::host_paths_with_spaces_and_non_ascii_survive_write_and_dump ... ok
test rustfs_ext4::tests::props_upsert_works_from_a_work_directory_with_spaces ... ok
test rustfs_ext4::tests::removes_files_and_empty_directories ... ok
test rustfs_ext4::tests::removing_an_absent_path_is_idempotent ... ok
test rustfs_ext4::tests::set_inode_field_mode_and_gid_and_the_selinux_label_land_on_disk ... ok
test rustfs_ext4::tests::writes_a_file_dumps_it_back_and_reports_existence ... ok
test sha256::tests::file_hash_matches_in_memory_hash ... ok
test sha256::tests::hex_is_upper_case ... ok
test sha256::tests::nist_448_bit_message ... ok
test sha256::tests::nist_896_bit_message ... ok
test sha256::tests::nist_abc ... ok
test sha256::tests::nist_empty_message ... ok
test sha256::tests::nist_one_million_a_repeating_block ... ok
test sha256::tests::padding_boundaries_round_trip_through_streaming ... ok
test slot::tests::equal_size_is_a_noop ... ok
test slot::tests::larger_file_in_soft_mode_is_kept_with_a_warning ... ok
test slot::tests::larger_file_is_a_hard_error_and_not_truncated ... ok
test slot::tests::missing_file_or_zero_slot_returns_zero ... ok
test pipeline::tests::an_image_with_fec_warns_that_fec_is_dropped ... ok
test sparse::tests::a1_dont_care_chunk_total_size_is_only_the_header ... ok
test sparse::tests::a1_fill_chunk_total_size_is_header_plus_four_bytes ... ok
test slot::tests::smaller_file_is_padded_with_zeros_and_content_kept ... ok
test sparse::tests::a1_raw_chunk_total_size_is_data_plus_the_header ... ok
test sparse::tests::a1_img2simg_style_sparse_image_is_accepted ... ok
test sparse::tests::a_block_size_not_multiple_of_four_is_rejected ... ok
test sparse::tests::a_chunk_count_that_cannot_fit_the_file_is_rejected ... ok
test sparse::tests::a_chunk_payload_size_that_disagrees_with_its_blocks_is_rejected ... ok
test sparse::tests::a_crc_chunk_that_declares_blocks_is_rejected ... ok
test sparse::tests::a_header_size_that_the_chunks_do_not_cover_is_an_error ... ok
test sparse::tests::a_short_chunk_header_is_rejected ... ok
test sparse::tests::a_short_file_header_is_rejected ... ok
test sparse::tests::a_zero_block_size_is_rejected ... ok
test sparse::tests::a_trailing_dont_care_chunk_extends_the_output ... ok
test sparse::tests::an_absurd_declared_size_is_rejected ... ok
test sparse::tests::an_all_dont_care_image_is_zero_filled_to_the_declared_size ... ok
test sparse::tests::an_image_declaring_no_blocks_is_rejected ... ok
test sparse::tests::an_unsupported_major_version_is_rejected ... ok
test sparse::tests::an_image_without_dont_care_keeps_the_exact_length ... ok
test sparse::tests::chunks_that_cover_more_than_the_header_declares_are_rejected_early ... ok
test sparse::tests::magic_detection ... ok
test sparse::tests::non_sparse_file_leaves_destination_absent ... ok
test sparse::tests::output_size_reads_the_declared_raw_size ... ok
test sparse::tests::oversized_headers_are_skipped ... ok
test sparse::tests::truncated_chunk_header_is_an_error ... ok
test sparse::tests::truncated_fill_pattern_is_an_error ... ok
test sparse::tests::truncated_raw_chunk_is_an_error ... ok
test sparse::tests::raw_fill_dont_care_crc_round_trip ... ok
test sparse::tests::unknown_chunk_type_is_an_error ... ok
test verify::tests::a_failed_dump_is_reported_as_failed ... ok
test verify::tests::a_copy_with_different_content_is_a_mismatch ... ok
test verify::tests::a_host_file_that_cannot_be_replaced_is_stale ... ok
test verify::tests::a_missing_copy_is_not_ignored ... ok
test verify::tests::a_stale_host_file_is_deleted_before_the_dump_and_never_hashed ... ok
test verify::tests::the_mtk_rc_name_is_verified_in_both_init_directories ... ok
test verify::tests::both_canonical_copies_must_match_the_payload ... ok
test verify::tests::without_a_source_payload_nothing_is_verified ... ok
test pipeline::tests::an_avb_backend_failure_skips_avb_and_exports_the_image ... ok
test pipeline::tests::an_image_without_fec_does_not_warn_about_fec ... ok

test result: ok. 268 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 137.58s

   Doc-tests volte_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 7.5. `cargo build --release --offline`

```text
   Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `release` profile [optimized] target(s) in 13.97s
```

### 7.6. Exe release

```text
target/release/volte-cli.exe
SHA-256 f33bfab6add60a2d58c3b16f1500ac173a2f2f243ea52a6841c5fb9f8f424513
size    1267200 bytes
```

---

## 8. EVIDENCE CLI `VOLTE_AVB_BACKEND` (chạy trên exe release)

```text
$ VOLTE_AVB_BACKEND=bogus ./target/release/volte-cli.exe vendor missing.img
volte-cli: error: VOLTE_AVB_BACKEND must be python or native, got bogus
bogus_exit=2

$ VOLTE_AVB_BACKEND=Native ./target/release/volte-cli.exe vendor missing.img
[AVB] backend=native
[ERROR] File not found: E:\Build_app\Volte\VoLTE-Vendor-Builder\missing.img
native_exit=2
```

Giá trị sai bị chặn trước pipeline (exit 2, không có `[AVB]` log, không chạm file).
Giá trị `Native` (hoa thường) được chấp nhận: log `[AVB] backend=native` xuất hiện
trước lỗi file không tồn tại của pipeline. Không có ảnh thật nào bị sửa.

---

## 9. ĐỐI CHIẾU GATE DONE (kế hoạch §10.1)

| # | Gate | Trạng thái |
|---|---|---|
| 1 | P0 + test T4 | ✅ |
| 2 | T1 byte-for-byte fixture; T2/T3/T5/T6/T7 pass | ✅ |
| 3 | Harness ≥ 3 input: toàn file IDENTICAL, `info_image` giống, `verify_image` PASS | ✅ (4 input) |
| 4 | Gate §5.3: ảnh nguồn thật đều `sha256` | ❌ FAIL — `E:\ROM\a3s volte\vendor.img` báo `Hash Algorithm: sha1` (xem §10) |
| 5 | Gate ảnh thật: rebuild Python vs Native cùng salt + full CLI `native` | ⛔ KHÔNG CHẠY — backend v1 từ chối `sha1` trước khi ghi; chạy tiếp sẽ chỉ chứng minh lỗi đã biết |
| 6 | `pipeline.rs` giữ `17adbce4…` | ✅ |
| 7 | Build release + SHA exe | ✅ `f33bfab6…` |

Theo §10.3, gate 4–5 chưa đạt nên **không đổi backend mặc định** và **không tuyên bố
DONE toàn phần**.

---

## 10. GATE ẢNH THẬT A3S — KẾT QUẢ VÀ ĐIỂM DỪNG

Người dùng cung cấp ảnh gốc: `E:\ROM\a3s volte\vendor.img`
(1.106.640.896 byte, ngày 2026-02-09). Kiểm tra read-only bằng chính
`tool/avbtool.py info_image` (không sửa ảnh):

```text
$ tool/python/python.exe tool/avbtool.py info_image --image "E:/ROM/a3s volte/vendor.img"
Footer version:           1.0
Image size:               1106640896 bytes
Original image size:      1089097728 bytes
VBMeta offset:            1106366464
VBMeta size:              512 bytes
--
Minimum libavb version:   1.0
Header Block:             256 bytes
Authentication Block:     0 bytes
Auxiliary Block:          256 bytes
Algorithm:                NONE
Rollback Index:           0
Flags:                    0
Rollback Index Location:  0
Release String:           'avbtool 1.0.0'
Descriptors:
    Hashtree descriptor:
      Version of dm-verity:  1
      Image Size:            1089097728 bytes
      Tree Offset:           1089097728
      Tree Size:             8585216 bytes
      Data Block Size:       4096 bytes
      Hash Block Size:       4096 bytes
      FEC num roots:         2
      FEC offset:            1097682944
      FEC size:              8683520 bytes
      Hash Algorithm:        sha1
      Partition Name:        vendor
      Salt:                  8ad963d3addc39584d2eb9791490aa37c8594f6e
      Root Digest:           9930f0f005dbd965e81d5566815edf6f7d6ce8da
      Flags:                 0
info_exit=0
```

Kết luận và điểm dừng:

- Ảnh thật **dùng `sha1`**, không phải `sha256`. Cây Merkle dùng digest 20 byte
  (tree 8.585.216 byte cho 1.089.097.728 byte nội dung), không phải digest 32 byte.
- Ảnh còn có **FEC 2 root** (`FEC offset 1097682944`, `FEC size 8683520`); pipeline
  hiện tại luôn chạy `--do_not_generate_fec` nên bản rebuild của Python cũng bỏ FEC
  (hành vi đã có từ trước, không thuộc phase này).
- `NativeAvb` v1 từ chối `sha1` **trước khi ghi byte nào** — đúng hợp đồng đã duyệt ở
  kế hoạch §5.2 ("hard error on unsupported"), nhưng nghĩa là **gate 4 FAIL** và
  **gate 5 không thể chạy**: không thể `cmp` Python vs Native trên ảnh này.
- Theo đúng chỉ thị của prompt triển khai ("ảnh nguồn thật dùng thuật toán khác
  sha256: DỪNG LẠI, không tự mở rộng phạm vi, báo lại để xin duyệt riêng"), tôi
  **không sửa gì thêm**.

Nếu sau này muốn hỗ trợ `sha1`, phạm vi không chỉ là thêm hàm băm:

1. Viết SHA-1 (hiện chỉ có `sha256.rs`), hoặc thêm dependency — cần duyệt.
2. Tổng quát hóa `hash_level_sizes`/`tree_size`/`fit_avb_part` theo digest length
   (20 vs 32). `tree_size` đang được `pipeline.rs` và `rebuild_hashtree` dùng để tính
   `max_image_size`/slot, nên thay đổi này chạm ra ngoài 2 file AVB.
3. Ghi `hash_algorithm` thật vào hashtree descriptor thay vì cố định `sha256`
   (`avb.rs:872`).
4. Bổ sung test vector SHA-1 + harness parity cho ảnh sha1 thật.

Đây là thay đổi phạm vi, cần approve riêng trước khi làm.

---

## 11. GHI NHẬN / CHỜ DUYỆT

1. **Ảnh thật**: đã chạy gate 4 trên ảnh A3s gốc — kết quả `sha1` (xem §10). Gate 5
   không chạy được cho tới khi có quyết định về `sha1`.
2. **README**: thêm `VOLTE_AVB_BACKEND` là file thứ ba, ngoài phạm vi 2 file — chưa làm,
   cần duyệt riêng (kế hoạch §1.3.3).
3. **Backend mặc định**: vẫn `python`. Với ảnh sha1, nếu ai đó bật `native` thì bước AVB
   sẽ `Err` rồi pipeline đi nhánh SKIP (avb backend failure là recoverable) — ảnh xuất
   ra giữ footer cũ, không khớp nội dung mới. Vì vậy **không được đổi mặc định** khi
   chưa hỗ trợ `sha1`.
4. **Chưa commit/push**. Diff hiện tại: 2 file, 735 insertions, 3 deletions.
5. Không flash/root/reboot thiết bị; ảnh gốc chỉ được đọc.

---

## 12. KẾT LUẬN

`NativeAvb` đã triển khai đúng phạm vi 2 file, tái hiện byte-for-byte output của
`avbtool.py` 1.3.0 trên 4 layout khác nhau (kể cả cây 3 tầng), validate trước khi ghi,
hard-fail hash ngoài `sha256`, P0 đúng hành vi `avbtool.py:3730-3744`. Toàn bộ test
mặc định tự chứa và pass offline; harness parity thật pass với AOSP tool làm trọng tài.
Gate ảnh thật đã kiểm tra và **dừng đúng quy tắc**: ảnh A3s gốc dùng `sha1`, ngoài phạm
vi v1. Backend mặc định giữ `python`.

**Chờ duyệt report và quyết định có mở phase hỗ trợ `sha1` hay không.**
