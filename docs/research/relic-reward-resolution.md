# Relic reward resolution

How TennoScope determines the four relic reward cards and their order, and the evidence behind each
decision. Written 2026-07-26 from five labelled live runs.

## What each source can answer

| question | source | reliability |
|---|---|---|
| screen order of the four cards | EE.log squad ring, rotated so the local player is first | confirmed in all five labelled runs |
| the local player's own reward | `VoidProjections: <id> gets reward <path>` | exact, always present |
| the other three rewards | reading the reward screen | the only source, hosting or not |

Memory answers none of these. It was tried first for the three remote rewards until 2026-07-27 and
never once answered on a live run; see below.

## The ordering model

EE.log emits, on the reward screen:

```
VoidProjections: Client got reward info from <id>      # or "Host got reward info from"
VoidProjections: Still waiting on response from <id>
VoidProjections: <id> gets reward /Lotus/StoreItems/...   # local player only
VoidProjections: Client has reward info for all players now
```

The first responder plus the `Still waiting` identities, in log order, give the squad roster.
Rotating that list left so the local player is first reproduces the on-screen left-to-right order.

Note the reward screen also prints player names under the cards. Those are the **selection**
indicator — who has picked which reward — not a mapping of who contributed what. Reading them as
attribution is wrong and briefly looked like it falsified the ordering model.

## Response record layouts

Both captured verbatim from a live process and committed as fixtures.

Outgoing, what a client serialises for itself (`tests/fixtures/void-response-record.bin`):

```
18 <24-byte account id> <len> <display name> 00 <len> <session key> .. <len> 00 <reward path>
```

Host-side, one per squad member (`tests/fixtures/void-response-record-host.bin`):

```
.. <len=0x48> <reward path> .. <len=0x18> <account id> <len> <display name>
```

The path precedes the identity in the host-side record, 181 bytes back in the captured sample. A
scan that only looks forward from an identity hit reads the first layout and never the second,
which is why hosting produced nothing for a long time.

## Why memory cannot answer this

Three routes ruled out against labelled runs. The four rewards are always resident in memory;
nothing observed links them to a player or to a slot.

| route | result |
|---|---|
| per-player record beside the account id | does not exist as a client — nearest identity was 90 KB from any reward path |
| pointer array to the four reward strings | tightest cluster containing all four spanned 425 KB, and not in screen order |
| four display names in an ordered UI buffer | scattered; nearest pair 45 KB apart |

Useful incidental facts: a pointer to a reward string aims at `path_start - 24`, the string object
header. A reward may be resident under `/Lotus/Types/Recipes/...`, the `/Lotus/StoreItems/...`
alias, or both — filtering on the alias alone silently dropped a quarter of one squad.

### It was not a timing artefact

Every capture above was triggered by `Got rewards` in EE.log, which arrives with the flush delay
described below — so all of them may have run after the screen tore down. Interned path strings
outlive the screen, per-player structures would not, and "no such record exists" and "we looked
after it was freed" produce identical evidence. That confound made the whole table suspect.

`scripts/watch_reward_visual.py` removes it by firing the captures on the screen being *visible*.
Run of 2026-07-27 01:10:48, captured 6.5s into a fifteen-second screen:

| squad member | account id resident | response record | reward path |
|---|---|---|---|
| `…5e5292` remote | yes, 12 hits | no | no |
| `…126865` remote | yes, 8 hits | no | no |
| `…22563e` remote | yes, 8 hits | no | no |
| `…57b7f7` local | yes, 100 hits | yes, at three addresses | `PrimeDaikyuBlueprint` |

The local record is the control: found in-window, at three addresses, carrying the reward the
screenshot shows in slot 1. The scan therefore works at that moment, so the three remote absences
are absences rather than a scan that ran too late. Reading in-window changes nothing — a client
holds a response record for itself and for nobody else.

