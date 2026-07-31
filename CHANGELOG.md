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

- **Mods, arcanes and rivens are in the collection**, as two new categories. They were never read
  at all, which on one account meant 1,011 rows worth 156,015p showing up as nothing.
- **Each rank is its own row.** warframe.market only quotes a card at rank 0 and at its ceiling —
  Serration is 3p unranked and 48p at rank 10 — so a part-ranked copy shows both ends rather than
  picking one, and every card carries its rank. Rivens are ranked but never maxed; their published
  ceiling is a placeholder, not a rank.
- **Ayatan sculptures and stars, and built Railjack armaments**, are tracked too. Kubrow imprints
  are not: the snapshot has no section for them.
- **What the market would actually take**, as a second figure under the collection's worth. The
  unit prices were right and the total still wasn't reachable — trades happen one at a time between
  two players, and nobody buys your 182 Quickdraws. Each stack now also counts at no more copies
  than the whole game trades in a month. On one account that reads 24,086p under a market rate of
  30,817p.
- **A price floor in Settings**, 0 to 20 platinum, for the part no measurement settles. 14,936p of
  that sellable total is 1–5p items the market really does trade, an evening of haggling at a time.
  Whether those count is your call, not ours. It only affects the sellable figure, never the
  market rate.

### Changed

- **The worth cell is two figures and a clause** instead of five numbers. It had been showing the
  market rate, the trades it would take, how many items were priced, and a copy of the counter on
  the line below — an argument about the collection rather than a valuation of it.
- **Settings and About are separate pages.** Settings has the price floor and the overlay preview;
  About has the licence and the disclosures, including what the overlay does to read your screen.
- **Relics are no longer priced live at every launch.** That pass existed because the daily dump's
  relic prices were unusable, and they aren't any more. Keeping each day's relic prices for thirty
  days instead of discarding them took one account's coverage from 45% to 96% — for no extra
  request, since the dump was already being downloaded. It saves about seventy requests and
  twenty-two seconds off every launch. Live pricing is now only ever the refresh button.

### Fixed

- **Prices come from completed trades, not asking prices.** An asking price covers a bulk seller's
  whole lot, so anything sold in stacks read high — a Lith T11 relic at 30p against the 4.5p it
  traded at, and the same for gems, fish and fragments. Completed trades have the opposite problem
  when they're thin: Vitality closed at 115p on four trades against a 1p ask backed by 3,186
  listings. Taking the lower of the two fixes both directions, and trades with fewer than three
  sales are ignored. 1,442 of 3,059 items moved, and 139 relics gained a price they never had.
- **A stale price cache is re-downloaded rather than trusted.** This is why 0.3.1's subtype fix
  reached the code but not the file already on your disk. Costs 3.9 MB, once.
- **A mod is filed as a mod**, not as the thing it fits. Warframe stores an augment under its
  Warframe and a precept under the pet, so twenty-one mod stacks were showing up as companions.
- **An unranked card is no longer priced from a maxed one's quote.** A few listings are only ever
  quoted at their ceiling, which had a 0/3 Scan Matter valued at the 240p a rank 3 one sells for.
  Those now show no unranked price until you ask for one, which is the honest answer.
- **Development builds link again.** `opt-level = 2` and incremental compilation together produced
  a binary that couldn't be linked. Incremental is off for that profile now; a clean build of the
  workspace is 22 seconds.

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
