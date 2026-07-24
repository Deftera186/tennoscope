# Live Warframe acquisition spike

Date: 2026-07-24

## Outcome

The proposed Linux/Proton acquisition path works against a live, logged-in Warframe process without Overwolf and without writing to the game process.

The spike verified this chain:

1. discover the Wine/Proton Warframe process, including Wine's truncated process name;
2. enumerate its readable mappings through `/proc`;
3. read process memory as the same desktop user;
4. find an ephemeral account/nonce authorization tuple without printing it;
5. call Warframe's inventory endpoint; and
6. receive and parse a large, structurally complete account snapshot containing equipment, resources, recipes, and mastery data.

No authorization value, account identifier, raw response, or player-specific inventory value was saved in this repository.

## Environment observed

- Warframe was running through Proton/Wine.
- The executable mapping and active `EE.log` were discovered from the running process rather than from a hard-coded Steam layout.
- `/proc/<pid>/maps` and `/proc/<pid>/mem` were readable under the current same-user configuration.
- Reading the image base returned the same PE header as the mapped game executable, confirming that the reader was inspecting the intended process.

These permissions are environment-dependent. Yama settings, UID boundaries, containers, Flatpak, and launcher isolation may prevent the same operation and must produce actionable diagnostics rather than silent failure.

## Log findings

The active `EE.log` records inventory synchronization lifecycle messages and the inventory response body size, but it does not contain the response body itself. It is therefore useful for:

- detecting that the game is ready;
- triggering a refresh after an inventory synchronization; and
- diagnosing prefix/log discovery.

It is not sufficient as the inventory data source.

## Memory findings

A read-only scan found both:

- JSON-shaped inventory objects in anonymous readable mappings; and
- account/nonce URL fragments suitable for the inventory request.

An existing open implementation was then exercised against the live process as an independent behavioral check. Process discovery, authorization scanning, and inventory fetching all succeeded. The returned JSON was roughly one megabyte and contained the expected broad account sections. Only structural metadata and aggregate test diagnostics were observed; secrets were redacted and were not persisted.

The native Rust `/proc` adapter was later validated against the same class of live Proton session: executable-confirmed discovery, maps parsing, and bounded memory samples succeeded while transient unreadable mappings were skipped. A naive full-address-space authorization scan exceeded ninety seconds and was stopped without exposing data. Refresh orchestration must therefore prioritize likely mappings or add another evidence-based early-exit strategy before running scans automatically; correctness does not make an unbounded startup delay acceptable.

## Architectural decision

Use two separate acquisition modules:

### Primary inventory adapter

- Discover Warframe by process identity and executable mappings.
- Read only readable process regions.
- Scan for conservative, validated authorization patterns across chunk boundaries.
- Hold authorization values in memory only and zero/drop them promptly.
- Fetch the authoritative inventory snapshot directly into the parser.
- Require valid JSON plus structural invariants before replacing the stored snapshot.
- Treat a validated snapshot as authoritative, including legitimate item removals.

This is preferred over reconstructing the full in-memory JSON because it is smaller, easier to validate, and already proved end-to-end on Linux/Proton.

### Direct-memory fallback

Adapt the GPLv3 FrameForge full-account-blob algorithm to a Linux memory reader. Accept a candidate only after complete JSON parsing and strict structural validation. This fallback is not required for the first vertical slice, but preserves a route if the endpoint or authorization representation changes.

Reward recognition remains independent: `EE.log` supplies timing signals and the desktop portal supplies screenshots for OCR.

## Safety and privacy constraints

- No process writes, code injection, debugger stop, input automation, or packet interception.
- Never log or persist account IDs, nonces, platform IDs, login responses, or raw inventory snapshots by default.
- Expose the feature's account-policy risk during initial setup; the user has chosen read-only acquisition enabled by default after that disclosure.
- Fail closed: an incomplete, malformed, or structurally suspicious response must not replace the last coherent snapshot.
- Diagnostics describe stages and error classes, not secret values.

## Maintenance implications

Warframe updates can change process names, memory layouts, authorization encodings, endpoints, and schema fields. The adapter must isolate those volatile details behind a small interface, support multiple scanners, retain redacted fixtures, and report which acquisition stage failed. Compatibility is maintained through adapters and tests, not by assuming the current patterns are permanent.