Note the sweep's other "records" are false positives: `BeastNeutralStance` entries belonging to pet
companion ids, not squad members.

### Hosting does not help either

The table above once had a separate row promising that a host keeps a record per squad member, on
the strength of `tests/fixtures/void-response-record-host.bin` — 288 bytes captured while hosting,
with the reward path 181 bytes *before* the account id. `RECORD_LOOKBEHIND` exists to read that
layout, and `structured_response_reads_a_host_record_whose_reward_precedes_the_identity` still
passes against those bytes.

A fixture pins a byte layout. It does not prove the record is there to be found on a live screen,
and the host run of 2026-07-27 01:25 says it is not:

```
[DEBUG-evidence]      responders=4 ... record_headers=15 reward_hits=25 structured_records=1
[DEBUG-player-record] responders=4 elapsed_ms=304 resolution=Incomplete
```

One structured record out of four responders, and it is the local player's. The same shape as every
client run. Across ten reward events that day the memory path returned `Incomplete` ten times, and
the only `Confirmed` responder in the whole log is `…57b7f7`, the local account.

The `associations` the sweep reported for remote `…7b9ba5` are all false positives: six are
`compact_utf8` coincidences in high-entropy pages with no readable path anywhere near them, and the
seventh is the in-game chat scrollback. The record the scan rejected at that identity holds
extraction-timer HUD JSON and a melee sound path.

So the reward path a host serialises for a remote player is not resident at the moment the screen is
up, whatever that fixture caught. Memory was removed from the live reward path on 2026-07-27; the
scanner and its fixtures stay in `warframe-acquisition` so the question can be reopened against
evidence rather than against a layout.

## Reading the screen

Warframe runs under Proton as an XWayland client, so `import -window` reaches it with no compositor
portal. Capture writes PPM rather than PNG: the frame is discarded after four crops, and
PNG-encoding 1920x1080 costs 1.9s against 0.04s.

This is not general OCR. EE.log names the squad's relics before the screen renders, so each card is
matched to the nearest of roughly two dozen known rewards by normalised edit distance.

### Separating the title from the card art

An earlier version of this note said tesseract read the UI font cleanly from a plain greyscale crop
and that thresholding made it worse. That was wrong, and it was wrong in the direction that costs
accuracy. The title is near-white text laid over arbitrary card art, and handed the raw greyscale
crop tesseract reads the art as well as the text: a dark helmet behind a word garbled it (`Prime` as
`Pritfie`), and card borders at the edge of the crop arrived as a leading `|`, `Fr` or `pA UY`.

Isolating the text takes two steps, and the first is what the earlier attempt was missing:

```
-colorspace gray -normalize -threshold 74% -negate -resize 300%
```

`-normalize` makes the cutoff relative to the crop's own brightness rather than an absolute grey
level, which is what lets a single constant hold across different card art — and should absorb
another machine's gamma. `-threshold` then drops everything dimmer than the text, and `-negate`
turns it dark-on-light, which is what tesseract is trained on.

74% is the middle of a plateau, not a tuned peak. Over the twelve labelled cards, every cutoff from
70% to 78% reads all twelve exactly. Plain thresholding without `-normalize` only manages that at
isolated values — 78%, 80%, 88% — and falls to 0.89 between them, which is a spike to fall off
rather than a setting to depend on. The plateau is the reason to trust the constant.

Result: **all twelve cards read exactly, 1.000 across the board**, with no leading or trailing junk
left. The non-reward screen that the poller has to keep rejecting still fails hard, at 0.286 against
the 0.6 floor, so the detector is not weakened by the cleaner reads.

### Card geometry

As fractions of the window so it carries across resolutions: cards on a 242/1920 pitch from
478/1920, titles at 408/1080, height 58/1080.

