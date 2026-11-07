# PHASE 2.5 — PARITY CHECK HAI BACKEND EXT4 (DEBUGFS ↔ RUSTFS)

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline của đợt:** `006e3f0` (Đợt 2: báo cáo bổ sung ea_set và xác nhận DebugfsBackend lenient)
**Commit harness của đợt:** `71bacac` — `volte-core/examples/parity.rs`
**Trạng thái đợt:** ĐO XONG — DỪNG chờ người dùng xác nhận; **chưa xóa bất kỳ dòng code debugfs nào**

---

## 1. Mục tiêu

Đo parity giữa `DebugfsBackend` (oracle, gọi `tool/debugfs.exe`) và `RustFsExt4Backend` (backend
thuần Rust, mặc định của CLI tại `volte-cli/src/main.rs:180`) trên hai ảnh vendor thật, để có bằng
chứng trước khi xóa code debugfs ở bước sau. Đợt này **chỉ đo và báo cáo**; không sửa pipeline,
không sửa backend, không xóa code.

## 2. Ràng buộc và phạm vi

- Chỉ làm việc trên **bản sao** trong `E:\Build_app\Volte\_work\p25_parity\`; ảnh gốc dưới `E:\ROM\`
  chỉ được mở để băm SHA-256 (read-only), không ghi.
- Không xóa file nguồn nào trong repo; không sửa test; không nới lỏng test.
- Không flash, không kết nối thiết bị thật; mọi thao tác là trên file ảnh trên đĩa.
- Không thay đổi `DebugfsBackend` hay `RustFsExt4Backend` trong đợt này.
- Không dùng `DebugfsBackend.run()` / `run_best_effort` lenient làm tiêu chuẩn thành công; mọi kết
  luận dựa trên ảnh kết quả đọc lại độc lập (manifest + e2fsck + avbtool + `ea_list`), đúng như
  xác nhận ở `reports/fixes/PHASE_2_EXT4_BACKEND.md` §2.5.

## 3. Tiêu chí và phương pháp

Tiêu chí áp dụng giống nhau cho cả hai thiết bị và cả hai backend:

1. **Build**: pipeline chạy hết, `result=OK`, file ra đúng kích thước slot.
2. **Nội dung + metadata**: manifest đệ quy toàn ảnh (đường dẫn, loại, mode, uid, gid, size,
   SHA-256 nội dung file / đích symlink) — hai backend phải trùng khớp tuyệt đối, trừ mtime.
3. **e2fsck**: `e2fsck -fn` trả rc 0 hoặc 1 trên mọi ảnh ra, và bản tóm tắt giống nhau.
4. **AVB**: `avbtool info_image` / `verify_image` trên ảnh ra; với ảnh có footer phải verify PASS.
5. **SELinux xattr**: `debugfs -R "ea_list <path>"` trên mọi đường dẫn bị build chạm tới — phải
   giống nhau từng byte.
6. **Hash toàn ảnh**: ghi nhận và báo cáo, **không** dùng làm tiêu chí pass (xem §14).

Công cụ đo là `volte-core/examples/parity.rs` (commit `71bacac`), hai lệnh:

```text
parity build <debugfs|rustfs> <src> <out> <qc|mtk> <root> <logs> <tool>
parity manifest <image> <out.tsv>
```

`build` dựng pipeline thật với `E2fsck` thật và `PythonAvbtool` thật (python bundled trong
`tool/python`), chọn backend theo tham số. `manifest` mount ảnh read-only bằng `fs_ext4`
(`FileDevice::open_path` + `Filesystem::mount`), duyệt từ inode 2, băm nội dung từng file.

## 4. Fixture, nguồn gốc và tính toàn vẹn

| Thiết bị | Nhánh | Ảnh gốc (chỉ đọc) | Kích thước | SHA-256 gốc = SHA-256 bản sao |
|---|---|---|---|---|
| OPPO A3s (CPH1803) | QC | `E:\ROM\a3s volte\vendor.img` | 1.106.640.896 B | `d87934e9a69ea7cc93489a19110e5ab546ce5216548ad5a11190bb8ec9fb5618` |
| F11 (MediaTek) | MTK | `E:\ROM\f11 A.34\vendor.img` | 1.124.073.472 B | `12bebea979aa2e5de6e2bb33096957de1ac093ba9a3838117f95c0efe24d1128` |

Bản sao dùng để đo: `_work\p25_parity\a3s\src.img` và `_work\p25_parity\f11\src.img`, hash trùng
khớp ảnh gốc. Bằng chứng: `_work\p25_parity\provenance.txt`, `originals_readonly.sha256`,
`hashes.txt`. (`E:\ROM\f11 A.34\dump\f11 khanh\vendor.img` là ảnh khác, hash
`90f2556f…eaff`, không dùng.)

## 5. Môi trường đo

- Windows 10 x64, Git Bash; workspace `E:\Build_app\Volte\VoLTE-Vendor-Builder`.
- `e2fsck 1.44.5 (15-Dec-2018)`, `debugfs` cùng bộ `tool/`.
- `avbtool.py` bundled, Release String trong footer: `avbtool 1.3.0`.
- `rust-fs-ext4` git rev `1aaea079…` (theo `Cargo.toml`).
- Payload thật trong `payload/`: APK `OPPO_VoLTE_Overlay.apk` (12.767 B), `overlay.bin` /
  `config.bin` (nhãn SELinux 34 B), `hd_oppo_qc_volte.rc`, `hd_oppo_mtk_volte.rc`,
  `hd_volte_features.xml`.
- Log build ghi `APK_SHA256=1010C1C7…B81694` và `RC_SHA256` (QC `F0044C6C…97AD4`, MTK
  `B8FE93CE…8E68D`) giống nhau giữa hai backend trong cùng thiết bị.

## 6. Kết quả build

Lệnh (ví dụ A3s; F11 thay `qc` bằng `mtk`):

```bash
cargo run --offline --example parity -- build debugfs \
  a3s/src.img a3s/out_debugfs.img qc \
  E:/Build_app/Volte/VoLTE-Vendor-Builder a3s/logs_debugfs \
  E:/Build_app/Volte/VoLTE-Vendor-Builder/tool
