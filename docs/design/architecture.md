# Warframe Helper Design

## Purpose

Warframe Helper is a GPLv3 alternative to AlecaFrame with local-only persistence. It connects only to public catalog, pricing, reader-definition, and release sources; player data never leaves the device. Linux and Windows are both first-class: on Linux the application must work across distributions, desktop environments, window managers, X11, and Wayland, and on Windows against the native client, in either case without Overwolf. The first release focuses on automatic relic reward advice and automatic inventory/mastery tracking.

The product is one packaged desktop application built from a modular Rust workspace and a Tauri 2 shell. Rust owns all game integration, recognition, persistence, and domain logic. HTML and CSS render a modern collection browser and a transient in-game overlay without carrying Electron's runtime overhead.

## MVP Scope

The first release provides:

- automatic detection of Warframe, running through Wine or Proton on Linux or natively on Windows;
- automatic synchronization of prime parts, relic quantities, and owned/mastered frames, weapons, and companions;
- automatic detection of the relic reward screen;
- a click-through reward advisor showing platinum value, ducats, ownership or set progress, and mastery relevance for every choice;
- a catalog-first desktop collection browser with search, filters, categories, item detail, and synchronization diagnostics;
- local caching of catalog and market data for degraded offline operation;
- English UI recognition, with locale-independent catalog identities in storage; and
- native packaging for Arch, Gentoo, Debian/Ubuntu, and Fedora, an AppImage fallback, and a per-user NSIS installer for Windows.

The MVP excludes macOS support — Warframe has no macOS client — as well as Flatpak, accounts, telemetry, cloud synchronization, trade automation, builds, farming planners, and non-English recognition.

## User Experience

### First Run

Setup performs four explicit actions:

1. Explain that the application reads the Warframe process and that even read-only inspection may create account-policy or anti-cheat risk.
2. Acquire screen-capture permission through PipeWire and `xdg-desktop-portal` on Wayland. X11 uses direct capture.
3. Verify game-process discovery, capture availability, reader-definition trust, and local storage.
4. Record setup completion so later launches require no manual inventory screens or repeated prompts unless a portal revokes capture permission.

Read-only memory acquisition is enabled by default after disclosure. There is no guided inventory scan.

### Normal Launch

The application detects Warframe running through Wine or Proton, fingerprints the game build, chooses a compatible reader definition, discovers the relevant structures, validates them, and atomically reconciles a collection snapshot. The desktop collection becomes usable without user interaction.

### Relic Reward Overlay

The screen observer detects the relic selection state and recognizes the four visible rewards. The Warframe library enriches each reward with cached or current market value, ducats, quantity owned, set completion, and mastery relevance.

The approved overlay is a decision advisor: it preserves four card-shaped choices while visibly highlighting best trade value and missing mastery. It remains click-through and disappears when the reward state ends. A low-confidence reward is marked uncertain and excluded from best-choice ranking rather than guessed.

### Collection Application

The approved desktop layout is a collection browser. Primary navigation exposes Overview, Collection, Relics, and Activity. Category navigation covers frames, weapons, companions, prime parts, and relics. Search and filters make individual items easy to locate; item details show quantity, mastery, set completion, value, and relevant relic relationships. Recommendations may appear in item detail or overview but do not dominate navigation.

A health panel reports capture, game reader, catalog, market, and database status separately so degraded behavior is understandable.

## Architecture

### Packaging Shape

Version 1 runs as one application process. The Rust workspace is modular internally, while Tauri owns the application lifecycle and windows. A separate daemon is deliberately deferred until sandboxing, privilege separation, or multi-client requirements justify its IPC complexity.

### Deep Modules and Interfaces

#### Game Acquisition

Game Acquisition hides Wine/Proton process topology, log formats, memory layouts, byte-pattern discovery, and reader definitions behind one interface. Its observable outputs are versioned inventory/mastery observations and backend health. Callers never see process handles, offsets, signatures, or raw memory.

