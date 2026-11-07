# PHASE 3 — AVB, SPARSE VÀ TRẠNG THÁI PIPELINE

**Ngày:** 2026-11-07
**Nhánh:** `ai2/fix-audit-findings`
**Baseline của đợt:** `5819c01` (HEAD sau khi đóng Đợt 2.5: `d444e43` hoàn nguyên `CLAUDE.md`,
`5819c01` bổ sung §17/§18 vào `PHASE_2_5_DEBUGFS_PARITY.md`)
**Commit của đợt:** `3fae1b7` — `Đợt 3: sửa false-success AVB/e2fsck và sparse DONT_CARE cuối file`
**Trạng thái:** HOÀN THÀNH — không nằm trong checkpoint bắt buộc, được phép nối sang Đợt 4

---

## 1. Mục tiêu

Đảm bảo chương trình không xác nhận ảnh đầu ra hợp lệ khi một công đoạn bắt buộc thất bại.
Bốn finding của đợt:

| Finding | Mức | Trạng thái audit | Kết quả đợt này |
|---|---|---|---|
| D01 | Critical | Reproduced | Đã sửa — backend `Err` không còn được báo thành công |
| A04 | High | Reproduced | Đã sửa — e2fsck `Err` làm pipeline trả `Err`, CLI exit 2 |
| B01 | High | Reproduced | Đã sửa — output sparse được mở rộng đúng `total_blks × blk_sz` |
| N05 | Low | Confirmed (static) | 1 phần độc lập đã sửa; 2 phần gộp vào D01/A04 (có bằng chứng ở §6) |

Nguyên tắc xuyên suốt: **truthful success** — không `[OK]` từ hằng số, không `[OK]` trước khi
thao tác tương ứng thành công và đã được kiểm chứng; trạng thái PASS/FAIL/NOTRUN phải phản ánh
đúng kết quả thực tế.

## 2. Baseline, phạm vi, môi trường

- Baseline code: `5819c01` (xác nhận bằng `git log --oneline -2` ngay trước commit đợt này).
- Phạm vi code: `volte-core/src/{avb,report,pipeline,sparse,ext4_tools}.rs`,
  `volte-cli/src/main.rs`, `README.md`, `docs/PORT_STATUS.md`.
- Không đụng `CLAUDE.md` (đã hoàn nguyên và giữ nguyên từ `d444e43`).
- Không đổi thuật toán AVB: tham số avbtool đóng băng (`--algorithm NONE`, block 4096,
  `--do_not_generate_fec`, `--fec_num_roots 2`, salt ngẫu nhiên khi `info.salt` rỗng) giữ nguyên.
- Không thêm dependency; chỉ dùng `std` và các module sẵn có (`crate::sha256`).

## 3. D01 — AVB backend lỗi nhưng footer cũ vẫn báo thành công

### 3.1 Vị trí và nguyên nhân (baseline `5819c01`)

Audit chỉ đúng vùng `volte-core/src/avb.rs:398-416`. Code baseline:

```rust
let last_err = backend.add_hashtree_footer(&req).err();   // lỗi bị nuốt vào biến

let after = avb_info_fast(image)?;
if after.has_footer {                                     // footer CŨ vẫn còn -> true
    ...
    log.log("[AVB] Rebuild hashtree completed.");         // báo thành công
    return Ok(Some(after));
}
if let Some(e) = last_err { log.log(format!("[WARN] {}", truncate_err(&e))); }
```

`after.has_footer` được kiểm tra **trước** `last_err`, nên khi backend trả `Err` trên ảnh đã có
footer cũ, hàm vẫn trả `Ok(Some(after))` và log "Rebuild hashtree completed.". Python reference
có cùng lỗi (`hd_vendor_build.py:1357-1372`: kiểm tra `has_footer` trước `last_err`).

### 3.2 Cách sửa