That box was 418/1080 high 76/1080 until 2026-07-27, and was wrong at both edges. The top started
below the ascenders of the first line of a title long enough to wrap onto two lines, and clipped
glyphs do not read as noise — they read as confident wrong letters, so `Caliban Prime Chassis` came
back as `Caliban Flime Gnassis` (`C`→`G`, `h`→`n`, and `Caliban`→`Laliban` on a worse frame). The
bottom reached into the divider ornament, which tesseract read as a trailing `4` or `ty` on *every*
card, costing every read an edit against its own name. Thresholding cannot recover either: clipped
pixels are not on the screen to be recovered, so the geometry had to be right first.

The failure mode is worth naming, because it is why this survived five live runs: a misaligned crop
does not fail. The closed-set match absorbed the damage and returned the right reward anyway, at
0.83 against a floor of 0.6, so nothing downstream ever complained. Only the score moved.

`tests/fixtures/reward-screen-wrapped-title.png` is that host screen, masked to the title band, so
the wrapped-title case has a fixture; the single-line fixture cannot catch this. The test asserts
scores rather than names, for the reason above.

When a card scores below 0.85 its crop is kept and its path logged, so the next fault can be
diagnosed from pixels rather than from garbled text. Both of the defects above were invisible in the
text alone — the geometry one was only found because an unrelated capture script had saved a
screenshot.

Two guards keep a bad read off the screen: anything below the match floor is dropped, and a read
that does not contain the log's local reward is discarded.

## Watching for the screen instead of being told about it

EE.log is flushed seconds after the events it describes -- measured at ~7.5s on 2026-07-27, against
a screen that lives for fifteen. Every reward capture triggered by `Got rewards` therefore starts
at or after the point the screen is already tearing down, which is why the overlay never appeared
on a live run and why the memory evidence below is weaker than it looks.

Relic *loading* is logged minutes earlier -- 125s ahead of the screen in the run replayed by
`app/src-tauri/tests/relic_run_replay.rs` -- so that is what arms a poller instead. The closed-set
match doubles as the detector: only the reward screen yields four names from this squad's relic
pool, so no separate "is the screen up" check is needed.

### Why it did not run

Arming claimed the "a poller is already running" flag *before* checking whether the relic pool was
empty, then returned without spawning a thread. Nothing clears that flag except a running poller
exiting or the screen shutting down, so one empty pool left it set with nothing behind it and every
later relic load in that fissure was declined as a duplicate. The pool is empty exactly when the
first relic pair does not resolve, which is the common case early in a fissure.

From outside this is invisible: no thread, no reads, no log line, indistinguishable from the poller
having run and found nothing. It stayed unnoticed for four live runs because the loop hardcoded its
screen source and could only be reached by playing a fissure. `spawn_reward_screen_poller_with`
takes the source and the timings as arguments so `app/src-tauri/tests/reward_poller.rs` can drive
it against a scripted screen in milliseconds; that test caught this on its first run.

The first poll now happens immediately rather than one interval in.

### The pool was frozen at arming

Three live runs on 2026-07-27 produced no overlay, with a perfectly working capture. The debug log
is unambiguous once the arming trace is read as a pair:

```
[DEBUG-poller] arm pool=11 already_running=false
[DEBUG-poller] arm pool=17 already_running=true
```

Squad relics are logged one at a time as they load, and `BaselineRequested` fires on the *second* of
four. The pool was handed to the poller by value at that instant, so the two relics that finished
loading afterwards only ever reached arming calls that the duplicate guard declined. The poller then
spent the whole fissure matching a four-card screen against a pool that knew two relics' worth.

What made it look like a capture failure is that one unmatched card fails the *entire* read: the
screen showed Kompressa Prime Blueprint, Banshee Prime Neuroptics Blueprint, Cedo Prime Receiver and
Caliban Prime Chassis Blueprint, of which only Banshee was absent from the stale pool — and the
whole screen was rejected on that one card, over and over, for fifteen seconds. Three of the four
were being read perfectly the entire time.

