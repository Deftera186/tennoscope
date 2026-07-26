# Relic reward resolution

How TennoScope determines the four relic reward cards and their order, and the evidence behind each
decision. Written 2026-07-26 from five labelled live runs.

## What each source can answer

| question | source | reliability |
|---|---|---|
| screen order of the four cards | EE.log squad ring, rotated so the local player is first | confirmed in all five labelled runs |
| the local player's own reward | `VoidProjections: <id> gets reward <path>` | exact, always present |
| the other three rewards, **hosting** | per-player response records in memory | record layout pinned by fixture |
| the other three rewards, **as a client** | reading the reward screen | no memory route exists |

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

## Why memory cannot answer this as a client

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

## Reading the screen

Warframe runs under Proton as an XWayland client, so `import -window` reaches it with no compositor
portal. Capture writes PPM rather than PNG: the frame is discarded after four crops, and
PNG-encoding 1920x1080 costs 1.9s against 0.04s.

This is not general OCR. EE.log names the squad's relics before the screen renders, so each card is
matched to the nearest of roughly two dozen known rewards by normalised edit distance. tesseract
reads the UI font cleanly from a plain greyscale crop — thresholding made it worse. The divider
below each title bleeds into the crop and garbles the last character or two, which the closed-set
match absorbs.

Card geometry, as fractions of the window so it carries across resolutions: cards on a 242/1920
pitch from 478/1920, titles at 430/1080, height 48/1080.

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

- The assembled chain has not run against a live reward screen. Every component is pinned by
  captured evidence; the composition is not.
- `mastery_relevant` on a reward card is always false. Doing it properly needs mastery tracking the
  app does not collect.
