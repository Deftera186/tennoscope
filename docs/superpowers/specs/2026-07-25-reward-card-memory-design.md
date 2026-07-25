# Reward Card Memory Design

## Goal

Resolve relic rewards from read-only Warframe memory in exact left-to-right order within 1.5 seconds, without trusting stale catalog strings or requiring OCR.

## Live Evidence

Warframe's live reward UI stores a one-based card tag and reward internal path in the same short allocation. A captured second card contained `RewardList.Item2.TagContainer.Tag1.IconText`, followed 80 bytes later by `/Lotus/Types/Recipes/Weapons/TenoraPrimeBlueprint`. Tenora Prime Blueprint was the second visible reward.

The local player's reward is logged explicitly by EE.log and is consistently the first visible card. Therefore Item1 is anchored by the log, while memory must resolve Item2 through ItemN.

## Recognition Rules

- Scan only during a log-confirmed reward lifecycle.
- Search candidate internal paths only within 256 bytes of `RewardList.Item2` through `RewardList.Item4` tags.
- Accept a slot only when exactly one candidate path is associated with that slot.
- Reject duplicate names, conflicting values for one slot, missing slots, or a count different from the rendered-card count.
- Build output in numeric slot order, with the log-confirmed local reward as Item1.
- Existing temporal clusters remain a secondary memory strategy. OCR remains fallback-only.

## Module Seam

`RewardFingerprint` owns card-slot evidence collected during its existing bounded scan. `resolve_reward_choices_with_anchor` hides slot validation and temporal fallback behind one interface: baseline, current fingerprint, rendered count, maximum cluster span, and optional local reward name in; `RewardResolution` out.

The Tauri adapter resolves the EE.log local path to a catalog name before invoking the memory source. Callers do not learn memory layout details.

## Performance and Safety

Card tags and reward paths are matched during the existing scan, adding no second process-memory traversal. Reads remain bounded, read-only, and lifecycle-gated. Exact slot evidence bypasses broad proximity heuristics.

## Acceptance Criteria

- A fixture containing Item2–Item4 tags and paths returns `[local, second, third, fourth]`.
- Missing, duplicate, or conflicting slots remain incomplete or ambiguous.
- Stale reward names without a nearby item tag cannot become choices.
- Three-card and four-card screens work.
- Existing reward-memory and source tests remain green.