1. **Lan truyền lỗi backend**: `backend.add_hashtree_footer(&req)?;` — không còn `last_err`.
2. **Chứng minh footer mới thay footer cũ**: trước khi gọi backend, băm vùng vbmeta cũ
   (`vbmeta_digest`, `avb.rs:441-454`); sau khi gọi, nếu ảnh vẫn có footer thì bắt buộc
   offset/kích thước phải khớp **và** digest phải khác. Vùng vbmeta không đổi ⇒ `Err`:

```rust
let old_vbmeta = if info.has_footer && info.vbmeta_size > 0 {
    Some(vbmeta_digest(image, info.vbmeta_offset, info.vbmeta_size)?)
} else {
    None
};
backend.add_hashtree_footer(&req)?;

let after = avb_info_fast(image)?;
if after.has_footer {
    if let Some(before) = old_vbmeta {
        let unchanged = after.vbmeta_offset == info.vbmeta_offset
            && after.vbmeta_size == info.vbmeta_size
            && vbmeta_digest(image, after.vbmeta_offset, after.vbmeta_size)? == before;
        if unchanged {
            return Err(Error::build(
                "AVB backend returned Ok but the vbmeta region is unchanged - \
                 the old footer was kept, so the rebuild is not verified",
            ));
        }
    }
    ...
```

3. **Chỉ log `[OK]` sau khi đã xác minh**: `[OK] Old AVB footer removed.` in trước lời gọi backend
   đã bị xóa; dòng `[OK] Old AVB footer replaced.` được đặt sau kiểm tra digest và kiểm tra
   kích thước (`avb.rs:426`), ngay trước `[AVB] Rebuild hashtree completed.` (`avb.rs:427`).
4. Xóa helper `truncate_err` không còn dùng; cập nhật doc comment của `rebuild_hashtree`
   (`avb.rs:333-340`) mô tả đúng ba kết cục `Ok(Some)` / `Ok(None)` / `Err`.

Digest luôn khác sau rebuild thật vì `avb_info_fast` không trích salt, nên `rebuild_hashtree`
sinh salt ngẫu nhiên (`random_salt_hex`) — footer mới mang salt mới. Phép so sánh chỉ chạy khi
footer cũ có vbmeta (`vbmeta_size > 0`) và offset/kích thước không đổi; mọi trường hợp khác vẫn
đi qua kiểm tra kích thước slot như trước.

### 3.3 Hành vi sau sửa

- Backend `Err` ⇒ `rebuild_hashtree` trả `Err` ⇒ pipeline log
  `[WARN] AVB hashtree: <lý do>`, đặt `avb_ok = false`, khôi phục snapshot `vendor_pre_avb.img`,
  khóa size theo slot và báo `AVB_HASHTREE=SKIP`. Không có PASS, không có "completed",
  không có `[OK] AVB ...`.
- `boot_safe = avb_ok && footer_ok && slot_size > 0 && cur == slot_size`
  (`pipeline.rs:528`) chỉ có thể `true` khi rebuild đã được xác minh, vì `avb_ok` giờ chỉ `true`
  khi `rebuild_hashtree` trả `Ok(Some)`.
- Không thay đổi tham số/hành vi avbtool ở đường thành công.

## 4. A04 — e2fsck thất bại nhưng pipeline trả `Ok`

### 4.1 Vị trí và nguyên nhân

Audit: `pipeline.rs:481-492`, `552-571`; `report.rs:72`. Baseline:

```rust
let mut fsck_status = "PASS";
match deps.fsck.check(&work, td) {
    Ok(out) => { ... log.log("e2fsck=PASS"); }
    Err(e) => { fsck_status = "FAIL"; log.log(format!("[WARN] e2fsck: {}", ...)); }
}
...
"[OK] EDL size matches - UnlockTool can flash it.".to_string(),   // report.rs:72, hằng số
```

