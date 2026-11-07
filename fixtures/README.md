# Fixtures

Generated test inputs only. No vendor image, payload APK, RC file or property dump from a real
device is stored here.

Each fixture is produced by a small generator committed next to the test that consumes it
(synthetic sparse images, synthetic EXT4 images built with `mke2fs`, synthetic property
files). Regenerate with `cargo test`; nothing in this tree is hand-edited evidence.
