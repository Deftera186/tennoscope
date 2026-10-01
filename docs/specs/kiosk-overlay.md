# Ducat Kiosk Overlay: Design Spec

## Goal

Show platinum values over Warframe's in-game Ducat Kiosk screen (InventoryTest.swf):
1. A small chip at each grid tile's top-right corner with the item's platinum price.
2. A platinum icon + value beside each basket row's ducat number.
3. A platinum icon + total value left of the TOTAL row's ducat number.
4. In place of a tile's chip, one strip for a part of an item the player has not
   mastered, built or started building: that the item is unmastered and what the part
   sells for, plus how many of the part is held against how many are needed, when more
   than one is needed and the strip has room for it.

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

- Ducats: not published. The game draws a ducat value on every tile and basket row itself.
- Platinum: `PriceTable::price_for(name)` (collection_prices.rs:421), daily dump source.
  Fallback: `MarketPriceCache::get`.
- Owned count: not published. `CellChip` and `BasketChip` carry a name and a platinum figure
  only, so the ✓N badge on screen is the game's own.
- Mastery: `CellChip.mastery` (`held`, `uses`) from the mastery ledger
  (`crates/warframe-acquisition/src/mastery.rs`), joined through the Prime recipe index
  (`CatalogIndex::part_parent`, catalog.rs) that says which recipe slot a part fills.
  Mastery is permanent but held, built and pending parts are not, so a saved collection
  can prove an item is mastered and never that it still needs parts: strips need this
  run's inventory. `KioskView.mastery_status` is `live` with live facts, `unavailable`
  with a saved collection or before this run's first sync, and `off` when the stored
  preference turns marks off. Only `live` draws strips.
- Strip recognition mask: `strip_mask_width_1080(held)` (`kiosk_geometry.rs`), the tile
  width less the badge clearance for the held count, so the mask covers the strip's full
  width and never the badge beside it.

### Geometry

All positions are fractions of **window height**, horizontal offsets as signed fractions of
height from the window's horizontal centre, the same convention as the `BLOCK_CENTRE` fractions
in `reward_ocr.rs`, because Warframe scales its HUD with height and centres horizontally.

Calibration constants @1920×1080 were measured from the committed kiosk fixtures and are
asserted by the calibration tests:

| Element | Value |
|---|---|
| Grid columns | 6, 207.5px pitch, first left edge x=76, tile width 190px |
| Grid rows | 4, card tops y=199/421/643/865 (222px pitch) |
| Label OCR crop | card top +122px, 68px tall; locator band starts at y=343 and is 46px tall |
| Basket rows | first digit baseline y=243, 38⅓px pitch, overlay pair right edge x=1750 |
| Total row | digit baseline y=875, overlay pair right edge x=1700 |
| Grid pane | clip edge y=983; tracked strip x=70..1310, y=193..983 |
| Price chip | 100px wide, anchored at the tile's top-right corner |
| Mastery strip | same anchor, `190 - 51`px wide, or `190 - 63`px when held >= 10 |

### Rendering

- Second always-on-top transparent click-through window `kiosk-overlay`, url `/kiosk`
  (clone of the `reward-overlay` window in `tauri.conf.json` plus
  `capabilities/default.json`).
- One window spans the whole game window rect (unlike reward-overlay's card-sized window):
  chips are absolutely positioned DOM nodes at fraction coordinates over the full screen.

### Mastery strips

- A strip reuses the chip's plate, anchor and font, so a tile reads in the corner it
  always did, and spends the room to the left instead. The game draws its owned badge
  (a check and a count) in the tile's top-left corner and a strip anchored at the
  top-right shares the tile with it, so the strip's max width is the tile width less
  how far the badge's glyphs reach: 51 design px at one digit, 63 at two, the second
  digit being about 12px wider (`BADGE_CLEARANCE_1080`, `BADGE_CLEARANCE_TWO_DIGITS_1080`).
- Three fit tiers, chosen by measurement rather than by guess: which fonts are
  installed decides how wide the strip's own type came out, so the strip measures its
  box and drops detail while `scrollWidth` exceeds `clientWidth`. Tier 1 is the word, the
  held/uses fraction (only when more than one copy is needed) and the platinum mark with
  the price; tier 2 drops the mark; tier 3 drops the fraction too. The word and the price
  are the reason the strip exists and never drop.
- In DejaVu Sans Condensed, the stack's fallback when Liberation Sans Narrow is missing, a
  part needed once keeps the platinum mark up to a two-digit price, and a part needed more
  than once keeps its fraction up to a two-digit price while fewer than ten are held.
  Anything longer comes down to the word and the price, which is why the strip measures.
- A strip stands in for the price chip rather than joining it, because two readings in
  one corner of a 190px tile is one too many.

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
  one OpenMP thread (`OMP_THREAD_LIMIT=1`), because without that pin 12 concurrent spawns oversubscribe
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
- **Marks off, or nothing behind them**: no strip is drawn either way, and only the
  `unavailable` case says so, with one line over the grid's header reading "Mastery: no
  live inventory", so marks that are off and marks with no inventory behind them do not
  read the same.
- **A held count of 100 or more**: the strip keeps the two-digit clearance, so its left
  edge can meet the badge's third digit. A third clearance would leave less room than the
  word and a three-digit price need.

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
- Frontend tests cover epoch replacement, numeric scroll accumulation, fades, basket totals,
  and the mastery strip's tier fitting.
- Manual verification uses the live game under umu/Proton on Sway.