Lỗi e2fsck chỉ đổi token thành `FAIL` và **tiếp tục chạy** AVB, in tổng kết mở đầu bằng `[OK]`
và trả `Ok`; `volte-cli` do đó exit 0. Đây là false success nguy hiểm nhất trong nhóm vì ảnh
hỏng filesystem vẫn được quảng cáo là flashable.

### 4.2 Cách sửa

**Pipeline — fail-fast trước AVB** (`pipeline.rs:496-510`):

```rust
let fsck_status = match deps.fsck.check(&work, td) {
    Ok(out) => {
        log_lines(log, &out);
        log.log("e2fsck=PASS");
        report::FsckStatus::Pass
    }
    Err(e) => {
        log.log(format!("[ERROR] e2fsck: {}", trunc(e.message(), 200)));
        log.log("e2fsck=FAIL");
        return Err(Error::build(format!(
            "e2fsck failed - refusing to report the image as flashable: {}",
            trunc(e.message(), 200)
        )));
    }
};
```

Dừng ngay sau e2fsck (trước AVB) để không rebuild hashtree trên filesystem hỏng; không ghi
`vendor_VolLTE_report.txt`; không in bất kỳ dòng flashable nào.

**Report — render từ trạng thái, không từ hằng số** (`report.rs:56-75`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsckStatus { NotRun, Pass, Fail }

impl FsckStatus {
    pub fn token(self) -> &'static str {
        match self {
            FsckStatus::NotRun => "NOTRUN",
            FsckStatus::Pass => "PASS",
            FsckStatus::Fail => "FAIL",
        }
    }
}
```

`Summary.e2fsck` và `report_body(...)` nhận `FsckStatus` (`report.rs:82`, `134-153`). Dòng đầu
tổng kết chỉ là `[OK] EDL size matches...` khi `Pass`; ngược lại là
`[ERROR] e2fsck=<state> - the image is not confirmed flashable.` (`report.rs:93-101`).
Nhánh `UNIT`/`STANDARD` cũng chỉ phát câu "safe to boot" khi `Pass` (`report.rs:110-118`).
`NotRun` được định nghĩa để không bao giờ render thành PASS (dùng cho test và cho các đường
tương lai), dù pipeline hiện fail-fast nên trạng thái này không tới được phần tổng kết.

**CLI — exit code kiểm thử được** (`volte-cli/src/main.rs:195-211`):

```rust
let result = pipeline::run(&opts, &mut deps, &mut log, &mut progress);
match &result {
    Ok(out) => println!("{}{}", config::TAG_OK, out.display()),
    Err(e) => println!("{}{}", config::TAG_ERROR, e.message()),
}
exit_code(result)
...
fn exit_code(result: volte_core::error::Result<PathBuf>) -> i32 {
    if result.is_ok() { 0 } else { 2 }
}
```

**e2fsck thiếu executable** (`ext4_tools.rs:159-168`): lỗi spawn của Windows không nêu tên tool,
nên thông điệp được bọc lại thành `e2fsck (<đường dẫn exe>): <lỗi gốc>`, giữ nguyên `ErrorKind`
để test phân biệt được `Io` với lỗi kiểm tra filesystem thật.

### 4.3 Về tương thích Python reference

Python (`:1650-1655`) log `e2fsck=FAIL` rồi vẫn in `[OK] EDL size matches` và trả success.
Đợt này **không** thêm cờ opt-out để tái lập hành vi đó: e2fsck là bước bắt buộc, im lặng chấp
nhận rủi ro bằng cờ sẽ tạo lại đúng lỗi A04. Khác biệt được ghi nhận là chủ đích ở
`docs/PORT_STATUS.md` (ID 29, mục (d)) và `README.md`; hành vi cũ vẫn truy được qua git.

## 5. B01 — Sparse `DONT_CARE` cuối file làm mất phần đuôi

### 5.1 Vị trí và nguyên nhân

Audit: `sparse.rs:109-113`. Baseline:

```rust
DONT_CARE => {
    let n = csize * blk_sz;
    out.seek(SeekFrom::Current(n as i64))?;
    written += n;
}
```

`BufWriter::seek` chỉ dời vị trí ghi; không có lần `write` nào sau đó thì file không được mở rộng.
Kết quả: `unsparse` trả `written = total_blks × blk_sz` nhưng file trên đĩa ngắn hơn (ảnh toàn
`DONT_CARE` ra file 0 byte). Python reference cũng chỉ seek (`:504-507`).

### 5.2 Cách sửa (`sparse.rs:125-142`)

```rust
let expected = total_blks
    .checked_mul(blk_sz)
    .ok_or_else(|| Error::build("sparse header declares an overflowing image size"))?;
