# MediaTek profile

Constants used by the `--mtk` branch (and by blind detection, which defaults to MTK). All
values are ports of module constants in `vendor_patcher\hd_vendor_build.py`; the code lives in
`volte-core\src\profile.rs`.

| Constant | Python | Value |
| --- | --- | --- |
| APK candidate chain | `APK_MTK_CANDIDATES` `:81-91` | `OPPO_VoLTE_Overlay.apk` and legacy names |
| RC candidate chain | `RC_MTK_CANDIDATES` `:81-91` | `oppo_mtk_volte.rc` (renamed, was `hd_oppo_mtk_volte.rc`), then legacy names |
| Import lines | `IMPORT_LINE_MTK`, `IMPORT_LINE_MTK_HW` | `import /vendor/etc/init/oppo_mtk_volte.rc` (+ `hw`) |
| Overlay command counts | `overlay_cmds` `:857` | 20 clean, 44 write |
| Extra QC RC write | `overlay_cmds` `:948-961` | MTK also writes `oppo_qc_volte.rc` (kept) |
| Init target candidates | `link_init_target` `:965` | 21 MTK candidates, then 3 shared |
| Chip keywords | `MTK_KEYWORDS` `:798` | 37 keywords |
| Binary markers | `MTK_BIN_MARKERS` `:782` | 5 markers |

The payload files themselves are never copied into this project; the CLI reads them from the
same directories Python searches, relative to the executable.
