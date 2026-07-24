# Existing Warframe acquisition implementations

Research date: 2026-07-24. This report uses first-party documentation and source code. “Verified” means the behavior is visible in cited code or official documentation; claims about Overwolf's private provider internals are explicitly left unresolved.

## Executive conclusion

AlecaFrame itself does **not** contain a process-memory inventory reader. Its shipped JavaScript asks Overwolf's Game Events Provider (GEP) for `inventory` and `match_info`, receives a JSON string at `info.match_info.inventory`, and passes that string to its unobfuscated .NET parser. The private Overwolf Warframe provider that produces this field is not published, so public evidence does not establish whether that provider obtains the JSON through memory scanning, network interception, an injected hook, or another game-specific integration.

For a Linux-native implementation, the strongest already-demonstrated route is instead:

1. discover the Wine/Proton Warframe process;
2. read its memory without writing to it;
3. locate `accountId=<24 hex characters>&nonce=<digits>` (optionally `steamId`/platform context);
4. request the game's undocumented `https://api.warframe.com/api/inventory.php` or `https://mobile.warframe.com/api/inventory.php`; and
5. parse the authoritative account JSON.

This is implemented directly by `Sainan/warframe-api-helper`, while `Sikewyrm/FrameForge` implements both credential/API acquisition and direct capture of the in-memory full-account JSON. Local experimentation in this project independently corroborated Linux process discovery, auth-tuple discovery, and receipt of a structurally complete inventory response of roughly 964 KB; no credentials or player-specific values are recorded here.

## AlecaFrame

### What is actually public

