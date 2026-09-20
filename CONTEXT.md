# TennoScope

TennoScope helps Warframe players understand their collection and the rewards offered during play.

## Reward recognition

**Relic pool**:
The possible rewards of the relics loaded by the current squad, as identified in the game log and resolved against the relic catalog. It is not the set of rewards actually displayed.
_Avoid_: Memory-derived rewards

**Reward screen**:
The temporary choice of two to four relic reward cards shown after a squad opens its relics. A solo reward is not a multi-card choice.

**Reward recognition**:
Identifying the reward cards currently displayed, using screen text constrained by the squad's relic pool and any available log evidence. Recognition must distinguish a current reward screen from a closed or replaced one.

**Overlay mode**:
Observation using process presence, the game log and screen capture, without reading game-process memory or acquiring inventory from it.

**Full mode**:
Observation that additionally permits read-only game-process memory access and inventory acquisition. It does not change which relics belong to the squad's relic pool.

**Companion mode**:
Use of catalogs, saved collection and market features without observing the running game.
