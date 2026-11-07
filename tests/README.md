# Tests

Cross-crate contract artefacts for Phase 4 verification: captured EXT4 command files (debugfs
grammar, replayed by the in-process engine), `e2fsck` verdicts and parity dumps produced while
running the ported tool against copies of test images.

Per-crate automated tests live inside each crate (`volte-core\src\*\tests`, `volte-cli\tests\`).
Anything written here is evidence for a specific gate measurement, not a fixture the build
reads.
