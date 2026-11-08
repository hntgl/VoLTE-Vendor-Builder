# Giai đoạn 3 — A7 (import trong comment) + A6 (3 nhánh WARN và 4 trạng thái flags)

STATUS: DONE (phạm vi A7 + A6)

> Re-check 2026-11-08: mục 6.7–6.9 phản ánh cây tại thời điểm viết report. Cây hiện tại
> đã khác (xem mục 11): test target biên dịch được và clippy PASS, nhưng
> `cargo test --workspace` vẫn FAIL vì stack overflow trong `modem.rs`.

Phạm vi đúng 2 bug:

- **A7** — `volte-core/src/init_rc.rs`: kiểm tra import dùng `text.contains(n)` nên dòng comment `# import ...` cũng được tính là "đã có".
- **A6** — `volte-core/src/pipeline.rs`: 3 nhánh `match` (RC link, props upsert, flags patch) chỉ log WARN. Phải quyết định nhánh nào fail build, nhánh nào giữ WARN; riêng flags phải phân biệt 4 trạng thái (thành công / không tìm thấy / lỗi đọc / lỗi ghi) và không được tự gán `0` khi lỗi I/O.

Không sửa gì ngoài 2 bug này, không đổi thuật toán QC/MTK, không đổi MBN injection, không đổi dependency.

## 1. Thay đổi

- `volte-core/src/init_rc.rs`
  - Thêm `has_import()`: duyệt từng dòng, bỏ dòng bắt đầu bằng `#`, chỉ khớp khi `line.trim() == needle`.
  - Thay `!text.contains(n)` bằng `!has_import(&text, n)`.
  - Thêm 2 regression test tiền tố `a7_`.
- `volte-core/src/error.rs`
  - Thêm `ErrorKind::Read` và `ErrorKind::Write` để tách lỗi đọc/ghi khỏi `Io` chung.
- `volte-core/src/flags.rs`
  - Thêm `io_read()` / `io_write()`; map toàn bộ I/O trong `patch_flags_keep_size` sang đúng loại lỗi.
  - Thêm 3 regression test tiền tố `a6_`.
- `volte-core/src/pipeline.rs`
  - Nhánh flags: lỗi I/O giờ là lỗi build (`Error::build`), không còn `WARN` + gán `0`.
  - Thêm `DroppingFlagProbeOps` và 1 regression test pipeline tiền tố `a6_`.
- `reports/phase-3-a7-a6-initrc-flags.md` (báo cáo này).

Hai nhánh còn lại (RC link, props upsert) **giữ WARN**, có lý do ở mục 3.

Ghi chú minh bạch: test target của workspace vẫn không biên dịch được vì lỗi có sẵn trong `modem.rs` (thiếu `Debug`, ngoài phạm vi Giai đoạn 3). Để chạy được các test mới, tôi đã **tạm thời** thêm `#[derive(Debug)]` vào `Fat16Image` và `ModemReport`, chạy test, rồi **đã gỡ bỏ**. Trong cây cuối cùng tại thời điểm viết report, `Select-String ... derive\(Debug\)` trên `modem.rs` đếm được `0`. `modem.rs` không nằm trong phạm vi sửa của Giai đoạn 3. Re-check 2026-11-08: cây hiện tại lại có `derive(Debug)` tại dòng 282/918 (mtime sau report) — xem mục 11.3.

Ghi chú minh bạch thứ hai: lần chạy "trước khi sửa" của A6 được thực hiện sau khi đã thêm 2 biến thể `ErrorKind::Read`/`Write` vào `error.rs` (để test biên dịch được), nhưng **hành vi của `flags.rs` và `pipeline.rs` vẫn là code cũ**. Bằng chứng cốt lõi là `left: Io` (code cũ) so với `right: Read`/`right: Write` (kỳ vọng), và log `[WARN] FLAGS ... [STEP] VoLTE flag: 0` của pipeline cũ.

## 2. Điều tra bắt buộc trước khi sửa A6 (payload RC)

Yêu cầu: đọc nội dung thật của `payload/oppo_qc_volte.rc` và `payload/oppo_mtk_volte.rc`, xem có khai báo `service` hay chỉ `on <trigger>` + `setprop`.

Lệnh đã chạy:

```powershell
foreach ($f in @("payload\oppo_qc_volte.rc","payload\oppo_mtk_volte.rc")) {
  $lines = Get-Content $f
  $svc = ($lines | Where-Object { $_ -match '^\s*service\s' }).Count
  $on  = ($lines | Where-Object { $_ -match '^\s*on\s' }).Count
  $imp = ($lines | Where-Object { $_ -match '^\s*import\s' }).Count
  $com = ($lines | Where-Object { $_ -match '^\s*#' }).Count
  Write-Output "$f : lines=$($lines.Count) service=$svc on=$on import=$imp comment=$com"
}
Select-String -Path payload\oppo_qc_volte.rc,payload\oppo_mtk_volte.rc -Pattern "service"
```

Kết quả thật:

```text
=== service/on/import counts ===
payload\oppo_qc_volte.rc : lines=127 service=0 on=4 import=0 comment=2
payload\oppo_mtk_volte.rc : lines=102 service=0 on=4 import=0 comment=1
=== any 'service' word ===
```

Nghĩa là:

- Cả hai file **không có một dòng `service` nào**.
- Chỉ có 4 block `on <trigger>` mỗi file, nội dung là các lệnh `setprop` (ví dụ `persist.sys.oppo.radio.show_volte 1`, `persist.mtk.volte_support 1`...).
- Không có `import` nội bộ.

Hệ quả cho quyết định A6:

- Bản RC trong `/vendor/etc/init/` được Android 9+ tự load, bản trong `/vendor/etc/init/hw/` được `init.target.rc` import tường minh. Nếu cả hai đường cùng chạy thì cùng một tập `setprop` được set lặp lại **với cùng giá trị**, nên idempotent, không sinh tiến trình/service trùng.
- Vì không có `service`, không có nguy cơ chạy 2 lần một daemon, không có nguy cơ bind port trùng, không có nguy cơ tranh chấp socket.
- Do đó mức nghiêm trọng của hiện tượng trùng RC là **thấp**. Không đủ căn cứ để biến lỗi link/import RC thành lỗi build chết.
- **Không xóa bất kỳ dòng ghi file hay import `hw/` nào trong Giai đoạn 3.** Việc có nên bỏ bản `hw/` hay không là quyết định của người dùng, ghi ở mục 8.

## 3. Quyết định thiết kế A6

Bảng quyết định cho 3 nhánh:

