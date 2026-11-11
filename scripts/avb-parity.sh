#!/usr/bin/env bash
# Byte-compares the native AVB backend with the bundled avbtool.py.
#
# These tests are #[ignore]d by default because they need the Python oracle
# (and, for the real-image case, a multi-GB vendor dump). Run this before a
# release or on a schedule so they are not forgotten:
#
#   scripts/avb-parity.sh
#   VOLTE_AVB_PARITY_PYTHON=/path/to/python scripts/avb-parity.sh
#   VOLTE_AVB_REAL_IMAGE=D:/dump/vendor.img scripts/avb-parity.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

python="${VOLTE_AVB_PARITY_PYTHON:-$root/tool/python/python.exe}"
if [[ ! -f "$python" ]]; then
    echo "python not found at $python; set VOLTE_AVB_PARITY_PYTHON" >&2
    exit 1
fi
if [[ ! -f "$root/tool/avbtool.py" ]]; then
    echo "tool/avbtool.py is missing" >&2
    exit 1
fi

export VOLTE_AVB_PARITY_PYTHON="$python"
cargo test -p volte-core avb::tests::avb_parity -- --ignored --nocapture
