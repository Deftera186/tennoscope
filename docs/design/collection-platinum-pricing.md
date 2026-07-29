# Collection Platinum Pricing Design

## Goal

Put a platinum price on every tradeable item in the collection, so the question "what is this pile
worth, and what should I sell" is answered in the browser rather than in a second window against a
website.

The reward overlay already prices four cards live against warframe.market. The collection is a
different problem wearing similar clothes: hundreds of items rather than four, and a valuation
rather than a decision made inside fifteen seconds. It is seeded from a different source, and asks
warframe.market only about the items the player actually acts on.

## Evidence

Measured on 2026-07-29, and the source of the constants and rules below.

| Measurement | Value | Consequence |
| --- | --- | --- |
| `relics.run/history/price_history_<date>.json` | 3,835 items, 3.9 MB, one request | The entire collection is priced by a single daily download |
| `sell` statistics present | 3,831 of 3,835 items | The field the collection reads |
| `closed` statistics present | 2,489 of 3,835 items | Rejected as primary: a third of the collection would show nothing |
| Newest dump on 2026-07-29 | `price_history_2026-07-27.json` | The fetcher walks back from today; the data is a day or two old by construction |
| Three name rules, no manifest | 2,940 of 3,835 dump items reachable from the 15,972 catalog names | Name matching alone is enough |
| A fourth rule appending ` Blueprint` | 25 firings against a real 1,106-item collection, 25 of them wrong, 0 right | Rejected: it prices built equipment at its blueprint's listing |
| Prime parts in that collection | 75 of 75 resolved by rule 1 | The rejected rule had nothing left to reach |
| `/v2/items` manifest on top | 257 more, 228 of them sets | Rejected: its entire contribution is items that must not be priced |
| Mirage Prime Systems, dump median sell | 20p, against 19p live from an online seller | The daily median tracks the live number closely |
| Mirage Prime Systems, dump minimum sell | 10p | Rejected: the day's cheapest listing includes offline lowballers |
| Relic sell orders with `perTrade > 1` | 5–29% of orders, typically `perTrade: 6` | Prices are per unit after dividing by trade size |

The rejected manifest is the load-bearing negative result. Its unique contribution is warframe.market
*sets* -- `Vectis Prime Set`, `Xiphos Set` -- which exist as market listings but never as inventory
items. A player owns the parts, or owns the built weapon, and neither is a sellable set. Pricing a
built and mastered Vectis Prime at 144p states something false about what can be traded, so the
257 items the manifest would add are the 257 items most worth leaving alone. Name matching declines
them naturally, because no dump key is a bare `Vectis Prime`.

## Price Source

One request per day to `https://relics.run/history/price_history_<date>.json`, the warframe.market
price history dump, named to us by the warframe.market team.

The file is keyed by English item name and holds, per item, a record per order type. The collection
reads the `sell` record's `median`: the middle of that day's sell listings. `min_price` is the day's
cheapest listing and includes sellers who are offline, which is how the same item reads 10p on a
number nobody could trade at and 19p on one they could. `closed` prices describe completed trades
and would be the most honest number of all if they existed, but they cover only 2,489 items, and a
collection where a third of the entries have no price is not a valuation.

Dumps are daily and lag: on 2026-07-29 the newest was dated the 27th. The fetcher asks for today's
file and walks backwards up to five days until one answers, recording the date it actually got. A
price two days old is the correct precision for a question about what a collection is worth.

## Name Resolution

The dump is keyed by warframe.market's English names and the collection by WFCD's. Three rules close
the gap, in order, with no network call between them:

1. The name as it stands.
2. The name with ` Blueprint` removed, reconciling `Forma Blueprint` with `Forma`.
3. A relic's refinement suffix -- `Intact`, `Exceptional`, `Flawless`, `Radiant` -- replaced by ` Relic`, reconciling `Axi A1 Radiant` with `Axi A1 Relic`.

There is deliberately no mirror of rule 2 appending ` Blueprint`. It looks symmetrical and it is not:
the names it reaches are *built* equipment, and a built Warframe is not a thing anybody can sell.
Measured against a real 1,106-item collection it fired 25 times -- `Ash Prime` at its blueprint's
14p, `Octavia Prime` at 50p, `Banshee Prime` at 10p on an item the player does not own -- and was
wrong all 25 times, with no correct firing anywhere in the collection. Every prime part the player
can actually sell is in the dump under its own name and resolves by rule 1. Dropping the
rule took the collection from 266 priced items to 241, and all 25 lost were false prices.

Rule 3 recovers 772 relics that no other rule reaches, at the cost of one known imprecision: the
dump prices a relic without regard to refinement, so all four tiers of `Axi A1` read the same. A
radiant relic is worth more than an intact one and this design does not say so. That is a real
loss, accepted because the alternative is pricing no relics at all.

