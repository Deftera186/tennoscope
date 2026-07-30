---
name: TennoScope
description: A Linux-first Warframe companion rendered as an assay office register — struck marks on touchstone, two metals appraised side by side.
colors:
  touchstone: "#0b0c0e"
  touchstone-plate: "#131417"
  touchstone-raised: "#1a1c20"
  well: "#0e0f12"
  rule: "#2a2d33"
  rule-bright: "#3c4048"
  rule-control: "#5c616b"
  platinum: "#e8ebef"
  platinum-dim: "#9aa1ab"
  platinum-deep: "#868d97"
  gold: "#d8b04a"
  gold-dim: "#a8863a"
  oxblood: "#dd6f60"
typography:
  mark:
    fontFamily: "\"Liberation Sans Narrow\", \"DejaVu Sans Condensed\", \"Archivo Narrow\", \"Roboto Condensed\", ui-sans-serif, system-ui, sans-serif"
    fontSize: "clamp(2.6rem, 4.6vh + 2.8vw, 5.5rem)"
    fontWeight: 700
    lineHeight: 0.84
    letterSpacing: "-0.03em"
  figure:
    fontFamily: "{typography.mark.fontFamily}"
    fontSize: "clamp(2.5rem, 5vw, 4rem)"
    fontWeight: 700
    lineHeight: 0.82
    letterSpacing: "-0.03em"
  body:
    fontFamily: "ui-sans-serif, system-ui, \"DejaVu Sans\", \"Liberation Sans\", sans-serif"
    fontSize: "0.88rem"
    fontWeight: 400
    lineHeight: 1.65
    letterSpacing: "normal"
  register:
    fontFamily: "ui-monospace, \"DejaVu Sans Mono\", \"Liberation Mono\", \"Cascadia Mono\", monospace"
    fontSize: "0.64rem"
    fontWeight: 500
    lineHeight: 1.5
    letterSpacing: "0.14em"
rounded:
  none: "0"
  seat: "2px"
spacing:
  hair: "4px"
  tight: "8px"
  step: "16px"
  course: "32px"
  field: "64px"
  bed: "112px"
components:
  punch-nav:
    backgroundColor: "{colors.touchstone}"
    textColor: "{colors.platinum-deep}"
    rounded: "{rounded.none}"
    padding: "12.5px 18.4px 18.4px"
    typography: "{typography.register}"
  punch-nav-active:
    backgroundColor: "{colors.touchstone-raised}"
    textColor: "{colors.platinum}"
  shield:
    backgroundColor: "transparent"
    textColor: "{colors.platinum-dim}"
    rounded: "{rounded.none}"
    padding: "9.3px 12.8px 15.2px"
    typography: "{typography.register}"
  shield-struck:
    backgroundColor: "{colors.touchstone-raised}"
    textColor: "{colors.platinum}"
  seal-action:
    backgroundColor: "{colors.platinum}"
    textColor: "{colors.touchstone}"
    rounded: "{rounded.none}"
    padding: "20px 16px"
    typography: "{typography.register}"
  seal-action-hover:
    backgroundColor: "#ffffff"
    textColor: "{colors.touchstone}"
---

# Design System: TennoScope

## Overview

TennoScope is an **assay office**. The product's job is appraisal — it tells a player what a
thing is worth, in two metals, and keeps the register of everything they own. So the interface is
the assay office's own apparatus: a touchstone slab you test metal against, marks struck into the
surface rather than printed on it, and a ruled register where every entry is a certified reading.

This is a replacement world. The previous interface was a dark neon dashboard — glass cards, glowing
cyan status dots, rounded corners, icon sidebar. That is the arrangement this category always ships,
and it is the explicit anti-reference. Nothing here glows, nothing floats, nothing is rounded.

The one idea the system owns: **two metals, appraised at equal weight.** Platinum and the gold ducat
are the product's two currencies, and the whole reward mechanic is that the best card by one is
often not the best card by the other. So platinum and gold are the palette's two named roles, not
decoration — a reading in platinum is rendered in platinum, a reading in ducats is rendered in gold.
A competitor cannot copy this, because it comes from the product's mechanic rather than from taste.

Mode is **Operate** throughout. Expression never obscures the task, the state, or a familiar
affordance. The register is dense and exact; the whitespace around it is what makes it readable.

## Colors

Strategy: **full palette, three named roles**, on a single achromatic ground. Colour owns whole
regions — the platinum column and the gold column — never scattered accents.

