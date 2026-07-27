# Memory-Gated Relic Reward Recognition Design

## Goal

Add a fast, read-only relic reward acquisition path that recognizes the online squad's displayed reward choices from Warframe memory. Memory recognition is the preferred source when it produces a coherent result; OCR remains a compatibility fallback until the memory path has been validated across multiple missions and game updates.

## Evidence and Constraints

The first synchronized online capture displayed Perigale Prime Receiver, Burston Prime Receiver, Trumna Prime Blueprint, and Forma Blueprint. All four display names existed as UTF-8 in writable anonymous Warframe memory. Some occurrences belonged to the general loaded relic-description catalog, so an unqualified string search cannot distinguish live choices. The local generated reward also appeared in `EE.log` and as an internal StoreItems path in memory.

Solo fissures do not display a choice screen and therefore serve as negative controls. Online fissures provide the multi-choice state. The integration must remain read-only, avoid continuous process scanning, and never delay or freeze the game.

## Acquisition Pipeline

`EE.log` is the lifecycle clock. The monitor records loaded relic paths during the mission and recognizes `OpenVoidProjectionRewardScreenRMI`, `GetVoidProjectionRewards`, `Got rewards`, selection completion, and shutdown. Memory work only runs during this bounded reward window.

The first implementation uses temporal, candidate-limited scanning:

1. Build the allowed reward-name and internal-path set from the loaded relics and WFCD `Relics.json`.
2. Before reward initialization, record lightweight occurrence fingerprints for those candidates in prioritized readable regions.
3. Immediately after `Got rewards`, rescan the same regions and candidates.
4. Prefer new or changed, tightly clustered occurrences that resolve to the expected online choice count.
5. Repeat a narrow confirmation scan of only the winning regions before publishing the choices.

The scanner searches writable anonymous regions first, followed by writable private file-backed regions. It does not scan executable mappings and does not scan the entire process when the candidate set or event state is unavailable. Chunk buffers are reused, overlap only by the longest candidate length, and are wiped after use.

## Reliability Rules

A memory result is publishable only when:

- every result maps to the loaded relic candidate set;
- the number of distinct choices matches the observed online squad reward count;
- choices occur in a bounded cluster or another repeated structural relationship discovered during capture runs;
- a confirmation read produces the same ordered set;
- no equally ranked conflicting cluster exists.

An ambiguous, incomplete, stale, solo, or unsupported result is not guessed. TennoScope falls back to event-gated OCR. During the validation phase, memory and OCR results are compared and disagreements are recorded as secret-free diagnostics; memory does not silently override a conflicting visual result.

## Module Boundaries

- `warframe-acquisition::reward_memory` owns candidate encoding, region prioritization, temporal fingerprints, cluster ranking, confirmation, and bounded read policy. It depends only on `MemoryReader` and validated candidate metadata.
- The Tauri log monitor owns reward-window lifecycle and invokes memory recognition only for relevant online events.
- The existing OCR observer remains a separate adapter implementing the same reward-choice result contract.
- A small coordinator selects confirmed memory results first, then OCR fallback, and exposes source/confidence diagnostics to `app-core`.
- Experimental capture artifacts remain opt-in development tooling and never contain authorization values or arbitrary memory dumps.

## Performance Budget

- No memory scanning outside a log-confirmed reward window.
- Initial scan is restricted to candidates from loaded relics and prioritized regions.
- Confirmation reads only regions containing the leading cluster.
- Target: first result within 500 ms after `Got rewards` on the current Linux/Proton setup.
- Hard timeout: 1.5 seconds, after which OCR proceeds independently.
- Memory and temporary buffers are bounded; no core dumps or multi-gigabyte snapshots are written.

## Validation Plan

Before enabling memory recognition by default, capture at least three online reward screens covering different relics and squad compositions, plus one solo negative control. For each online run retain only:

- the relevant `EE.log` delta and event timestamps;
- loaded relic paths;
- expected choices from a screenshot;
- secret-free candidate hit locations expressed relative to their mapping;
- scan duration, bytes read, selected cluster, and confirmation result.

Recorded synthetic fixtures reproduce those relative layouts for deterministic tests. Tests cover candidate filtering, temporal subtraction, cluster ranking, conflicts, confirmation failure, solo suppression, timeouts, buffer boundaries, and OCR fallback selection.

## Rollout

The first release labels the source as experimental in diagnostics and keeps OCR enabled. After the validation set passes and live results agree, memory becomes the preferred default source. OCR remains available for Warframe updates, unsupported platforms, changed layouts, or failed validation. A future stable pointer/signature fast path may replace temporal scanning only after it survives multiple game builds; it must use the same validation and fallback contract.

## Acceptance Criteria

1. Memory reads begin only inside a log-confirmed online reward window.
2. The scanner reads only prioritized regions for rewards possible from the loaded relics.
3. A coherent confirmed result reaches the overlay within the performance budget.
4. Ambiguous or slow scans fall back to OCR without blocking the overlay lifecycle.
5. Solo fissures never produce a fabricated multi-choice result.
6. Automated fixtures and at least three online live captures agree with the visible reward choices before memory recognition is enabled by default.
