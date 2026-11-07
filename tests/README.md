# Tests

Reserved for cross-crate contract artefacts captured during a gate measurement: EXT4 command
files, `e2fsck` verdicts and parity dumps produced while running the ported tool against copies
of test images. The directory is currently empty; committed evidence lives in `reports/fixes/`
and the raw command output stays outside the repository.

Automated tests live inside each crate: `volte-core\src\**\tests` (unit tests, including the
`rustfs_ext4` tests that build synthetic images with `mke2fs`) and the unit tests in
`volte-cli\src\main.rs`. There is no `volte-cli\tests\` directory. Anything written here would be
evidence for a specific gate measurement, not a fixture the build reads.