if written != expected {
    return Err(Error::build(format!(
        "sparse chunks cover {written} bytes but the header declares {expected} \
         ({total_blks} blocks x {blk_sz})"
    )));
}
out.flush()?;
let file = out.get_ref();
file.set_len(written)?;
let actual = file.metadata()?.len();
if actual != written {
    return Err(Error::build(format!(
        "sparse output is {actual} bytes after conversion, expected {written}"
    )));
}
```

- `total_blks` đọc từ header (offset 16..20) và nhân có kiểm tràn.
- Header khai báo kích thước khác tổng chunk ⇒ `Err` thay vì im lặng trả success.
- `set_len(written)` mở rộng file cho hole cuối (trên Windows, vùng mở rộng đọc ra zero).
- Đọc lại `metadata().len()` để xác nhận đúng kích thước vật lý; sai ⇒ `Err`.
- Chunk `DONT_CARE` không ở cuối vẫn giữ hành vi seek tạo hole; các write sau đó ghi tiếp đúng chỗ.

## 6. N05 — khảo sát toàn văn báo cáo audit và kết luận

Audit gộp N05 vào nhóm "Truthful success" ở khuyến nghị P0 #3
(`reports/AI2_INDEPENDENT_AUDIT.md:1049`). Nội dung đầy đủ của N05 ở
`reports/AI2_INDEPENDENT_AUDIT.md:838-857`, với 4 vị trí được nêu:
`pipeline.rs:383-384`, `pipeline.rs:526`, `report.rs:72`, `report.rs:82-84`.

Đối chiếu trên baseline `5819c01`:

| # | Dòng bị nêu | Vị trí thực trên baseline | Điều kiện phát | Kết luận |
|---|---|---|---|---|
| 1 | `[OK] Old AVB footer removed.` | `avb.rs:384` | In **trước** `add_hashtree_footer`, không phụ thuộc kết quả backend | **Lỗi độc lập** — đã sửa: xóa dòng trước lời gọi, chỉ in `[OK] Old AVB footer replaced.` sau khi digest+kích thước được xác minh |
| 2 | `[AVB] Rebuild hashtree completed.` | `avb.rs:412` | In khi `after.has_footer`, kể cả backend đã `Err` | **Trùng D01** — cùng root cause (`has_footer` kiểm trước lỗi backend), cùng vị trí code; sửa D01 xử lý luôn |
| 3 | `[OK] EDL size matches - UnlockTool can flash it.` | `report.rs:72` | Hằng số, in bất kể `e2fsck=FAIL` | **Trùng A04** — cùng root cause (fsck không được lan truyền), cùng vị trí render; `FsckStatus` xử lý luôn |
| 4 | `[OK] AVB hashtree + EDL size match - safe to boot.` | `pipeline.rs:541` (audit ghi `526`) | Có guard `boot_safe`, nhưng `avb_ok` bị D01 đánh lừa nên guard có thể `true` sai | **Trùng D01** — sau khi D01 sửa, guard chỉ `true` khi rebuild đã xác minh; không cần đổi thêm dòng này |

Kiểm tra toàn bộ `[OK]` còn lại trong mã sản phẩm (grep, loại dòng `assert` trong test):
`avb.rs:426`, `init_rc.rs:89,130`, `pipeline.rs:425,544,556`, `props.rs:227`,
`report.rs:95`, `slot.rs:23,57`, `verify.rs:135`. Từng dòng đều nằm trong nhánh đã kiểm tra
trạng thái thực tế:

- `slot.rs:23,57`: chỉ in khi `cur == slot` hoặc sau khi resize đọc lại đúng slot.
- `init_rc.rs:89`: chỉ khi mọi needle đã có trong file; `:130` sau khi batch lệnh `ext4` trả `Ok`.
- `props.rs:227`: chỉ trong nhánh `write_back` trả `Ok`.
- `pipeline.rs:425`: chỉ khi `init_rc::link` trả `Ok(true)`; `:544` theo `boot_safe`;
  `:556` chỉ khi `out_sz == slot_size` và không rơi vào nhánh lỗi.
- `verify.rs:135`: chỉ khi cả APK và RC có `Verdict::Verified`.
- `report.rs:95`: chỉ khi `FsckStatus::Pass` (mới).

**Kết luận N05:** không phải một lỗi đơn lẻ, cũng không trùng hoàn toàn. N05 gồm **1 lỗi độc lập**
(dòng `[OK] Old AVB footer removed.` in trước thao tác) đã sửa riêng, và **2 phần gộp** vào
D01/A04 vì cùng root cause và cùng vị trí code. Ghi nhận theo yêu cầu của brief:
"N05 — gộp chung với D01/A04" cho phần 2–4, "N05 — lỗi độc lập" cho phần 1.

Về khuyến nghị "state machine" của audit: đợt này hiện thực hoá bằng `FsckStatus` cho e2fsck và
bằng mô hình `Result<Option<...>>` + xác minh digest cho AVB, không dựng máy trạng thái toàn
pipeline (giữ thay đổi tối thiểu theo `AGENTS.md`).

## 7. Bảng thay đổi code

| File | Vùng | Thay đổi |
|---|---|---|
| `volte-core/src/avb.rs` | 333-340, 382, 396-415, 426-427, 441-454 | Lan truyền lỗi backend; xác minh digest vbmeta; dời `[OK]` sau thao tác; thêm `vbmeta_digest`; xóa `truncate_err` |
| `volte-core/src/report.rs` | 56-75, 79-131, 134-153 | `FsckStatus{NotRun,Pass,Fail}`; tổng kết và nội dung report render từ trạng thái |
| `volte-core/src/pipeline.rs` | 496-510, 571-587 | e2fsck `Err` ⇒ `Err` trước AVB; summary/report dùng enum |
| `volte-core/src/sparse.rs` | 60, 125-142 | Đọc `total_blks`; kiểm `written == total_blks×blk_sz`; `set_len`; xác nhận độ dài đĩa |
| `volte-core/src/ext4_tools.rs` | 159-168 | Bọc lỗi spawn e2fsck kèm đường dẫn exe |
| `volte-cli/src/main.rs` | 195-211 | Tách `exit_code(...)`, lỗi pipeline ⇒ 2 |
| `README.md` | 51-58 | Nêu rõ e2fsck lỗi ⇒ exit 2, không dòng flashable; AVB lỗi ⇒ SKIP, không PASS |
| `docs/PORT_STATUS.md` | 7-10, 24-45, 63-100 | Số test 163/11; ID 09/27 `diverges` + giải thích; ID 29 thêm khác biệt (d) |

Diff tổng: `8 files changed, 472 insertions(+), 46 deletions(-)`.

## 8. Regression tests

Test mới/viết lại (tất cả chạy qua backend mặc định `RustFsExt4Backend` hoặc seam trực tiếp,
không có `DebugfsBackend`):

| Finding | Test | Vị trí | Điều được chứng minh |
|---|---|---|---|
| D01 | `a_failing_backend_is_an_error_and_keeps_the_image_untouched` | `avb.rs:684` | Backend `Err` ⇒ `Err`; ảnh byte-identical, không đổi kích thước |
| D01 | `a_failing_backend_over_an_old_footer_is_never_reported_as_success` | `avb.rs:698` | Ảnh có footer cũ + backend `Err` ⇒ `Err`; log không có "completed"/`[OK] Old AVB footer`/`[OK] AVB` |
| D01 | `a_backend_that_returns_ok_without_rebuilding_is_an_error` | `avb.rs:735` | Backend `Ok` nhưng vbmeta không đổi ⇒ `Err` chứa "unchanged" |
| D01 | `a_verified_rebuild_reports_the_new_footer` | `avb.rs:753` | Rebuild thật ⇒ `Ok(Some)`, log `[OK] Old AVB footer replaced.` + "completed" |
| D01 | `an_avb_backend_failure_never_claims_a_passed_hashtree` | `pipeline.rs:1123` | Pipeline: `[WARN] AVB hashtree:`, `AVB_HASHTREE=SKIP`, không PASS, không "completed", không `[OK] AVB`; source không đổi, size không đổi |
| A04 | `an_e2fsck_failure_fails_the_build_before_avb` | `pipeline.rs:1083` | `Err` chứa "e2fsck failed"; fsck gọi 1 lần, AVB 0 lần; không `[OK] EDL size matches`, không token AVB/FLASH, không file report; source không đổi |
| A04 | `a_missing_e2fsck_executable_is_an_error` | `ext4_tools.rs:253` | Executable thiếu ⇒ `ErrorKind::Io`, thông điệp chứa "e2fsck" |
| A04 | `a_failed_fsck_is_never_rendered_as_flashable` | `report.rs:255` | `FsckStatus::Fail` ⇒ không dòng `[OK]` nào; dòng đầu là `[ERROR] e2fsck=FAIL ...`; nhánh AVB ghi "do not flash" |
| A04 | `a_never_run_fsck_is_never_rendered_as_flashable` | `report.rs:269` | `NotRun` ⇒ token `NOTRUN`, không `[OK]` |
| A04 | `a_pipeline_error_exits_non_zero` | `volte-cli/src/main.rs:335` | `Err` ⇒ exit 2; `Ok` ⇒ 0 |
| B01 | `a_trailing_dont_care_chunk_extends_the_output` | `sparse.rs:282` | Chunk cuối `DONT_CARE` ⇒ `written == 16384` **và** file dài 16384, toàn zero |
| B01 | `an_all_dont_care_image_is_zero_filled_to_the_declared_size` | `sparse.rs:298` | Ảnh toàn hole ⇒ file đúng `5 × 4096`, zero-filled |
| B01 | `an_image_without_dont_care_keeps_the_exact_length` | `sparse.rs:313` | RAW+FILL ⇒ đúng 3 block, không thay đổi hành vi cũ |
| B01 | `a_header_size_that_the_chunks_do_not_cover_is_an_error` | `sparse.rs:332` | Header khai 5 block nhưng chunk chỉ phủ 2 ⇒ `Err` "header declares" |

Không xóa/không nới lỏng test cũ. Test cũ `rebuild_keeps_original_when_backend_fails` được thay
bằng `a_failing_backend_is_an_error_and_keeps_the_image_untouched` vì hành vi đúng đã đổi từ
`Ok(None)` sang `Err` theo đúng yêu cầu D01.

Số test: **163** trong `volte-core` + **11** trong `volte-cli` (baseline 161 = 151 + 10;
+12 core, +1 CLI).

## 9. Đối chiếu repro harness gốc

Harness ngoài tại `%TEMP%\volte_audit_repro`, chỉ đọc public API của `volte-core`, không ghi vào
repo. Ba dòng đã được sửa để biên dịch sau Đợt 2.5 (import `ext4_tools`, `RustFsExt4Backend::new()`,
literal `"debugfs:"` → `"ext4:"`); bản gốc lưu ở
`E:\Build_app\Volte\_work\repro_original_before_d3.rs`.

Chạy `cargo test --offline --test repro -- --test-threads=1`:

| Test | Trước Đợt 3 (`repro_run2_literal_ext4.txt`) | Sau Đợt 3 (`repro_run3_after_phase3.txt`) |
|---|---|---|
| `a04_fsck_failure_still_returns_ok` | `ok` (bug tái hiện) | **FAILED** — `pipeline_ok=false e2fsck_fail_logged=true edl_ok_logged=false`; panic tại `repro.rs:451` vì giờ là `Err` |
| `b01_trailing_dont_care_loses_the_tail` | `ok` (bug tái hiện) | **FAILED** — `written=16384 actual=16384 expected=16384`; panic tại `repro.rs:289` (assert cũ đòi `actual == 0`) |
| `d01_failing_backend_keeps_old_footer_and_reports_success` | `ok` (bug tái hiện) | **FAILED** — panic tại `repro.rs:396` khi `unwrap()` nhận `Err("simulated avbtool.py failure")` |
| `a01`, `a01b`, `c01`, `c02` | FAILED | FAILED (không đổi — đã sửa ở Đợt 1/2, harness vẫn assert hành vi cũ) |
| `e01`, `f01`, `g01`, `g02` | `ok` | `ok` (không đổi — finding của Đợt 4) |

Tổng: trước `7 passed / 4 failed`, sau `4 passed / 7 failed`. Chênh lệch **đúng 3 test** là
A04/B01/D01 chuyển từ "tái hiện bug" sang "không còn bug"; không có test nào khác đổi trạng thái.
Các dòng `A01/A01b/C02` in ra giống hệt giữa hai lần chạy, xác nhận không có thay đổi phụ.

## 10. Cổng build / test / clippy (bằng chứng thô)

`E:\Build_app\Volte\_work\p3_build_final.txt`:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in ...
build_rc=0
```

