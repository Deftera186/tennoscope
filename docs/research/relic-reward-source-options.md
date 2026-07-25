# Relic reward acquisition: logs, memory, and OCR

Research date: 2026-07-25

## Bottom line

For a FOSS, cross-platform companion without Overwolf, the strongest demonstrated design is **EE.log as an event/timing and candidate source, followed by window capture and OCR for the actual choices**.

- `EE.log` exposes enough information to know that the reward screen is opening, how many squad responses exist, which relics were loaded, when the screen closes, and which reward the local player ultimately received. In the implementations reviewed, it does **not** expose the complete set of reward choices displayed to the player.
- Read-only process memory demonstrably exposes the large account/inventory JSON blob. No reviewed open-source implementation demonstrates a stable, update-resistant memory locator for the transient set of relic reward choices.
- Three independent open-source relic helpers use OCR for the choices. The most mature hybrid implementation, FrameForge, also reads process memory for inventory, yet still uses `EE.log` plus OCR for relic choices. That separation is strong practical evidence that the account blob is not a substitute for reading the reward cards.

Therefore OCR is justified for the MVP, but continuous blind polling is not. It should be event-gated by `EE.log`, restricted to the Warframe window/reward region, and matched against the small reward set implied by the squad's loaded relics.

## What `EE.log` can and cannot provide

### Demonstrated useful signals

FrameForge parses these log sequences:

- `VoidProjections: GetVoidProjectionReward...` as the reward-screen trigger.
- `Still waiting on response from ...` and `Client has reward info for all players now` to infer squad size.
- `Resource load completed ... (/Lotus/Types/Game/Projections/...)` to collect the exact relic paths loaded by the squad and reduce the OCR candidate catalog.
- `gets reward /Lotus/...` for the local player's final granted item.
- reward-screen shutdown/close lines as dismissal signals.

