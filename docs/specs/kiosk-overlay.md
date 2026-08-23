# Ducat Kiosk Overlay — Design Spec

## Goal

Show platinum values over Warframe's in-game Ducat Kiosk screen (InventoryTest.swf):
1. A small chip at each grid tile's top-right corner with the item's platinum price.
2. A platinum icon + value beside each basket row's ducat number.
3. A platinum icon + total value left of the TOTAL row's ducat number.

Visual style is locked to the approved v8 mockup (`~/.cache/tmp/opencode/mock_recommended.png`):
overlay digits match the game's own numeral size (16px total row / 15px basket rows at
1920×1080), compressed width ratio (~12.3px/digit advance), baselines aligned to the game's,
ice-blue fill `rgb(222,238,252)` with a subtle 1px shadow; grid chips are dark rounded chips
`rgba(8,16,26,.84)` outlined `rgba(120,180,220,.35)` with the platinum mark, CondensedBold 17px.

## Architecture

Mirrors the existing reward-overlay pipeline:

```
EE.log tail ──> MonitorMachine (existing, lib.rs:1355)
                   └──> KioskLogMachine (new, observe_bytes -> events)
                           KioskOpened ──> spawn kiosk poller thread + show overlay
                           GridPopulated ──> force re-anchor
                           (no close marker) ──> poller miss-streak closes
kiosk poller ──> capture_game_window() (reuse) ──> OCR grid/basket labels
             ──> closed-set match (reuse best_match) ──> data join ──> shared KioskState
             ──> emit_to("kiosk-overlay","kiosk-updated")
frontend /kiosk route ──> KioskOverlay.tsx renders fraction-positioned chips
scroll tracker ──> phase correlation dy ──> CSS transform; settle -> re-anchor
```

### Detection (EE.log)

Open markers (from 2026-08-23 investigation):
- `InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts`
- `Created /Lotus/Interface/InventoryTest.swf`
- `PopulateGrid()` (fires on every repopulation: open, filter change, basket edit)

There is **no close marker**. Close = poller OCR miss-streak (same pattern as
`spawn_reward_screen_poller_with`, `POLLER_GONE_STREAK = 2`).

### Recognition

OCR text only — **no digit OCR**. Ducat totals come from static data joined on recognized
names, so the game's numbers never need reading:
- Grid cells: crop each tile's label band (below thumbnail), preprocess exactly like
  `reward_ocr::prepare_crop`, tesseract psm 11, closed-set `best_match` against prime-part
  names (`CatalogIndex::reward_entries`). Score floor 0.6 (reuse `MATCH_FLOOR`).
- Basket rows: same pipeline on right-pane row crops.
- Total row: never read. Total plat = sum over basket rows.

### Data joins

- Ducats: `RewardCatalogEntry.ducats` via `reward_name_matches` (catalog.rs:98).
- Platinum: `PriceTable::price_for(name)` (collection_prices.rs:421), daily dump source.
  Fallback: `MarketPriceCache::get`.
- Owned count: `CollectionView::items()` name join, quantity (lib.rs:2313 pattern).
  Owned count renders inside the existing ✓N badge area — we do NOT draw our own owned badge;
  the game draws one. (v8 has no owned element.)

### Geometry

All positions are fractions of **window height**, horizontal offsets as signed fractions of
height from the window's horizontal centre — the same convention as `reward_ocr.rs:33-39`,
because Warframe scales its HUD with height and centres horizontally.

Calibration constants @1920×1080 (measured from `/tmp/kiosk-after.png`; the calibration task
re-measures from the committed fixture and asserts these):

| Element | Value |
|---|---|
| Grid columns | 6, column pitch 206px, first column left x≈68, tile width ≈198 |
| Grid rows | 3, thumbnail tops ≈ y197/423/640 (pitch ≈220) |
| Label band | below thumbnail, ≈43px tall, ≈2 lines of ~17px |
| Basket rows | baseline y ≈ 224 + 38.7·i + 19, ducat right edge x≈1790–1792 |
| Total row | digits h16 baseline y877, right edge x1792, gold glyph x1729–1749 h19 |
| Pane safe right edge | ≈1855 |

### Rendering

- Second always-on-top transparent click-through window `kiosk-overlay`, url `/kiosk`
  (clone of the reward-overlay declarations in tauri.conf.json:26-41 +
  capabilities/default.json).
- One window spans the whole game window rect (unlike reward-overlay's card-sized window):
  scroll-following requires moving DOM elements inside the window, not the window itself.
- Chips are absolutely positioned DOM nodes at fraction coordinates; scroll offset applies as
  a single CSS `translateY` on the chip container (GPU-composited, no IPC per frame beyond
  the offset event).

### Scroll sync

1. While scrolling, the poller captures a thin horizontal strip of the grid region each tick
   (~30ms) and computes vertical displacement vs the previous strip by phase correlation
   (FFT cross-correlation on luma, sub-pixel not required).
2. Offset streams to the frontend (`kiosk-scroll`) and chips translate in lockstep.
3. When |dy| stays under ε for ~120ms, one full OCR pass re-anchors and clears drift.
4. Confidence guard: if correlation peak ratio drops below threshold, or fewer than half the
   previously anchored cells re-match after settle, chips fade out rather than mislabel.

## Edge cases

- **Partial last grid row / filtered inventory**: render only cells whose match clears the
  floor; unmatched slots render nothing.
- **Hover card covering tiles**: covered cells fail their read → dropped individually;
  if the majority fails, treat as occluded frame → fade, keep previous anchors until a clean
  re-anchor.
- **Empty basket**: no basket chips, total chip hidden.
- **Basket overflow ("≥ X" display)**: game may show a capped total; we still sum actual
  basket rows and label the total chip with the plain sum.
- **Scroll**: handled above; partial rows at strip edges are fine for correlation.
- **Window moved/resized/alt-tab**: capture failure keeps last state for ≤1s then hides;
  window rect changes re-run `configure` geometry.
- **Non-16:9 / scaled displays**: inherited free from the height-fraction convention and the
  existing resampling capture path.
- **Kiosk opened while a reward overlay is active**: independent windows/machines; both may
  coexist (they never occur simultaneously in practice).

## Non-goals

- Reading the game's ducat numerals via OCR.
- Memory-based reading of SWF pools (rejected: recycled slots, stale strings).
- Windows-only concerns beyond what xcap already abstracts.

## Testing

- Rust unit/integration tests headless (`cargo test --workspace`, CI parity):
  log machine, geometry (fixture-exact at 1920×1080 + scaled variants), matcher joins,
  phase correlation on synthetic shifted images, poller miss-streak logic.
- OCR recognition tests run against the committed fixture PNGs and shell out to tesseract
  (CI installs it).
- Manual verification against the live game (running under umu/Proton).