| Token | Value | Role |
|---|---|---|
| `touchstone` | `#0b0c0e` | The basalt slab. The application ground, everywhere. |
| `touchstone-plate` | `#131417` | A seated region — the register bed, the overlay plate. |
| `touchstone-raised` | `#1a1c20` | A struck mark's floor: the fill of an active punch or shield. |
| `rule` | `#2a2d33` | Engraved hairline. The primary structural device. |
| `rule-bright` | `#3c4048` | A hairline under emphasis, and hover borders. 1.88:1 — never carries text. |
| `rule-control` | `#5c616b` | Form-control boundaries only. 3.06:1, the WCAG 1.4.11 floor for non-text. |
| `well` | `#0e0f12` | A recessed seat: input interiors and the item artwork well. |
| `platinum` | `#e8ebef` | The metal. Primary text, primary figures, the pass state. |
| `platinum-dim` | `#9aa1ab` | Secondary text and register labels. |
| `platinum-deep` | `#868d97` | Tertiary text and small tracked labels. 5.11:1 — AA at any size. |
| `gold` | `#d8b04a` | The ducat first: every ducat figure and label. It also carries the degraded state and the reward count, because "look here, but nothing has failed" is the same signal at a glance. It never carries platinum. |
| `gold-dim` | `#a8863a` | A ducat label beside its figure. |
| `oxblood` | `#dd6f60` | The failed assay. Errors and failed states only. |

Health has **four** states, not three, and each gets its own silhouette so colour is never
load-bearing on its own:

| State | Mark | Ink | Means |
|---|---|---|---|
| `ready` | solid house cartouche | `platinum` | working, verified |
| `idle` | plain rectangle — an unstruck blank | `platinum-deep` | enabled, nothing to do yet, nothing verified |
| `degraded` | notched cartouche | `gold` | working but impaired |
| `failed` | struck cross | `oxblood` | broken |

`idle` exists because reporting "waiting for work" as a fault teaches the reader to ignore the
colour that means a real fault — and because the OCR observer shells out to tools nothing probes
until the first read, so a green state before then would be a guess.

Every ink clears WCAG AA (4.5:1) against all three grounds. Verified, not assumed.

**Dark is not a default here.** The player is at a desk at night with a fullscreen dark game on the
monitor, alt-tabbing in for ten seconds, and the overlay sits directly over that game. A light
certificate would be a flashbang. The touchstone is also, literally, the black slab an assayer rubs
metal against.

## Typography

No webfonts. The Tauri CSP is `font-src 'self'` and this is a Linux desktop target, so the system
stack is both the honest and the correct choice. Extreme contrast comes from **scale and weight**,
not from an exotic face.

- **Mark** — condensed grotesque, weight 700, tight negative tracking, uppercase. This is a struck
  punch: page titles, the largest figures. Ranges to `5.5rem`, and scales on viewport height as
  well as width so it stays the largest thing on a 760px-tall window.
- **Figure** — same face, used for readings inside the register. Always `tabular-nums`.
- **Body** — system sans. Prose, descriptions, disclosure copy.
- **Register** — monospace, `0.14em` tracked, uppercase, `0.62–0.7rem`. This is the ledger's own
  voice: column heads, labels, states, metadata. It is a workhorse here, not micro-decoration.

The system's signature is the **ratio**: a `0.64rem` tracked mono label sitting directly beneath the
page mark, with nothing between them. The mark scales on viewport *height* as well as width, so the
ratio runs about **6.5:1 in the 1180×760 window the app ships in** and about 8.5:1 on a large
monitor. Never soften it with a mid-size step between those two.

## Layout

- One rhythm: `4 / 8 / 16 / 32 / 64 / 112`. Nothing off-scale.
- More space above a heading than below it, always.
- **Massive field.** A page opens with `clamp(course, 5vh, bed)` of clear touchstone above its mark —
  112px on a large monitor, 32px in the shipping window. Density is earned: a dense register passage
  is paid for by an empty one.
- Content is a single full-width column — no sidebar. Navigation is the hallmark row in the masthead.
- The register grid is built from `1px` gaps filled by `rule`, not from borders on each entry. The
  rules belong to the sheet, not to the items.

## Elevation & Depth

**There is no elevation.** Nothing is above the surface. Marks are struck *into* it.

The only depth device is the **strike**: a dark lip above and a catch-light below, which reads as a
punch pressed into metal. On rectangles it is `box-shadow: inset 0 1px 0 <dark>, inset 0 -1px 0 <light>`
(the `--strike` token). On anything wearing the cartouche `clip-path` it must instead be a
`linear-gradient` top lip in `background-image` — an inset shadow is sheared away by the clip's
diagonals and renders as nothing. Lettering gets the same strike as a two-stop `text-shadow`.

Explicitly banned: `box-shadow` with a blur radius, `filter: drop-shadow`, glow of any kind,
`backdrop-filter` outside the overlay window (where it is a legibility requirement over live game
imagery, not an effect), and any gradient used to fake a light source.

The bans govern what this interface *draws*. They do not govern the two bundled game icons, which
are quoted artwork and carry their own gloss and glow — see **Currency mark** under Components.
Nothing else may borrow that exemption.

## Shapes

- **Radius is `0`.** The one exception is `seat: 2px` on an image well.
- The **punch shield** is the system's one non-rectangular form: a `clip-path` pentagon derived from
  the UK platinum standard mark's house-shaped cartouche. It carries badges, category filters, and
  status marks. It is never a rounded pill.
- Status is communicated by **form and colour together**, never colour alone: a filled shield is
  ready, a half-struck shield is degraded, a struck cross is failed.

