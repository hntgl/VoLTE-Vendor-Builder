# PHASE `NativeAvb` HỖ TRỢ `sha1` — BÁO CÁO THỰC THI

**STATUS: CODE XONG — TOÀN BỘ GATE PASS — CHƯA COMMIT — CHỜ DUYỆT.**

Kế hoạch đã duyệt: `reports/plan-native-avb-sha1.md`. Phase v1 (sha256):
`reports/phase-native-avb-implementation.md`. Không sửa ảnh gốc, không flash.

---

## 1. PHẠM VI THỰC TẾ

| File | Thay đổi | Dòng |
|---|---|---|
| `volte-core/src/sha1.rs` | **Mới**: SHA-1 streaming + NIST vectors | 207 |
| `volte-core/src/lib.rs` | `pub mod sha1;` | +1 |
| `volte-core/src/avb.rs` | `HashKind`, tree/descriptor theo digest length, test + 2 harness | thêm phần sha1 |

Ngoài phạm vi **không bị đổi**: `sha256.rs`, `pipeline.rs`, `report.rs`, `slot.rs`,
`env.rs`, `main.rs`, `Cargo.toml`, `tool/avbtool.py`, fixtures.

`git diff --numstat`:

```text
44      3       volte-cli/src/main.rs
951     0       volte-core/src/avb.rs
1       0       volte-core/src/lib.rs
```

(`main.rs` 44/3 và phần lớn `avb.rs` là diff v1 đã có từ trước, chưa commit; phase này
chỉ thêm phần sha1.)

---

## 2. CHECKSUM TRƯỚC / SAU

**Trước phase (baseline kế hoạch §2):**

```text
sha256  volte-core/src/avb.rs        0c87d71aa2bacdc8e73aaa19af5e41e8bed6ecb6cb583e1cb34e1f36a2467b10
sha256  volte-core/src/lib.rs        12b1035bc4afa673d119633abc6661a5c253f1cf7a4f90acb68a1c4cbf3328a8
sha256  volte-core/src/sha256.rs     12fa0d49c0f7dfe1d59f4c77c1a3b32a05a96aaca473324d802e08b9d01091a8
sha256  volte-core/src/pipeline.rs   17adbce4ed3eeb3e664bb7bfd5b326ecbfa14f4da681e4fe81d24098db617bd3
sha256  volte-cli/src/main.rs        aded2f7d0b2c7309683055094f133ff2bf732025ff65407e9a5b535583d7af11
```

**Sau phase:**

```text
sha256  volte-core/src/avb.rs        abc79ff08826fc542cd57187d78561751454eb2094b5f9564ce6f84d6170c55a
sha256  volte-core/src/lib.rs        4c841f457511fb6ea95e61b3f962e179eed2e7ca711a3ef26d30cf78bf1dea31
sha256  volte-core/src/sha1.rs       b41f8df84ef74a876679e8929b9737bc24c9f9637cac5f7f8de62a514649fd3a
sha256  volte-core/src/sha256.rs     12fa0d49c0f7dfe1d59f4c77c1a3b32a05a96aaca473324d802e08b9d01091a8  (không đổi)
sha256  volte-core/src/pipeline.rs   17adbce4ed3eeb3e664bb7bfd5b326ecbfa14f4da681e4fe81d24098db617bd3  (không đổi)
sha256  volte-cli/src/main.rs        aded2f7d0b2c7309683055094f133ff2bf732025ff65407e9a5b535583d7af11  (không đổi)
sha256  target/release/volte-cli.exe 0e1eae143dd80dea877ce503d152dc1789f41a666d65454c83a6d9579940404c
size    target/release/volte-cli.exe 1.266.176 bytes
```

---

## 3. TRIỂN KHAI

### 3.1. `sha1.rs`

- `Sha1::new/update/finish -> [u8; 20]`, `digest`, `hex_upper`; cùng cấu trúc
  buffer/padding với `sha256.rs` (0x80, độ dài bit 64-bit big-endian).
- Test: NIST `""`, `"abc"`, 448-bit, 896-bit, 1 triệu `a`, vector "quick brown
  fox", biên padding 0/1/55/56/57/63/64/65/119/120/127/128/129/1000 qua
  streaming chunk 7 byte, hex upper-case. **8/8 pass.**

### 3.2. `HashKind` (`avb.rs`)

```rust
enum HashKind { Sha1, Sha256 }
HashKind::from_name(name) -> Result<HashKind>  // ASCII case-insensitive
HashKind::name() -> "sha1" / "sha256"
HashKind::digest_len() -> 20 / 32
```

- `sha512`, `sha999`, `blake2b-256` → `Err("unsupported AVB hash algorithm \"…\"
  (supported: sha1, sha256)")` **trước khi ghi**.
- Stride cây vẫn **32 byte** (`DIGEST_LEN`): sha1 ghi 20 byte digest, 12 byte còn
  lại của slot giữ 0 — đúng `avbtool.py:3692-3693` (`round_to_pow2(20) - 20 = 12`).
  Vì vậy `tree_size`/`fit_avb_part`/`pipeline.rs` không đổi.