An unresolved name means no price. It is not an error, and it is not evidence that the item is
untradeable -- the remaining gap is largely non-prime weapon components, which the catalog does not
index today. Extending that index is a separate change.

These rules do double duty. What they return is warframe.market's own English name for the item,
which is exactly what a live lookup needs in order to build a slug: `Axi A1 Radiant` resolves to
`Axi A1 Relic` and from there to `axi_a1_relic`, which no derivation from the catalog name would
have produced. The dump is therefore both the price source and the identity map, which is the second
reason the `/v2/items` manifest is not needed.

## Cache And Refresh

Following the pattern `catalog_cache.rs` established: atomic writes, serde, a file under the
application data directory.

What is cached is the resolved map, not the download. The 3.9 MB dump reduces to a few thousand
name-and-price pairs, so the cached file is small, loads instantly at startup, and prices the
collection before any network call is attempted. The dump date is stored with it, and when a fetch
fails the cached prices stay and are described by their date rather than discarded.

The stored date is also what decides whether to fetch at all. The dumps lag -- on 2026-07-29 the
newest published was dated the 27th -- so "the cache is not dated today" is no evidence a newer file
exists, and a cached table dated today or yesterday is left alone. Anything older costs one attempt
per launch, which is the price of not remembering when we last asked.

The response is capped, as every remote read in this codebase is, because the body is streamed into
memory and an uncapped `read_to_end` against an untrusted host is an out-of-memory waiting for a bad
day. The measured file is 3.9 MB and the cap is 32 MB.

## Live Prices For Live Decisions

The dump is a day old by construction, and there is a moment where that is the wrong number: when
the player has stopped valuing the collection and started trading out of it. The week an item is
unvaulted its price moves faster than a daily file can follow, and that is precisely the week
somebody goes looking.

So the dump seeds everything and warframe.market answers for whatever the player is actually acting
on. Two triggers, both deliberate:

- Selecting a single item prices that one item live.
- Refreshing the current page prices the items on screen, at most forty-eight, paced at three requests a second, which takes about sixteen seconds.

Neither is a background sweep. Nothing is fetched that the player did not ask about, which is what
keeps a feature that could have cost hundreds of requests down to the handful somebody deliberately
clicked.

A quoted price is per unit, derived from the warframe.market order's `platinum` field divided by
its `perTrade` count. Most sell orders are for a single item, but relic sellers routinely list six
at a time; quoting a six-pack's total as a single item's price would rank two different quantities
as though they were the same thing.

Live prices land in the cache the overlay already keeps, keyed by name with a fifteen-minute life,
so a relic pool warmed during a mission also prices those items in the collection, and an item
priced in the collection is already warm if it appears on a reward screen. One live cache, two
readers. The paced refresh is the `warm` function that cache already has.

The three requests a second are the *client's* budget, not each caller's, so the pacing lives in
the cache rather than in any caller. Three call paths share it -- the pool warm, the page refresh
and the reward screen's fill -- and any two can overlap; each politely waiting 334ms of its own
would still have put six or nine requests a second on the API. Every request claims a slot from one
shared clock before it leaves, so a caller in a hurry (the reward fill, which has fifteen seconds of
screen and skips its own extra delay) can spend the budget sooner but cannot exceed it.

Nothing is *fetched* for the valuation. The value sort and the collection worth need every item
priced to mean anything, and live-pricing hundreds of items to compute a total is exactly the
behavior the API rules ask clients not to attempt. But they read the best price already in hand:
live where the player has priced something, the dump everywhere else.

That was a decision between two defensible answers. Dump-only would make the total one consistent
measurement, comparable with itself hour to hour. Best-available makes it more accurate -- every
item the player deliberately priced is a better number than the dump's, and refusing to use it
would mean showing a total the app knows to be stale. The cost is a figure that moves as the player
prices things, which is the honest behaviour of a valuation that is being improved in front of you.
The card still says which measurement each price is, so the mix is visible rather than silent.

The reward overlay is untouched. Its question was always "what is the cheapest online seller asking
right now", answered live, and it stays that way.

## Two Numbers, Never Silently Mixed

A live price and a dump price are different measurements: the cheapest online seller right now
against the middle of yesterday's listings. Showing 19p beside 20p with nothing to say which is
which invites the reader to compare two numbers that were never comparable.

So every price carries its provenance. Dump prices are attributed to the dump and its date; a price
fetched live is marked as live. The distinction is visible on the card, not buried in a tooltip,
because the whole reason the live path exists is that the difference matters.

## Application View

`CollectionItemView` gains `platinum: Option<u32>` and `live: bool`. A separate `tradeable` flag
would be a second name for "has a price", since those are the same fact here; the `Tradeable` filter
reads the price field directly. `live` is not that -- it says which of two different measurements
the number is, and nothing else in the view carries it.