The kept crops are what settled it: `tennoscope-reward-crop-…-44.png` is a clean, unmistakable
`Banshee Prime Neuroptics Blueprint`, held only because it scored below the keep threshold. Reading
that alongside the reconstructed pool took the diagnosis from "OCR is broken again" to "OCR is fine
and the pool is stale" in one step.

The pool is now a shared cell the poller re-reads every poll, and every baseline publishes into it
rather than only the first. `a_relic_that_loads_after_arming_still_reaches_the_running_poller`
covers it, and fails against the old capture-by-value.

Still open: a single unrecognised card still discards the other three. That is correct when the read
is garbage, and wrong when it is one genuinely unknown reward — a new prime the catalog has not
caught up with would silently cost the whole overlay.

### What the layer-shell path was skipping

`show_reward_overlay` returns as soon as `configure_linux_layer` succeeds, so on Wayland every
property set in `configure_reward_overlay` was skipped: `set_size`, `set_position`,
`set_focusable(false)`, `set_ignore_cursor_events(true)` and `set_always_on_top(true)`. Two reported
symptoms came from that one early return.

Click-through was the felt one — without it the strip is an input-grabbing surface parked over the
game, so the pointer catches on it for as long as the overlay is up. The other was width: the layer
path sized itself with `set_default_size`, which is only an initial hint, and a layer surface
anchored on two edges is free to come out wider. The overlay is a four-column grid sized to the
game's four cards, so any extra width is shared out and every card renders wider than the reward it
sits under. `set_size_request` pins it.

### The retry that blocked the hide

`visual_choices` runs synchronously on the monitor thread, and that thread is also the one that
watches `visual_screen_gone` and takes the overlay down. Because EE.log is flushed late, the retry is
routinely entered *after* the screen has already closed — and it then spent its full eight-second
deadline capturing a screen that was not there, with the monitor blocked behind it, unable to act on
a hide it had already been told to perform.

It now takes the screen-gone flag and returns immediately when it is set. The 8.1-second run of
failing captures at 355ms intervals visible in the 2026-07-27 02:44 crop timestamps is exactly this
loop.

### Timestamps

Every line in the debug log used to be untimed, which answers "did this happen" and not "how long
did it take". A report that the overlay lingered for about five seconds could not be checked against
it at all; the only clock available was the mtimes of the kept crop files, by accident. `[HH:MM:SS.mmm]`
now prefixes every line, and the overlay's own show and hide are traced, since "the overlay
lingered" could otherwise belong to the poller, the monitor, or the hide call itself.

### Taking it down again

The same flush delay applies to `ProjectionRewardChoice.lua: Relic reward screen shut down`, so
hiding on that line left the overlay up for seconds after the screen it describes had gone. The
poller keeps looking after it has found the cards and reports the screen disappearing instead,
which is the same signal the show path uses. Two consecutive failed reads are required, because a
card reads blank often enough mid-screen that one miss is not evidence.

