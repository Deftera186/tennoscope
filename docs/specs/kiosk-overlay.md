# Ducat Kiosk Overlay — Design Spec

## Goal

Show platinum values over Warframe's in-game Ducat Kiosk screen (InventoryTest.swf):
1. A small chip at each grid tile's top-right corner with the item's platinum price.
2. A platinum icon + value beside each basket row's ducat number.
3. A platinum icon + total value left of the TOTAL row's ducat number.

Visual style follows the approved kiosk screenshot (`../screenshots/ducat-kiosk.png`):
overlay digits match the game's own numeral size (16px total row / 15px basket rows at
1920×1080), compressed width ratio (~12.3px/digit advance), baselines aligned to the game's,
ice-blue fill `rgb(222,238,252)` with a subtle 1px shadow; grid chips are dark rounded chips
`rgba(8,16,26,.84)` outlined `rgba(120,180,220,.35)` with the platinum mark, CondensedBold 17px.

## Architecture

The kiosk uses the same capture and closed-set OCR foundations as the reward overlay, but its
session lifecycle comes from `EE.log` rather than image readability:

```
EE.log tail ──> KioskLogMachine
                  ├── KioskOpened / GridPopulated ──> show + re-anchor
                  └── KioskClosed ──> hide, clear state, stop poller
kiosk poller ──> capture full grid strip ──> frame-to-frame scroll delta
             ├── moving ──> emit_to("kiosk-overlay", "kiosk-scroll")
             └── settled ──> locate label rows ──> OCR grid/basket
                         ──> closed-set match + data join ──> KioskState
                         ──> emit_to("kiosk-overlay", "kiosk-updated")
frontend /kiosk route ──> KioskOverlay.tsx renders fraction-positioned chips
```

### Detection (`EE.log`)

Open markers observed from `InventoryTest.swf` are:

- `InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts`
- `InventoryTest.lua: DBG: HudVis 1`
- `Created /Lotus/Interface/InventoryTest.swf`
- `Subscribing for /Lotus/Interface/InventoryTest.swf`
- `PopulateGrid()`

`HudVis 0` closes the session. An input subscription for another interface is an independent
close witness; whichever arrives first closes once, and a later kiosk subscription opens a new
session. OCR and capture failures never decide presence: they retain the last published view and
the next poll retries. This prevents transient unreadable frames from tearing down the overlay.

### Recognition

- Grid cells: crop each tile's label band, preprocess it with the reward OCR pipeline, run
  Tesseract in sparse-text mode, and closed-set match against prime-part names from
  `CatalogIndex::reward_entries`. Slots below the match floor render nothing.
- Basket rows: recognize item names inside the basket pane with the same closed-set matcher.
- Stack quantities: independently OCR the optional bounded `N X` prefix with a digit/separator
  whitelist. Missing or malformed prefixes mean quantity one; the observed `K`/`k` Tesseract
  confusion is accepted as the separator.
- Total row: never OCR'd. Total platinum is the checked sum of each matched basket item's unit
  price multiplied by its recognized quantity.

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

Calibration constants @1920×1080 were measured from the committed kiosk fixtures and are
asserted by the calibration tests:

| Element | Value |
|---|---|
| Grid columns | 6, 207.5px pitch, first left edge x=76, tile width 190px |
| Grid rows | 3, card tops y=199/421/643 (222px pitch) |
| Label OCR crop | card top +122px, 68px tall; locator band starts at y=343 and is 46px tall |
| Basket rows | first digit baseline y=243, 38⅓px pitch, overlay pair right edge x=1750 |
| Total row | digit baseline y=875, overlay pair right edge x=1717 |
| Grid pane | clip edge y=983; tracked strip x=70..1310, y=193..983 |

### Rendering

- Second always-on-top transparent click-through window `kiosk-overlay`, url `/kiosk`
  (clone of the reward-overlay declarations in tauri.conf.json:26-41 +
  capabilities/default.json).
- One window spans the whole game window rect (unlike reward-overlay's card-sized window):
  chips are absolutely positioned DOM nodes at fraction coordinates over the full screen.

### Scroll sync

The poller separates motion tracking from the expensive OCR pass:

1. Capture the whole grid pane and compare its row-luma profile with the previous frame using
   bounded normalized cross-correlation.
2. A confident displacement greater than one pixel emits its numeric delta. The frontend
   accumulates those deltas, translating grid chips with the game while the basket stays fixed.
3. An unreadable displacement emits a fade verdict instead of inventing motion.
4. After two still looks, fold the current profile over the 222px row pitch to locate the topmost
   readable label band at any scroll position.
5. OCR at that absolute offset and publish a fresh epoch. The fresh `scroll_dy` replaces any
   accumulated frontend transform and removes the fade.

Reader failures do not close the session or erase the last good state. Only the log-owned close
flag stops the poller; a close arriving during OCR discards that in-flight result.

### Speed notes (measured, live machine)

- One tesseract spawn ≈ 165ms; crops fan out across up to 12 threads with each child pinned to
  one OpenMP thread (`OMP_THREAD_LIMIT=1`) — without that pin, 12 concurrent spawns oversubscribe
  and the grid pass goes from 385ms to 6s.
- A uniform-background prefilter (`band_has_text`) skips the spawn for slots whose label band
  has no glyphs: empty basket rows and partial grids cost ~nothing.
- Screen capture (~750ms, compositor-bound; xcap and grim agree) dominates every budget and is
  the next lever, but it is platform work, not app work.

## Edge cases

- **Partial last grid row / filtered inventory**: render only cells whose match clears the
  floor; unmatched slots render nothing.
- **Hover card covering tiles**: covered cells fail independently and do not take other chips
  down. If no label band can be located, keep the last view and retry.
- **Empty basket**: publish the grid without basket chips or a total chip.
- **Stacked basket row**: multiply its unit platinum price by the recognized quantity in both
  the row and total; malformed quantity text safely falls back to one.
- **Scroll**: stream measurable deltas, fade on an unreadable frame, then replace accumulated
  motion with the settled frame's absolute offset.
- **Window moved/resized/alt-tab**: capture failures keep the last good state; the log closes the
  session, and a newly matched window rect reconfigures overlay geometry.
- **Non-16:9 / scaled displays**: height fractions and centred horizontal offsets preserve the
  game's own HUD scaling convention.
- **Kiosk opened while a reward overlay is active**: independent windows and state machines can
  coexist, although the game does not normally present both screens together.

## Non-goals

- Reading the game's ducat or platinum numerals.
- Memory-based reading of SWF pools (rejected: recycled slots and stale strings).
- Platform-specific capture work beyond the existing reward-capture backends.

## Testing

- Rust unit and integration tests cover log-owned open/close transitions, exact and scaled
  geometry, closed-set OCR, quantity parsing, checked quantity pricing, scroll delta streaming,
  settled absolute offsets, capture selection, and external-close cancellation.
- OCR regression tests use committed fixture PNGs and shell out to Tesseract; CI installs its
  English language data.
- Frontend tests cover epoch replacement, numeric scroll accumulation, fades, and basket totals.
- Manual verification uses the live game under umu/Proton on Sway.
