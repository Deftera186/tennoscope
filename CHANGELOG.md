# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

While the major version is `0`, the public surface — the application's behaviour, its on-disk
schema, and its configuration — may change in any minor release. `0.x.y` bumps `y` for fixes and
`x` for anything else.

## [Unreleased]

## [0.6.0] - 2026-08-19

### Added

- **A Support section on the Settings page.** The report actions Diagnostics shows only when it
  detects a failure — *Open an issue*, *Copy diagnostics*, *Save logs* — are now always available
  in Settings, so a problem that automatic detection misses can still be reported.

### Changed

- **Save logs now includes the game's own log.** The report folder carries a copy of EE.log
  whenever the game is running (or was — the last known location is remembered), not just after a
  failed acquisition stage. The copy is scrubbed of IP and email addresses so it is safe to attach
  to a public issue.

### Fixed

- **A fresh launch no longer looks broken.** The game reader, log monitor and catalog reported
  Degraded before Warframe had even started, which lit up the failure banner on an idle app. They
  now say Idle until something actually goes wrong.
- **Relaunching the game on Linux reconnects to the new process.** After a restart the app used to
  attach to whichever game process it found first, which could be the stale one; it now picks the
  newest.

## [0.5.7] - 2026-08-17

### Fixed

- **Acquisition health no longer misreports idle states.** "Not logged in yet" is now Degraded
  rather than Failed, so the Diagnostics screen stops showing the problem banner on a perfectly
  healthy idle app. Detecting the Warframe launcher on Linux now shows "waiting for the game to
  launch" instead of a bare "not running", and a brief game-not-found blip during startup no longer
  sticks around after the game connects.

## [0.5.6] - 2026-08-12

### Fixed

- **The log file stops flooding when the game is closed.** Every poll of the game process used to
  write "game process gone" and "EE.log not found" warnings to the new log file several times a
  second for as long as the app sat idle. Monitor health now records transitions; the steady
  state is silent.
- **Open an issue actually opens the issue page.** The opener plugin allowed the command but no
  URL, so every open was rejected with "Not allowed to open url". The issue form and the saved
  report folder are now the only two places the app may open.
- **The report block describes faults, not the booth.** It used to appear whenever any system was
  merely waiting at boot; it now appears when something failed, or degraded after having worked.
- **The saved report stops hoarding.** Report folders are pruned to the newest five, and the
  ebuild-scale EE.log is copied only when an acquisition stage actually failed, not on any
  degraded stage.
- **Windows reports do not leak the user name.** The sanitizer read only the Unix home
  variables; on Windows the real user name could reach a pasted report.

### Changed

- **The report reads as a report.** "Copy report" is now "Copy diagnostics", the pasted text is
  rows rather than a JSON dump, and it carries the deduplicated WARN/ERROR tail of the session
  log instead of a raw excerpt. EE.log content still never enters report text.

## [0.5.5] - 2026-08-12

### Added

- **A report you can hand over.** Diagnostics now has *Open an issue*, *Copy report* and
  *Save logs*. The report carries the version, the health of every system and the tail of the log,
  with your home directory and username scrubbed out. Nothing is sent anywhere — it leaves the
  machine only when you paste or attach it.
- **A stable build now keeps a log.** The reward-reading diagnostics used to exist only in a
  debug build, which is not the build anyone plays. They are written to the platform log
  directory, capped at four files of 5 MiB.

### Fixed

- **Reward cards are read on a scaled display.** A capture that came back larger than the window
  it was asked for was cropped rather than scaled, leaving a magnified top-left corner in which
  no card is where the reader looks — every read failed, and from outside that is
  indistinguishable from OCR simply not working.

## [0.5.4] - 2026-08-10

### Fixed

- **Reward cards are read where the game draws them on non-16:9 screens.** Card positions
  were fractions of the window's width; Warframe scales its HUD with height. On a 16:10 screen —
  a Steam Deck's 1280x800 — the reader looked a fifth of a card away from the real slots, clipped
  titles read as confident wrong letters, both outer cards fell under the match floor, and every
  poll failed for the whole life of the screen. The overlay strip was drawn in the wrong place on
  the same screens; both are right now.

