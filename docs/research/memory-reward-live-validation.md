# Memory reward recognition live validation

Date: 2026-07-25

## Current evidence

- Warframe process discovery and read-only `/proc` memory access succeeded under Proton.
- One captured online reward screen exposed all four visible reward display names as plain UTF-8 in writable process memory.
- The same process also contained unrelated/stale reward catalog strings, proving that a single exact-name hit is not sufficient.
- EE.log exposed the online reward lifecycle, four distinct reward responders, loaded projection paths, and the selected reward's internal StoreItems path.
- A bounded release-mode probe scanned 384 MiB in 89 ms while Warframe was running. It reported only candidate hit counts and aggregate timing; it emitted no memory contents, absolute addresses, credentials, or process paths.

## Implemented validation rule

1. Wait for `OpenVoidProjectionRewardScreenRMI`.
2. Build candidates only from projection paths observed in EE.log.
3. Take one bounded baseline fingerprint.
4. Wait until reward information for all online players is ready.
5. Take one bounded current fingerprint and subtract unchanged occurrences.
6. Require exactly the expected number of distinct candidates in one bounded region cluster.
7. Re-read only that region and require the same ordered choices.
8. Fall back to OCR for incomplete, ambiguous, timed-out, or failed memory recognition.

## Remaining live acceptance captures

The implementation is running in experimental comparison mode. Default memory-only operation still requires:

- Two additional online reward screens with visible choices recorded against the overlay result.
- One solo fissure negative control confirming that no multi-choice scan is requested.
- No memory/OCR disagreement across the accepted online captures.

These are runtime validation requirements, not missing implementation pieces.

## Persistent UI model correction

Transient network-response records were found to be unsuitable as a publication source: they can disappear before a post-log scan reaches them, and proximity matching can associate stale reward strings with unrelated player-ID copies. That publication path is now disabled.

Archived reward-screen captures were re-evaluated using complete aligned 64-bit pointers instead of four-byte pointer fragments. Four captures contain 27 genuine reward-object references after full-width validation. This supports reverse-pointer graph discovery from persistent reward strings to an ordered UI container while eliminating the majority of prior false references.

The current experimental resolver contains no absolute addresses, Warframe build identifiers, player IDs, or fixed pointer chains. It requires an exact card count, a contiguous ordered native-pointer container, a unique order, and a successful live reread before publishing. Candidate strings without a container and competing containers are rejected. Live acceptance of the new resolver remains outstanding.