- Descriptor: field `hash_algorithm` 32 byte ghi theo `kind.name()`; `root_digest_len`
  lấy `root.len()` (20 với sha1); `nbf`/`total` vẫn `round8(180 + name + salt + root)`.

### 3.3. Không đổi

P0 chỉ đọc 64 byte cuối; `original_image_size`; decode salt (nhận 20 byte);
validate 12 bước trước khi ghi; `max_image_size`; layout vbmeta/footer; release
string `avbtool 1.3.0`; `sync_all`; backend mặc định `python`.

---

## 4. TEST MẶC ĐỊNH (OFFLINE, KHÔNG PYTHON)

| # | Test | Kết quả |
|---|---|---|
| S1 | SHA-1 NIST + streaming boundaries (8 test) | PASS |
| S2 | `native_sha1_full_image_matches_the_recorded_oracle` | PASS |
| S3 | `native_sha1_descriptor_reads_back_with_the_sha1_algorithm` | PASS |
| S4 | `native_sha1_second_call_is_idempotent` | PASS |
| S5 | `native_rejects_an_unsupported_hash_algorithm_before_writing` (3 thuật toán) | PASS |
| S6 | `native_sha1_keeps_the_unrounded_original_size` | PASS |

Oracle S2 ghi nhận **một lần** từ `avbtool.py` bundled với cùng input (1 MiB
zero, partition 2 MiB, salt 20 byte `00112233445566778899aabbccddeeff00112233`):

```text
image         2097152 bytes
sha256        787864a333f030fd58fe41fce759f8e28dcd9a200cc638555dd5ed5e2a57b62a
VBMeta offset 1060864
VBMeta size   512
Tree Size     12288
Hash Algorithm sha1
Root Digest   fd0afda78324e67fe4672c26f18bd5cdb2452488
verify_image  Successfully verified sha1 hashtree
```

S3 đọc lại bằng `avb_info_fast`: `hash_algorithm == "sha1"`, salt 20 byte đúng,
`partition_name == "vendor"`, `fec_roots == 0`, `vbmeta_offset == 1048576 + 12288`,
`vbmeta_size == 512`. S2 ghi đúng SHA-256 toàn file của avbtool.

---

## 5. TOOLCHAIN (NGUYÊN VĂN)

```text
$ cargo fmt --all -- --check
FMT_OK

$ cargo check --offline --all-targets
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.72s

$ cargo clippy --offline --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.77s

$ cargo test --workspace --offline
running 30 tests
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 282 tests
test result: ok. 280 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 172.94s
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo build --release --offline
    Finished `release` profile [optimized] target(s) in 7.41s
```

Hai test ignored là harness cần Python/ảnh thật (mục 6–7), không chạy mặc định.

---

## 6. HARNESS PARITY VỚI `avbtool.py` BUNDLED (7 LAYOUT)

```text
pattern: size=16777216 sha256=4443FE9D0CD72B1D02AEE5E7D39D454B4EAD30A511CDC47AF8109B99C4D66E7B cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
unaligned: size=2097152 sha256=4E22342E0D46D60FB43CF0134722FD8AB3D874EB0916B920338A8D3FD5833F14 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
zeros: size=2097152 sha256=251E4EF4A679A7C0749206EBD1BB40CC9F7804EDB14F061349F0C972019E5CEB cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
deep: size=100663296 sha256=B7CFEAF0C469C0D9A958B1F100424B7AFBEA3541775C2D099F4383B0FE241420 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
sha1_pattern: size=16777216 sha256=E6D0EA90E6750254D00E549CFE16876CBB627E85AD2B8353D9243AB4DC1B2335 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
sha1_unaligned: size=2097152 sha256=D4F2663B5559490524200E1EE663124617F0B88911A992A6F6347379F00AE93B cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
sha1_deep: size=100663296 sha256=7D2CD9EC146E8448710336216FFF8536FD14BAA9564FB9724A113D4BCC781A9C cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL

test result: ok. 1 passed; 0 failed; 0 ignored; ... finished in 67.86s
```

sha256 cũ giữ nguyên (chống regression); 3 case sha1 mới dùng salt 20 byte và có
case 3 tầng (70 MiB / 96 MiB). Toàn file IDENTICAL với avbtool.

---

## 7. HARNESS ẢNH THẬT A3s

Nguồn: `E:\ROM\a3s volte\vendor.img` (1.106.640.896 byte; sha1; FEC 2 root).
Ảnh gốc chỉ đọc; hai bản copy trong temp.

```text
real_image: size=1106640896 sha256=F019A521A9D27881E2CF9D37C511182B64102AE7CC8F3F1D2D79EE2FB942C7E2 cmp=IDENTICAL info_image=MATCH verify_image=exit=0 idempotent=IDENTICAL
test result: ok. 1 passed; 0 failed; ... finished in 446.18s
```

