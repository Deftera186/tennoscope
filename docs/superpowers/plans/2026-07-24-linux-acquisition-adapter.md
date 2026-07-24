# Linux Inventory Acquisition Adapter Plan

**Goal:** Add a privacy-preserving Linux/Proton adapter that discovers Warframe, reads memory without modifying the process, extracts ephemeral inventory authorization, fetches a complete account snapshot, and publishes only validated observations to `app-core`.

**Architecture:** A new `warframe-acquisition` crate owns volatile game integration. Deep interfaces separate process discovery, memory reading, authorization scanning, HTTP transport, and snapshot parsing so each can be tested with fixtures and replaced after game updates. The production Linux backend uses `/proc`; tests use byte regions and fake transports. `app-core` receives a coherent snapshot only after the entire acquisition transaction succeeds.

**Licensing:** Independently implement the verified technique. GPLv3 FrameForge may inform or supply compatible fallback work with attribution. Do not copy code from MIT-plus-Commons-Clause implementations or the all-rights-reserved AlecaFrame distribution.

## Task 1: Define acquisition contracts and secret-safe errors

- Add the workspace crate and public acquisition result/health types.
- Define narrow traits for process discovery, readable regions, inventory transport, and snapshot decoding.
- Add a redacted secret wrapper whose `Debug` and `Display` never reveal its value.
- Test that every public error and diagnostic is secret-free.

## Task 2: Implement authorization scanning test-first

- Add redacted byte fixtures for URL-encoded and login-response authorization forms.
- Scan incrementally with overlap so matches spanning read chunks are found.
- Validate account identifiers and numeric nonces conservatively.
- Rank complete candidates and reject ambiguous or malformed matches.
- Keep credential values ephemeral and out of serialized types.

## Task 3: Implement Linux/Proton process memory access

- Discover both full and Wine-truncated Warframe process names.
- Confirm candidates using executable mappings, excluding launchers and helper processes.
- Parse readable ranges from `/proc/<pid>/maps`.
- Read bounded chunks from `/proc/<pid>/mem`, tolerating individual vanished/unreadable mappings.
- Return actionable permission diagnostics for Yama, UID, sandbox, and process-exit failures.

## Task 4: Fetch and validate the inventory snapshot

- Use a Rust HTTP client with explicit timeouts, response-size limits, and no automatic secret-bearing logs.
- Send the ephemeral authorization only to the configured Warframe HTTPS inventory origin.
- Parse the response without writing the raw body to disk.
- Require complete JSON and conservative top-level structural fields before conversion.
- Convert supported inventory/mastery sections into one coherent domain snapshot.
- Ensure a failed or partial acquisition leaves the prior snapshot untouched.

## Task 5: Integrate refresh orchestration and diagnostics

- Add an app-core acquisition port and transaction boundary.
- Run on game detection and after an `EE.log` inventory-sync trigger, with rate limiting.
- Publish stage health: game discovery, memory permission, authorization discovery, endpoint fetch, schema validation, and last successful synchronization.
- Never include credentials or raw player data in diagnostics.

## Task 6: Verify against fixtures and the live opt-in environment

- Run unit, integration, formatting, and clippy checks.
- Add redacted schema fixtures derived from synthetic data, not the player's raw snapshot.
- Provide an ignored/manual live test that prints only stage success and structural counts.
- Re-run the manual test against the current Proton session and confirm no secret appears in captured output.
- Document permission troubleshooting and the account-policy disclosure.

## Deferred fallback

After the primary adapter is stable, add a Linux implementation of FrameForge's GPLv3 full-account-blob scanner behind the same memory-reader interface. It must pass strict whole-document validation before it can publish a snapshot.