Production adapters cover `EE.log` and passive local artifacts, process memory, and process lifecycle. These are real internal seams because replay and synthetic adapters exercise the same acquisition behavior in tests.

Memory access has one adapter per platform behind the same `MemoryReader`/`ProcessDiscovery` traits: procfs with `process_vm_readv` on Linux, `VirtualQueryEx` with `ReadProcessMemory` on Windows. They differ in one observable way. Linux resets and reads `/proc/<pid>/clear_refs` soft-dirty bits, so a poll rescans only the pages the game wrote; Windows has no equivalent that works on another process, so its adapter falls back to the trait defaults and rescans everything. Neither crate contains `unsafe` — every platform call is made through a crate that encapsulates its own.

#### Screen Observer

Screen Observer hides PipeWire portal sessions, compositor differences, X11 and Windows Graphics Capture, frame selection, and reward recognition. Capture and the preprocessing pipeline are in-process on both platforms; the only external tool is Tesseract, which Windows bundles and Linux takes from the distribution. Its interface emits a reward-screen observation containing recognized catalog identities and confidence. It neither prices rewards nor renders the overlay.

Wayland requires a functioning PipeWire plus `xdg-desktop-portal` backend. The tested matrix includes representative GNOME, KDE, wlroots, and Hyprland sessions. Minimal window managers must install and configure a compatible portal backend.

Windows captures through Windows Graphics Capture rather than GDI, because `BitBlt` against a D3D swapchain returns a black frame. Placement is the one capability that does not reach parity: an override-redirect X11 window sits above a fullscreen Wine client, but nothing short of a swapchain hook draws over an exclusive-fullscreen Windows game. Borderless is therefore a documented requirement, surfaced in the health panel rather than only in the README.

#### Observation Pipeline

The Observation Pipeline validates, deduplicates, and orders acquisition and screen observations. It is the only route by which observed game state reaches the Warframe library, which owns reconciliation. Each result records source, game build, reader-definition version, timestamp, and validation outcome.

#### Warframe Library

The Warframe Library owns collection state, mastery state, relic relationships, reward enrichment, and recommendation rules. It accepts validated observations and produces immutable collection and reward views. It does not know how memory is read, screens are captured, prices are transported, SQLite is queried, or windows are rendered.

#### Local Store

The Local Store owns SQLite schema, transactions, migrations, current state, authoritative snapshot history, reconciliation audits, and cached external data. Its interface exposes domain-shaped reads and atomic commits rather than SQL or table-shaped records.

#### External Data

External Data adapters fetch the public Warframe item catalog and market pricing. They normalize remote representations into catalog identities and timestamped price observations. The Warframe Library receives normalized data through an injected interface; tests use deterministic in-memory adapters.

#### warframe.market Account

TennoScope has no accounts. Linking a warframe.market account is a separate, opt-in connection to
a third party: it is off by default, every other feature works without it, and it is the one place
in the application where player data leaves the device. Inventory, mastery, and reward history stay
local and are never uploaded.

The account module hides the credential lifecycle, the order endpoints, and credential storage
behind interfaces that take a transport. Its observable outputs are an order list and a link
state. Callers never see the token, which is held in a keyring where one is available and in the
local database where none is, with the choice reported in the health panel. A keyring is treated
as available only if it answers a read: constructing an entry succeeds against a locked or
still-starting secret service, and a launch that trusted that would report the credential as held
somewhere it could not be read from. Because which store is chosen can differ between launches, a
miss on the keyring reads the database before concluding the account is unlinked, and anything
found there is promoted to the keyring and cleared from the database.

Authentication has one route available to third parties. `/v2/auth/signin` requires a Firebase App
Check header only first-party clients can produce, and OAuth 2.0 registration is closed, so the
v1 signin route is used -- which warframe.market's own documentation directs integrations to. That
route is undocumented in the sense that matters: it can be withdrawn without notice. Linking with
a token pasted from a signed-in browser session is therefore offered as an equal path rather than
as a fallback.