## Components

- **Masthead + hallmark row** — the office name struck at display scale, with navigation as a row of
  punch cartouches beneath it. Semantically an ordinary tab row: real `<button>`s, `aria-current`,
  full keyboard operation. Radical look, conventional affordance.
- **Assay slip** (reward card) — the centrepiece. Name struck across the top; beneath it the two
  metals side by side at *equal* weight, platinum left, gold right, each a large tabular figure with
  a tracked mono label. The leader in each metal takes a struck hallmark naming which metal it won.
  Never one headline number with the other as a footnote.
  The slip is a fixed four-row grid and the name reserves two lines whether it needs them or not, so
  the figures land at the same y across all four columns. Each metal reserves its hallmark row too,
  so a card that wins nothing stays level with a card that wins both. A player scans this row
  sideways under a countdown; ragged baselines make that scan impossible.
- **Currency mark** — the one deliberate foreign body in this world. Digital Extremes' own platinum
  canister and Orokin ducat sigil, bundled as bitmaps, riding the tracked label under each figure.
  They are rendered 3D objects with a cyan glow, which every other rule here forbids, and that is
  accepted on purpose: recognition on this surface is not a design problem to solve but a memory the
  player already has from the game running behind the overlay. Marks drawn in the house grammar were
  built and rejected — six flattenings of the canister read as a battery, a SIM card, or a media
  control, which is a *third* shape to learn and a wrong one.
  They ride the label, never the figure: beside a figure they competed for a column barely three
  digits wide and a 220p reading pushed the ducat column off the card. They are sized in `rem`, not
  `em`, and held between two walls found by rendering them: below about 18px the canister loses its
  dark chip and reads as a grey lozenge, and much above the figure's own cap height it outweighs the
  name it sits under. So the register's price row runs its mark at `1.15rem` against a `.95rem`
  figure — sized to it, never scaling with it. Where a row carries two figures in one currency it
  takes one leading mark, not one per figure.
  Every figure they mark is also named in text beside it, so they are decorative to a screen reader
  and removable by a distributor who cannot carry game art.
- **Register entry** (collection item) — artwork in a recessed platinum well, name struck, quantity
  and mastery as punch shields. No card border; the rule grid does the separating.
- **Ruled grid** — cells draw their own right and bottom rule with an inset shadow; the container
  draws only top and left. Never a container background showing through `1px` gaps: a partial last
  row then leaves grey blocks where cells would have been.
- **Caution band** — a disclosure is not a feature. Seated `plate` ground, gold hairlines top and
  bottom, and a named `Caution` label where the clause numeral would sit. Never a thick coloured
  side stripe.
- **Sort tally** — the same segmented button group as the ownership filter. A native `<select>`
  opens an OS popup this world cannot reach, so a small closed set of options is a tally instead.
- **Assay procedure** (diagnostics) — the five acquisition stages as a ruled ledger with struck
  ordinals, each carrying its state as a shield.
- **Seal action** — the primary button. Solid platinum, square, tracked mono caps. It is the office's
  stamp coming down.

## Do's and Don'ts

**Do**
- Let platinum mean platinum and gold mean ducats, everywhere, without exception.
- Show a dash when a price is genuinely unknown. Untradeable items have no listing and never will.
- Keep the ratio extreme: struck mark against tracked mono, nothing in between.
- Give the page air before you give it another rule.
- Pair every colour-carried state with a distinct shield form.

**Don't**
- Don't add a glow, a blur shadow, or a rounded corner. The world is struck metal, not glass.
- Don't put a thick coloured border on one side of anything. It is the most recognisable tell there
  is, and the register already has a band for a notice.
- Don't describe a mechanism the code does not have. Reward names come from OCR only — the memory
  reward path exists in `reward_source.rs` but nothing in the live flow calls it.
- Don't crown a reading whose OCR confidence is below 0.8, on either metal.
- Don't put a button, a link, or any focusable element in the overlay window. It is non-focusable and
  click-through by product constraint, so it can never be reached by keyboard or screen reader —
  everything it shows must also be obtainable in the main window.
- Don't let the overlay's whitespace grow at the cost of glance-reading. The main window gets the
  massive field; the overlay gets the figures, because a player reads it in under a second over a
  moving game under a countdown.
- Don't give a clipped cartouche an `outline` for focus — the clip eats it. Invert the mark to
  platinum instead, and keep the `forced-colors` block that restores a real outline.
- Don't use `rule-bright` for text at any size — it is 1.88:1 and exists only for hairlines, hover
  borders and inset outlines. Every other ink in the table clears AA at any size, `platinum-deep`
  included (5.11:1), so small tracked labels may use it freely.
- Don't let the field push the register off a 760px-tall window. The app ships at 1180×760, so
  below `max-height: 840px` the page compresses: marks drop to ~3rem, band padding halves, section
  gaps fall to one step. On a large monitor the field stays wide open. Operate mode — the task
  outranks the air, and this is calibration, not a retreat from the world.
