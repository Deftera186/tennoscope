# Research scripts

One-off instruments from the investigation that produced [`docs/research/`](../docs/research). They
are **not** part of the application, not covered by CI, and not supported — they are kept because
they are the reproducible evidence behind the claims in those documents, and because the memory
path they exercise is still live code in `warframe-acquisition`.

Two files are exceptions, being real build tooling rather than research instruments:

- `build-linux-bundles.sh` — the bundle helper, documented in [`packaging/`](../packaging).
- `tauri.mjs` — what `pnpm tauri` runs. It forwards to the Tauri CLI, setting `NO_STRIP` on Linux
  because linuxdeploy's bundled `strip` predates RELR relocations and fails on distributions whose
  toolchain emits `.relr.dyn`. An explicit `NO_STRIP` from the caller always wins.

## Requirements

Python 3.11+, a running Warframe session under Wine/Proton, and the same `/proc` access the app
needs. These are Linux-only research instruments, not shipped code: the screen ones still shell
out to `xwininfo`, `import`, `magick` and `tesseract`, which the app itself no longer does.

No paths are hardcoded: [`_paths.py`](_paths.py) finds the Wine prefix from the live process's own
mappings, the same way the app does. Override with `TENNOSCOPE_EE_LOG` and `TENNOSCOPE_CATALOG` to
run against an archived log or catalog with no game running.

## What each one does

| Script | Purpose |
| --- | --- |
| `watch_reward_visual.py` | Fire captures when the reward screen becomes *visible*, not when the log says so. |
| `watch_reward_capture.py` | Same, triggered from the `Got rewards` log line. |
| `capture_reward_screen.py` | Grab frames of the reward screen while it is up. |
| `capture_reward_players.py` | Bounded player-id to reward-pointer associations. |
| `capture_reward_order.py` | Bounded reward string/object evidence, in screen order. |
| `capture_reward_layout.py` | Dump the object layout around a reward site. |
| `scan_response_records.py` | Sweep every readable mapping for reward artefacts. One run answers where remote players' rewards live. |
| `find_reward_array.py` | Look for an ordered reward array among a sweep's pointer sites. |
| `analyze_reward_ui_graph.py` | Walk the persistent reward UI graph from a capture. |
| `ocr_reward_cards.py` | Read card titles off a screenshot against the run's relic pool. The prototype for `reward_ocr.rs`. |

Nothing here writes to the application's database, and none of them modify game memory.