Reconciliation is the Warframe Library's, not this module's: it joins an order list to an
inventory snapshot, and the account module has no concept of a collection. A mismatch is claimed
only when the snapshot is coherent and newer than the order; every other case is reported as
unverifiable and carries no claim.

Writes -- publishing a listing, taking one down, lowering an oversold quantity -- are authorized
against the held view before any transport is built, and address items by the collection's own
`/Lotus/` path rather than by a market identifier the presentation layer supplied. Publishing is
offered only for items whose listing needs a price and a quantity and nothing else. `POST /order`
requires contextual fields exactly when the item supports the dimension -- a rank, star counts, a
per-trade size -- and refuses the request either way, so an item wanting any of them is not
offered rather than refused after the fact. That is a narrower question than whether an owned
count can be read off the collection: a ranked mod reconciles exactly and still cannot be listed.

Presence -- what warframe.market shows the account as -- is a separate crate, because it is a held
WebSocket with a reconnect lifecycle rather than request and response over a transport. It reports
only what the server has confirmed; what was last asked for is held by the presentation layer
beside it, so a press registers before the socket answers. `offline` is not a settable value on
that protocol and is implemented as closing the connection.

#### Presentation

The Tauri presentation layer owns the collection window, reward overlay, setup, and diagnostics. It consumes immutable application views and sends user intents. It never reads game memory or SQLite directly and contains no inventory reconciliation or recommendation rules.

## Data Flow

### Inventory Synchronization

1. Detect the game process and determine its Wine or Proton topology.
2. Fingerprint the current game build.
3. Load a compatible signed reader definition.
4. Discover structures using stable byte patterns rather than fixed runtime addresses.
5. Capture collection metadata before and after reading inventory/mastery structures.
6. Validate generation and count consistency, bounds, unique item identities, and known catalog relationships.
7. Emit one coherent snapshot or emit a read failure; no partial snapshot exists at the external interface.
8. Transactionally replace current collection state and append an audit record.
9. Publish a new immutable collection view to the UI.

A coherent snapshot is authoritative. Quantity reductions, absent items, and zero quantities represent legitimate sales, consumption, or deletion and must be applied. A failed invariant produces no snapshot and leaves the last coherent state unchanged.

### Reward Advice

1. Screen Observer detects the reward-selection state.
2. Recognition maps each visible English item name to a stable catalog identity and confidence.
3. The Observation Pipeline rejects duplicates or malformed reward sets and marks uncertain identities.
4. The Warframe Library joins rewards with inventory, mastery, ducats, and timestamped prices.
5. Deterministic rules rank trade value and mastery relevance while excluding uncertain entries from recommendations.
6. Presentation shows the four-card decision advisor and removes it when the reward state ends.

## Warframe Update Resilience

Memory discovery uses structural patterns and validation rather than fixed addresses. Reader definitions are small, human-auditable, versioned files delivered independently from application releases. Every definition is signed with the project's embedded trust root; invalid, downgraded, or incompatible definitions are rejected.

Routine game changes should require only a reader-definition update. A major internal rewrite may invalidate all known discovery patterns or require application code. In that case the app explicitly reports that inventory reading is unsupported for the current build and retains the last coherent inventory. Log-derived functionality and reward-screen recognition continue when their inputs remain compatible. Maintaining the reader is an acknowledged ongoing project responsibility; uninterrupted support through arbitrary internal rewrites is not promised.

## Failure Handling

