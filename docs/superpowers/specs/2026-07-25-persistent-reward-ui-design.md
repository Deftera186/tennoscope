# Persistent Reward UI Memory Design

## Goal

Resolve the visible relic rewards in left-to-right order from the persistent reward-screen model, without racing transient network-response objects or relying on fixed Warframe addresses and offsets.

## Architecture

`warframe-acquisition` gains a deep `PersistentRewardResolver` module with one interface: resolve the expected number of visible choices from a `MemoryReader`, process, and relic-limited candidate set. The implementation owns string discovery, reverse-pointer graph construction, container ranking, repeated-read confirmation, and resource limits. Callers never know heap addresses, object layouts, or pointer chains.

The resolver seeds its graph from canonical display names and internal reward paths. It searches aligned native pointers to those seeds, walks a bounded number of reverse-reference levels, and ranks shared containers whose ordered children reach exactly the expected number of candidate rewards. A result is publishable only when two reads produce the same ordered choices while the reward screen remains open.

## Update Resistance

The resolver must not contain absolute addresses, module-relative offsets, Warframe build numbers, player identities, or fixed Lua/Scaleform field offsets. Small self-describing string-header variants may be recognized only through runtime validation. Graph traversal accepts changing allocation addresses and intermediate object layouts.

If no unique stable container exists, the resolver returns `Incomplete` or `Ambiguous`; it never falls back to proximity or allocation order. Existing transient responder scanning remains diagnostic-only and cannot publish choices.

## Performance and Safety

Scanning is event-gated to the 15-second reward window and candidate-limited to loaded relics. Pointer searches operate on bounded writable mappings, reuse buffers, and stop after a fixed graph depth and byte budget. Once a container is discovered, subsequent reads use only its small graph neighborhood.

All access remains read-only. Captured fixtures contain only bounded graph neighborhoods needed for deterministic replay.

## Validation

Synthetic tests cover duplicate rewards, three- and four-card screens, decoy catalog strings, competing containers, changed intermediate layouts, unstable order, and confirmation failure. Archived captures are used to validate pointer encoding and reject false 32-bit matches. Live acceptance requires correct order within three seconds and identical results across repeated reads during one reward window.

