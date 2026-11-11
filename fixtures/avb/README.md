# AVB golden fixture

`vbmeta_vendor_golden.bin` is the 704-byte vbmeta region of a synthetic image
built by the bundled `tool/avbtool.py` 1.3.0:

```
python fixtures/avb/generate_vbmeta_golden.py
```

The script writes `vbmeta_vendor_golden.info.txt` from
`avbtool.py info_image`; the Rust golden test asserts against those reference
values, not against the parser's own output. No vendor image is used.

Reference values: partition `vendor`, hash algorithm `sha256`, FEC roots `0`,
salt `00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`,
root digest `ed31474c9a630faa3981531d8b443d325734e82b7e4a360ac60d1b4b416ad0ca`,
properties `com.android.build.vendor=v2` and
`com.android.build.vendor.extra=hello`.