- **Unknown build or broken memory discovery:** disable memory synchronization, keep the last coherent inventory, continue compatible passive and screen features, and expose the exact backend failure in diagnostics.
- **Snapshot validation failure:** commit nothing and record a local diagnostic outcome.
- **Portal permission revoked:** request capture permission again without blocking inventory synchronization.
- **Uncertain reward recognition:** label that reward uncertain and exclude it from best-choice ranking.
- **Catalog unavailable:** use the last valid catalog and show its age.
- **Market unavailable:** use timestamped cached prices and show their age; never present them as current.
- **Database transaction or migration failure:** preserve the existing database, do not publish an uncommitted view, and surface recovery guidance.

## Security and Privacy

- Game interaction is read-only. The application never injects code, patches memory, automates input, or writes to the game process.
- Read-only access is enabled by default only after first-run risk disclosure.
- Raw memory and captured frames are processed transiently and are not retained by default.
- Reader-definition updates require a valid project signature and monotonic compatible version.
- Distribution packages are updated through their package managers. AppImage users receive release notifications; application binaries are not silently replaced.
- In-app updating is limited to signed reader definitions and public catalog/price data.
- The product has no accounts of its own, and no telemetry, analytics, cloud storage, or
  background upload. A warframe.market account may be linked by explicit opt-in; doing so sends
  that account's credential and, from phase 2, order contents the player chooses to publish.
  Nothing else leaves the device.
- All persistent application data is local and deletable by the user.

## Packaging and Platform Support

The supported release artifacts are:

- Arch package and AUR metadata;
- Gentoo ebuild/overlay metadata;
- Debian/Ubuntu `.deb`;
- Fedora `.rpm`; and
- AppImage as a cross-distribution fallback.

Package definitions declare WebKitGTK, PipeWire, portal, and other system dependencies appropriate to each distribution. AppImage cannot replace the host's compositor portal integration. Flatpak is excluded because its sandbox conflicts with default process-memory inspection; it may be designed later with a host helper or reduced capabilities.

The initial verification matrix covers the supported distribution families, X11, GNOME Wayland, KDE Wayland, a wlroots compositor, and Hyprland. Packaging more distributions is welcome after the core matrix is stable.

## Testing Strategy

Interfaces are the test surfaces. Tests assert observable snapshots, reward views, persisted state, and health—not internal offsets or helper call sequences.

- Game Acquisition tests replay synthetic and redacted memory layouts, process topologies, and logs through test adapters.
- Snapshot contract tests cover additions, legitimate deletions, zero quantities, changing generations, corrupt counts, invalid bounds, duplicate identities, and unknown catalog relationships.
- Screen Observer tests replay representative reward frames across supported resolutions, scaling factors, display modes, and the initial English locale.
- Warframe Library tests use table-driven inventory, mastery, price, ducat, uncertainty, and recommendation cases.
- Local Store tests cover atomic replacement, audit history, rollback, schema migration, and recovery from interrupted writes.
- External Data tests use deterministic adapters for current, stale, malformed, and unavailable sources.
- End-to-end tests feed a fake game session through acquisition, storage, reward advice, and presentation state.
- Manual release verification exercises portal setup, permission restoration, click-through overlay placement, multi-monitor behavior, process detection, and packaging on the supported Linux matrix.

Real player memory or screenshots are not committed to the repository unless explicitly sanitized and licensed for that purpose.

## MVP Acceptance Criteria

The MVP is successful when a supported Linux installation can:

1. complete disclosure and capture setup once;
2. automatically detect Wine- or Proton-hosted Warframe on later launches;
3. produce a coherent automatic collection/mastery snapshot without opening specific in-game screens;
4. accurately apply inventory additions and legitimate deletions;
5. detect an English relic reward screen and display four enriched advisor cards without intercepting game input;
6. retain useful collection and price data while external sources are offline;
7. degrade explicitly without corrupting collection state when a reader or portal fails; and
8. install and run from the supported Arch, Gentoo, Debian/Ubuntu, Fedora, and AppImage artifacts across the stated X11/Wayland matrix.

## License

The application and repository are licensed under GPLv3. Third-party assets and dependencies must be compatible with GPLv3 distribution, and their notices are included in release artifacts.