## [0.5.3] - 2026-08-08

### Fixed

- **One unreadable item no longer fails your whole collection.** The inventory response can
  contain a row the game's own client refuses — it logs `Inventory has NULL item` and carries
  on — and TennoScope turned that single row into "Inventory snapshot was invalid" for the
  entire account. Unreadable rows are now skipped the way the game skips them; a response with
  no readable holdings at all is still refused.
- **A credential the first pass misses is now searched for.** Warframe's memory is sampled
  within a budget, and on a smaller machine the credential can sit outside what the budget
  reached — the same session read fine once and reported "inventory authorization was not
  found" on the retry. Finding nothing now widens the search to the rest of the process rather
  than reporting an answer it had not earned.

### Changed

- Debug builds record what a read had to throw away — how many rows were skipped and which
  item path was first, how many bytes of memory were sampled and how many candidates were seen
  — so a failed read can be explained. Counts and item paths only; no account data.

## [0.5.2] - 2026-08-07

### Fixed

- **Presence switches no longer hang on "Asking warframe.market…".** The first status change
  on a connection went through and every later one waited on the ask, even though the site had
  applied it. The site confirms a change twice — in its reply to the change itself, and, once
  per connection, as the status it last recorded — and TennoScope was reading only the second,
  announcing at the start of a connection. It now reads the reply too, and a change that gets
  no reply at all asks again, reconnecting after a quiet connection instead of waiting forever.

## [0.5.1] - 2026-08-05

### Fixed

- **Your collection reads on an account that does not own everything yet.** If you had no
  Necramech, no Amp, or nothing at all in any one category, the read failed outright — "reader
  failed", and a collection of zeroes — even though everything else about it had worked. Warframe
  leaves a category out of its reply when you own nothing in it, and TennoScope was treating that
  as a broken reply rather than an empty shelf.

## [0.5.0] - 2026-08-04

### Added

- **Windows support.** TennoScope runs on Windows 10 and 11 against the native client, with the
  same collection browser, reward overlay and local-only storage. The installer is a per-user NSIS
  package — no admin prompt, no prerequisites, its own Tesseract — but it is unsigned, so
  SmartScreen warns on first run. Support is best-effort: it is run before a release, but Linux is
  the first-class platform and nothing is guaranteed.
- **Warframe's display mode is checked.** Exclusive fullscreen owns the display on Windows, so the
  diagnostics panel asks for Borderless when it cannot find the game window instead of reporting a
  generic capture failure.
- **Optional warframe.market account link.** Off by default. Sign in or paste a token and see your
  orders beside the collection — total listed value, fetch age, and any order that no longer
  matches what you own, fixable in one action. The credential is kept in your OS keyring (the
  local database where there is no keyring); unlinking removes it. Nothing else about the account
  is uploaded.
- **Publish a sell listing**, from a collection card or the orders screen. Price and quantity are
  yours to set; quantity cannot exceed what this device says you hold. Items needing more than a
  price and a quantity — relics, sets, anything with a rank — are not offered.
- **Take down any listing**, not only ones flagged as wrong. A listing nothing is wrong with asks
  once before it goes.
- **Set what warframe.market shows you as** — online, in game, invisible or offline — while
  TennoScope runs. "Follow the game" reports in game while Warframe is running and online
  otherwise.
- **Where to find your token**, on the screen that asks for one, and what it is worth to anyone
  holding it.

### Changed

- **The reward reader no longer shells out to anything but Tesseract.** Capture, cropping, contrast
  and thresholding are in-process Rust now — `xwininfo` and ImageMagick drop off the Linux
  recommended dependencies, and every read is one process spawn instead of four.

### Fixed

- **A mistyped password no longer locks acquisition out for the session.** Failed and successful
  logins both leave credentials in memory, so two existed and the reader refused to choose —
  "multiple inventory authorizations were found", cleared only by a restart. It now takes the
  newest, which is the live session. Two *different* accounts still refuse, because that case has
  no right answer.
