# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Warframe players on Linux who run the game through Wine or Proton, on their own
desktop, with the game in focus. Two situations, treated as co-equal:

- **Mid-mission, seconds to decide.** A Void Fissure relic has cracked and the
  reward screen is up. The player has a hard time limit to pick one of four
  rewards and wants to know which is worth the most platinum and which is worth
  the most ducats — often different cards.
- **Out of mission, planning.** The same player wants to know what they already
  own, what is missing, and what still counts toward mastery, without trusting a
  hand-maintained spreadsheet or a third-party account.

The player is the only user. There is no team, no admin, no second role.

## Product Purpose

TennoScope is a local-first desktop companion for Warframe. It discovers the
running game process, reads its memory without modifying it, obtains an
ephemeral inventory authorization value, validates a complete inventory
snapshot, enriches it with WFCD catalog data and artwork, and stores the
collection in SQLite on the local machine.

Success is that a Linux player gets the reward call and the collection index
that Windows players already have, without an Overwolf runtime, without an
account, and without their collection leaving the machine.

## Positioning

The incumbent is **AlecaFrame** — the most widely used Warframe companion, with a
relic reward overlay covering the same job. As observed 2026-07-27, AlecaFrame
is distributed through the Overwolf app store, targets Windows/PC, is not open
source, and gates part of its analytics behind a Patreon tier.

Two differences a neighboring tool could not truthfully copy:

- **Linux-native, no Overwolf.** TennoScope is a real Linux desktop application
  that treats Wine/Proton as a supported environment, not a compatibility
  accident. Overwolf-based companions cannot follow the player onto this
  platform. This is the reason the product exists.
- **Local-only, no account.** No login, no telemetry, no cloud sync, no remote
  account, no premium tier. The durable player data is a normalized local
  snapshot in the application data directory. Network access is limited to the
  pinned Warframe inventory origin, the pinned WFCD catalog source, and
  warframe.market price lookups.

Not claimed as positioning: reading inventory from live process memory, and
stating the risk and untested-compositor limits openly. Both are true and both
are load-bearing for trust, but they are not the sales argument. Note that
memory reading applies to the **inventory snapshot only** — reward recognition
is OCR (see Capabilities).

## Operating Context

- The game is fullscreen and in focus. The reward overlay appears **over a
  running game**, competing with Warframe's own HUD for attention, read at a
  glance under a countdown. It is not a window the player looks at deliberately.
- The desktop application is the opposite scene: a normal 1180×760 window
  (min 360×520), read at leisure, undecorated.
- Warframe runs under Proton as an XWayland client. Reward capture goes through
  X11 (`xwininfo`, `import`), `magick` (ImageMagick 7), and `tesseract` with
  English data. Overlay *placement* reads the game window rectangle via
  `swaymsg`; on other compositors it falls back to centring on the primary
  monitor. Layering uses GTK layer-shell and is skipped outside Wayland.
- Refresh is normally automatic: on Warframe start, and when the active prefix's
  `EE.log` reports a completed inventory sync. Manual refresh stays available.
- Process inspection requires the same Unix user and a permissive
  `kernel.yama.ptrace_scope`.

## Capabilities and Constraints

Confirmed and shipped:

- Four surfaces in the desktop window — Collection, Rewards, Diagnostics,
  Settings — plus a one-time risk-disclosure setup screen and a separate
  non-focusable overlay window (`/overlay`, 1100×190, transparent, always on
  top, skips the taskbar).
- Collection: canonical artwork, search, category and ownership filters, sort,
  48-item pagination, mastery state, and exact snapshot metadata rendered as a
  human-readable freshness indicator.
- Rewards: reward names are read **by OCR only**. A memory reward path exists in
  `app/src-tauri/src/reward_source.rs` but nothing in the live flow calls it —
  only `coordinator.visual_choices` runs. Names are matched against **the squad's
  own relic pool**, not the whole catalog, with consecutive-frame debounce. Live warframe.market platinum
  prices quoted **from in-game sellers only**, alongside ducat values. The best
  card by platinum and the best by ducats are reported separately because they
  frequently disagree.
- Diagnostics: six services across four health states (ready / idle / degraded / failed) and a
  five-stage acquisition pipeline. Idle is distinct from degraded on purpose — a subsystem that is
  enabled and simply has nothing to do yet must not be reported as a fault.
