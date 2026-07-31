# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

While the major version is `0`, the public surface — the application's behaviour, its on-disk
schema, and its configuration — may change in any minor release. `0.x.y` bumps `y` for fixes and
`x` for anything else.

## [Unreleased]

## [0.4.0] - 2026-07-31

### Added

- Mods, arcanes and rivens are part of the collection, under two new categories. The decoder read
  sixteen sections of the account snapshot and never opened the two the cards live in, so the
  largest tradable holding most accounts have was invisible to the collection, its filters and its
  total — on the account this was measured against, 1,011 rows worth 156,015p that the app reported
  as nothing.
- Each rank a card is held at is its own row, priced as its own holding. warframe.market quotes a
  rankable listing at two ranks and no others — rank 0 and the ceiling — and the gap between them is
  the whole valuation: Arcane Reaper sells at 15p unranked and 400p at rank 5, Serration at 3p and
  48p. A copy part-way up shows the two as a range rather than picking one, and each card carries
  its rank so two rows of one name can be told apart. A riven has no ceiling to reach, so it is
  ranked but never maxed; the catalogue's `515` for one is a sentinel, not a rank anybody can reach.
- Ayatan sculptures and stars, and built Railjack armaments, are tracked too. Kubrow imprints were
  in scope and are not here: the section is absent from the snapshot, so there was nothing to read.
- The collection's worth carries what the market would actually take, as a second small figure under
  the market-rate total. The rate is right and the total was never achievable: warframe.market trades
  are arranged one at a time between two players, and nobody buys 182 Quickdraws. Each stack is now
  also valued at its unit price times the smaller of what is owned and what the whole game completes
  in a month, counted from the same daily dump the prices come from — no threshold to tune, and it
  correctly separates a 240p mod that nobody has bought in a month from a 3p one the game trades five
  times a day. On the account this was measured against it reads 24,086p under a market rate of
  30,817p, with 89 of 1,215 priced stacks capped.
- A price floor in Settings, for the part of that no measurement can answer. 14,936p of the sellable
  total is items at 1–5p that the market genuinely does trade in bulk, and the reason nobody realises
  them is that each is an evening's hand-arranged trade rather than that the price is wrong. Whether
  that is worth counting is the player's call, so it is a slider from 0 to 20 platinum rather than a
  constant somebody invented: stacks cheaper than it per copy leave the sellable figure, and never
  the market-rate total. It reads out what it is doing as it moves — stacks counted and platinum left
  — and it is kept in the window that draws it, not in the database.

### Changed

- The collection's platinum cell is two figures and one clause instead of five numbers. It had been
  carrying the trades the figure would take, the market rate, how many items were priced, and a copy
  of the live pass's own counter, which read as an argument about the collection rather than a
  valuation of it. The pass counter was already on the register line below; the priced-item count
  mostly measured how much of a collection is untradeable; and the price floor now answers the
  question the trade count was there to raise. Both figures carry the platinum icon, since the
  sellable line sits where the three cells beside it hold item counts. Five-figure totals are
  grouped, so they are read rather than counted.
- Settings and About are two pages rather than one. A preference changes what the application does
  and is there to be operated; a disclosure states what it already does and is there to be read, and
  the two had been stacked under a heading that said "Settings & about". Settings now holds the price
  floor and the reward overlay preview — a control that moves a window, not a statement about one.
  About holds the licence line and the four notices, including what the overlay does to read the
  screen. The first-run disclosure points at About, which is where it can be read again.

- Owned relics are no longer swept live at every launch. The sweep existed because the daily dump's
  relic prices were unusable, and they are not any more: the same file carries completed trades, and
  each day's file prices whichever relics traded that day. Those prices now carry forward for up to
  thirty days instead of dying with the file that produced them, which took one account's relic
  coverage from 45% of its relics to 96% — at no extra request, since the dump was already being
  downloaded daily and the previous day's relic prices simply thrown away. The pass it replaces
  spent around seventy rate-limited requests and twenty-two seconds of every launch, and again after
  every inventory refresh, to refine a holding worth 391p. Live pricing still exists and is now only
  ever what the player asks for: the page refresh button.

### Fixed

