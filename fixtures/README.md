# Fixtures

Generated test inputs only. No vendor image, payload APK, RC file or property dump from a real
device is stored here.

`avb/` holds the committed vbmeta golden region and its generator,
`avb/generate_vbmeta_golden.py`; run that script with the bundled `tool/python/python.exe` when
the fixture or its reference values change. The other tests build their inputs at run time in the
system temp directory (`mke2fs` images, sparse images, property files), so `cargo test` does not
regenerate anything in this tree. Nothing here is hand-edited evidence.