`AppCore` holds the dump's price table and the live cache, both cheap `Arc` clones, and
`current_view()` reads the live cache first and the table second. A live price that has aged past
the cache's fifteen minutes stops being live and the item falls back to the dump, so a price never
claims a freshness it has lost.

The frontend already polls the view every 2.5 seconds, so both the dump loading and a live refresh
landing appear on their own through plumbing that already exists. That poll must keep running while
a page refresh is in flight -- it is the only thing that makes sixteen seconds of pricing visible as
prices arriving rather than as a frozen button -- so the live refresh is deliberately not treated as
a foreground operation. Ordering is still safe: a view is applied only while its request is the
newest one started, so an older response can never land on top of a newer one.

## Presentation

A priced card shows `20p`, and `20p · 60p total` once more than one is owned. Both numbers earn
their place: the unit price is what gets quoted in trade chat, the stack total is what decides
whether the pile is worth clearing.

Unpriced and untradeable are not distinguished, because with a single dump this design genuinely
cannot tell them apart -- an item absent from the file may be untradeable or may be one the name
rules failed to reach. Claiming to know which would be a guess dressed as a fact. Unpriced items say
only that there is no price, and the collection price row in Diagnostics carries the dump's date so
a collection full of dashes is legible as a stale or failed download rather than as a worthless
account. That row is separate from the overlay's market row on purpose: one answers "which day are
these prices from", the other "could we reach warframe.market a moment ago", and while they shared a
row whichever wrote last erased the other's answer.

Three affordances follow the prices. A `Value` sort orders by stack value with unpriced entries
sinking to the bottom. A `Tradeable` filter narrows to items that have a price. A fourth cell in the
assay band sums price times quantity across priced items and carries the count it was computed from,
because a partial sum presented as a total is a lie the reader cannot detect. All three read the
best price in hand for each item, live where there is one and the dump otherwise.

Two more invoke the live path. A card is selectable, and selecting it prices that item live. The
register carries a refresh control that prices the page on screen, showing its progress, because
sixteen seconds of silence reads as a broken button. A live price is marked as live on the card;
everything else is understood to come from the dump, whose date the collection price row carries.

The interface work is done through the `impeccable` skill.

## Boundaries

- `warframe-acquisition` owns the download, the name rules, the cache and the price table. It knows nothing about collections or presentation.
- `app-core` joins prices onto collection items and publishes them in the immutable view.
- The Tauri shell owns when the refresh runs.
- React owns the price line, the sort, the filter and the worth cell, and formats nothing it was not given.

## Failure Handling

An unreachable dump leaves the cached prices in place, dated. A first run with no cache and no
network shows no prices and a collection price row that says why. A malformed dump is rejected whole
rather than partially applied, so a truncated download cannot silently halve the collection's worth.

A live refresh that fails changes nothing: the item keeps the dump's price and its dump attribution.
Failing back to a dash would be worse than the day-old number it replaced, and would punish the
player for asking.

A response over the size cap is reported as its own outcome rather than as an absent price, and the
outcome is counted and returned to whoever asked for the prices, because an outcome nothing reads is
the same as no outcome. The live lookup's `None` conflated four different facts -- priced, no online
seller, endpoint unreachable, and response over the cap. The last is the dangerous member: it is the
failure that arrives the day warframe.market widens its payload, it stops every price at once, and
as an `Option` it presents as "every item is worthless" with nothing anywhere saying otherwise. It
now reaches the market health row by name, ahead of the quieter failures.

No pricing failure can affect inventory synchronization.

## Verification

Automated tests cover each name rule and a name no rule reaches; that built equipment does *not*
borrow its blueprint's price, which is the rule this design rejected and the one somebody will
helpfully re-add; the market name a rule resolves to, since the live path builds its slug from that
rather than from the catalog's name; the dump parser against a trimmed fixture, including an item
whose `sell` record is absent and one whose body is one byte over the cap; date walk-back, proving a
missing file for today falls through to an older one and records the date it used; a cached dump
dated today or yesterday not being downloaded again while an older one is; the cache round-tripping
through disk and pricing a collection before any network call; malformed input rejected whole; a
live price taking precedence over the dump's and being marked as such; a live price that has aged
out falling back to the dump rather than lingering as live; two concurrent warms unable to put
requests closer together than the shared floor; and each live-lookup outcome distinctly, including
oversize reaching the market health row. Frontend tests cover the value sort with unpriced entries
present, the worth arithmetic and its count, the tradeable filter, that a live price is visibly
distinguished from a dump price, and that the view poll keeps running through a live page refresh.

## Out of Scope

Price history and trends, buy-order prices, sell recommendations, per-item manual refresh, relic
refinement pricing, and extending the catalog index to non-prime weapon components.