| Nhánh | Vị trí hiện tại (`pipeline.rs`) | Quyết định | Lý do |
|---|---|---|---|
| RC link (`init_rc::link`) | ~dòng 524–534 | **Giữ WARN** | `link` trả `Ok(false)` khi không dump được init file; đây là hành vi cố ý vì Android 9+ tự load `/vendor/etc/init`. Payload RC chỉ có `on` + `setprop` nên thiếu import không làm sai giá trị boot. Fail build sẽ chặn oan các image không có `init.target.rc`. |
| Props upsert (`props::upsert`) | ~dòng 537–543 | **Giữ WARN** | `upsert` duyệt 75 prop trên nhiều file prop; mỗi thiết bị có thể thiếu một file. Hàm đã tự log và bỏ qua từng file lỗi (`Per-file failures are logged and skipped, never fatal`). Ép fail build sẽ từ chối image hợp lệ chỉ vì một file prop tùy chọn không tồn tại. Mục tiêu VoLTE vẫn được kiểm chứng độc lập ở bước `verify_payload`. |
| Flags patch (`patch_flags_keep_size`) | ~dòng 548–564 | **Fail build** | Nếu không đọc/ghi được image thì **không biết** trạng thái flag. Gán `0` là bịa dữ liệu và có thể xuất image không bật VoLTE. Đây là bất biến an toàn, phải dừng. |

4 trạng thái của flags sau khi sửa:

| Trạng thái | Biểu hiện | Pipeline |
|---|---|---|
| Thành công (có patch) | `Ok(n)` với `n >= 1` | Tiếp tục, log `VoLTE flag: 1 (0->1 patched in the image)` |
| Không tìm thấy | `Ok(0)` — quét hết file, không cặp nào khớp | Tiếp tục, log `VoLTE flag: 0 (the original vendor has no volte_support=1)` |
| Lỗi đọc | `Err(ErrorKind::Read)` | Fail build: `VoLTE flag patch failed while reading the image - refusing to continue: ...` |
| Lỗi ghi | `Err(ErrorKind::Write)` | Fail build: `VoLTE flag patch failed while writing the image - refusing to continue: ...` |

Ánh xạ I/O trong `patch_flags_keep_size`:

- `std::fs::copy` → `Write`
- `metadata` → `Read`
- `OpenOptions::open` → `Write` (mở để ghi)
- `seek` trước khi đọc, `read_chunk` → `Read`
- `seek` trước khi ghi, `write_all`, `flush` → `Write`

Riêng `Ok(0)` của một file rỗng vẫn giữ nguyên là "không có gì để vá", không phải lỗi.

## 4. Diff tập trung của Giai đoạn 3

Cây làm việc có nhiều thay đổi chưa commit từ trước (đặc biệt trong `flags.rs` và `init_rc.rs`). Để tách đúng phần Giai đoạn 3, tôi dựng lại bản baseline trước Giai đoạn 3 trong `%TEMP%\opencode\p3_baseline\` rồi diff bằng `git diff --no-index`. Bốn diff dưới đây chỉ chứa thay đổi của Giai đoạn 3.

### 4.1. `volte-core/src/error.rs`

```diff
@@ -13,6 +13,10 @@ pub enum ErrorKind {
     Build,
     /// Filesystem or stream error.
     Io,
+    /// Reading the image failed while applying a byte patch.
+    Read,
+    /// Writing the image failed while applying a byte patch.
+    Write,
     /// The image is not EXT4 (`rust-fs-ext4: bad magic number`).
     BadMagic,
     /// A required external tool or payload file is missing.
```

### 4.2. `volte-core/src/init_rc.rs`

```diff
@@ -53,6 +53,15 @@ fn split_vpath(vpath: &str) -> Option<(String, String)> {
     })
 }
 
+/// True when an uncommented line is exactly the import command.
+///
+/// The old check used `str::contains`, so the text inside a `# import ...`
+/// comment also counted as present and the real import was never written.
+fn has_import(text: &str, needle: &str) -> bool {
+    text.lines()
+        .any(|line| !line.trim_start().starts_with('#') && line.trim() == needle)
+}
+
 /// Returns `true` when an init file was dumped and now imports every needle,
 /// `false` when no candidate could be dumped. Errors from the final write are
 /// propagated, like the Python call inside its `try` block.
