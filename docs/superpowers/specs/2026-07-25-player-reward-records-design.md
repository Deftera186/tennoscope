# Player Reward Record Recognition Design

## Goal

Resolve relic rewards from the transient per-player response records in Warframe memory, in left-to-right order, within one second of the game receiving the complete reward set.

## Evidence

The July 25 live capture showed all four network responses arriving within 106 ms and the complete-set line 241 ms later. A bounded memory scan found all four real rewards in 694 ms. The rejected UI-card matcher instead selected a compact stale internal-path block, proving that spatially grouped reward strings are not evidence of active cards.

## Architecture

`RewardLogMachine` emits each responder identity in arrival order and a complete-set event as soon as EE.log reports that all rewards are present. A deep `PlayerRewardRecordScanner` module receives the ordered identities plus the current relic candidate set. It performs one bounded scan for player identities and compact/internal reward identities, accepts only candidate names structurally adjacent to a responder identity, and returns either one unambiguous reward per responder or no result.

The local reward path logged by Warframe anchors the first card. Remote cards follow responder arrival order with the local identity removed. This ordering rule is kept behind one resolver interface so later evidence can replace it without changing overlay code.

## Safety Rules

- Never publish a result derived only from clustered reward strings.
- Require exactly one candidate reward for every expected responder.
- Reject duplicate rewards, missing responders, and multiple nearby candidates.
- Keep the scan read-only, bounded to writable anonymous memory, and capped below one second.
- Keep OCR as fallback only when memory cannot prove a complete set.

## Testing

Replay the exact observed event sequence through `RewardLogMachine`. Unit-test record resolution with fixtures containing real rewards adjacent to player identities and a tighter stale reward block elsewhere; the stale block must never be selected. Existing memory, coordinator, observer, and log tests remain green.