- The collection is priced from the lower of what an item sold for and what sellers are asking,
  rather than from the asking price alone. warframe.market's `statistics_live` quotes a bulk
  listing's whole lot, so anything sold in stacks read at several times its worth: a Lith T11 relic
  at 30p against the 4.5p it traded at, Star Crimzian at 5p against 1p, and the same for Proof
  Fragments, gems, fish and imprints. `statistics_closed` — completed trades — carries no such fault
  and is in the same daily dump. Neither number survives being trusted alone, though: a thinly
  traded item reads high the other way, and Vitality closed at 115p on four trades against a 1p ask
  backed by 3,186 listings, which priced 113 unranked copies at 35p and put 3,955p on one account's
  total. Taking the lower of the two corrects both directions without a tuned constant. 1,442 of
  3,059 non-relic items moved, and 139 relics gained a dump price they never had. A closed price
  standing on fewer than three trades is still ignored as one player's odd deal — that is the guard
  against a thin trade reading *low*, where the ask cannot help — and it is only ever read against
  its own rank and subtype, so an unranked mod cannot inherit a maxed copy's trade.
- A price cache written by an older reading of the daily dump is discarded rather than trusted. The
  cache is kept for as long as its dump is current, so 0.3.1's subtype fix reached the code and not
  the file already on disk: every rank of every mod went on showing the maxed median it had been
  stored under. The cache now records which parse wrote it, and one that does not match is
  re-downloaded — 3.9 MB, once.
- A mod is categorised as a mod rather than as the thing it fits. Warframe files an augment under
  its Warframe and a precept under the pet, so read by path, twenty-one of one account's mod stacks
  came out as companions and every augment as a Warframe it had never owned.
- An unranked card is no longer priced from a quote that only a maxed one earned. Seven listings in
  the daily dump are quoted at one rank and no other, and the lowest quote in the file was taken as
  the unranked price for want of anything else — so a Scan Matter sitting at 0/3 was valued at the
  240p a rank 3 copy sells for, and six more between 80p and 300p the same way. Such a listing now
  has no unranked price at all, which is the honest answer: the maxed quote still stands for the
  copies that earned it, and the name still resolves, so the page refresh can go and ask.
- Development builds link again. `opt-level = 2` on the dev profile and incremental compilation
  together produce a binary that cannot be linked: incremental reuses a codegen unit while its
  neighbours are regenerated, and under optimisation the private symbols they share are named after
  the unit that emitted them, so the reused object references an `anon.<hash>.llvm.<id>` nobody emits
  any more. Incremental is now off for that profile, which costs little at `opt-level = 2` — a clean
  build of the whole workspace is 22 seconds.

## [0.3.1] - 2026-07-30

### Fixed

- Relics are priced for the refinement tier the player holds. warframe.market quotes the four tiers
  as four subtypes of one listing, and all four resolved to the bare listing name, so a Radiant was
  priced at whatever an Intact was going for — a median 1.46x understatement across the 80 relics
  measured, and 1p against 17p on Requiem I-IV. Refined tiers are thinly traded, so a tier nobody is
  selling still falls back to the Intact listing, which is what every tier fell back to before.
- An item the daily dump quotes more than once is priced at the lowest of them rather than whichever
  record the file happened to list first. Thirty-nine of the sixty are fish, whose subtype is a size
  the inventory does not record — a Tromyzon is a Tromyzon whether it is the 2p basic or the 10p
  magnificent — so an unknown was being valued at its best case.
- Archon shards are listed under their own names and drawn as the shard. The catalogue publishes the
  twelve with the game's inline icon tag, `<Shard_red_simple> Crimson Archon Shard`, which only
  Warframe's text renderer draws, and publishes the six Tauforged as the glow layer alone — a
  coloured smudge with no crystal in it. Neither needs a re-download; the cached catalogue is parsed
  again at launch.

## [0.3.0] - 2026-07-30

### Added

- Platinum and ducat figures carry the game's own icon, on the reward slips and throughout the
  collection. The two currencies were told apart by hue and a tracked 8px word, over a bright
  moving game, under a countdown.
- Live pricing reports the pass that is running — sweep or page refresh alike, since both spend one
  budget — as a count beside the provenance line and a rule that fills as it advances.

### Changed

- Closing the main window quits the application. The reward overlay is a hidden window that is
  never destroyed, so the process used to survive the only window a person can close, and went on
  tailing the log and drawing the overlay with nothing left to close it by.
- A second launch raises the window already open instead of starting a rival process. Two instances
  place an override-redirect overlay at the same coordinates over the game and write the same
  database. A development build shares the bundle identifier, so `tauri dev` now stands down for an
  installed copy rather than running beside it.

### Fixed

