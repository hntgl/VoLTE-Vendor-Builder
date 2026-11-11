"""Regenerates the AVB vbmeta golden fixture with the bundled avbtool.py.

The reference values consumed by `volte-core/src/avb.rs` come from the
`info_image` output written next to the binary, not from this repository's
parser. Run from any directory:

    python fixtures/avb/generate_vbmeta_golden.py

Requires Python 3; `tool/avbtool.py` is the bundled avbtool 1.3.0.
"""

import pathlib
import struct
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
AVBTOOL = ROOT / "tool" / "avbtool.py"
IMAGE = HERE / "_golden.img"
VBMETA_BIN = HERE / "vbmeta_vendor_golden.bin"
INFO_TXT = HERE / "vbmeta_vendor_golden.info.txt"

PARTITION_SIZE = 2 * 1024 * 1024
ORIGINAL_SIZE = 1024 * 1024
SALT = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"


def run(*args):
    subprocess.run([sys.executable, str(AVBTOOL), *args], check=True)


def main():
    IMAGE.write_bytes(b"\0" * ORIGINAL_SIZE)
    run(
        "add_hashtree_footer",
        "--image", str(IMAGE),
        "--partition_size", str(PARTITION_SIZE),
        "--partition_name", "vendor",
        "--hash_algorithm", "sha256",
        "--block_size", "4096",
        "--salt", SALT,
        "--algorithm", "NONE",
        "--rollback_index", "0",
        "--rollback_index_location", "0",
        "--flags", "0",
        "--do_not_generate_fec",
        "--fec_num_roots", "2",
        "--prop", "com.android.build.vendor:v2",
        "--prop", "com.android.build.vendor.extra:hello",
    )
    data = IMAGE.read_bytes()
    footer = data[-64:]
    if footer[:4] != b"AVBf":
        raise SystemExit("the image has no AVB footer")
    vbmeta_offset, = struct.unpack("!Q", footer[20:28])
    vbmeta_size, = struct.unpack("!Q", footer[28:36])
    VBMETA_BIN.write_bytes(data[vbmeta_offset:vbmeta_offset + vbmeta_size])
    info = subprocess.run(
        [sys.executable, str(AVBTOOL), "info_image", "--image", str(IMAGE)],
        check=True,
        capture_output=True,
        text=True,
    )
    INFO_TXT.write_text(info.stdout, encoding="utf-8", newline="\n")
    IMAGE.unlink()
    print(f"{VBMETA_BIN.name}: {vbmeta_size} bytes")


if __name__ == "__main__":
    main()