The implementation then explicitly captures the Warframe reward area and passes its pixels to the OCR matcher. The log-derived relic paths only prefilter the possible names; they do not supply the four displayed choice names. See [FrameForge's log watcher and OCR handoff](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/lib.rs#L5136-L5680) and its [screen-capture/Windows OCR module](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/ocr.rs#L1-L28).

`wfinfo-go` independently uses the same division of labor: its log parser recognizes `OpenVoidProjectionRewardScreenRMI`, `Relic rewards initialized`, and `GetVoidProjectionRewards`, then calls `screenshot()` and `DetectItems(...)`. See [the trigger and capture code](https://github.com/simon-wg/wfinfo-go/blob/03eddac30fdeefe8a6b72fa1a0ca767c14097849/internal/app.go#L149-L175).

`wfinfo-ng` likewise watches `EE.log` for reward-screen events, captures a frame, and runs Tesseract over the reward region. Its own README notes that log buffering can deliver the event after the screen has disappeared, so a robust implementation should retain a lightweight visual fallback/retry rather than trusting log timing alone. See [its event loop](https://github.com/knoellle/wfinfo-ng/blob/2c6fbe6a2be160b6996857f0e72f339fad5273d3/src/bin/main.rs#L72-L112), [OCR pipeline](https://github.com/knoellle/wfinfo-ng/blob/2c6fbe6a2be160b6996857f0e72f339fad5273d3/src/ocr.rs#L335-L371), and [buffering caveat](https://github.com/knoellle/wfinfo-ng/blob/2c6fbe6a2be160b6996857f0e72f339fad5273d3/README.md#L37-L50).

### What the log does not demonstrate

None of these source trees parses four reward item paths or names from `EE.log`. The only direct item assignment demonstrated in the log is the local player's eventual `gets reward /Lotus/...` line. That arrives too late to advise the player's selection and represents one granted item, not all displayed choices.

This is an evidence-bounded conclusion, not a claim that no game build could ever log more. A future Warframe update could add or remove log fields. The parser should keep raw trigger diagnostics and fixtures so new signals can be adopted quickly.

## Memory access

FrameForge provides a concrete read-only memory implementation. It opens Warframe with `PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`, scans committed readable regions, and searches for the account JSON marker `"MiscItems":[`. See [the one-shot memory scanner](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L435-L519).

That scanner is explicitly an **inventory/account blob** scanner. It locates durable account sections such as `MiscItems`, not transient reward-card state. In the same application, relic choices still go through the capture/OCR subsystem. FrameForge's own feature description distinguishes [memory-based inventory](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/README.md#L11-L14) from its [OCR relic overlay](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/README.md#L59-L60).

A transient reward structure probably exists somewhere in the client because the UI must render it, but no reviewed primary source supplies a signature, schema, pointer chain, or validation method for it. Finding one would require a separate reverse-engineering effort across multiple game updates. It would be more update-sensitive than the account JSON marker and would need strong validation before it could replace OCR.

Practical recommendation:

1. Keep process-memory inventory extraction as a separate acquisition adapter.
2. Do not claim memory-based relic-choice detection until a repeatable structure has been captured across several missions, squad sizes, languages, refinements, and game builds.
3. If investigated later, treat memory recognition as an optional fast path and retain OCR as the compatibility fallback.

## Existing OCR data path

OCR does not require training a custom model. The reviewed tools use a general OCR engine and constrain/match its output against canonical Warframe item names:

- `wfinfo-go` configures Tesseract, captures the Warframe window through X11, preprocesses text regions, and fuzzy-matches output against its item cache. See [OCR initialization and capture invocation](https://github.com/simon-wg/wfinfo-go/blob/03eddac30fdeefe8a6b72fa1a0ca767c14097849/internal/app.go#L45-L68) and [X11 capture](https://github.com/simon-wg/wfinfo-go/blob/03eddac30fdeefe8a6b72fa1a0ca767c14097849/internal/x11.go).
- `wfinfo-ng` uses Tesseract's English model and crops known reward-card geometry before recognition. See [reward geometry and Tesseract use](https://github.com/knoellle/wfinfo-ng/blob/2c6fbe6a2be160b6996857f0e72f339fad5273d3/src/ocr.rs#L5-L25).
- FrameForge uses the operating system OCR engine on Windows, detects reward-card layout/rarity bars, fuzzy-matches names, and limits candidates using the relic paths collected from `EE.log`. See [its OCR engine setup](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/ocr.rs#L570-L620) and [candidate filtering from exact relic paths](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/lib.rs#L5471-L5505).

This means TennoScope's OCR “data” should be:

- the installed OCR language model;
- fixed/scaled reward-region geometry plus preprocessing rules;
- canonical reward names from WFCD;
- per-relic candidate mappings from WFCD `Relics.json`;
- fuzzy-match thresholds and regression screenshots.

## Item/reward metadata and images

WFCD's `warframe-items` repository is the primary catalog used by the reviewed tools. Its README states that each item's image filename is in `item.imageName`, the image files live under `data/img`, and the hosted form is `https://cdn.warframestat.us/img/${item.imageName}`. See [WFCD image documentation](https://github.com/WFCD/warframe-items/blob/81c893536dee6de23fbf114cf52d1b01d23bd65d/README.md#L100-L110).

FrameForge obtains:

- general items and `imageName` from WFCD `All.json`;
- exact relic/refinement reward mappings from WFCD `Relics.json`;
- images from the WFCD CDN, with an optional local cache.

See [WFCD catalog parsing and Relics.json fetch](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/wfcd.rs#L545-L600) and [image caching](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/lib.rs#L7517-L7560).

Missing images in TennoScope are therefore not necessarily missing upstream. For example, WFCD `All.json` contains `/Lotus/Types/Items/MiscItems/Alertium` as **Nitain Extract** with `imageName: "Alertium.png"`; the same item often appears nested as a recipe component rather than as a convenient top-level record. See [one canonical occurrence](https://github.com/WFCD/warframe-items/blob/81c893536dee6de23fbf114cf52d1b01d23bd65d/data/json/All.json#L64565-L64570).

The catalog importer should recursively index nested components by `uniqueName`, merge duplicate records, and propagate images across same-`uniqueName` and same-display-name aliases. FrameForge applies a similar same-name image propagation for StoreItems proxy entries that lack `imageName`; see [its propagation pass](https://github.com/Sikewyrm/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/wfcd.rs#L1144-L1158).

## Recommended TennoScope pipeline

1. Tail `EE.log` continuously and parse reward trigger/dismissal, squad-size hints, loaded relic paths, and the final local reward.
2. On trigger, identify the actual Warframe window/output and capture only the reward-card region. Do not capture whichever application happens to be focused.
3. Build a small allowed-name set from the loaded relic paths and WFCD `Relics.json`; fall back to the complete relic-reward catalog if the mission began before TennoScope started.
4. Run OCR in short retries while the reward screen is active, fuzzy-match within that candidate set, and require a coherent 1–4-card result before displaying the overlay.
5. Place a compositor-native, always-above, input-transparent overlay relative to the Warframe window. Capture and overlay adapters must be compositor/platform specific; the recognition core can remain shared.
6. Use the final `gets reward` line and the next inventory snapshot as post-selection consistency checks, not as the decision source.

This approach is demonstrably used by existing FOSS tools, minimizes OCR work and false matches, and is more likely to survive ordinary Warframe updates than undocumented transient-memory signatures.
