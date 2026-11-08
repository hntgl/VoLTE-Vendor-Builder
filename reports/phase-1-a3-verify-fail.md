# Giai đoạn 1 — A3: Verify payload fail vẫn export BUILD SUCCESS

STATUS: DONE

## 1. Thay đổi

- `volte-core/src/pipeline.rs`
  - Nhánh `!payload_ok` không còn chỉ ghi WARN rồi tiếp tục export. Pipeline trả ngay `Err(Error::build("Overlay APK/RC not verified inside the image - refusing to export"))`.
  - Thêm regression test `a3_payload_mismatch_after_injection_aborts_build` cùng test backend `CorruptingApkOps` trong `#[cfg(test)] mod tests`.
- `reports/phase-1-a3-verify-fail.md` (báo cáo này).

Không có thay đổi nào khác trong cây làm việc cuối cùng.

Ghi chú minh bạch: để test target biên dịch được, tôi đã **tạm thời** thêm `#[derive(Debug)]` vào `Fat16Image` và `ModemReport` trong `volte-core/src/modem.rs` (file untracked, lỗi có sẵn, ngoài phạm vi A3), chạy test, rồi đã gỡ bỏ. Hai dòng đó **không còn** trong cây cuối cùng; `modem.rs` không nằm trong diff A3.

## 2. Diff

### 2.1. Diff tập trung đúng phạm vi A3 (so với bản sao trước khi sửa)

```diff
git : warning: in the working copy of 'C:\Users\HT\AppData\Local\Temp\opencode\pipeline.a3.baseline.rs', LF will be 
replaced by CRLF the next time Git touches it
At line:1 char:1
+ git diff --no-index -- "$env:TEMP\opencode\pipeline.a3.baseline.rs" " ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (warning: in the... Git touches it:String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
warning: in the working copy of 'volte-core/src/pipeline.rs', LF will be replaced by CRLF the next time Git touches it
diff --git "a/C:\\Users\\HT\\AppData\\Local\\Temp\\opencode\\pipeline.a3.baseline.rs" b/volte-core/src/pipeline.rs
index b8b8749..bb22169 100644
--- "a/C:\\Users\\HT\\AppData\\Local\\Temp\\opencode\\pipeline.a3.baseline.rs"
+++ b/volte-core/src/pipeline.rs
@@ -589,7 +589,9 @@ pub fn run(
     if payload_ok {
         log.log("[STEP] Overlay APK + RC are inside the image (VoLTE switch).");
     } else {
-        log.log("[WARN] Overlay payload is not fully verified - still exporting the file.");
+        return Err(Error::build(
+            "Overlay APK/RC not verified inside the image - refusing to export",
+        ));
     }
 
     log.log("e2fsck -fn");
@@ -1412,4 +1414,113 @@ mod tests {
         );
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    /// The real backend, but it flips one byte of the overlay APK inside the
+    /// image immediately after the overlay write batch (A3). Verification then
+    /// has to notice the mismatch before the pipeline exports anything.
+    struct CorruptingApkOps {
+        inner: RustFsExt4Backend,
+        corrupted: bool,
+    }
+
+    impl Ext4Ops for CorruptingApkOps {
+        fn run(
+            &mut self,
+            image: &Path,
+            commands: &[String],
+            workdir: &Path,
+            log: &mut Logger,
+        ) -> Result<String> {
+            let out = self.inner.run(image, commands, workdir, log)?;
+            let wrote_apk = commands
+                .iter()
+                .any(|c| c.starts_with("write ") && c.ends_with(&format!(" {}", config::APK_NAME)));
+            if wrote_apk && !self.corrupted {
+                let host = workdir.join("a3_corrupt_dump.apk");
+                let dump = format!(
+                    "dump -p /overlay/{} {}",
+                    config::APK_NAME,
+                    crate::env::forward_slashes(&host)
+                );
+                self.inner
+                    .run(image, &[dump], workdir, &mut Logger::new())?;
+                let mut bytes = std::fs::read(&host)?;
+                assert!(!bytes.is_empty(), "the injected APK must not be empty");
+                bytes[0] ^= 0xFF;
+                std::fs::write(&host, &bytes)?;
+                let replace = format!(
+                    "write {} {}",
+                    crate::env::forward_slashes(&host),
+                    config::APK_NAME
+                );
+                self.inner.run(
+                    image,
+                    &["cd /overlay".to_owned(), replace],
+                    workdir,
+                    &mut Logger::new(),
+                )?;
+                self.corrupted = true;
+            }
+            Ok(out)
+        }
+
+        fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
+            self.inner.path_exists(image, vpath, workdir)
+        }
+    }
+
+    #[test]
+    fn a3_payload_mismatch_after_injection_aborts_build() {
+        let dir = tmp("volte_pipeline_a3_payload_mismatch");
+        let src = make_ext4(&dir, "vendor.img");
+        seed_payloads(&dir);
+        let logs = dir.join("Logs");
+
+        let mut ops = CorruptingApkOps {
+            inner: RustFsExt4Backend::new(),
+            corrupted: false,
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
+        assert!(
+            ops.corrupted,
+            "the test must corrupt the APK inside the image after injection"
+        );
+
+        let text = lines.borrow().join("\n");
+        assert!(text.contains("content mismatch"), "{text}");
+        assert!(text.contains("APK_SHA256=UNVERIFIED"), "{text}");
+
+        match result {
+            Ok(out) => panic!(
+                "A3: the build exported {} even though the overlay APK was corrupted after injection\n{text}",
+                out.display()
+            ),
+            Err(err) => assert_eq!(
+                err.message(),
+                "Overlay APK/RC not verified inside the image - refusing to export"
+            ),
+        }
+        assert!(!text.contains("[OUTPUT] "), "{text}");
+        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

### 2.2. `git diff -- volte-core/src/pipeline.rs` (nguyên văn)

Lưu ý: repo đang có sẵn các thay đổi chưa commit khác trong `pipeline.rs` (đổi tên file tạm và hai assertion của test cũ). Hai hunk đó **không phải** thay đổi của A3; hunk A3 là hunk tại `@@ -589,7 +589,9 @@` và phần test thêm ở cuối file.

```diff
git : warning: in the working copy of 'volte-core/src/pipeline.rs', LF will be replaced by CRLF the next time Git 
touches it
At line:1 char:1
+ git diff -- volte-core/src/pipeline.rs 2>&1 | Tee-Object -FilePath "$ ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (warning: in the... Git touches it:String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
diff --git a/volte-core/src/pipeline.rs b/volte-core/src/pipeline.rs
index 81d1003..bb22169 100644
--- a/volte-core/src/pipeline.rs
+++ b/volte-core/src/pipeline.rs
@@ -57,7 +57,7 @@ impl WorkDir {
             .map(|d| d.subsec_nanos())
             .unwrap_or(0);
         let path =
-            std::env::temp_dir().join(format!("hd_hue_chuan_{}_{nanos}", std::process::id()));
+            std::env::temp_dir().join(format!("volte_patch_{}_{nanos}", std::process::id()));
         std::fs::create_dir_all(&path)?;
         Ok(WorkDir(path))
     }