AlecaFrame is **not open source**. Its official FAQ says the .NET libraries are deliberately unobfuscated and include debug symbols for inspection with ILSpy, while the frontend is readable HTML/CSS/JavaScript. Its official About page states “All rights reserved.” Therefore inspectability is not permission to copy, modify, or redistribute its code. [Official FAQ `G3`](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/faq.md#G3-is-alecaframe-open-source) · [official About page](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/about.md)

There is no official AlecaFrame application-source repository. `seganku/AlecaFrame` is an independently uploaded unpacked Overwolf package (commit `09f9134`, 2026-07-10, importing AlecaFrame 2.6.90), contains compiled DLLs and web assets, and has no license file. It is useful as evidence of shipped behavior, not as reusable source. [Repository](https://github.com/seganku/AlecaFrame/tree/09f91347eaa2212e0b48e04ee532f0df07320c4b)

### Verified inventory flow

The unpacked manifest targets Overwolf game ID `8954`, registers the `AlecaFrameClientLib.OverwolfWrapper` .NET object, and declares Game Events access. [Manifest](https://github.com/seganku/AlecaFrame/blob/09f91347eaa2212e0b48e04ee532f0df07320c4b/manifest.json#L388-L410)

The background script:

- requests GEP features `inventory` and `match_info` through `overwolf.games.events.setRequiredFeatures`;
- listens to `overwolf.games.events.onInfoUpdates2`;
- passes `info.info.match_info.inventory` to `.NET SetWarframeData`; and
- can also poll the same value with `overwolf.games.events.getInfo`.

[Shipped `background.js`](https://github.com/seganku/AlecaFrame/blob/09f91347eaa2212e0b48e04ee532f0df07320c4b/web/assets/js/background.js#L178-L255) · [official Overwolf Warframe GEP page](https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/warframe/)

Decompilation of the deliberately unobfuscated `AlecaFrameClientLib.dll` shows that `SetWarframeData` treats the input as JSON, validates that it contains `LastInventorySync` and ends with `}`, caches the last good value, and falls back to cached data on malformed/unavailable updates. `DataHandler.LoadWarframeData` deserializes a `WarframeRootObject`; it also accepts a wrapper whose `InventoryJSON` property contains the inner JSON. This verifies the consumer and schema, but not the private GEP producer. The independently uploaded repository does not include the decompiled C# text, so these exact functions should be reproduced by locally decompiling the cited DLL rather than copied from this report. [DLL](https://github.com/seganku/AlecaFrame/blob/09f91347eaa2212e0b48e04ee532f0df07320c4b/NET/AlecaFrameClientLib.dll)

AlecaFrame's own FAQ says inventory refreshes only at login or loading screens because of “Overwolf limitations,” and suggests travelling to a relay/dojo and back. That timing is consistent with a full account snapshot becoming available during game API traffic, but this is an **inference**, not proof of how GEP captures it. [Official FAQ `G6`](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/faq.md#G6-when-is-my-inventory-data-updated-how-can-i-force-it) · [official connecting guide](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/get-started/connecting.md)

### Reward overlay is separate from inventory acquisition

The shipped .NET code monitors Warframe log messages to trigger relic handling and captures the Warframe window for OCR; it does not derive reward names from the inventory GEP event. This separation is also visible in the official troubleshooting requirements (English UI in the documented version, supported resolution/scaling). [DLL containing `EELogProcessor` and `OCRHelper`](https://github.com/seganku/AlecaFrame/blob/09f91347eaa2212e0b48e04ee532f0df07320c4b/NET/AlecaFrameClientLib.dll) · [official FAQ `T7`–`T9`](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/faq.md#T7-the-relic-overlay-is-showing-the-wrong-rewards)

## Reusable acquisition implementations

### `Sainan/warframe-api-helper` (cross-platform proof, restricted license)

This small C++ program is the clearest proof of the API route. `gruzzleAuthz` scans every readable allocation for the ASCII marker `?accountId=`, reads the 24-character account ID and following numeric nonce, then requests `/api/inventory.php` from `mobile.warframe.com`. It writes the decoded JSON and additionally emits an AlecaFrame-compatible encrypted `lastData.dat`. [Implementation at commit `ca43440` (2026-03-12)](https://github.com/Sainan/warframe-api-helper/blob/ca43440400d331e890dae7704feee3960c555906/main.cpp#L17-L125)

On non-Windows systems it first tries `Warframe.x64.exe`, then Wine's truncated `/proc/<pid>/comm` name `Warframe.x64.ex`. [Process selection](https://github.com/Sainan/warframe-api-helper/blob/ca43440400d331e890dae7704feee3960c555906/main.cpp#L60-L72) Its pinned Soup library enumerates numeric `/proc` entries and reads `/proc/<pid>/comm`; it gets mappings from `/proc/<pid>/maps` and reads bytes through `/proc/<pid>/mem`. [Soup `Process.cpp`](https://github.com/calamity-inc/Soup/blob/11de3330f97c5adbf0041851a52710f86955135a/soup/Process.cpp#L26-L94) · [Soup `ProcessHandle.cpp`](https://github.com/calamity-inc/Soup/blob/11de3330f97c5adbf0041851a52710f86955135a/soup/ProcessHandle.cpp#L25-L84)

Its license is MIT plus Commons Clause, which prohibits selling the software's functionality. That is not an OSI-approved FOSS license and should be treated as incompatible with this project's intended GPLv3 distribution. Reuse the documented technique, not its code, unless legal review says otherwise. [License](https://github.com/Sainan/warframe-api-helper/blob/ca43440400d331e890dae7704feee3960c555906/LICENSE)

### `Sikewyrm/FrameForge` (GPLv3, Windows implementation)

FrameForge 2.6.0 (commit `0949f53`, 2026-07-24) is a GPLv3 Rust/Tauri companion. It uses `OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION)`, `VirtualQueryEx`, and `ReadProcessMemory`; process detection enumerates processes with Toolhelp and selects names beginning with `warframe` while excluding launchers/companions. [PID detection](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L1449-L1486) · [GPLv3 license](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/LICENSE)

Its auth scanner searches both login-response JSON (`"id":"<24 hex>"` near `"Nonce":<digits>`) and URL-encoded `accountId=...&nonce=...`; it can also find `steamId=`. [Credential patterns](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L204-L298) The Tauri command then calls the inventory endpoint using those values. [API fetch path](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/lib.rs#L851-L893)

FrameForge also scans for a full JSON account blob directly in memory. It uses stable semantic needles such as `"RegularCredits":`, reconstructs blobs across adjacent regions, finds a terminator near `"DeathSquadable":`, requires a large minimum size, and only accepts JSON that parses into its structured `BlobInventory`. Its parsed categories include unique equipment, stackable items, mods by rank, mastery from `XPInfo`, pending recipes, consumed suits, and rivens. [Parser and schema](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L89-L154) · [blob parser](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L525-L842) · [multi-region scan](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L855-L1247)

Important limitation: FrameForge's process-memory implementation is currently compiled only for Windows; non-Windows functions return no result. Its algorithms and GPLv3 schemas are reusable, but Linux needs a native `/proc`/`process_vm_readv` backend. [Platform guards](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/src-tauri/src/memory_scanner.rs#L299-L302)

### Reward-only tools: useful supporting patterns, not inventory solutions

`soramanew/wfinfo-linux` (commit `9c8169e`, 2024-09-04) discovers the newest `EE.log` under the user's home directory, tails it, triggers on `Pause countdown done`, `Got rewards`, or creation of `ProjectionRewardChoice.swf`, captures the relevant wlroots output with `grim`, then OCRs the screenshot. Its README warns that Warframe buffers log output, so automatic detection can be inconsistent. [Linux implementation](https://github.com/soramanew/wfinfo-linux/blob/9c8169e09b74eaaa19a416b610960e36fbb92f44/ags/modules/fissure_display.js#L9-L14) · [trigger/capture/OCR](https://github.com/soramanew/wfinfo-linux/blob/9c8169e09b74eaaa19a416b610960e36fbb92f44/ags/modules/fissure_display.js#L121-L171) · [buffering warning](https://github.com/soramanew/wfinfo-linux/blob/9c8169e09b74eaaa19a416b610960e36fbb92f44/README.md#L46-L54)

The main `WFCD/WFInfo` project similarly treats `EE.log`/Warframe's debug-log stream as a reward-screen trigger and uses screenshots plus OCR. It scans profile screens only when the user explicitly invokes mastery scanning; it is not evidence that `EE.log` contains the full inventory. [Process finder](https://github.com/WFCD/WFInfo/blob/161f371422fde5aa68ed02f6d827ec6dabda9cc0/WFInfo/Services/WarframeProcess/WarframeProcessFinder.cs#L103-L183) · [reward trigger](https://github.com/WFCD/WFInfo/blob/161f371422fde5aa68ed02f6d827ec6dabda9cc0/WFInfo/Data.cs#L1568-L1606)

## Schema and design implications

- The authoritative inventory shape is a large account snapshot, not a stream of per-item events. Useful stable top-level fields visible across AlecaFrame/FrameForge include `LastInventorySync`, `Suits`, `LongGuns`, `Pistols`, `Melee`, `Sentinels`, `MiscItems`, `XPInfo`, `Recipes`, `PendingRecipes`, `RawUpgrades`, and `Upgrades`.
- `LastInventorySync` is useful as a freshness/version indicator, but correctness should come from successful full JSON parsing plus structural checks. A valid snapshot should authoritatively replace prior quantities, including real removals.
- Prefer API acquisition when a valid ephemeral auth tuple is found: it avoids heuristically reconstructing a megabyte-scale blob. Keep direct full-blob scanning as a fallback and as an empirical cross-check.
- Never persist or log `accountId`, nonce, Steam ID, raw login responses, or full raw snapshots by default. Treat the nonce as a live session credential.
- Linux process discovery must handle Proton/Wine naming and permissions. `/proc/<pid>/mem` is demonstrated, but `process_vm_readv` is a cleaner Rust backend where permitted; both may be constrained by Yama `ptrace_scope`, UID boundaries, Flatpak, or launch context.
- Inventory acquisition and reward recognition should remain separate modules: memory/API snapshot for inventory/mastery, and EE.log plus portal screenshot/OCR for relic rewards.

## Policy and risk evidence

Digital Extremes' official third-party-software guidance does not provide a blanket safe list and says external software is used at the player's own risk; enforcement can change and automated systems may react to software that interacts with the game. [Official forum policy thread](https://forums.warframe.com/topic/1383123-third-party-software-usage/)

AlecaFrame's official FAQ argues its inventory access is acceptable because Overwolf performs it and Overwolf apps are reviewed; its unofficial Linux guide separately warns that Wine/Proton/ProtonHax are not explicitly allowed and are used at the player's risk. [AlecaFrame safety FAQ](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/faq.md#G1-is-alecaframe-safe-to-use) · [Linux warning](https://github.com/alecamaracm/AlecaFrame-Docs/blob/80038d51686fb5975197a1a985f20e2e0fe73fe5/docs/linux-support.md#disclaimer-read-first)

FrameForge labels memory reading a EULA grey area, keeps it opt-in, and says DE did not clarify whether the undocumented companion endpoint is permitted; its API feature was suspended pending clearer guidance. This is a maintainer statement, not an official DE ruling. [FrameForge EULA section](https://github.com/WyrmStudios/FrameForge/blob/0949f53e8e5e0e24268184d5adea74f813312623/README.md#eula-transparency)

## Recommended next experiment

Implement a small read-only Linux spike before more product work: enumerate Warframe's Proton process, list readable regions, scan chunk boundaries for both auth encodings, call the inventory endpoint once without persisting credentials, validate a conservative set of structural fields, and hash/redact any diagnostics. Then compare that API snapshot against a direct full-account-blob capture after one controlled inventory change. This validates completeness and deletion semantics while avoiding assumptions about Overwolf's private provider.