The screen's life is deterministic once the cards render -- `ProjectionsCountdown.lua: Initialize
timer nil 15` to `Countdown timer expired` was exactly 15.000s in both captured runs -- but every
one of those lines is lagged, so the timer is useful for understanding and useless for triggering.

### Instrumentation

`append_debug_line` honours `TENNOSCOPE_DEBUG_LOG`. Tests set it to a scratch file, because fixture
output in the live log is indistinguishable from a real fissure -- 68 lines of it were briefly read
as evidence from a live run.

## Pricing

Ducats cannot rank relic rewards on their own — most commons are worth the same 15. Platinum comes
from warframe.market v2, `/v2/orders/item/{slug}`, quoting the lowest visible sell order from a
seller who is **in game**; offline sellers list prices nobody can trade at. Slugs are derived from
the reward name (lowercase, non-alphanumerics to underscore) and verified against the live API for
every reward observed. Untradeable items, Forma among them, have no entry and stay unpriced.

Ducats are not a tiebreak. Most commons share the same 15, so platinum is what separates them — but
a card worth almost nothing on the market can still be the right take for a player saving for Baro,
and the two orderings disagree often. Both winners are computed and shown separately; collapsing
ducats into a platinum tiebreak meant the ducat answer was only ever visible when the platinum
values happened to tie.

### Pricing before the screen, not during it

Pricing used to start when the cards were published, which is the worst possible moment: the screen
lives fifteen seconds, the player is deciding during them, and every card showed a dash until the
requests came back.

The relic pool is known far earlier. It is the same signal that arms the screen poller — relics are
logged when they load, 125s ahead of the screen in the replayed run. Every reward that pool can drop
is priced in that window, into a cache that outlives the mission, so the common case at publish time
is zero requests and no dashes. Only a reward the warm pass missed is fetched late, and that one is
fetched without pacing because the screen is already up.

The warm pass paces itself at 250ms per request; two dozen names is six seconds against a two-minute
budget, so there is no reason to arrive as a burst. Misses are not cached — an untradeable item and
an unreachable API are indistinguishable from here, and caching the second would leave a card
unpriced for the rest of the session.

## Labelled runs

Ground truth reported by the user, left to right. Account ids truncated to their last six
characters. Every run confirms slot 1 is the local player.

| mode | squad roster (log order) | local reward | left-to-right |
|---|---|---|---|
| host | 106ec2, **57b7f7**, 09203e, 036a61 | BroncoPrimeReceiver | Bronco Prime Receiver, Gyre Prime Blueprint, Paris Prime Lower Limb, Burston Prime Receiver |
| client | 106459, 08b53d, 40f2c4, **57b7f7** | PrimeLightningGunStock | Vadarya Prime Stock, Fang Prime Handle, Perigale Prime Blueprint, Xaku Prime Neuroptics Blueprint |
| client | 162a02, 0501a1, 000006, **57b7f7** | LexPrimeBlueprint | Lex Prime Blueprint, Forma Blueprint, Paris Prime Upper Limb, Cedo Prime Receiver |
| client | d2d281, 0f662c, 000000, **57b7f7** | TrumnaPrimeBlueprint | Trumna Prime Blueprint, Titania Prime Systems Blueprint, Lex Prime Receiver, Caliban Prime Blueprint |
| client | 0a4b30, cd9e53, 95a935, **57b7f7** | BratonPrimeBlueprint | Braton Prime Blueprint, 2 X Forma Blueprint, Burston Prime Stock, Trumna Prime Blueprint |

Internal names that do not follow the display name: Daikyu Prime String is `PrimeDaikyuString`,
Paris Prime Lower Limb is `PrimeBowLowerLimb`, Orthos Prime Blade is `PrimePolearmBlade`, Xaku Prime
Neuroptics is `XakuPrimeHelmet`, Fang Prime Handle is `PrimeFangHandle`, Vadarya Prime Stock is
`PrimeLightningGunStock`.

## Still open

- `mastery_relevant` on a reward card is always false. Doing it properly needs mastery tracking the
  app does not collect.

## Settled

- The chain published a correct overlay on a live reward screen on 2026-07-27, after the poller
  arming bug and the shared-scratch-file race were fixed.
- Reading memory inside the fifteen-second window gives the same answer as reading it late, so the
  attribution gap is real and not an artefact of the log delay.
- Hosting does not close that gap. Memory is off the live reward path; the screen is the only
  source for the three remote cards, and EE.log for the fourth.
- The card title crop was 10px too low and 18px too tall. Fixed, and pinned by a wrapped-title
  fixture that asserts scores, because a misaligned crop still returns the right name.
- Separating the title from the card art with `-normalize -threshold 74% -negate` takes all twelve
  labelled cards to an exact read. The earlier claim that thresholding made things worse was wrong;
  it was tried without `-normalize`, which is the step that makes one cutoff hold across card art.

