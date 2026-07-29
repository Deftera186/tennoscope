# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

While the major version is `0`, the public surface — the application's behaviour, its on-disk
schema, and its configuration — may change in any minor release. `0.x.y` bumps `y` for fixes and
`x` for anything else.

## [Unreleased]

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

[Unreleased]: https://github.com/Deftera186/tennoscope/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Deftera186/tennoscope/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Deftera186/tennoscope/releases/tag/v0.1.0