@@ -101,7 +110,7 @@ pub fn link(
     let missing: Vec<&str> = needles
         .iter()
         .copied()
-        .filter(|n| !n.is_empty() && !text.contains(n))
+        .filter(|n| !n.is_empty() && !has_import(&text, n))
         .collect();
     if missing.is_empty() {
         log.log(format!("[OK] {found} already imports HD VoLTE."));
@@ -460,4 +469,67 @@ mod tests {
         assert!(!ops.commands.iter().any(|c| c == "cd /"));
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    #[test]
+    fn a7_a_commented_import_does_not_count_as_present() {
+        let dir = tmp("volte_initrc_a7_commented_import");
+        std::fs::write(dir.join("config.bin"), b"label\0").unwrap();
+        let body = format!(
+            "on boot\n# {}\n    # {}\n    exec x\n",
+            profile::IMPORT_LINE,
+            profile::IMPORT_LINE_HW
+        );
+        let mut ops = FakeOps::new(&[("/etc/init/hw/init.target.rc", body.as_bytes())]);
+        let mut log = Logger::new().with_sink(Vec::<String>::new());
+        let cfg = dir.join("config.bin");
+        assert!(link(
+            Path::new("img"),
+            &dir,
+            Some(&cfg),
+            &qc_needles(),
+            &mut ops,
+            &mut log
+        )
+        .unwrap());
+        assert_eq!(
+            ops.written.len(),
+            1,
+            "a commented import must not satisfy the check: {:?}",
+            ops.commands
+        );
+        let text = String::from_utf8_lossy(&ops.written[0].1).into_owned();
+        assert!(
+            text.contains(&format!("# {}\n", profile::IMPORT_LINE)),
+            "the comment must be preserved:\n{text}"
+        );
+        for needle in qc_needles() {
+            let exact = text.lines().filter(|l| l.trim() == needle).count();
+            assert_eq!(exact, 1, "expected exactly one real {needle:?} in:\n{text}");
+        }
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a7_a_real_import_plus_comments_stays_idempotent() {
+        let dir = tmp("volte_initrc_a7_idempotent");
+        let body = format!(
+            "on boot\n# {}\n{}\n{}\n",
+            profile::IMPORT_LINE,
+            profile::IMPORT_LINE,
+            profile::IMPORT_LINE_HW
+        );
+        let mut ops = FakeOps::new(&[("/etc/init/hw/init.target.rc", body.as_bytes())]);
+        let mut log = Logger::new().with_sink(Vec::<String>::new());
+        assert!(link(
+            Path::new("img"),
+            &dir,
+            None,
+            &qc_needles(),
+            &mut ops,
+            &mut log
+        )
+        .unwrap());
+        assert!(ops.written.is_empty(), "a real import is already present");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

### 4.3. `volte-core/src/flags.rs`

```diff
@@ -9,7 +9,7 @@ use std::io::{Read, Seek, SeekFrom, Write};
 use std::path::Path;
 
 use crate::config::{CHUNK, OVERLAP};
-use crate::error::Result;
+use crate::error::{Error, ErrorKind, Result};
 use crate::logging::Logger;
 use crate::path_id;
 
@@ -218,15 +218,26 @@ fn read_chunk(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
     Ok(done)
 }
 
+/// A failed read must stay a read failure: the pipeline uses it to tell an
+/// unreadable image from a scan that legitimately found zero flags.
+fn io_read(e: std::io::Error) -> Error {
+    Error::new(ErrorKind::Read, e.to_string())
+}
+
+/// A failed write must stay a write failure, never "0 flags patched".
+fn io_write(e: std::io::Error) -> Error {
+    Error::new(ErrorKind::Write, e.to_string())
+}
+
 /// `patch_flags_keep_size` (`:689`): copy `src` to `dst` when they differ, then
 /// apply every flag pair inside `limit` bytes (0 = whole file). Windows advance
 /// by `CHUNK - OVERLAP` so a pair straddling a boundary is still seen.
 pub fn patch_flags_keep_size(src: &Path, dst: &Path, limit: u64, _log: &mut Logger) -> Result<u64> {
     if !path_id::same_file(src, dst) {
-        std::fs::copy(src, dst)?;
+        std::fs::copy(src, dst).map_err(io_write)?;
     }
     let mut hits = 0u64;
-    let size = std::fs::metadata(dst)?.len();
+    let size = std::fs::metadata(dst).map_err(io_read)?.len();
     if size == 0 {
         return Ok(0);
     }
@@ -236,13 +247,17 @@ pub fn patch_flags_keep_size(src: &Path, dst: &Path, limit: u64, _log: &mut Logg
         .filter(|(a, b)| !a.is_empty() && a.len() == b.len())
         .map(|(a, b)| (*a, *b))
         .collect();
-    let mut f = OpenOptions::new().read(true).write(true).open(dst)?;
+    let mut f = OpenOptions::new()
+        .read(true)
+        .write(true)
+        .open(dst)
+        .map_err(io_write)?;
     let mut buf = vec![0u8; CHUNK];
     let mut pos = 0u64;
     while pos < span {
         let n = (span - pos).min(CHUNK as u64) as usize;
-        f.seek(SeekFrom::Start(pos))?;
-        let got = read_chunk(&mut f, &mut buf[..n])?;
+        f.seek(SeekFrom::Start(pos)).map_err(io_read)?;
+        let got = read_chunk(&mut f, &mut buf[..n]).map_err(io_read)?;
         if got == 0 {
             break;
         }
@@ -258,14 +273,14 @@ pub fn patch_flags_keep_size(src: &Path, dst: &Path, limit: u64, _log: &mut Logg
                 start = i + b.len();
             }
         }
-        f.seek(SeekFrom::Start(pos))?;
-        f.write_all(&buf[..got])?;
+        f.seek(SeekFrom::Start(pos)).map_err(io_write)?;
+        f.write_all(&buf[..got]).map_err(io_write)?;
         if n < CHUNK {
             break;
         }
         pos += (n - OVERLAP) as u64;
     }
-    f.flush()?;
+    f.flush().map_err(io_write)?;
     Ok(hits)
 }
 
@@ -532,4 +547,49 @@ mod tests {
         );
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    #[test]
+    fn a6_a_missing_image_is_a_read_error_not_a_zero_hit() {
+        let dir = tmp("volte_flags_a6_read");
+        let missing = dir.join("missing.img");
+        let mut log = Logger::new().with_sink(Vec::<String>::new());
+        let err = patch_flags_keep_size(&missing, &missing, 0, &mut log).unwrap_err();
+        assert_eq!(
+            err.kind(),
+            ErrorKind::Read,
+            "an unreadable image must be reported as a read error, never as 0 hits: {err}"
+        );
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a6_a_copy_failure_is_a_write_error_not_a_zero_hit() {
+        let dir = tmp("volte_flags_a6_write");
+        let src = dir.join("in.bin");
+        std::fs::write(&src, b"volte_support=0").unwrap();
+        let dst = dir.join("dest");
+        std::fs::create_dir(&dst).unwrap();
+        let mut log = Logger::new().with_sink(Vec::<String>::new());
+        let err = patch_flags_keep_size(&src, &dst, 0, &mut log).unwrap_err();
+        assert_eq!(
+            err.kind(),
+            ErrorKind::Write,
+            "a failed write must be reported as a write error, never as 0 hits: {err}"
+        );
+        let _ = std::fs::remove_dir_all(&dir);
+    }
+
+    #[test]
+    fn a6_no_pair_found_is_still_ok_zero() {
+        let dir = tmp("volte_flags_a6_not_found");
+        let p = dir.join("img.bin");
+        std::fs::write(&p, b"ro.keep=1\n").unwrap();
+        let mut log = Logger::new().with_sink(Vec::<String>::new());
+        assert_eq!(
+            patch_flags_keep_size(&p, &p, 0, &mut log).unwrap(),
+            0,
+            "a complete scan with no matching pair is a valid zero, not an I/O error"
+        );
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

### 4.4. `volte-core/src/pipeline.rs`

```diff
@@ -12,7 +12,7 @@ use std::time::{Instant, SystemTime, UNIX_EPOCH};
 use crate::avb;
 use crate::chip_detect;
 use crate::config;
-use crate::error::{Error, Result};
+use crate::error::{Error, ErrorKind, Result};
 use crate::ext4_tools::{self, Ext4Ops, Fsck};
 use crate::flags;
 use crate::init_rc;
@@ -548,8 +548,18 @@ pub fn run(
     let nflag = match flags::patch_flags_keep_size(&work, &work, 0, log) {
         Ok(n) => n,
         Err(e) => {
-            log.log(format!("[WARN] FLAGS: {}", trunc(e.message(), 160)));
-            0
+            // A read/write failure means we do not know the flag state; it must
+            // not be reported as "0" and the build must not continue.
+            let phase = match e.kind() {
+                ErrorKind::Read => "reading",
+                ErrorKind::Write => "writing",
+                _ => "accessing",
+            };
+            let why = trunc(e.message(), 160);
+            log.log(format!("[ERROR] FLAGS {phase}: {why}"));
+            return Err(Error::build(format!(
+                "VoLTE flag patch failed while {phase} the image - refusing to continue: {why}"
+            )));
         }
     };
     let on: u8 = if nflag >= 1 || orig_bit >= 1 { 1 } else { 0 };
@@ -1523,4 +1533,77 @@ mod tests {
         assert!(!logs.join("vendor_VolLTE_report.txt").exists());
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    /// The real backend, but it removes the output image right after the flag
+    /// probe dump (A6). The flag patch then has to report a read failure
+    /// instead of silently treating it as "0 flags".
+    struct DroppingFlagProbeOps {
+        inner: RustFsExt4Backend,
+    }
+
+    impl Ext4Ops for DroppingFlagProbeOps {
+        fn run(
+            &mut self,
+            image: &Path,
+            commands: &[String],
+            workdir: &Path,
+            log: &mut Logger,
+        ) -> Result<String> {
+            let out = self.inner.run(image, commands, workdir, log);
+            if commands.iter().any(|c| c.ends_with("flg_build.prop")) {
+                let _ = std::fs::remove_file(image);
+            }
+            out
+        }
+
+        fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
+            self.inner.path_exists(image, vpath, workdir)
+        }
+    }
+
+    #[test]
+    fn a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero() {
+        let dir = tmp("volte_pipeline_a6_flag_read");
+        let src = make_ext4(&dir, "vendor.img");
+        seed_payloads(&dir);
+        let logs = dir.join("Logs");
+
+        let mut ops = DroppingFlagProbeOps {
+            inner: RustFsExt4Backend::new(),
+        };
+        let mut fsck = PassingFsck { calls: 0 };
+        let mut avb = NoFooterAvb { calls: 0 };
+        let mut deps = Deps {
+            ops: &mut ops,
+            fsck: &mut fsck,
+            avb: &mut avb,
+        };
+        let lines = Rc::new(RefCell::new(Vec::new()));
+        let mut log = Logger::new().with_sink(SharedSink(lines.clone()));
+        let mut progress = |_n: u8, _l: &str| {};
+        let opts = Options {
+            src: &src,
+            out_path: "",
+            chip: "mtk",
+            keep_out: false,
+            root: &dir,
+            logs_dir: &logs,
+        };
+
+        let result = run(&opts, &mut deps, &mut log, &mut progress);
+        let text = lines.borrow().join("\n");
+        match result {
+            Ok(out) => panic!(
+                "A6: the build exported {} after the flag patch could not read the image\n{text}",
+                out.display()
+            ),
+            Err(err) => assert!(
+                err.message().contains("flag patch failed while reading"),
+                "A6: expected a flag read error, got: {}\n{text}",
+                err.message()
+            ),
+        }
+        assert!(!text.contains("[OUTPUT] "), "{text}");
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

## 5. Test đã thêm

### A7 — `volte-core/src/init_rc.rs`

1. `a7_a_commented_import_does_not_count_as_present`
   - Init file chỉ có 2 dòng comment chứa `import /vendor/etc/init/oppo_qc_volte.rc` và `import /vendor/etc/init/hw/oppo_qc_volte.rc`.
   - Kỳ vọng: phải ghi lại file (1 lần write), comment cũ giữ nguyên, và mỗi import thật xuất hiện đúng 1 lần.
   - Trên code cũ: `ops.written.len()` là `0` → FAIL.

2. `a7_a_real_import_plus_comments_stays_idempotent`
   - Init file có cả import thật lẫn comment.
   - Kỳ vọng: không ghi lại file (`written.is_empty()`).
   - Test này PASS trên cả code cũ; nó bảo vệ để fix A7 không phá tính idempotent.

### A6 — `volte-core/src/flags.rs`

3. `a6_a_missing_image_is_a_read_error_not_a_zero_hit`
   - Gọi `patch_flags_keep_size` trên file không tồn tại.
   - Kỳ vọng: `Err(ErrorKind::Read)`.
   - Trên code cũ: `Err(ErrorKind::Io)` → FAIL.

4. `a6_a_copy_failure_is_a_write_error_not_a_zero_hit`
   - Nguồn là file, đích là một thư mục đã tồn tại → `std::fs::copy` thất bại.
   - Kỳ vọng: `Err(ErrorKind::Write)`.
   - Trên code cũ: `Err(ErrorKind::Io)` → FAIL.

5. `a6_no_pair_found_is_still_ok_zero`
   - File tồn tại, không có cặp `=0 → =1` nào.
   - Kỳ vọng: `Ok(0)` — đây là "không tìm thấy", không phải lỗi I/O.
   - PASS trên cả code cũ (bảo vệ để fix không biến zero hợp lệ thành lỗi).

### A6 — `volte-core/src/pipeline.rs`

6. `a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero`
   - `DroppingFlagProbeOps` bọc backend thật, xóa output image ngay sau lệnh dump `flg_build.prop` (bước đọc flag gốc).
   - Kỳ vọng: `run()` trả `Err` với message chứa `flag patch failed while reading`, và không có `[OUTPUT]`.
   - Trên code cũ: build chỉ log `[WARN] FLAGS: ...` rồi `VoLTE flag: 0`, sau đó chết ở bước verify payload (`Overlay APK/RC not verified inside the image - refusing to export`) — tức là lỗi thật bị che, message sai. FAIL.

## 6. Kết quả chạy thật

Môi trường:

- `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- `rustc 1.101.0-nightly (12433c307 2026-10-24)`
- Windows, workspace `E:\Build_app\Volte\VoLTE-Vendor-Builder`

Lưu ý: các output dưới đây được trích nguyên văn từ terminal. Phần đầu mỗi lần chạy có vài dòng PowerShell bọc stderr của `cargo` (`NativeCommandError`, `Blocking waiting for file lock...`); tôi bỏ các dòng đó cho dễ đọc, không sửa nội dung test. Ở mục 6.7/6.8, các khối lỗi `ModemReport` giống hệt nhau được ghi gọn thành `(×4)` thay vì lặp 4 lần.

### 6.1. A7 trước khi sửa

Lệnh:

```text
cargo test -p volte-core "a7_" -- --nocapture
```

```text
running 2 tests

thread 'init_rc::tests::a7_a_commented_import_does_not_count_as_present' (5856) panicked at
volte-core\src\init_rc.rs:485:9:
assertion `left == right` failed: a commented import must not satisfy the check: ["dump /etc/init/hw/init.target.rc
C:/Users/HT/AppData/Local/Temp/volte_initrc_a7_commented_import/init.target.rc"]
  left: 0
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test init_rc::tests::a7_a_commented_import_does_not_count_as_present ... FAILED
test init_rc::tests::a7_a_real_import_plus_comments_stays_idempotent ... ok

failures:
    init_rc::tests::a7_a_commented_import_does_not_count_as_present

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 235 filtered out; finished in 0.01s
```

Đúng triệu chứng: chỉ có comment, code cũ coi là đã import nên **không ghi file** (`left: 0`).

### 6.2. A7 sau khi sửa

```text
cargo test -p volte-core "a7_" -- --nocapture
```

```text
running 2 tests
test init_rc::tests::a7_a_real_import_plus_comments_stays_idempotent ... ok
test init_rc::tests::a7_a_commented_import_does_not_count_as_present ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 235 filtered out; finished in 0.02s
```

### 6.3. A6 trước khi sửa

Lệnh:

```text
cargo test -p volte-core "a6_" -- --nocapture
```

```text
running 4 tests

thread 'flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit' (10800) panicked at
volte-core\src\flags.rs:542:9:
assertion `left == right` failed: an unreadable image must be reported as a read error, never as 0 hits: The system
cannot find the file specified. (os error 2)
  left: Io
 right: Read
test flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit ... FAILED
test flags::tests::a6_no_pair_found_is_still_ok_zero ... ok

thread 'flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit' (10612) panicked at
volte-core\src\flags.rs:559:9:
assertion `left == right` failed: a failed write must be reported as a write error, never as 0 hits: Access is denied.
(os error 5)
  left: Io
 right: Write
test flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit ... FAILED

thread 'pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero' (7516) panicked at
volte-core\src\pipeline.rs:1590:25:
A6: expected a flag read error, got: Overlay APK/RC not verified inside the image - refusing to export
[CHIP] Forced MEDIATEK branch (user choice).
================================================================
HD Vendor Fix VoLTE
Version: 2.9.2 / V298
Mode   : STANDARD - e2fsck + AVB signature, EDL-sized
Engine : hoangduc
Chip   : MTK
Input  : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a6_flag_read\vendor.img
Output : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a6_flag_read\vendor_VolLTE.img
Log    : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a6_flag_read\Logs\hue_chuan_20261108_034653.log
================================================================
[STATUS] Inspecting source image...
[STATUS] Copying source image...
avbtool: Given image does not look like a vbmeta image.
[AVB] File has no AVB footer - still building STANDARD (318 baseline).
[STATUS] Injecting VoLTE payload...
EXT4 engine: rust-fs-ext4 (in-process)
ext4: rm /overlay/OPPO_VoLTE_Overlay.apk
ext4: rm /etc/init/oppo_mtk_volte.rc
ext4: rm /etc/init/oppo_qc_volte.rc
ext4: rm /etc/init/hw/oppo_qc_volte.rc
ext4: rm /etc/init/hw/oppo_mtk_volte.rc
ext4: mkdir /overlay
ext4: mkdir /etc
ext4: mkdir /etc/init
ext4: mkdir /etc/init/hw
[STEP] Linking init.target.rc so the VoLTE RC runs at boot...
[INFO] Could not dump init.target.rc - Android 9+ loads /vendor/etc/init itself.
[STEP] Writing the VoLTE switch properties into build.prop...
[WARN] Could not write the VoLTE show props (build.prop dump/write failed).
[STEP] Enabling the VoLTE switch (overlay) + reading the original vendor flag...
[WARN] FLAGS: The system cannot find the file specified. (os error 2)
[STEP] VoLTE flag: 0 (the original vendor has no volte_support=1)
[STEP] VoLTE flag: 0
[STEP] Flag =0 - the original vendor has no volte_support=1 and no 0->1 patch was applied.
APK_SHA256=UNVERIFIED
RC_SHA256=UNVERIFIED
[WARN] Payload copy /overlay/OPPO_VoLTE_Overlay.apk: dump failed
[WARN] Payload copy /etc/init/oppo_mtk_volte.rc: dump failed
[WARN] Payload copy /etc/init/hw/oppo_mtk_volte.rc: dump failed
[WARN] Overlay payload not verified (APK: not verified, RC: not verified).
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... FAILED

failures:
    flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit
    flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit
    pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 233 filtered out; finished in 2.41s
```

Hai điểm quan trọng trong log cũ:

- `[WARN] FLAGS: The system cannot find the file specified. (os error 2)` rồi `[STEP] VoLTE flag: 0` — lỗi đọc bị biến thành "không có flag".
- Build chỉ chết ở bước sau với lý do khác (`Overlay APK/RC not verified...`), che mất nguyên nhân thật.

### 6.4. A6 sau khi sửa

```text
cargo test -p volte-core "a6_" -- --nocapture
```

```text
running 4 tests
test flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit ... ok
test flags::tests::a6_no_pair_found_is_still_ok_zero ... ok
test flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit ... ok
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 233 filtered out; finished in 2.32s
```

### 6.5. Toàn bộ module liên quan (không hồi quy)

```text
cargo test -p volte-core "init_rc::"
```

```text
running 11 tests
test init_rc::tests::a7_a_real_import_plus_comments_stays_idempotent ... ok
test init_rc::tests::a_dump_error_skips_the_candidate_instead_of_aborting ... ok
test init_rc::tests::a7_a_commented_import_does_not_count_as_present ... ok
test init_rc::tests::a_nested_init_file_is_written_back_to_its_own_directory ... ok
test init_rc::tests::candidate_counts_are_21_for_mtk_and_8_for_qc ... ok
test init_rc::tests::existing_directories_drop_their_mkdir_commands ... ok
test init_rc::tests::inserts_the_missing_imports_and_keeps_the_file_lf_terminated ... ok
test init_rc::tests::a_root_init_file_is_written_back_to_the_root ... ok
test init_rc::tests::existing_imports_are_idempotent_and_skip_the_write_back ... ok
test init_rc::tests::split_vpath_keeps_the_directory_the_file_came_from ... ok
test init_rc::tests::undumpable_init_file_returns_false_without_error ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 226 filtered out; finished in 0.04s
```

```text
cargo test -p volte-core "flags::"
```

```text
running 14 tests
test flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit ... ok
test flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit ... ok
test flags::tests::boundary_uses_ascii_names_only ... ok
test flags::tests::a6_no_pair_found_is_still_ok_zero ... ok
test flags::tests::flag_pairs_are_65_length_equal_and_non_empty ... ok
test flags::tests::every_pair_is_applied_once_and_size_is_kept ... ok
test flags::tests::flag_bit_reads_both_dumps_and_separates_them ... ok
test flags::tests::case_alias_patches_in_place_instead_of_copying ... ok
test flags::tests::hard_link_alias_patches_in_place_when_the_platform_allows_it ... ok
test flags::tests::limit_restricts_the_scanned_span ... ok
test flags::tests::same_path_patches_in_place_and_empty_file_returns_zero ... ok
test flags::tests::patches_only_whole_property_names ... ok
test flags::tests::window_start_accepts_a_match_python_also_accepts ... ok
test flags::tests::pair_straddling_a_chunk_boundary_is_still_patched ... ok

test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 223 filtered out; finished in 4.74s
```

```text
cargo test -p volte-core "pipeline::"
```

```text
running 23 tests
test pipeline::tests::a_backup_with_the_wrong_hash_makes_restore_fatal ... ok
test pipeline::tests::a_failed_atomic_replace_is_fatal_and_removes_the_side_file ... ok
test pipeline::tests::a_pre_avb_backup_is_verified_by_size_and_hash ... ok
test pipeline::tests::a_truncated_backup_makes_restore_fatal_and_leaves_the_image_untouched ... ok
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... ok
test pipeline::tests::a3_payload_mismatch_after_injection_aborts_build ... ok
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image has been running for over 60 seconds
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree has been running for over 60 seconds
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb has been running for over 60 seconds
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb ... ok
test pipeline::tests::case_only_output_alias_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::hard_linked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::log_stamp_has_the_python_shape ... ok
test pipeline::tests::missing_source_is_a_build_error ... ok
test pipeline::tests::output_equal_to_the_source_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::output_path_defaults_next_to_the_source ... ok
test pipeline::tests::output_path_honours_keep_out_and_renames_a_src_collision ... ok
test pipeline::tests::relative_source_with_absolute_output_is_refused ... ok
test pipeline::tests::restore_replaces_the_image_with_the_verified_snapshot ... ok
test pipeline::tests::space_need_counts_the_copy_the_restore_side_and_the_temp_backup ... ok
test pipeline::tests::space_need_saturates_instead_of_overflowing ... ok
test pipeline::tests::symlinked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::the_space_probe_allocates_and_cleans_up ... ok
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree ... ok
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image ... ok
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted has been running for over 60 seconds
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted ... ok

test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 214 filtered out; finished in 121.09s
```

### 6.6. `cargo build --workspace`

```text
cargo build --workspace
```

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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.95s
```

PASS. Chỉ còn warning dead-code có sẵn trong `modem.rs`, ngoài phạm vi.

### 6.7. `cargo test --workspace` (trạng thái tại thời điểm viết report)

> Mục này chỉ đúng cho cây tại thời điểm viết report. Cây hiện tại đã khác — xem mục 11.

Trên cây tại thời điểm viết report (đã gỡ bản vá tạm `Debug` ở `modem.rs`), test target **không biên dịch được** vì lỗi có sẵn, ngoài phạm vi Giai đoạn 3:

```text
error[E0277]: `modem::Fat16Image` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1240:49
     |
1240 |         let e = Fat16Image::new(vec![0u8; 100]).unwrap_err();
     |                                                 ^^^^^^^^^^ the trait `Debug` is not implemented for
`modem::Fat16Image`

error[E0277]: `ModemReport` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1453:59
    --> volte-core\src\modem.rs:1462:74
    --> volte-core\src\modem.rs:1509:79
    --> volte-core\src\modem.rs:1551:74

error: could not compile `volte-core` (lib test) due to 5 previous errors
```

Để lấy bằng chứng test ở mục 6.1–6.5, tôi đã tạm thêm `#[derive(Debug)]` vào `Fat16Image` và `ModemReport`, chạy toàn bộ các test trên, rồi gỡ. Tại thời điểm viết report, cây cuối cùng không còn `derive(Debug)` nào trong `modem.rs`; cây hiện tại khác — xem mục 11.3.

### 6.8. `cargo clippy --workspace --all-targets` (trạng thái tại thời điểm viết report)

> Mục này chỉ đúng cho cây tại thời điểm viết report. Cây hiện tại PASS (exit 0) — xem mục 11.

```text
warning: field `spc` is never read
warning: this manual char comparison can be written more succinctly
warning: manually reimplementing `div_ceil`
warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2
suggestions)
error[E0277]: `modem::Fat16Image` doesn't implement `std::fmt::Debug`
error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`  (×4)
error: could not compile `volte-core` (lib test) due to 5 previous errors
```

FAIL ở bước biên dịch `--all-targets` vì cùng lỗi `Debug` có sẵn trong `modem.rs`. Cả 3 warning clippy đều nằm trong `modem.rs`; **không có warning nào ở `init_rc.rs`, `flags.rs`, `pipeline.rs`, `error.rs`** do thay đổi Giai đoạn 3.

### 6.9. `git diff --check`

```text
git diff --check
```

Exit code: `0`. Chỉ có cảnh báo LF/CRLF có sẵn của Git cho các file đang thay đổi; không có whitespace error.

## 7. Đánh giá kết quả

- **Đã chứng minh bug A7:** Có. Init file chỉ chứa comment → code cũ không ghi import thật (`left: 0`, `right: 1`).
- **Đã chứng minh bug A6:** Có. Lỗi đọc/ghi trả `Io` thay vì `Read`/`Write`; pipeline biến lỗi đọc thành `VoLTE flag: 0` và chỉ chết ở bước verify với message sai.
- **Đã sửa:** Có, đúng 2 bug, không đổi kiến trúc, không đổi thuật toán QC/MTK/MBN.
- **Regression test:** Có. A7 `2 passed`; A6 `4 passed`; các module `init_rc::` (11), `flags::` (14), `pipeline::` (23) đều PASS.
- **Kiểm tra workspace:** một phần (tại thời điểm viết report).
  - `cargo build --workspace`: PASS.
  - `cargo test --workspace`: FAIL do lỗi biên dịch test có sẵn trong `modem.rs`, không liên quan Giai đoạn 3.
  - `cargo clippy --workspace --all-targets`: FAIL vì cùng lỗi biên dịch test của `modem.rs`.
  - Re-check 2026-11-08 trên cây hiện tại: build PASS, clippy PASS, nhưng `cargo test --workspace` vẫn FAIL vì stack overflow trong `modem.rs`; xem mục 11.
- **Giới hạn bằng chứng:** đây là test đơn vị/tích hợp trên image ext4 tổng hợp và backend thật trong repo, không phải kiểm chứng trên máy thật. Chưa có xác nhận flash được trên thiết bị.
- **Chưa xác minh được:** toàn bộ workspace test PASS, vì blocker có sẵn ở `modem.rs`. Re-check 2026-11-08: clippy đã PASS; workspace test vẫn FAIL do stack overflow (xem mục 11).

## 8. Phát hiện ngoài phạm vi và mục chờ người dùng quyết định

Không tự sửa các mục sau:

1. **Trùng lặp RC (`/vendor/etc/init/` vs `/vendor/etc/init/hw/`)** — Đã điều tra: cả hai payload RC **không có `service`**, chỉ có `on <trigger>` + `setprop`. Vì vậy nếu cả hai đường cùng load thì chỉ set lặp cùng giá trị, idempotent, không chạy daemon trùng. **Không xóa bất kỳ dòng ghi file hay import `hw/` nào trong Giai đoạn 3.** Cần người dùng quyết định sau:
   - giữ nguyên cả hai đường (an toàn, chấp nhận setprop lặp), hay
   - bỏ bản `hw/` và chỉ dựa vào auto-load `/vendor/etc/init/`, hay
   - giữ `hw/` và bỏ import để tránh trùng.
   Mọi thay đổi theo hướng này nên làm ở giai đoạn riêng, có test và có xác nhận trên máy thật.

2. **`volte-core/src/modem.rs` (untracked) không biên dịch được test target**: `Fat16Image`/`ModemReport` thiếu `Debug` tại `modem.rs:1240, 1453, 1462, 1509, 1551`. Theo chỉ đạo trước đó, việc này gộp vào đầu Giai đoạn 4 (A4). Re-check 2026-11-08: cây hiện tại đã có `#[derive(Debug)]` tại `modem.rs:282` và `918`, nên lỗi biên dịch này không còn tái hiện; vấn đề còn lại là stack overflow (xem mục 11).

3. **Khi tạm thêm `Debug`, workspace test gặp `STATUS_STACK_OVERFLOW`** tại `modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn`. Cũng theo chỉ đạo trước đó, cần điều tra ngay khi bắt đầu Giai đoạn 4, trước khi sửa A4. Re-check 2026-11-08: còn thêm `modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image` cũng overflow (xem 11.2).

4. **Warning clippy có sẵn trong `modem.rs`**: `spc` never read, `manual_pattern_char_comparison` (dòng 373), `manual_div_ceil` (dòng 691). Không đụng tới.

5. **Cây làm việc có nhiều thay đổi/xóa file chưa commit từ trước** (README, profiles, reports cũ, `modem.rs`, `modem_cli.rs`, zip...). Không stage, không revert, không "dọn dẹp".

## 9. File đã thay đổi

- `volte-core/src/init_rc.rs`
- `volte-core/src/error.rs`
- `volte-core/src/flags.rs`
- `volte-core/src/pipeline.rs`
- `reports/phase-3-a7-a6-initrc-flags.md` (mới)

`volte-core/src/modem.rs` chỉ được sửa tạm thời để chạy test trong lúc viết report và đã gỡ tại thời điểm đó. Tuy nhiên tại cây hiện tại (re-check 2026-11-08), file này có `#[derive(Debug)]` tại dòng 282 và 918; mtime `2026-11-08 10:58:03` sau report `10:52:56`. Không xác định được ai thêm; không phải thay đổi của Giai đoạn 3. Xem mục 11.

## 10. Kết luận

A7 được sửa bằng cách parse từng dòng và bỏ comment, nên `# import ...` không còn được tính là import thật; import thật vẫn idempotent. A6 được xử lý theo hướng: RC link và props upsert giữ WARN (có căn cứ từ điều tra payload: không có `service`, nên trùng lặp idempotent), riêng flags là lỗi build khi đọc/ghi thất bại, với 4 trạng thái phân biệt rõ và không tự gán `0`. Cả hai đều có regression test thật, fail trên code cũ và pass sau khi sửa.

STATUS: DONE cho phạm vi A7 + A6. Các kiểm tra workspace rộng hơn vẫn bị chặn bởi lỗi có sẵn trong `modem.rs` (ngoài phạm vi), đã ghi nhận và không tự sửa.

Re-check 2026-11-08: kết luận DONE cho A7 + A6 không đổi. Trên cây hiện tại, blocker biên dịch đã không còn (do `modem.rs` có `derive(Debug)`), clippy PASS, nhưng `cargo test --workspace` vẫn FAIL vì stack overflow trong các test `modem.rs`; xem mục 11.

## 11. Re-check 2026-11-08 — đối chiếu report với cây hiện tại

Mục này được thêm sau khi kiểm tra lại Giai đoạn 3. Lần re-check này **không sửa source code**; chỉ chạy test/build/clippy và đối chiếu diff.

### 11.1. Tổng hợp lệnh và kết quả hiện tại

| Lệnh | Kết quả | Exit |
|---|---|---|
| `cargo test -p volte-core a7_ -- --nocapture` | 2 passed; 0 failed | 0 |
| `cargo test -p volte-core a6_ -- --nocapture` | 4 passed; 0 failed | 0 |
| `cargo test -p volte-core "pipeline::"` | 23 passed; 0 failed | 0 |
| `cargo build --workspace` | PASS | 0 |
| `cargo clippy --workspace --all-targets` | PASS; 3 warning trong `modem.rs` | 0 |
| `cargo test --workspace` | FAIL — stack overflow trong `modem.rs` | 127 |
| `cargo test -p volte-core --lib -- --skip build_modem_fix_from_mbn_injects_a_missing_viettel_mbn` | FAIL — stack overflow ở test modem thứ hai | 127 |
| `git diff --check` | không có whitespace error | 0 |

### 11.2. Output nguyên văn (phần quyết định)

#### `cargo test -p volte-core a7_ -- --nocapture`

```text
running 2 tests
test init_rc::tests::a7_a_real_import_plus_comments_stays_idempotent ... ok
test init_rc::tests::a7_a_commented_import_does_not_count_as_present ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 238 filtered out; finished in 0.07s
```

#### `cargo test -p volte-core a6_ -- --nocapture`

```text
running 4 tests
test flags::tests::a6_a_missing_image_is_a_read_error_not_a_zero_hit ... ok
test flags::tests::a6_no_pair_found_is_still_ok_zero ... ok
test flags::tests::a6_a_copy_failure_is_a_write_error_not_a_zero_hit ... ok
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 236 filtered out; finished in 2.08s
```

#### `cargo test -p volte-core "pipeline::"`

```text
running 23 tests
test pipeline::tests::a_backup_with_the_wrong_hash_makes_restore_fatal ... ok
test pipeline::tests::a_failed_atomic_replace_is_fatal_and_removes_the_side_file ... ok
test pipeline::tests::a_pre_avb_backup_is_verified_by_size_and_hash ... ok
test pipeline::tests::a_truncated_backup_makes_restore_fatal_and_leaves_the_image_untouched ... ok
test pipeline::tests::a6_a_flag_patch_read_error_aborts_instead_of_reporting_zero ... ok
test pipeline::tests::a3_payload_mismatch_after_injection_aborts_build ... ok
test pipeline::tests::an_e2fsck_failure_fails_the_build_before_avb ... ok
test pipeline::tests::case_only_output_alias_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::hard_linked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::log_stamp_has_the_python_shape ... ok
test pipeline::tests::missing_source_is_a_build_error ... ok
test pipeline::tests::output_equal_to_the_source_is_refused_and_the_source_is_untouched ... ok
test pipeline::tests::output_path_defaults_next_to_the_source ... ok
test pipeline::tests::output_path_honours_keep_out_and_renames_a_src_collision ... ok
test pipeline::tests::relative_source_with_absolute_output_is_refused ... ok
test pipeline::tests::restore_replaces_the_image_with_the_verified_snapshot ... ok
test pipeline::tests::space_need_counts_the_copy_the_restore_side_and_the_temp_backup ... ok
test pipeline::tests::space_need_saturates_instead_of_overflowing ... ok
test pipeline::tests::symlinked_output_is_refused_when_the_platform_allows_it ... ok
test pipeline::tests::the_space_probe_allocates_and_cleans_up ... ok
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image has been running for over 60 seconds
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree has been running for over 60 seconds
test pipeline::tests::a_full_build_reaches_the_final_tokens_on_a_synthetic_image ... ok
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted has been running for over 60 seconds
test pipeline::tests::an_avb_backend_failure_never_claims_a_passed_hashtree ... ok
test pipeline::tests::an_explicit_out_path_that_differs_from_the_source_is_accepted ... ok

test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 217 filtered out; finished in 91.36s
```

#### `cargo build --workspace`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib) generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.02s
```

#### `cargo clippy --workspace --all-targets`

```text
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:384:31
    |
384 |             .trim_end_matches(|c: char| c == '\u{0}' || c == ' ')
    |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using an array of `char`: `['\u{0}', ' ']`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/main/index.html#manual_pattern_char_comparison
    = note: `#[warn(clippy::manual_pattern_char_comparison)]` on by default

warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:707:13
    |
707 |             (blob.len() + self.cluster_size as usize - 1) / self.cluster_size as usize
    |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using `.div_ceil()`: `blob.len().div_ceil(self.cluster_size as usize)`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/main/index.html#manual_div_ceil
    = note: `#[warn(clippy::manual_div_ceil)]` on by default

warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 suggestions)
warning: `volte-core` (lib test) generated 3 warnings (3 duplicates)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.91s
```

#### `cargo test --workspace` — trích đoạn quyết định

```text
running 240 tests
...
thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (1524) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

#### `cargo test -p volte-core --lib -- --skip build_modem_fix_from_mbn_injects_a_missing_viettel_mbn` — trích đoạn quyết định

```text
thread 'modem::tests::build_modem_fix_viettel_pulls_the_mbn_from_a_donor_image' (11328) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe --skip build_modem_fix_from_mbn_injects_a_missing_viettel_mbn` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
```

Nghĩa là có ít nhất 2 test trong `modem.rs` bị stack overflow, không chỉ 1 test như ghi chú cũ ở mục 8.3.

#### `git diff --check`

```text
Exit code: 0
```

Chỉ có cảnh báo LF/CRLF có sẵn của Git; không có whitespace error.

#### Đối chiếu baseline Giai đoạn 3

`diff -u` giữa `%TEMP%\opencode\p3_baseline\` và cây hiện tại cho 4 file `error.rs`, `init_rc.rs`, `flags.rs`, `pipeline.rs` khớp đúng các hunk A6/A7 trong mục 4; không có hunk lạ ngoài phạm vi.

### 11.3. Chênh lệch so với mục 6.7–6.9

- Report nói cây cuối cùng không còn `derive(Debug)` trong `modem.rs`. Cây hiện tại **có** `#[derive(Debug)]` tại `modem.rs:282` và `modem.rs:918`.
- Mtime: `modem.rs` = `2026-11-08 10:58:03`; report = `2026-11-08 10:52:56`. Tức file bị sửa **sau** khi report được ghi. **Không xác định được ai/cách nào thêm**; chỉ xác định được bằng mtime. `modem.rs` là file untracked nên không có Git diff để đối chiếu.
- Vì vậy, so với mục 6.7–6.8:
  - `cargo test --workspace`: từ "không biên dịch" → biên dịch được nhưng stack overflow.
  - `cargo clippy --workspace --all-targets`: từ FAIL → PASS (exit 0).
  - `cargo build --workspace`: vẫn PASS; warning `spc` chuyển từ dòng 285 sang 286.
- Kết luận về A7/A6 **không đổi**.

### 11.4. Đánh giá re-check

- **A7:** đã xác nhận đúng. `has_import()` bỏ comment và khớp nguyên dòng; 2 test A7 pass; module `init_rc::` 11/11 pass.
- **A6:** đã xác nhận đúng. `ErrorKind::Read/Write` được map tường minh; pipeline trả `Err` trước verify/export; 4 test A6 pass; module `pipeline::` 23/23 pass.
- **Phạm vi:** đúng. Chỉ 4 file + report; không đổi thuật toán QC/MTK hay inject MBN.
- **Workspace:** chưa xanh. `cargo test --workspace` vẫn FAIL vì stack overflow trong `modem.rs` (ngoài phạm vi Giai đoạn 3).
- **Ngoài phạm vi, chưa sửa:** `ext4_tools::read_vendor_flag_bit` vẫn nuốt lỗi dump và trả `0`; nếu dump thất bại nhưng patch chạy được, pipeline có thể log sai "the original vendor has no volte_support=1". Hành vi này có từ trước Giai đoạn 3 và không nằm trong 3 nhánh WARN của A6.
- **Chưa xác minh:** không revert code để chạy lại trạng thái "trước khi sửa"; không kiểm chứng trên thiết bị thật.

### 11.5. Trạng thái sau re-check

STATUS: DONE cho phạm vi A7 + A6. Workspace chưa xanh vì stack overflow trong `modem.rs` (ngoài phạm vi); mục 6.7–6.8 chỉ còn giá trị lịch sử cho cây tại thời điểm viết report.
