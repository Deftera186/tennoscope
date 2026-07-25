# TennoScope Product Pass Design

## Goal

Turn the functional Warframe Helper vertical slice into a credible, distinctive MVP named **TennoScope**. The release must make a collection of roughly 1,000 entries pleasant to browse, expose canonical item artwork and snapshot freshness, and restore the originally approved automatic relic reward overlay to MVP scope.

## Product Identity

TennoScope is a local-first instrument for inspecting a Tenno account and making time-sensitive reward choices. The visual language is a restrained field console: near-black mineral surfaces, warm ivory text, a single void-teal signal color, fine technical rules, and asymmetric clipped corners. It avoids generic dashboard tropes such as oversized metric tiles, glowing gradient blobs, letter avatars, and interchangeable rounded cards.

The user-facing name, window titles, package metadata, executable, desktop entry, documentation, and setup copy become TennoScope. The existing application identifier and data directory remain readable through an explicit migration so current users keep setup consent and snapshots.

## Collection Browser

The collection is a compact visual index rather than one enormous card wall. Each row contains canonical artwork, name, category, owned quantity, and mastery state. The WFCD catalog's `imageName` field becomes normalized image metadata and is exposed as a stable HTTPS URL through the application view. Missing or failed artwork falls back to a deliberately designed category sigil.

Filtering and sorting stay client-side because 1,000 small records are cheap to search, but rendering is paginated at 48 entries per page. Filter or sort changes return to page one. Pagination shows current range, total filtered count, previous/next controls, and a bounded set of page numbers. The main window owns the only vertical scrollbar; nested panels do not create competing scroll regions. Navigation and filter controls remain sticky where useful.

## Freshness

The immutable application view gains explicit snapshot metadata: observed timestamp, source, and game build. The header shows a human-readable freshness label such as `Synced 4 minutes ago`; hover/focus reveals the exact local timestamp and source. The collection heading repeats a quieter exact sync state. The label updates locally with time even when no new backend view arrives. Missing timestamps are described honestly as `No successful sync yet`.

## Reward Observer and Overlay

The overlay is a separate, borderless, transparent window aligned over Warframe's reward choices rather than a miniature desktop page. It contains only the four enriched choices and small status affordances. Cards are horizontally aligned to the game's selectable columns, preserve the central game view, and use translucent backing. The best-value marker, ownership, mastery relevance, ducats, price age, and uncertain recognition state remain visible without stealing focus.

The Linux observer is built behind a `RewardFrameSource` interface. For the first supported path it captures the active Warframe output on X11 directly and on wlroots through `grim`, then crops the resolution-relative reward-name region and passes it to Tesseract. Recognition normalizes OCR text and resolves it against prime-part catalog names with confidence. The observation state machine requires consecutive matching frames before showing the overlay and consecutive misses before hiding it, preventing flicker. Portal/PipeWire capture remains the portable Wayland adapter boundary for GNOME and KDE; unsupported capture reports a precise diagnostic rather than pretending the overlay works.

On wlroots, GTK layer-shell anchors the overlay above the game and makes it keyboard-transparent. On X11, always-on-top plus input pass-through is used. The app records overlay geometry per display mode and exposes a calibration preview from settings. The normal overlay never has a title bar or close button and never takes keyboard or pointer focus.

## Boundaries

- `warframe-acquisition` parses WFCD image metadata without knowing presentation URLs.
- `app-core` publishes item image identity and snapshot metadata in immutable views.
- the Tauri shell resolves safe CDN URLs, owns capture/OCR lifecycle, and owns native window behavior.
- React owns pagination, freshness formatting, image fallbacks, and overlay rendering only.
- recognition and window placement are independently testable with recorded/synthetic frames and geometry fixtures.

## Failure Handling

Artwork failure affects only that item's presentation. Capture/OCR failure never affects inventory synchronization. An uncertain reward is displayed as uncertain and excluded from automatic ranking. Loss of the reward state hides the overlay. If click-through/layer placement cannot be established, automatic overlay display is disabled and diagnostics explain the missing host capability.

## Verification

Automated tests cover catalog image extraction, view serialization, timestamp formatting, pagination boundaries and reset behavior, artwork fallback, OCR normalization/matching, reward-state debounce, overlay card count, and geometry. Release verification runs the complete Rust and frontend suites, lint/type checks, production build, and a live Wayland/wlroots smoke test with Warframe running.

## Acceptance Criteria

1. The application is branded TennoScope everywhere visible and preserves existing local data.
2. Collection entries display canonical artwork or a polished fallback.
3. At most 48 collection entries render at once, with stable pagination and one natural page scrollbar.
4. The latest successful snapshot time is visible and its exact timestamp/source are discoverable.
5. A detected English relic reward screen automatically shows a non-focusable, click-through four-choice overlay positioned over Warframe and hides it after the screen ends.
6. Unsupported capture or OCR states are explicit and do not degrade inventory acquisition.