- Strict, bounded parsing — an incomplete snapshot is rejected rather than
  partially replacing the collection.
- Catalog is cached and integrity-checked, with offline fallback to the last
  complete generation.

Durable constraints future work must not design away:

- **The overlay is non-focusable and click-through.** It must never take input
  from the game. Hard requirement; no hover-only affordance, no focus trap, no
  interactive control can be the only path to information there.
- **English-only, for now.** UI copy and OCR are English. Recorded as *undecided*
  rather than permanent — do not design as if i18n has shipped, and do not
  design it shut either.

Factual limits, not commitments:

- Platinum prices are best-effort. The relic pool is priced during the mission so
  cards are usually warm by the reward screen; a miss shows a dash until it
  lands. Untradeable items — Forma among them — have no listing and stay
  unpriced permanently. The UI must never fake a number in place of a dash.
- Reward card geometry is calibrated against 16:9 captures and scales by window
  width; ultrawide behavior is untested.
- Capture is exercised only on sway. GNOME, KDE, Hyprland and bare X11 are
  untested, not unsupported.
- The memory technique depends on undocumented game behavior and may need
  maintenance after a Warframe update.
- Cross-platform acquisition (Windows, macOS) is **not** ruled out. It is simply
  not built. The user declined to make Linux-only durable.

## Brand Commitments

- Name: **TennoScope**. Window title *TennoScope — Local-first Warframe
  Companion*. Bundle identifier `org.warframehelper.app`.
- Licensed GNU GPLv3-only. Free and open source. No paid tier exists or is
  planned as a product fact.
- Unofficial. Not affiliated with or endorsed by Digital Extremes, and must
  never present itself as official.
- Voice as established in shipped copy and the README: plain, specific, and
  candid about uncertainty. It names the risk on the setup screen and names the
  untested compositors in the README rather than burying either. Copy states
  what is true and what is unknown; it does not reassure.

## Evidence on Hand

- Working application: `app/src/` (React 19, TypeScript, Vite), `app/src-tauri/`
  (Tauri 2), `crates/` (Rust workspace: `app-core`, `local-store`,
  `warframe-acquisition`, `warframe-domain`).
- Visual system: `DESIGN.md` at the repo root, implemented in `app/src/App.css`,
  `app/src/index.css`, `app/src/RewardCards.tsx`, `app/src/RewardOverlay.tsx`.
- Real item artwork from the WFCD catalog (`https://raw.githubusercontent.com`,
  allowed in CSP `img-src`).
- Research notes: `docs/research/` — five documents on reward-source options,
  live acquisition, and reward resolution.
- Packaging guidance: `packaging/` (AppImage, Arch, Gentoo).
- Attribution: `THIRD_PARTY_NOTICES.md`.

Absent — must not be fabricated: user counts, testimonials, download numbers,
benchmarks, press coverage, published package repositories (no AUR package,
Gentoo overlay, or Debian/Fedora repository exists), and any release version
beyond `0.1.0`.

## Product Principles

1. **Two jobs, one snapshot.** Reward decisions and collection tracking are
   co-equal. Neither surface may be treated as an accessory to the other, and
   both read from the same authoritative local snapshot.
2. **The overlay serves a player who is not looking at it.** Glanceable in under
   a second, over a bright moving game, under a countdown, without ever taking a
   click. This constraint outranks every expressive impulse.
3. **Say what is unknown.** A dash, a degraded state, or an untested caveat is
   shown plainly. Never a plausible number in place of a missing one, never a
   green state that is a guess.
4. **Local is the product, not a setting.** No account, no telemetry, no cloud.
   Anything that would require a server is out of scope by definition.
5. **Refuse partial truth.** An incomplete snapshot is rejected rather than
   merged. The collection is either authoritative or explicitly stale.

## Accessibility & Inclusion

No product-specific standard has been established with the user. What the
shipped code already commits to, and future work must preserve: labelled
navigation and controls, `aria-current` on the active page, `role="alert"` on
error banners, `aria-live` on loading state, decorative glyphs marked
`aria-hidden`, and text alternatives for item artwork with a fallback when the
image fails.

The overlay's non-focusable, click-through nature means it can never be reached
by keyboard or screen reader. Everything it shows must therefore also be
obtainable in the main window, which is fully keyboard-navigable.