- Warframe parts are priced in ducats and counted as owned. The reward screen names a part by the
  blueprint the player picks up, "Voruna Prime Chassis Blueprint", where the item catalogue names
  the component it builds — 153 of the 596 names a relic can drop read as 0 ducats and as not
  owned. Weapon parts, whose two spellings agree, were always right. Platinum was never affected.
- A relic nobody is selling no longer costs a request on every inventory sync. The absence of an
  order book is now recorded as an answer and carried across a refresh, while an outage still
  retries.

## [0.2.0] - 2026-07-29

### Added

- Collection items show a platinum price and stack total, seeded from the daily warframe.market
  price dump — one request a day for the whole collection rather than one per item.
- Live pricing on request for the current page, marked apart from the daily figures with an inline
  "checked live" line rather than a badge. A live price now updates the stored prices, so it
  outlives its cache entry instead of expiring back to the daily figure.
- Owned relics are priced by a bounded live sweep at startup, and again after an inventory refresh
  picks up new ones.
- Collection sorting by unit value, a tradeable filter, and a collection worth summary that carries
  the count it was computed from.
- Collection pricing reports its own diagnostics row, separate from the reward overlay's.

### Changed

- Prices are quoted per unit rather than per trade. A warframe.market listing's platinum is the
  price of a whole trade, and relics are routinely listed six at a time, so comparing a six-pack's
  total against a single item's price ranked two different quantities as one.
- Relics are priced live rather than from the daily dump. The dump is pre-aggregated with no
  per-trade count to divide out, which overstated relic medians by up to half.
- Only items the player actually owns are priced. Mastered-but-unowned equipment no longer carries
  a platinum figure, appears under the tradeable filter, or inflates the collection worth.
- The per-item price-check button is gone. Pricing is a single page-level control naming how many
  items it will price, with real progress while it runs.
- The reward overlay's price lookup uses warframe.market's top-orders endpoint, which returns the
  same answer in a fraction of the bytes, and paces every caller behind one shared 3-requests-per-
  second floor.

### Fixed

- An oversize price response is reported as its own outcome instead of being indistinguishable from
  an item nobody is selling, an unreachable endpoint, and an untradeable item.
- A cache write failure is no longer reported as an unreadable price dump.

## [0.1.0] - 2026-07-28

First release.

### Added

- Native Linux discovery of a Warframe process running under Wine or Proton, including the
  truncated `Warframe.x64.ex` process name.
- Read-only inventory acquisition through `/proc/<pid>/maps` and `/proc/<pid>/mem`, with strict
  bounded parsing that rejects an incomplete snapshot rather than partially replacing a collection.
- Local SQLite snapshots with authoritative replacement semantics, and persisted snapshot metadata
  behind a human-readable freshness indicator.
- A cached, integrity-checked WFCD item catalog with offline fallback to the last complete
  generation, and canonical item artwork.
- A paginated visual collection index with search, category and ownership filters, mastery state,
  and pipeline diagnostics.
- Automatic refresh when Warframe starts and when `EE.log` reports a completed inventory sync, plus
  a manual refresh, both under a cooldown.
- Relic reward recognition by reading the reward screen through X11 capture and Tesseract, matched
  against the squad's own relic pool rather than the whole catalog, with consecutive-frame debounce.
  Squads of two, three or four are all read: the game centres the card block on however many cards
  it drew, so the layout is identified from the pixels rather than assumed.
- Live warframe.market platinum prices for recognised rewards, quoted from in-game sellers only,
  alongside ducat values, with the best card by each measure called separately.
- A non-focusable, click-through reward strip aligned below the in-game reward row, hidden
  automatically when recognition ends. It is an override-redirect X11 window placed against the
  game's own window, so it behaves the same under every window manager and compositor.
- A one-time first-run disclosure of the read-only process inspection and its account-policy
  uncertainty. Nothing runs until it is accepted.
- AppImage, Debian and RPM bundles, attached to the release. Gentoo packages in the `deftera`
  overlay, as `games-util/tennoscope-bin` and `games-util/tennoscope`; an Arch `PKGBUILD` in the
  repository. All of them install a `tennoscope` command and a desktop entry.

### Security

- Account identifiers and nonces are held in memory only, redacted from `Debug` and `Display`, and
  never written to the database or any log.
- Raw inventory responses are validated in memory and are not persisted.
- No telemetry, no analytics, no remote account, no secret persistence.

[Unreleased]: https://github.com/Deftera186/tennoscope/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/Deftera186/tennoscope/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/Deftera186/tennoscope/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Deftera186/tennoscope/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Deftera186/tennoscope/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Deftera186/tennoscope/releases/tag/v0.1.0
