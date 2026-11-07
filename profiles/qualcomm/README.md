# Qualcomm profile

Constants used by the `--qc` branch. All values are ports of module constants in
`vendor_patcher\hd_vendor_build.py`; the code lives in `volte-core\src\profile.rs`.

| Constant | Python | Value |
| --- | --- | --- |
| APK candidate chain | `APK_QC_CANDIDATES` `:81-91` | `APK_NAME_QC` first, then MTK names as fallback |
| RC candidate chain | `RC_QC_CANDIDATES` `:81-91` | `hd_oppo_qc_volte.rc`, then `ha_oppo_qc_volte.rc` etc. |
| Import lines | `IMPORT_LINE`, `IMPORT_LINE_HW` | `import /vendor/etc/init/hd_oppo_qc_volte.rc` (+ `hw`) |
| Overlay command counts | `overlay_cmds` `:857` | 20 clean, 38 write |
| Init target candidates | `link_init_target` `:965` | 5 QC candidates, then 3 shared |
| Chip keywords | `QC_KEYWORDS` `:786` | 56 keywords |
| Binary markers | `QC_BIN_MARKERS` `:778` | 6 markers |

The payload files themselves are never copied into this project; the CLI reads them from the
same directories Python searches, relative to the executable.