@@ -170,7 +170,7 @@ fn probe_space(dir: &Path, needed: u64) -> std::io::Result<()> {
     if needed == 0 {
         return Ok(());
     }
-    let path = dir.join(format!(".hd_volte_space_probe_{}.tmp", std::process::id()));
+    let path = dir.join(format!(".volte_space_probe_{}.tmp", std::process::id()));
     let file = std::fs::OpenOptions::new()
         .write(true)
         .create_new(true)
@@ -589,7 +589,9 @@ pub fn run(
     if payload_ok {
         log.log("[STEP] Overlay APK + RC are inside the image (VoLTE switch).");
     } else {
-        log.log("[WARN] Overlay payload is not fully verified - still exporting the file.");
+        return Err(Error::build(
+            "Overlay APK/RC not verified inside the image - refusing to export",
+        ));
     }
 
     log.log("e2fsck -fn");
@@ -1240,8 +1242,8 @@ mod tests {
             commands.contains("\n\n"),
             "clean and write blocks are separated"
         );
-        assert!(commands.starts_with("rm /overlay/HDVoLTE/OPPO_VoLTE_Overlay.apk\n"));
-        assert!(commands.contains("\nmkdir /overlay/HDVoLTE\n"));
+        assert!(commands.starts_with("rm /overlay/OPPO_VoLTE_Overlay.apk\n"));
+        assert!(commands.contains("\ncd /overlay\n"));
         let report = std::fs::read_to_string(logs.join("vendor_VolLTE_report.txt")).unwrap();
         assert_eq!(report.lines().count(), 6, "{report}");
         assert!(report.starts_with("File="));
@@ -1412,4 +1414,113 @@ mod tests {
         );
         let _ = std::fs::remove_dir_all(&dir);
     }
+
+    /// The real backend, but it flips one byte of the overlay APK inside the
+    /// image immediately after the overlay write batch (A3). Verification then
+    /// has to notice the mismatch before the pipeline exports anything.
+    struct CorruptingApkOps {
+        inner: RustFsExt4Backend,
+        corrupted: bool,
+    }
+
+    impl Ext4Ops for CorruptingApkOps {
+        fn run(
+            &mut self,
+            image: &Path,
+            commands: &[String],
+            workdir: &Path,
+            log: &mut Logger,
+        ) -> Result<String> {
+            let out = self.inner.run(image, commands, workdir, log)?;
+            let wrote_apk = commands
+                .iter()
+                .any(|c| c.starts_with("write ") && c.ends_with(&format!(" {}", config::APK_NAME)));
+            if wrote_apk && !self.corrupted {
+                let host = workdir.join("a3_corrupt_dump.apk");
+                let dump = format!(
+                    "dump -p /overlay/{} {}",
+                    config::APK_NAME,
+                    crate::env::forward_slashes(&host)
+                );
+                self.inner
+                    .run(image, &[dump], workdir, &mut Logger::new())?;
+                let mut bytes = std::fs::read(&host)?;
+                assert!(!bytes.is_empty(), "the injected APK must not be empty");
+                bytes[0] ^= 0xFF;
+                std::fs::write(&host, &bytes)?;
+                let replace = format!(
+                    "write {} {}",
+                    crate::env::forward_slashes(&host),
+                    config::APK_NAME
+                );
+                self.inner.run(
+                    image,
+                    &["cd /overlay".to_owned(), replace],
+                    workdir,
+                    &mut Logger::new(),
+                )?;
+                self.corrupted = true;
+            }
+            Ok(out)
+        }
+
+        fn path_exists(&mut self, image: &Path, vpath: &str, workdir: &Path) -> Result<bool> {
+            self.inner.path_exists(image, vpath, workdir)
+        }
+    }
+
+    #[test]
+    fn a3_payload_mismatch_after_injection_aborts_build() {
+        let dir = tmp("volte_pipeline_a3_payload_mismatch");
+        let src = make_ext4(&dir, "vendor.img");
+        seed_payloads(&dir);
+        let logs = dir.join("Logs");
+
+        let mut ops = CorruptingApkOps {
+            inner: RustFsExt4Backend::new(),
+            corrupted: false,
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
+        assert!(
+            ops.corrupted,
+            "the test must corrupt the APK inside the image after injection"
+        );
+
+        let text = lines.borrow().join("\n");
+        assert!(text.contains("content mismatch"), "{text}");
+        assert!(text.contains("APK_SHA256=UNVERIFIED"), "{text}");
+
+        match result {
+            Ok(out) => panic!(
+                "A3: the build exported {} even though the overlay APK was corrupted after injection\n{text}",
+                out.display()
+            ),
+            Err(err) => assert_eq!(
+                err.message(),
+                "Overlay APK/RC not verified inside the image - refusing to export"
+            ),
+        }
+        assert!(!text.contains("[OUTPUT] "), "{text}");
+        assert!(!logs.join("vendor_VolLTE_report.txt").exists());
+        let _ = std::fs::remove_dir_all(&dir);
+    }
 }
```

## 3. Test đã thêm

- Tên test: `a3_payload_mismatch_after_injection_aborts_build`
- Đường dẫn: `volte-core/src/pipeline.rs`, trong `#[cfg(test)] mod tests`
- Mục đích: chứng minh nếu APK overlay bị thay đổi **bên trong image sau khi inject** thì verify phát hiện mismatch và pipeline phải trả `Err`, không được export/BUILD SUCCESS.
- Kịch bản:
  1. Tạo image EXT4 thật bằng `mke2fs.exe` của project (`make_ext4`), seed payload APK/RC.
  2. Chạy pipeline với `RustFsExt4Backend` thật, bọc trong `CorruptingApkOps`.
  3. Sau batch `write ... OPPO_VoLTE_Overlay.apk` (inject xong), wrapper dump chính file `/overlay/OPPO_VoLTE_Overlay.apk` ra host, lật 1 byte (`bytes[0] ^= 0xFF`), rồi ghi lại vào đúng đường dẫn trong image (`cd /overlay` + `write`).
  4. Pipeline chạy `verify_payload`; test khẳng định log có `content mismatch` và `APK_SHA256=UNVERIFIED`.
  5. Nếu pipeline trả `Ok`, test panic với thông báo nêu rõ file đã bị export. Nếu trả `Err`, test khẳng định chính xác thông báo `Overlay APK/RC not verified inside the image - refusing to export`.
  6. Test cũng khẳng định không có token `[OUTPUT] ` và không có `vendor_VolLTE_report.txt`.

Không sửa production code để tạo điều kiện cho test; test dùng đúng seam `Ext4Ops` sẵn có.

## 4. Kết quả chạy thật

Môi trường:

- `cargo 1.101.0-nightly (cad07cf8e 2026-10-23)`
- `rustc 1.101.0-nightly (12433c307 2026-10-24)`
- Windows, workspace `E:\Build_app\Volte\VoLTE-Vendor-Builder`
- `tool\mke2fs.exe` và `tool\e2fsck.exe` có mặt.

### Trước khi sửa — chứng minh bug tồn tại

Lệnh:

```text
cargo test -p volte-core a3_payload_mismatch_after_injection_aborts_build -- --nocapture
```

Kết quả: FAILED đúng lý do cần chứng minh. Log cho thấy verify đã phát hiện `content mismatch`, nhưng pipeline vẫn ghi `[WARN] Overlay payload is not fully verified - still exporting the file.`, chạy tiếp e2fsck/AVB và xuất `[OUTPUT]`. Test panic vì nhận `Ok` thay vì `Err`.

```text
cargo :    Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
At line:1 char:1
+ cargo test -p volte-core a3_payload_mismatch_after_injection_aborts_b ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (   Compiling vo...der\volte-core):String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code 
analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 14.34s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 1 test

thread 'pipeline::tests::a3_payload_mismatch_after_injection_aborts_build' (8184) panicked at 
volte-core\src\pipeline.rs:1512:24:
A3: the build exported C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor_VolLTE.img even though 
the overlay APK was corrupted after injection
[CHIP] Forced MEDIATEK branch (user choice).
================================================================
HD Vendor Fix VoLTE
Version: 2.9.2 / V298
Mode   : STANDARD - e2fsck + AVB signature, EDL-sized
Engine : hoangduc
Chip   : MTK
Input  : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor.img
Output : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor_VolLTE.img
Log    : C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\Logs\hue_chuan_20261108_031300.log
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
[STEP] VoLTE flag: 0 (the original vendor has no volte_support=1)
[STEP] VoLTE flag: 0
[STEP] Flag =0 - the original vendor has no volte_support=1 and no 0->1 patch was applied.
APK_SHA256=UNVERIFIED
RC_SHA256=D721EC4741CC0EABF6CD4B6D475818FC68D95B2231C414EEFA942428995FF4DD
[WARN] Payload copy /overlay/OPPO_VoLTE_Overlay.apk: content mismatch
[WARN] Overlay payload not verified (APK: not verified, RC: verified).
[WARN] Overlay payload is not fully verified - still exporting the file.
e2fsck -fn
Pass 1: checking C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor_VolLTE.img
e2fsck=PASS
[STATUS] Pre-AVB snapshot verified (size + SHA-256).
[STATUS] Generating hashtree and FEC for the image...
[STATUS] Erasing old AVB footer...
[STATUS] Rebuilding AVB hashtree...
[AVB] locking partition_size=67108864 (EDL slot, no growth).
[AVB] add_hashtree partition_size=67108864 (attempt 1)
[WARN] AVB hashtree: AVB backend returned Ok but no AVB footer exists after the rebuild
[STEP] Dropping AVB + EDL size - no footer truncation when the hashtree grows.
[OK] Restored the pre-AVB image (size + SHA-256 verified).
[OK] Size BOOT-SAFE matches slot: 67108864 (0x4000000)
[OK] Size BUILD matches slot: 67108864 (0x4000000)
File=C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor_VolLTE.img
Size=67108864
[OK] EDL size matches - UnlockTool can flash it.
e2fsck=PASS
AVB_HASHTREE=SKIP
[NOTE] AVB hashtree skipped - EDL + overlay; the device boots the original image (footer not truncated).
VOLTE_FLAGS=0
AVB_ALGORITHM=NONE
APK_SHA256=UNVERIFIED
RC_SHA256=D721EC4741CC0EABF6CD4B6D475818FC68D95B2231C414EEFA942428995FF4DD
FLASH=NO
[DONE] Total time: 00:56
[OUTPUT] C:\Users\HT\AppData\Local\Temp\volte_pipeline_a3_payload_mismatch\vendor_VolLTE.img
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test pipeline::tests::a3_payload_mismatch_after_injection_aborts_build ... FAILED

failures:

failures:
    pipeline::tests::a3_payload_mismatch_after_injection_aborts_build

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 226 filtered out; finished in 57.22s

error: test failed, to rerun pass `-p volte-core --lib`
```

Phân loại: FAIL do assertion mong đợi `Err` nhưng nhận `Ok` — tái hiện đúng bug A3, không phải lỗi compile/fixture/môi trường.

### Sau khi sửa

Lệnh:

```text
cargo test -p volte-core a3_payload_mismatch_after_injection_aborts_build -- --nocapture
```

```text
cargo :    Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
At line:1 char:1
+ cargo test -p volte-core a3_payload_mismatch_after_injection_aborts_b ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (   Compiling vo...der\volte-core):String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code 
analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib test) generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.84s
     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 1 test
test pipeline::tests::a3_payload_mismatch_after_injection_aborts_build ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 226 filtered out; finished in 18.53s
```

### Cargo test workspace

Lệnh:

```text
cargo test --workspace
```

Kết quả trên cây cuối cùng (đã gỡ bản vá tạm ở `modem.rs`): **không biên dịch được test target** vì lỗi có sẵn trong `modem.rs` (ngoài phạm vi A3).

```text
cargo :    Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
At line:1 char:1
+ cargo test --workspace 2>&1 | Tee-Object -FilePath "$env:TEMP\opencod ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (   Compiling vo...der\volte-core):String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
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

error[E0277]: `modem::Fat16Image` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1240:49
     |
1240 |         let e = Fat16Image::new(vec![0u8; 100]).unwrap_err();
     |                                                 ^^^^^^^^^^ the trait `Debug` is not implemented for 
`modem::Fat16Image`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::Fat16Image` with `#[derive(Debug)]`
     |
 282 + #[derive(Debug)]
 283 | pub struct Fat16Image {
     |

error[E0277]: `ModemReport` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1453:59
     |
1453 |         let err = build_modem_volte(&src, None, &mut log).unwrap_err();
     |                                                           ^^^^^^^^^^ the trait `Debug` is not implemented for 
`ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `ModemReport` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1462:74
     |
1462 |         let err = build_modem_volte(&src, Some(src.as_path()), &mut log).unwrap_err();
     |                                                                          ^^^^^^^^^^ the trait `Debug` is not 
implemented for `ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `ModemReport` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1509:79
     |
1509 |         let err = build_modem_fix_from_mbn(&dest, &[0u8; 10], None, &mut log).unwrap_err();
     |                                                                               ^^^^^^^^^^ the trait `Debug` is 
not implemented for `ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `ModemReport` doesn't implement `Debug`
    --> volte-core\src\modem.rs:1551:74
     |
1551 |         let err = build_modem_fix_viettel(&dest, &donor, None, &mut log).unwrap_err();
     |                                                                          ^^^^^^^^^^ the trait `Debug` is not 
implemented for `ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

warning: `volte-core` (lib) generated 1 warning
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
For more information about this error, try `rustc --explain E0277`.
error: could not compile `volte-core` (lib test) due to 5 previous errors
warning: build failed, waiting for other jobs to finish...
```

Để có thêm bằng chứng, tôi đã chạy lại workspace test khi còn bản vá tạm `#[derive(Debug)]` (chỉ để test target biên dịch). Kết quả: 30/30 test `volte-cli` PASS; `volte-core` chạy được 227 test nhưng **stack overflow** tại một test modem không liên quan A3. Sau đó bản vá tạm đã được gỡ.

```text
cargo :    Compiling volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
At line:1 char:1
+ cargo test --workspace 2>&1 | Tee-Object -FilePath "$env:TEMP\opencod ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (   Compiling vo...der\volte-core):String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
warning: field `spc` is never read
   --> volte-core\src\modem.rs:286:5
    |
283 | pub struct Fat16Image {
    |            ---------- field in this struct
...
286 |     spc: u32,
    |     ^^^
    |
    = note: `Fat16Image` has a derived impl for the trait `Debug`, but this is intentionally ignored during dead code 
analysis
    = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `volte-core` (lib test) generated 1 warning
warning: `volte-core` (lib) generated 1 warning (1 duplicate)
   Compiling volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.21s
     Running unittests src\main.rs (target\debug\build\volte-cli/d68a8014c9353575\out\volte_cli-d68a8014c9353575.exe)

running 30 tests
test modem_cli::tests::fix_modem_accepts_equals_forms ... ok
test modem_cli::tests::fix_modem_parses_donor ... ok
test modem_cli::tests::fix_modem_rejects_both_donor_and_mbn ... ok
test modem_cli::tests::fix_modem_parses_mbn ... ok
test modem_cli::tests::fix_modem_requires_a_source ... ok
test modem_cli::tests::fix_modem_unknown_option_is_a_usage_error ... ok
test modem_cli::tests::modem_accepts_out_equals_form ... ok
test modem_cli::tests::modem_help_prints_and_returns_none ... ok
test modem_cli::tests::modem_parses_image_and_out ... ok
test modem_cli::tests::modem_rejects_a_second_positional ... ok
test modem_cli::tests::modem_requires_an_image ... ok
test tests::a_bare_image_with_no_subcommand_is_rejected ... ok
test modem_cli::tests::out_path_turns_empty_string_into_none ... ok
test tests::a_pipeline_error_exits_non_zero ... ok
test tests::a_second_positional_is_a_usage_error ... ok
test tests::double_dash_ends_option_parsing ... ok
test tests::defaults_are_empty_like_argparse ... ok
test tests::gui_flag_alone_selects_the_divergence_not_a_build ... ok
test tests::help_wins_from_any_position ... ok
test tests::missing_out_value_is_a_usage_error ... ok
test tests::no_subcommand_is_a_usage_error ... ok
test tests::out_accepts_both_forms ... ok
test tests::modem_and_fix_modem_subcommands_are_unaffected ... ok
test tests::parse_accepts_the_five_python_options ... ok
test tests::qc_wins_over_mtk_like_python ... ok
test tests::top_level_help_is_not_a_subcommand ... ok
test tests::unknown_option_is_a_usage_error ... ok
test tests::unknown_subcommand_is_a_usage_error ... ok
test tests::vendor_subcommand_help_still_works ... ok
test tests::vendor_subcommand_with_no_image_hits_the_gui_divergence ... ok

test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running unittests src\lib.rs (target\debug\build\volte-core/f4cc364b59c33ac2\out\volte_core-f4cc364b59c33ac2.exe)

running 227 tests
test avb::tests::a_footer_that_does_not_reparse_is_not_erased ... ok
test avb::tests::a_failing_backend_over_an_old_footer_is_never_reported_as_success ... ok
test avb::tests::a_failing_backend_is_an_error_and_keeps_the_image_untouched ... ok
test avb::tests::cli_args_carry_the_frozen_parameters ... ok
test avb::tests::a_verified_rebuild_reports_the_new_footer ... ok
test avb::tests::fit_partition_size_properties ... ok
test avb::tests::info_defaults_without_footer ... ok
test avb::tests::info_reads_the_golden_vbmeta_fields ... ok
test avb::tests::missing_avbtool_names_the_exact_path ... ok
test avb::tests::parse_accepts_valid_footer ... ok
test avb::tests::parse_rejects_magic_size_and_offsets ... ok
test avb::tests::random_salt_is_64_lower_hex_chars_and_changes ... ok
test avb::tests::info_scans_back_past_padding ... ok
test avb::tests::a_backend_that_returns_ok_without_writing_a_footer_is_an_error ... ok
test avb::tests::the_golden_fixture_matches_its_reference_digest ... ok
test avb::tests::a_truncated_or_malformed_vbmeta_is_an_error ... ok
test avb::tests::tree_size_uses_32_byte_digests ... ok
test avb::tests::twenty_five_salts_are_all_distinct ... ok
test avb::tests::the_rebuild_request_carries_the_parsed_metadata ... ok
test chip_detect::tests::clash_prefers_the_basename_then_qc ... ok
test chip_detect::tests::binary_markers_win_and_qc_is_checked_first ... ok
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
test avb::tests::rebuild_detects_footer_and_keeps_the_locked_size ... ok
test error::tests::build_error_display_is_the_message ... ok
test error::tests::io_error_maps_to_io_kind ... ok
test error::tests::missing_names_the_exact_path ... ok
test avb::tests::rebuild_refuses_to_shrink_a_grown_image ... ok
test ext4_tools::tests::e2fsck_falls_back_to_the_bare_name ... ok
test ext4_tools::tests::a_hung_tool_is_killed_at_the_deadline ... ok
test ext4_tools::tests::tail_chars_counts_characters_not_bytes ... ok
test chip_detect::tests::markers_beyond_16_mib_are_not_read ... ok
test flags::tests::boundary_uses_ascii_names_only ... ok
test flags::tests::case_alias_patches_in_place_instead_of_copying ... ok
test flags::tests::every_pair_is_applied_once_and_size_is_kept ... ok
test flags::tests::flag_bit_reads_both_dumps_and_separates_them ... ok
test flags::tests::flag_pairs_are_65_length_equal_and_non_empty ... ok
test flags::tests::hard_link_alias_patches_in_place_when_the_platform_allows_it ... ok
test flags::tests::limit_restricts_the_scanned_span ... ok
test ext4_tools::tests::a_missing_e2fsck_executable_is_an_error ... ok
test flags::tests::patches_only_whole_property_names ... ok
test flags::tests::same_path_patches_in_place_and_empty_file_returns_zero ... ok
test ext4_tools::tests::vendor_flag_bit_reads_the_dumped_build_prop ... ok
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
test modem::tests::a_blank_image_parses_and_reports_its_oem ... ok
test modem::tests::a_non_power_of_two_sectors_per_cluster_is_rejected ... ok
test modem::tests::a_too_short_buffer_is_rejected ... ok
test modem::tests::a_zero_bytes_per_sector_is_rejected ... ok

thread 'modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn' (8456) has overflowed its stack
error: test failed, to rerun pass `-p volte-core --lib`

Caused by:
  process didn't exit successfully: `E:\Build_app\Volte\VoLTE-Vendor-Builder\target\debug\build\volte-core/f4cc364b59c3
3ac2\out\volte_core-f4cc364b59c33ac2.exe` (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)
note: test exited abnormally; to see the full output pass --no-capture to the harness.
```

### Cargo build workspace

Lệnh:

```text
cargo build --workspace
```

Kết quả: PASS (chỉ còn warning dead-code có sẵn trong `modem.rs`).

```text
cargo : warning: field `spc` is never read
At line:1 char:1
+ cargo build --workspace 2>&1 | Tee-Object -FilePath "$env:TEMP\openco ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (warning: field `spc` is never read:String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
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
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.75s
```

### Cargo clippy workspace

Lệnh:

```text
cargo clippy --workspace --all-targets
```

Kết quả: FAIL ở bước biên dịch `--all-targets` do cùng lỗi `Debug` có sẵn trong `modem.rs`; ngoài ra có các warning clippy có sẵn trong `modem.rs`.

```text
cargo :     Checking volte-core v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-core)
At line:1 char:1
+ cargo clippy --workspace --all-targets 2>&1 | Tee-Object -FilePath "$ ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (    Checking vo...der\volte-core):String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
error[E0277]: `modem::Fat16Image` doesn't implement `std::fmt::Debug`
    --> volte-core\src\modem.rs:1240:49
     |
1240 |         let e = Fat16Image::new(vec![0u8; 100]).unwrap_err();
     |                                                 ^^^^^^^^^^ the trait `std::fmt::Debug` is not implemented for 
`modem::Fat16Image`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::Fat16Image` with `#[derive(Debug)]`
     |
 282 + #[derive(Debug)]
 283 | pub struct Fat16Image {
     |

error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`
    --> volte-core\src\modem.rs:1453:59
     |
1453 |         let err = build_modem_volte(&src, None, &mut log).unwrap_err();
     |                                                           ^^^^^^^^^^ the trait `std::fmt::Debug` is not 
implemented for `modem::ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`
    --> volte-core\src\modem.rs:1462:74
     |
1462 |         let err = build_modem_volte(&src, Some(src.as_path()), &mut log).unwrap_err();
     |                                                                          ^^^^^^^^^^ the trait `std::fmt::Debug` 
is not implemented for `modem::ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`
    --> volte-core\src\modem.rs:1509:79
     |
1509 |         let err = build_modem_fix_from_mbn(&dest, &[0u8; 10], None, &mut log).unwrap_err();
     |                                                                               ^^^^^^^^^^ the trait 
`std::fmt::Debug` is not implemented for `modem::ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

error[E0277]: `modem::ModemReport` doesn't implement `std::fmt::Debug`
    --> volte-core\src\modem.rs:1551:74
     |
1551 |         let err = build_modem_fix_viettel(&dest, &donor, None, &mut log).unwrap_err();
     |                                                                          ^^^^^^^^^^ the trait `std::fmt::Debug` 
is not implemented for `modem::ModemReport`
     |
note: required by a bound in `std::result::Result::<T, E>::unwrap_err`
    --> /rustc/12433c307b5a497388ffe2f4e49dbf1ab8c71fa0/library/core\src\result.rs:1323:4
help: consider annotating `modem::ModemReport` with `#[derive(Debug)]`
     |
 902 + #[derive(Debug)]
 903 | pub struct ModemReport {
     |

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

warning: this manual char comparison can be written more succinctly
   --> volte-core\src\modem.rs:373:31
    |
373 |             .trim_end_matches(|c: char| c == '\u{0}' || c == ' ')
    |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using an array of `char`: 
`['\u{0}', ' ']`
    |
    = help: for further information visit 
https://rust-lang.github.io/rust-clippy/main/index.html#manual_pattern_char_comparison
    = note: `#[warn(clippy::manual_pattern_char_comparison)]` on by default

warning: manually reimplementing `div_ceil`
   --> volte-core\src\modem.rs:691:13
    |
691 |             (blob.len() + self.cluster_size as usize - 1) / self.cluster_size as usize
    |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: consider using 
`.div_ceil()`: `blob.len().div_ceil(self.cluster_size as usize)`
    |
    = help: for further information visit https://rust-lang.github.io/rust-clippy/main/index.html#manual_div_ceil
    = note: `#[warn(clippy::manual_div_ceil)]` on by default

warning: `volte-core` (lib) generated 3 warnings (run `cargo clippy --fix --lib -p volte-core -- ` to apply 2 
suggestions)
    Checking volte-cli v0.1.0 (E:\Build_app\Volte\VoLTE-Vendor-Builder\volte-cli)
For more information about this error, try `rustc --explain E0277`.
error: could not compile `volte-core` (lib test) due to 5 previous errors
```

### Git diff --check

Lệnh:

```text
git diff --check
```

Exit code: `0`. Chỉ có cảnh báo LF/CRLF của Git cho các file đang có sẵn thay đổi; không có whitespace error.

```text
git : warning: in the working copy of 'README.md', LF will be replaced by CRLF the next time Git touches it
At line:1 char:1
+ git diff --check *> "$env:TEMP\opencode\git_diff_check_final.txt"; "g ...
+ ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : NotSpecified: (warning: in the... Git touches it:String) [], RemoteException
    + FullyQualifiedErrorId : NativeCommandError
 
warning: in the working copy of 'profiles/mediatek/README.md', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'profiles/qualcomm/README.md', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-cli/src/main.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/config.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/ext4_tools.rs', LF will be replaced by CRLF the next time Git touches 
it
warning: in the working copy of 'volte-core/src/flags.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/init_rc.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/lib.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/payload.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/pipeline.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/profile.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/props.rs', LF will be replaced by CRLF the next time Git touches it
warning: in the working copy of 'volte-core/src/verify.rs', LF will be replaced by CRLF the next time Git touches it
```

## 5. Đánh giá kết quả

- **Đã chứng minh được bug cũ:** Có. Trước khi sửa, test FAIL vì pipeline trả `Ok` và ghi `[OUTPUT]` dù verify báo `content mismatch` / `APK_SHA256=UNVERIFIED`.
- **Đã sửa code:** Có. Nhánh `!payload_ok` trả `Err(Error::build("Overlay APK/RC not verified inside the image - refusing to export"))` trước e2fsck/AVB/export.
- **Đã xác nhận regression test:** Có. Sau khi sửa, đúng test đó PASS, `1 passed; 0 failed`.
- **Đã kiểm tra workspace:** một phần.
  - `cargo build --workspace`: PASS.
  - `cargo test --workspace`: FAIL do lỗi biên dịch test có sẵn trong `modem.rs` (thiếu `Debug`), không liên quan A3.
  - Khi tạm vá để biên dịch: 30/30 test CLI PASS, nhưng `volte-core` gặp stack overflow ở test modem, cũng không liên quan A3.
  - `cargo clippy --workspace --all-targets`: FAIL vì cùng lỗi biên dịch test của `modem.rs`.
- **Hạng mục chưa thể xác minh:** Không thể xác nhận toàn bộ workspace test/clippy PASS trên cây hiện tại vì các lỗi có sẵn ngoài phạm vi A3. Không có bằng chứng nào cho thấy A3 gây ra các lỗi này.
- Không lấy `cargo build` thành công làm bằng chứng VoLTE hoạt động thực tế trên thiết bị. Đây chỉ là kiểm chứng logic pipeline và regression test trên image EXT4 tổng hợp.

## 6. Phát hiện ngoài phạm vi

Không tự sửa các mục sau:

1. `volte-core/src/modem.rs` (untracked) không biên dịch được test target:
   - `Fat16Image` thiếu `Debug` tại `modem.rs:1240`.
   - `ModemReport` thiếu `Debug` tại `modem.rs:1453`, `1462`, `1509`, `1551`.
2. Khi tạm thêm `Debug`, workspace test gặp `STATUS_STACK_OVERFLOW` tại `modem::tests::build_modem_fix_from_mbn_injects_a_missing_viettel_mbn`.
3. `cargo clippy` báo warning có sẵn trong `modem.rs`: `spc` never read, `manual_pattern_char_comparison`, `manual_div_ceil`.
4. `rustfmt --check volte-core/src/pipeline.rs` báo lệch ở dòng 56 — đây là định dạng có sẵn của thay đổi chưa commit trước đó (`std::env::temp_dir().join(...)`), không phải do A3. Phần code mới của A3 đã được định dạng theo rustfmt.
5. Cây làm việc có nhiều thay đổi/xóa file chưa commit từ trước (README, profiles, reports cũ, `modem.rs`, `modem_cli.rs`, zip...). Không đụng tới.

## 7. File đã thay đổi

- `volte-core/src/pipeline.rs`
- `reports/phase-1-a3-verify-fail.md` (mới)

`volte-core/src/modem.rs` đã được sửa tạm thời để chạy test rồi gỡ bỏ; nội dung cuối cùng không thay đổi so với trước khi thực hiện A3.

## 8. Kết luận

A3 đã được tái hiện bằng test thật trên image EXT4, sửa đúng một nhánh trong `pipeline.rs`, và regression test đã PASS sau khi sửa. Pipeline giờ trả `Err` với đúng thông báo yêu cầu và không đi tiếp tới bước export.

STATUS: DONE cho phạm vi A3. Các kiểm tra workspace rộng hơn đang bị chặn bởi lỗi có sẵn trong `modem.rs` (ngoài phạm vi), đã ghi nhận và không tự sửa.