Lần chạy đầu harness chết `STATUS_STACK_OVERFLOW` vì helper so file đặt hai buffer
1 MiB trên stack của test thread 2 MiB — đã đổi sang buffer heap (`vec!`). Đây là
lỗi của test helper, không phải lỗi sản phẩm.

---

## 8. GATE CLI ẢNH THẬT (NATIVE VS PYTHON)

Chạy trên hai bản copy; `--out` riêng; exe release trong repo root (đã xoá sau gate).

```text
[AVB] backend=native        exit=0  [DONE] Total time: 01:57
[AVB] backend=python        exit=0  [DONE] Total time: 01:41
```

Chỉ tiêu ổn định **giống nhau** ở hai bên:

```text
Hash Algorithm:           sha1
[OK] Old AVB footer replaced.
Size=1106640896  (0x41F60000)
e2fsck=PASS
AVB_HASHTREE=PASS
VOLTE_FLAGS=1
AVB_ALGORITHM=NONE
APK_SHA256=1010C1C7855C0150C204C7E6376F665FCB3B55C609BDFFAFA8114B0952B81694
RC_SHA256=E34F6321B8BDE3421A529E1E3BE9FA81082D422E26C4BA7D74AB5289F355A517
```

`avbtool.py info_image` hai output giống nhau **trừ** `Root Digest`
(`2a19606ed0bf2af0943554bcee17290702c886d4` vs `af2d56dbc08d36401209c76ae933bb9f732b62b4`).
`verify_image`:

```text
native: vbmeta: Successfully verified footer and NONE vbmeta struct
        vendor: Successfully verified sha1 hashtree ... for image of 1089097728 bytes
python: vbmeta: Successfully verified footer and NONE vbmeta struct
        vendor: Successfully verified sha1 hashtree ... for image of 1089097728 bytes
both exit=0
```

Hai output là hai lần build độc lập: `cmp` lệch từ byte 329485 (trong vùng nội
dung ext4, trước 1.089.097.728), nên SHA toàn file khác nhau
(`f7b00c43…` vs `535af61d…`) và root digest khác theo. Đây là dao động ext4 đã
biết từ nghiệm thu A3s, nên gate so chỉ tiêu ổn định thay vì SHA toàn ảnh.

Lưu ý: kế hoạch §4.4 ghi hex slot `0x41F80000`; giá trị thật của 1.106.640.896 là
`0x41F60000` (kế hoạch gõ nhầm hex, decimal và mọi gate dùng decimal đều khớp).

---

## 9. ĐỐI CHIẾU DONE GATES (kế hoạch §6)

| # | Gate | Trạng thái |
|---|---|---|
| 1 | SHA-1 pass NIST vectors | ✅ 8/8 |
| 2 | Test mặc định offline: fmt/check/clippy/test | ✅ 30 CLI + 280 core pass, 2 ignored |
| 3 | Harness sha256 cũ + 3 case sha1 | ✅ 7/7 IDENTICAL, info MATCH, verify PASS, idempotent |
| 4 | Harness ảnh A3s | ✅ IDENTICAL, info MATCH, verify PASS, idempotent; size + SHA ghi ở §7 |
| 5 | Gate CLI native vs python | ✅ cùng token, đúng slot, verify PASS, root digest khác do ext4 |
| 6 | `pipeline.rs` giữ `17adbce4…`; không file ngoài phạm vi | ✅ |
| 7 | Build release + SHA-256 exe | ✅ `0e1eae14…`, 1.266.176 B |
| 8 | Không commit/push, không flash/reboot, backend mặc định `python` | ✅ |

---

## 10. GHI NHẬN / CHỜ DUYỆT

1. **Chưa commit/push.** Tài liệu: `reports/phase-native-avb-sha1.md` (mới),
   `reports/plan-native-avb-sha1.md` (đổi STATUS).
2. **Backend mặc định vẫn `python`.** Đổi mặc định là quyết định riêng sau khi
   duyệt report này.
3. FEC vẫn bị bỏ khi rebuild (cả hai backend truyền `--do_not_generate_fec`), đúng
   phạm vi; ảnh A3s gốc có FEC nhưng output gate không có, và `verify_image` PASS.
4. Output test còn ở `E:\Build_app\Volte\_tmp_regress\sha1_impl\` (khoảng 2,2 GB)
   để đối chiếu; chưa xoá.
5. Không flash/root/reboot thiết bị nào.

---

## 11. KẾT LUẬN

`NativeAvb` giờ hỗ trợ `sha1` + `sha256`, tái hiện byte-for-byte output của
`avbtool.py` 1.3.0 trên 7 layout tổng hợp (kể cả 3 tầng, salt 20 byte) và trên
ảnh A3s thật 1,1 GB; `verify_image` PASS, idempotent. Full CLI với backend native
cho cùng kết quả boot-safe như backend python. `sha512`/`blake2b-256` vẫn hard-fail
trước khi ghi. `pipeline.rs` không đổi.

**Chờ duyệt report này trước khi commit hoặc đổi backend mặc định.**