`E:\Build_app\Volte\_work\p3_test_final.txt`:

```text
running 11 tests
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ...
running 163 tests
test result: ok. 163 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ...
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ...
test_rc=0
```

`E:\Build_app\Volte\_work\p3_clippy_final.txt`:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in ...
clippy_rc=0
```

Lệnh: `cargo build --workspace --offline`,
`cargo test --workspace --offline`,
`cargo clippy --workspace --all-targets --offline -- -D warnings`.

## 11. Khác biệt có chủ đích so với Python reference

| ID | Python | Port | Lý do |
|---|---|---|---|
| 09 `unsparse` | Seek qua `DONT_CARE`, không mở rộng file; header lệch vẫn success (`:504-507`) | `set_len` + xác nhận độ dài đĩa; lệch `total_blks×blk_sz` ⇒ `Err` | B01: không được xuất ảnh ngắn hơn khai báo |
| 27 `rebuild_hashtree` | `has_footer` kiểm trước `last_err` (`:1357-1372`); footer cũ ⇒ True | Lan truyền `Err`; digest vbmeta phải đổi; không đổi ⇒ `Err` | D01/N05: backend lỗi không được báo thành công |
| 29 `build` | e2fsck lỗi (`:1650-1655`) vẫn in `[OK] EDL size matches` và trả success | e2fsck lỗi ⇒ `Err`, exit 2, không dòng flashable, không report | A04: bước bắt buộc thất bại phải fail |

Các khác biệt này đã ghi vào `docs/PORT_STATUS.md` (ID 09, 27, 29(d)) và `README.md`.
Lưu ý: AVB backend lỗi **không** làm CLI exit 2 — nó đi vào đường `AVB_HASHTREE=SKIP` đã tài liệu
hoá, khôi phục snapshot và không tuyên bố PASS. Chỉ e2fsck lỗi mới là lỗi build.

## 12. Tài liệu đã cập nhật

- `README.md`: mục Run nói rõ e2fsck lỗi ⇒ exit 2, không in dòng flashable; AVB lỗi ⇒ SKIP,
  không PASS.
- `docs/PORT_STATUS.md`: số test 163/11; ID 09 và 27 chuyển `verified` → `diverges` kèm đoạn giải
  thích; ID 29 từ "Three differences" thành "Four differences" với mục (d) về e2fsck.
- Không sửa `CLAUDE.md`.

## 13. Rủi ro còn lại

1. Xác minh D01 bằng digest chỉ chạy khi footer cũ có `vbmeta_size > 0`. Ảnh có footer dị dạng
   (size 0) vẫn dựa vào kiểm tra `has_footer` + kích thước slot; ảnh do avbtool tạo luôn có
   vbmeta, nên trường hợp này không phát sinh trong luồng thật.
2. Salt ngẫu nhiên khi `info.salt` rỗng (hành vi có từ trước) làm digest chắc chắn đổi; nếu sau
   này `avb_info_fast` trích được salt và backend tái tạo footer **byte-identical**, digest sẽ
   không đổi và port báo `Err` oan. Khi đó cần so sánh thêm trường khác (ví dụ thời điểm/`props`)
   trước khi nới điều kiện — hiện tại chưa xảy ra và không đổi thuật toán AVB.
3. e2fsck là tool ngoài (`tool\e2fsck.exe`); thiếu tool giờ là lỗi rõ ràng thay vì panic/exit 0.
4. Các finding e01/f01/g01/g02 của harness vẫn `ok` (tức là bug còn tồn tại) — thuộc Đợt 4,
   không phải hồi quy của đợt này.
5. `a01/a01b/c01/c02` trong harness vẫn FAILED vì harness assert hành vi cũ; đây là bằng chứng
   các fix Đợt 1/2 còn hiệu lực, không phải lỗi mới.

## 14. Điều kiện hoàn thành

- [x] Không còn AVB false success — backend `Err` ⇒ `Err`; `Ok` mà vbmeta không đổi ⇒ `Err`;
      test `a_failing_backend_over_an_old_footer_is_never_reported_as_success`,
      `a_backend_that_returns_ok_without_rebuilding_is_an_error` (`avb.rs:698`, `735`).
- [x] Không còn e2fsck false success — pipeline `Err`, CLI exit 2, không dòng flashable, không
      report; test `an_e2fsck_failure_fails_the_build_before_avb` (`pipeline.rs:1083`),
      `a_pipeline_error_exits_non_zero` (`main.rs:335`).
- [x] Không còn sparse output sai kích thước — `written == total_blks×blk_sz == metadata.len()`;
      4 test sparse (`sparse.rs:282-341`).
- [x] N05 đã xác minh và xử lý — 1 lỗi độc lập đã sửa, 2 phần gộp D01/A04 có bảng đối chiếu §6.
- [x] Toàn bộ trạng thái pipeline phản ánh đúng kết quả thực tế — mọi `[OK]` còn lại đã rà soát §6.
- [x] `git commit` riêng cho đợt đã thực hiện: `3fae1b7`.

## 15. Git

- Baseline: `5819c01`.
- Commit đợt: `3fae1b7` — `Đợt 3: sửa false-success AVB/e2fsck và sparse DONT_CARE cuối file`
  (8 file, +472/−46).
- Báo cáo này được commit riêng sau `3fae1b7`; hash xem `git log --oneline -2`.
- Không `--amend`, không force-push, không viết lại lịch sử.

## 16. Kết luận

Đợt 3 hoàn tất: D01/A04/B01 đã được sửa và chứng minh bằng test trong repo lẫn harness độc lập;
N05 được khảo sát toàn văn, xác định gồm một lỗi độc lập và hai phần trùng root cause với
D01/A04. `cargo build`, `cargo test` (163 + 11, 0 failed) và `cargo clippy -D warnings` đều PASS.
Không phát hiện BLOCKED hay rủi ro mất dữ liệu/không thể đảo ngược, nên được phép nối sang Đợt 4
theo brief.