- **Pre-release builds no longer open a console window beside the app.** They keep debug
  assertions on so tracing survives; the console was suppressed by the absence of a flag rather
  than the build profile it should have followed.
- **A slow first start no longer reports the backend as unavailable.** The window opens before the
  backend finishes with its database, and the first status call could arrive in that gap; it now
  waits.

## [0.4.1] - 2026-07-31

### Fixed

- **The AppImage opens to a window you can see.** It carried its build machine's copy of a Wayland
  library, and the graphics driver on a newer distribution refuses to load against it, so the
  browser view gave up before drawing anything. That library now comes from your system, which is
  where every other package was already getting it. The `.deb`, `.rpm` and Gentoo packages were
  never affected.

## [0.4.0] - 2026-07-31

### Added

- **Mods, arcanes and rivens are in the collection**, as two new categories. They were never read
  at all, so for most players this is the largest thing in the collection finally showing up.
- **Each rank is its own row.** warframe.market only quotes a card at rank 0 and at its ceiling —
  Serration is a few platinum unranked and a good deal more maxed — so a part-ranked copy shows
  both ends rather than picking one. Rivens are ranked but never maxed; their published ceiling is
  a placeholder, not a rank.
- **Ayatan sculptures and stars, and built Railjack armaments**, are tracked too. Kubrow imprints
  are not: the snapshot has no section for them.
- **What the market would actually take**, beside the collection's worth. Trades happen one at a
  time between two players, and nobody is buying your two hundredth spare mod, so a stack now
  counts at no more copies than the game trades in a month.
- **A price floor in Settings**, 0 to 20 platinum. A lot of any collection is 1–5p items that do
  trade, an evening of haggling at a time; whether they count is your call. It only moves the
  sellable figure, never the market rate.

### Changed

- **The worth cell is two figures and a clause** instead of five numbers.
- **Settings and About are separate pages.** Settings has the price floor and the overlay preview;
  About has the licence and the disclosures, including what the overlay does to read your screen.
- **Relics are no longer priced live at every launch.** Keeping each day's relic prices for a month
  instead of discarding them covers nearly all of them for no extra request, and takes twenty-odd
  seconds off startup. Live pricing is the refresh button now.

### Fixed

- **Prices come from completed trades, not asking prices.** An asking price covers a bulk seller's
  whole lot, so anything sold in stacks read high — a Lith relic at 30p against the 4.5p it
  actually traded at, and the same for gems, fish and fragments. Thin trade data has the opposite
  problem, so the lower of the two wins and anything with fewer than three sales is ignored. Most
  of the collection moved, and relics that never had a price now have one.
- **A stale price cache is re-downloaded rather than trusted.** This is why 0.3.1's relic fix
  reached the code but not the file already on your disk. Costs 3.9 MB, once.
- **A mod is filed as a mod**, not as the thing it fits. Warframe stores an augment under its
  Warframe and a precept under the pet, so mods were turning up under companions.
- **An unranked card is no longer priced from a maxed one's quote.** Some listings are only ever
  quoted at their ceiling, which had unranked copies valued at what a maxed one sells for. Those
  show no price until you ask for one, which is the honest answer.
- **Development builds link again.** `opt-level = 2` and incremental compilation together produced
  a binary that couldn't be linked.

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

[Unreleased]: https://github.com/Deftera186/tennoscope/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/Deftera186/tennoscope/compare/v0.5.7...v0.6.0
[0.5.7]: https://github.com/Deftera186/tennoscope/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/Deftera186/tennoscope/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/Deftera186/tennoscope/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/Deftera186/tennoscope/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/Deftera186/tennoscope/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/Deftera186/tennoscope/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/Deftera186/tennoscope/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Deftera186/tennoscope/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/Deftera186/tennoscope/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/Deftera186/tennoscope/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/Deftera186/tennoscope/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Deftera186/tennoscope/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Deftera186/tennoscope/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Deftera186/tennoscope/releases/tag/v0.1.0