```

| Thiết bị | Backend | Kết quả | Thời gian | Kích thước ra |
|---|---|---|---|---|
| A3s | debugfs | OK | 393,3 s | 1.106.640.896 B |
| A3s | rustfs | OK | 408,6 s | 1.106.640.896 B |
| F11 | debugfs | OK | 339,4 s | 1.124.073.472 B |
| F11 | rustfs | OK | 360,7 s | 1.124.073.472 B |

Kích thước ra bằng đúng slot nguồn (A3s `0x41F60000`, F11 `0x43000000`), khớp dòng
`[OK] STANDARD size matches the slot` / `[OK] Size BOOT-SAFE matches slot` ở cả 4 log.
Bằng chứng: `<device>/build_debugfs.log`, `<device>/build_rustfs.log`.

## 7. So sánh nội dung và metadata (manifest)

So khớp từng trường `type, mode, uid, gid, size, sha256` trên toàn bộ cây; mtime ghi nhận riêng.

| Thiết bị | Số mục (mỗi bên) | only_left/right | type | mode | uid | gid | size | sha256 | mtime | Kết luận |
|---|---|---|---|---|---|---|---|---|---|---|
| A3s | 3.060 | 0 / 0 | 0 | 0 | 0 | 0 | 0 | 0 | 9 | IDENTICAL_CONTENT_METADATA |
| F11 | 5.425 | 0 / 0 | 0 | 0 | 0 | 0 | 0 | 0 | 10 | IDENTICAL_CONTENT_METADATA |

Nghĩa là: cùng tập đường dẫn, cùng loại inode, cùng mode/uid/gid/size, và **mọi file có nội dung
trùng khớp byte-for-byte** (SHA-256 từng file giống nhau). Bằng chứng: `compare.txt`,
`<device>/out_*.manifest.tsv`, `compare.py`.

## 8. Khác biệt mtime (đã kiểm tra từng đường dẫn)

A3s — 9 đường dẫn (đều là file/thư mục bị build ghi):

```text
/build.prop, /default.prop,
/etc/init/hd_oppo_qc_volte.rc, /etc/init/hw/hd_oppo_qc_volte.rc,
/etc/init/hw/init.target.rc, /etc/permissions/hd_volte_features.xml,
/overlay/HDVoLTE, /overlay/HDVoLTE/OPPO_VoLTE_Overlay.apk, /overlay/OPPO_VoLTE_Overlay.apk
```

F11 — 10 đường dẫn (tương tự, nhánh MTK, thêm `init.project.rc`; không có `init.target.rc`):

```text
/build.prop, /default.prop,
/etc/init/hd_oppo_mtk_volte.rc, /etc/init/hd_oppo_qc_volte.rc, /etc/init/hw/hd_oppo_mtk_volte.rc,
/etc/init/hw/init.project.rc, /etc/permissions/hd_volte_features.xml,
/overlay/HDVoLTE, /overlay/HDVoLTE/OPPO_VoLTE_Overlay.apk, /overlay/OPPO_VoLTE_Overlay.apk
```

Độ lệch mtime xấp xỉ khoảng cách giữa hai lần chạy (A3s ~408 s, F11 ~345 s): cả hai engine đóng
dấu thời gian hiện tại của lần chạy lên các file bị ghi. **3.051 / 5.415 mục còn lại có mtime giống
nhau tuyệt đối.** Bằng chứng: mục `MTIME` trong `compare.txt`.

## 9. Đối chiếu source → output (tập thay đổi của build)

Tập thay đổi do pipeline tạo ra là **giống nhau giữa hai backend** trên cùng thiết bị:

| Thiết bị | Đường dẫn mới | Mode đổi | Size đổi | Nội dung đổi |
|---|---|---|---|---|
| A3s | 6 | 2 (`build.prop`, `default.prop`: 100600 → 100644) | 3 (`build.prop` 3349→5685, `default.prop` 639→2975, `init.target.rc` 12490→12583) | 3 (3 file trên) |
| F11 | 7 | 2 (như trên) | 3 (`build.prop` 7343→9474, `default.prop` 280→2616, `init.project.rc` 1926→2021) | 4 (3 file trên + `/lib64/libmtk-ril.so`; size giữ nguyên, hash đổi) |

Đường dẫn mới A3s: 2 file `hd_oppo_qc_volte.rc` (init + init/hw), `hd_volte_features.xml`,
`/overlay/HDVoLTE`, `/overlay/HDVoLTE/OPPO_VoLTE_Overlay.apk`, `/overlay/OPPO_VoLTE_Overlay.apk`.
F11 thêm `hd_oppo_mtk_volte.rc` (init + init/hw). Hash sau build của các file bị đổi **trùng nhau
giữa hai backend** (ví dụ `libmtk-ril.so`: `3CDAD3B5…D5D949` → `11EFCCC4…6FD449` ở cả hai).
Bằng chứng: các khối `=== <device> src vs debugfs/rustfs` trong `compare.txt`.

## 10. e2fsck

`e2fsck -fn` (read-only) trên cả 4 ảnh ra:

```text
a3s/out_debugfs.img rc=0
a3s/out_rustfs.img rc=0
f11/out_debugfs.img rc=0
f11/out_rustfs.img rc=0
```

Tóm tắt Pass 5 giống nhau từng chữ:

- A3s: `vendor: 3070/66528 files (18.8% non-contiguous), 239262/265893 blocks`
- F11: `vendor: 5435/68688 files (0.9% non-contiguous), 250206/274432 blocks`

Bằng chứng: `e2fsck_rc.txt`, `<device>/out_*.e2fsck.txt`.

## 11. AVB — OPPO A3s (nhánh QC, ảnh có footer gốc)

| Thuộc tính | Source | out_debugfs | out_rustfs |
|---|---|---|---|
| Footer | 1.0 | 1.0 | 1.0 |
| Image size | 1.106.640.896 | 1.106.640.896 | 1.106.640.896 |
| Original image size | 1.089.097.728 | 1.089.097.728 | 1.089.097.728 |
| VBMeta offset | 1.106.366.464 | 1.097.682.944 | 1.097.682.944 |
| VBMeta size | 512 | 512 | 512 |
| FEC num roots | 2 (size 8.683.520) | 0 | 0 |
| Hash / partition | sha1 / vendor | sha1 / vendor | sha1 / vendor |
| Salt | `8ad963d3…4f6e` (20 B) | `28f66193…ec6b` (32 B) | `a2e2e4f4…0b64` (32 B) |
| Release string | avbtool 1.0.0 | avbtool 1.3.0 | avbtool 1.3.0 |

`avbtool verify_image`:

- **out_debugfs: PASS** — `vbmeta: Successfully verified footer and NONE vbmeta struct` +
  `vendor: Successfully verified sha1 hashtree … for image of 1089097728 bytes`.
- **out_rustfs: PASS** — cùng hai dòng, cùng kích thước 1.089.097.728 B.
- Source: FAIL — `hashtree of … does not match descriptor` (footer gốc vốn không tự verify; đây là
  hành vi của ảnh nguồn, không phải lỗi của build).

Log build cả hai backend: `[AVB] Rebuild hashtree completed.`, `AVB_HASHTREE=PASS`, `FLASH=NO`.
Salt khác nhau vì `avb_info_fast` không trích được salt từ footer (chỉ đọc vbmeta), nên pipeline
sinh salt ngẫu nhiên mỗi lần chạy — khác biệt này là **dự kiến**, không phải sai lệch backend.
Bằng chứng: `<device>/*.avbinfo.txt`, `verify/a3s_*/verify.txt`, `verify.txt`.

## 12. AVB — F11 (nhánh MTK, ảnh đóng gói đầy slot)

- Source, `out_debugfs`, `out_rustfs`: **không có footer AVB**;
  `avbtool info_image` trả rc≠0 với `Given image does not look like a vbmeta image.` trên cả ba.
- Log build cả hai backend giống nhau:
  `avbtool.py failed (1): … Image size of 1124073472 exceeds maximum image size of 1115148288 in
  order to fit in a partition size of 1124073472..` → `Hashtree does not fit the slot - keeping the
  original EXT4 overlay.` → `Could not create a real AVB footer - keeping the original footer
  (avoid bootloop).` → `AVB_HASHTREE=SKIP`, `FLASH=NO`.
- Đây là nhánh boot-safe SKIP có chủ đích của pipeline, xảy ra **giống hệt** ở hai backend.

## 13. SELinux xattr trên mọi đường dẫn bị chạm

`debugfs -R "ea_list <path>"` cho toàn bộ đường dẫn bị build ghi: A3s 9 đường dẫn (27 dòng bằng
chứng), F11 11 đường dẫn (33 dòng). `diff` giữa hai backend: **IDENTICAL** cho cả hai thiết bị.

Nhãn quan sát được (ví dụ A3s): overlay/APK `u:object_r:vendor_overlay_file:s0`,
rc/features/`init.target.rc` `u:object_r:vendor_configs_file:s0`, `build.prop`/`default.prop`
`u:object_r:vendor_file:s0`. Bằng chứng: `<device>/xattr_debugfs.txt` vs `xattr_rustfs.txt`.

## 14. Superblock và hash toàn ảnh

Superblock (`debugfs -R stats`), các trường tổng **giống nhau**: UUID, features, Inode count,
Block count, Free blocks, Free inodes.

| Thiết bị | Free blocks | Free inodes | Khác biệt còn lại |
|---|---|---|---|
| A3s | 26.631 = 26.631 | 63.458 = 63.458 | phân bố free block/inode giữa các group; `Last write time`; checksum group |
| F11 | 24.226 = 24.226 | 63.253 = 63.253 | như trên + `Lifetime writes` 978 MB (debugfs) vs 977 MB (rustfs) |

`Last write time` của ảnh debugfs = thời điểm build; của ảnh rustfs = giá trị cũ trong superblock
(2021) — RustFs không cập nhật `s_mtime` sau khi ghi. Tổng free space/inode không đổi; chỉ cách
phân bổ giữa các block group là khác.

Hash toàn ảnh (SHA-256):

| File | SHA-256 |
|---|---|
| `a3s/src.img` | `d87934e9a69ea7cc93489a19110e5ab546ce5216548ad5a11190bb8ec9fb5618` |
| `a3s/out_debugfs.img` | `92f3f32845e7ab08224b990beb9df0fd342d2d68e89dde4d7fde803d5a4aad86` |
| `a3s/out_rustfs.img` | `ded5d5bbb70e4561dd6eee55357a0582577c57b45c3dd808f8ac28e901055405` |
| `f11/src.img` | `12bebea979aa2e5de6e2bb33096957de1ac093ba9a3838117f95c0efe24d1128` |
| `f11/out_debugfs.img` | `ce986981a5a29064e19d00e15a0e6c300f40746b8c235e6cd7a902f556be0649` |
| `f11/out_rustfs.img` | `b866f94ebf8ae77f9d120cb034b466b759f1f004f25bdc33f1fca74438a56e39` |

**Hash toàn ảnh KHÔNG trùng** (kể cả hai output cùng thiết bị). Nguyên nhân đã kiểm chứng: mtime
của 9/10 file bị ghi, phân bố block/inode giữa group, `Last write time` (và `Lifetime writes` ở
F11), và salt AVB ngẫu nhiên ở A3s. Chưa đo: số inode, bố cục directory block, nội dung journal —
nên chưa giải thích byte-level đầy đủ. Bằng chứng: `hashes.txt`, `<device>/sb_*.txt`.

## 15. Kết luận parity, rủi ro còn lại và giới hạn

**ĐẠT parity trên các tiêu chí đã định:**

- Nội dung + metadata (type/mode/uid/gid/size/SHA-256 từng file): trùng khớp 100% (3.060 và 5.425
  mục), chỉ khác mtime trên đúng các file bị build ghi.
- Tập thay đổi source → output: giống nhau từng đường dẫn và từng hash.
- e2fsck rc=0 và tóm tắt giống nhau trên cả 4 ảnh.
- AVB: A3s rebuild + verify PASS ở cả hai backend; F11 SKIP giống hệt ở cả hai.
- SELinux xattr: giống nhau tuyệt đối trên mọi đường dẫn bị chạm.

**KHÔNG đạt byte-identical toàn ảnh** — và điều này không phải tiêu chí của đợt (debugfs cũng không
tạo ra byte giống nhau giữa hai lần chạy do mtime và salt ngẫu nhiên).

Giới hạn/rủi ro còn lại:

1. Chỉ đo 2 thiết bị / 2 cấu trúc đặc thù (QC có footer + FEC; MTK đầy slot, không footer).
2. Chưa chạy trên máy thật; chưa kiểm tra khả năng boot (ngoài phạm vi và bị cấm).
3. Manifest không so số inode, thứ tự directory entry, journal, inode generation — các khác biệt
   này tồn tại (thấy qua hash toàn ảnh) nhưng không ảnh hưởng nội dung.
4. RustFs không cập nhật `s_mtime`/`Lifetime writes` như debugfs; e2fsck vẫn sạch.
5. F11 không có AVB footer ở cả nguồn và đích; parity AVB của F11 chỉ là "cùng SKIP", không phải
   "cùng footer hợp lệ".

## 16. Trạng thái đợt và đề xuất bước tiếp theo

- Đợt 2.5 **chưa hoàn tất việc xóa code**: `DebugfsBackend`, `tool/debugfs.exe` và các tham chiếu
  vẫn còn nguyên. **Không có dòng code debugfs nào bị xóa trong đợt này.**
- Đã commit: `71bacac` (harness `volte-core/examples/parity.rs`). Báo cáo này được commit riêng;
  hash xem `git log --oneline -2`.
- Gate sản phẩm tại thời điểm đo: 165 test PASS (155 core + 10 CLI), `cargo clippy --workspace
  --all-targets -- -D warnings` sạch, `cargo fmt --check` không phát sinh drift mới (112 block
  baseline).
- **DỪNG tại đây.** Cần người dùng xác nhận một trong hai hướng trước khi tiếp tục:
  1. Chấp nhận parity theo nội dung/metadata/e2fsck/AVB/xattr (không đòi byte-identical) → cho
     phép xóa `DebugfsBackend` + `tool/debugfs.exe` và cập nhật test/harness liên quan.
  2. Yêu cầu thêm bằng chứng (ví dụ thêm thiết bị, hoặc đo sâu inode/directory layout) trước khi
     quyết định.
