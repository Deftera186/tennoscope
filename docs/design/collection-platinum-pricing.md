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
| `lith_t11_relic` intact, all 4 online sellers listing 6-packs at 28–30p (4.67–5.00p/unit), `statistics_live` for that hour | min 28, max 30, median 29.5, volume 330 | warframe.market's statistics quote the lot, not the unit, so the dump cannot price a relic (measured 2026-07-30, `volume` matching `sum(quantity)` on single-seller cases confirms the same order set) |
| Relic subtypes with ≥3 online sellers, `median(lot) / median(unit)` | 16 of 31 unaffected, 13 at ≥1.5x, 5 at ≥4x, worst 6.0x | The inflation is heavy-tailed, not a constant that could be divided back out |
| Dump items with more than one `sell` record | 60, of which 39 are fish, 13 crafted blueprints, 8 riven veils | The extra records are `subtype`s; the cheapest is taken, because a fish's size is not in the inventory |
| Refinement tiers with an ingame seller, over 80 relics | intact 85%, radiant 39%, exceptional 0%, flawless 0% | A tier is priced from its own subtype where anyone is selling it, and from intact where nobody is |
| Radiant against intact where both are quotable | median 1.46x, worst 17x (Requiem I–IV, 1p intact against 17p radiant) | Pricing every tier at intact is not a rounding error |

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

Sixty items carry more than one `sell` record, one per `subtype`, and the cheapest is taken. Thirty-
nine of them are fish, whose subtype is a size the inventory does not record: a `Tromyzon` is a
`Tromyzon` whether it is `basic` at 2p or `magnificent` at 10p. Taking whichever record the file
listed first meant valuing an unknown at its best case; taking the lowest states the least the
player is certainly holding. The rest are relic tiers, which the dump does not price at all, and
riven veils and crafted blueprints, which no catalog name reaches.

Dumps are daily and lag: on 2026-07-29 the newest was dated the 27th. The fetcher asks for today's
file and walks backwards up to five days until one answers, recording the date it actually got. A
price two days old is the correct precision for a question about what a collection is worth.

## Name Resolution

The dump is keyed by warframe.market's English names and the collection by WFCD's. Three rules close
the gap, in order, with no network call between them:

1. The name as it stands.
2. The name with ` Blueprint` removed, reconciling `Forma Blueprint` with `Forma`.
3. A relic's refinement suffix -- `Intact`, `Exceptional`, `Flawless`, `Radiant` -- replaced by warframe.market's own name for that tier: `Axi A1 Intact` becomes `Axi A1 Relic`, and `Axi A1 Radiant` becomes `Axi A1 Relic (Radiant)`.

There is deliberately no mirror of rule 2 appending ` Blueprint`. It looks symmetrical and it is not:
the names it reaches are *built* equipment, and a built Warframe is not a thing anybody can sell.
Measured against a real 1,106-item collection it fired 25 times -- `Ash Prime` at its blueprint's
14p, `Octavia Prime` at 50p, `Banshee Prime` at 10p on an item the player does not own -- and was
wrong all 25 times, with no correct firing anywhere in the collection. Every prime part the player
can actually sell is in the dump under its own name and resolves by rule 1. Dropping the
rule took the collection from 266 priced items to 241, and all 25 lost were false prices.

Rule 3 recovers 772 relic names that no other rule reaches, but no longer prices them from the
dump. Sellers list a relic six at a time, and warframe.market's `statistics` endpoint -- which the
dump mirrors verbatim -- reports the price of the whole lot, so a six-pack enters the day's median
at six times what one relic costs. This is not something `relics.run` does to the data; the
statistics themselves carry no `perTrade`, and `/v2/orders` is the only place the divisor exists,
which is why the live path can correct for it and the dump cannot. A relic's dump key still
resolves, because the live path needs that name to build its
warframe.market slug, but the price comes only from a live check -- the startup sweep, or a page
refresh the player asked for -- persisted back into this same table so it survives a restart. Until
a relic has been checked it has no price rather than an inflated one.

The four refinement tiers are four prices, not one. They are separate `subtype`s of a single
warframe.market listing, quoted separately, and a radiant sells for a median 1.46x its intact tier
and as much as 17x it. So each tier resolves to its own name, is asked about with its own
`?subtype=` query, and is stored under that name. Where nobody is selling the refined tier the
price falls back to the intact listing -- which is what the previous behaviour did for every tier
unconditionally, so coverage cannot regress. It is a real fallback and not a formality: over 80
relics, 85% of intact tiers had an ingame seller against 39% of radiants and none at all of
`exceptional` or `flawless`. The fallback is silent on the card; a borrowed intact price reads as
checked live, because it was.

An unresolved name means no price. It is not an error, and it is not evidence that the item is
untradeable -- the remaining gap is largely non-prime weapon components, which the catalog does not
index today. Extending that index is a separate change.

These rules do double duty. What they return is warframe.market's own English name for the item,
which is exactly what a live lookup needs in order to build a slug and, for a relic, the subtype
beside it: `Axi A1 Radiant` resolves to `Axi A1 Relic (Radiant)` and from there to `axi_a1_relic`
plus `subtype=radiant`, which no derivation from the catalog name would have produced. The dump is
therefore both the price source and the identity map, which is the second
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

That attempt usually returns the file we already had, so the refreshed table adopts the cached one's
checked prices whenever the two dates match. Without that, the ordinary launch would overwrite the
cache with a table whose checked prices are empty and then re-spend 22 seconds learning the same
numbers -- every launch, on the days the lag makes ordinary.

The response is capped, as every remote read in this codebase is, because the body is streamed into
memory and an uncapped `read_to_end` against an untrusted host is an out-of-memory waiting for a bad
day. The measured file is 3.9 MB and the cap is 32 MB.

## Live Prices For Live Decisions

The dump is a day old by construction, and there is a moment where that is the wrong number: when
the player has stopped valuing the collection and started trading out of it. The week an item is
unvaulted its price moves faster than a daily file can follow, and that is precisely the week
somebody goes looking.

So the dump seeds everything and warframe.market answers for the rest. Two triggers, both bounded:

- The startup relic sweep. Relics have no usable dump price at all -- the median is inflated by bulk listings the dump gives no way to divide out -- so the relics the player *owns* are priced live once, at start, and the results are written back into the price table. It sweeps each owned refinement tier and the intact listing that tier falls back to, so a collection of radiants costs two requests per relic rather than one; the intact name is shared by every tier of the same relic and folds away in the dedup. Bounded by ownership rather than by the catalog: 65 relics in the measured collection against the 772 the dump lists, which is 22 seconds at three requests a second rather than four minutes. It re-runs after an inventory refresh, because a refresh is the only thing that can add a relic, including the first snapshot a fresh install ever takes.
- The page refresh. It prices what is on screen, at most forty-eight items, paced at the same three requests a second, so a full page takes about sixteen seconds. It offers every item the player owns rather than only the ones already priced: an item with no price is exactly the one somebody would want to ask about, and unresolvable names are dropped by the backend before any request is made. Its results are written into the price table exactly as the sweep's are, for the reason below.

Neither is a sweep of the collection. Nothing is fetched for an item the player does not own, and
nothing outside the relic sweep is fetched without a click, which is what keeps a feature that could
have cost a thousand requests down to a bounded startup pass and the page somebody is looking at.

A quoted price is per unit, derived from the warframe.market order's `platinum` field divided by
its `perTrade` count, floored at 1p. Most sell orders are for a single item, but relic sellers
routinely list six at a time; quoting a six-pack's total as a single item's price would rank two
different quantities as though they were the same thing, and a 1p-for-six listing that divided to
zero would render as "0p", which reads as free rather than as cheap.

Live prices land in the cache the overlay already keeps, keyed by name with a fifteen-minute life,
so a relic pool warmed during a mission also prices those items in the collection, and an item
priced in the collection is already warm if it appears on a reward screen. One live cache, two
readers. The paced refresh is the `warm` function that cache already has.

The three requests a second are the *client's* budget, not each caller's, so the pacing lives in
the cache rather than in any caller. Four call paths share it -- the pool warm, the startup relic
sweep, the page refresh and the reward screen's fill -- and any two can overlap; each politely
waiting 334ms of its own would still have put six or nine requests a second on the API. Every
request claims a slot from one shared clock before it leaves, so a caller in a hurry (the reward
fill, which has fifteen seconds of screen and skips its own extra delay) can spend the budget sooner
but cannot exceed it. One relic sweep runs at a time for the same reason: two overlapping sweeps
would spend the same requests twice.

A checked price outlives that cache. Whichever trigger obtained it, it is written into the price
table beside the dump's own prices, because a fifteen-minute cache would mean a relic showing a dash
for most of a session, because re-spending 22 seconds of requests to learn a number we already had
is the behaviour the API rules ask clients not to have, and because a price the player deliberately
asked for is the best number the app has about that item -- letting it expire back to a day-old
figure discards a request they spent. The table therefore holds two maps: the dump's medians, and
whatever warframe.market has answered directly since. The second wins wherever both exist. That
precedence is invisible while only relics are checked, since the dump prices no relic at all, and
load-bearing the moment a prime part is: consulting the dump first would shadow the live number
with the one it was fetched to replace.

A checked price lives exactly as long as the dump it was checked against: a refresh that brings back
the same dump -- which is the ordinary case, since the dumps lag two days -- carries the checked
prices across, and a genuinely newer dump clears them, after which the sweep runs again and the page
offers to re-price what is on screen. That single rule is the whole freshness policy and the only
bound on how stale a stored checked price can get; a second date gate alongside it would only be a
second thing to keep in step.

"Nobody is selling this" travels the same road, because it is an answer and not a failed request.
The table records it beside the prices and the sweep skips it on the same terms, which is what makes
the sweep terminate. Filtering on "has a price" instead meant every relic with an empty order book
failed the test again on the next inventory sync, and the one after -- the same requests, the same
answer, all evening, for a set of relics a real collection is never short of. An *unreachable*
endpoint is deliberately not recorded: an outage is a reason to try again, and treating it as an
answer would blacklist a relic until tomorrow's dump over a router that rebooted mid-sweep.

Three callers write that table now -- the relic sweep, the page refresh, and the daily dump
download that replaces it -- so the read-modify-write that folds prices into it happens under the
runtime lock rather than beside it, against whatever the runtime is serving at that moment rather
than against a copy taken earlier. The network work stays outside: each caller does its fetching
first, paced at the shared floor where it makes per-item requests, and takes the lock only for a
fold and a file write. The dump download is the one that makes this load-bearing rather than
tidy -- it takes seconds, and a page refresh completing inside that window would otherwise be
overwritten by a table read before it started.

The valuation itself fetches nothing of its own. The value sort and the collection worth need every
item priced to mean anything, and live-pricing hundreds of items to compute a total is exactly the
behavior the API rules ask clients not to attempt. They read the best price already in hand: live
where something has been checked, the dump everywhere else.

That was a decision between two defensible answers. Dump-only would make the total one consistent
measurement, comparable with itself hour to hour. Best-available makes it more accurate -- every
item that has been priced live is a better number than the dump's, and refusing to use it would
mean showing a total the app knows to be stale. The cost is a figure that moves as prices land,
which is the honest behaviour of a valuation that is being improved in front of you. The card still
says which measurement each price is, so the mix is visible rather than silent.

The reward overlay is untouched. Its question was always "what is the cheapest online seller asking
right now", answered live, and it stays that way.

## Two Numbers, Never Silently Mixed

A checked price and a dump price are different measurements: the cheapest online seller against the
middle of a day's listings. Showing 19p beside 20p with nothing to say which is which invites the
reader to compare two numbers that were never comparable.

So every price carries its provenance. The register states which day's dump the collection is
priced from, and a card whose number came from a live check -- the page refresh, a warmed relic
pool, or the startup sweep persisted into the table -- says it was checked live. The distinction is
on the card, not in a tooltip, because the whole reason the live path exists is that the difference
matters.

"Checked live" covers both the fifteen-minute cache and a persisted checked price, deliberately.
Those are one measurement made at two different times, and the alternative -- an item that reads as
live for fifteen minutes and then quietly reverts to a dump price, or to a dash where the dump
prices nothing -- would attribute it to a file it did not come from.

## Application View

`CollectionItemView` gains `platinum: Option<u32>`, `live: bool` and `priceable: bool`. A separate
`tradeable` flag would be a second name for "has a price", since those are the same fact here; the
`Tradeable` filter reads the price field directly. The other two are not that. `live` says which of
two different measurements the number is. `priceable` says whether warframe.market can be asked
about the item at all -- the same question `market_names_for` answers when it drops every name the
price table cannot resolve -- and it is what the page control counts. It is deliberately not "has a
price": an unswept relic is priceable, unpriced, and exactly the item somebody clicks that control
for.

`CollectionView` gains `pricing: Option<PricingProgress>`: how far along the live pass in flight is,
or nothing when none is. One cell for both passes, because the requests come out of one
three-per-second budget and two counters would describe one queue twice.

`AppCore` holds the price table and the live cache, both cheap `Arc` clones, and `current_view()`
reads the live cache first and the table second. `live` is true for a price from that cache and for
any price persisted into the table's checked map -- swept relic or refreshed prime part alike --
because both were checked against warframe.market; it is false only for a number the dump supplied.

The frontend already polls the view every 2.5 seconds, so both the dump loading and a live refresh
landing appear on their own through plumbing that already exists. That poll must keep running while
a page refresh is in flight -- it is the only thing that makes sixteen seconds of pricing visible as
prices arriving rather than as a frozen button -- so the live refresh is deliberately not treated as
a foreground operation. Ordering is still safe: a view is applied only while its request is the
newest one started, so an older response can never land on top of a newer one.

## Presentation

A priced card shows `20p`, and `20p · 140p total` once more than one is owned. Both numbers earn
their place: the unit price is what gets quoted in trade chat, the stack total is what decides
whether the pile is worth clearing.

An item at quantity 0 is mastered, not owned, and carries no price at all. Pricing something the
player does not have inflates the collection's worth with platinum nobody could realise.

Unpriced and untradeable are not distinguished, because with a single dump this design genuinely
cannot tell them apart -- an item absent from the file may be untradeable or may be one the name
rules failed to reach. Claiming to know which would be a guess dressed as a fact. Unpriced items say
only that there is no price, and the collection price row in Diagnostics carries the dump's date so
a collection full of dashes is legible as a stale or failed download rather than as a worthless
account. That row is separate from the overlay's market row on purpose: one answers "which day are
these prices from", the other "could we reach warframe.market a moment ago", and while they shared a
row whichever wrote last erased the other's answer.

Three affordances follow the prices. A `Value` sort orders by unit price with unpriced entries
sinking to the bottom: stack value answers "where is my platinum", unit price answers "what is worth
the most", and the sort is for the second question while the card still shows the first. A
`Tradeable` filter narrows to items that have a price. A fourth cell in the assay band sums price
times quantity across priced items and carries the count it was computed from, because a partial sum
presented as a total is a lie the reader cannot detect. All three read the best price in hand for
each item.

One control invokes the live path: the register's refresh, which names its scope and how many items
it will price -- everything on the page the backend can actually ask about, whether or not it has a
number yet. It counts `priceable` rather than everything owned, because counting items the backend
drops before it makes a request promised prices that were never coming. It sits at the end of the
register bar, after the provenance line and the range readout: those two are one statement about
what is on screen, and an action set between them broke a line meant to read as one. There is no
per-item control. It was one click for one request, in a register where the row-level answer is the
same request; the page control subsumes it and one affordance is easier to understand than two.

A pass in flight is visible, because sixteen or twenty-two seconds of silence reads as a broken
button -- and because the background sweep, which nobody clicked, moves the collection's worth the
whole time it runs. Three things say so, and they say it once each. The register bar's own bottom
rule fills with platinum as the pass advances: an engraved hairline is already this interface's
device for dividing the sheet, so a reading struck into one needs no progress bar, no spinner and no
new component. Beside it, in the same voice as the provenance line, sits the count. And the worth
cell's note carries it too, because that is the figure that moves, and a total climbing with nothing
to account for it is a moving target rather than a valuation.

The count is the backend's, not the page's. Reconstructing it in the client -- which of the
requested ids have gone live since the click -- could only ever describe the pass the client
started, and the sweep is the one that runs for twenty-two seconds unasked. The control itself
carries no number while a pass runs: it is disabled, because both passes spend the same
rate-limited budget, and a second copy of the same figures on the disabled thing reads as a
different pass.

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
rather than from the catalog's name, for all four refinement suffixes; the dump parser against a
trimmed fixture, including an item whose `sell` record is absent and one whose body is one byte over
the cap; date walk-back, proving a missing file for today falls through to an older one and records
the date it used; a cached dump dated today or yesterday not being downloaded again while an older
one is; a refresh of the same dump keeping the prices checked against it -- relic and dump-priced
item alike -- and a newer dump discarding them, the dump-priced one falling back to the new dump
rather than to the stale number; a cache written under the map's former name still carrying its
prices; the cache round-tripping through disk and pricing a collection before any network call;
malformed input rejected whole; a checked price taking precedence over the dump's for the same item
and being marked as such; a persisted checked price still reading as checked live once the
fifteen-minute cache has dropped it, for a relic and for an item the dump prices; an item at quantity 0 carrying no price; the sweep bounded to owned relics and collapsing
refinement tiers; two concurrent warms unable to put requests closer together than the shared floor;
and each live-lookup outcome distinctly, including oversize reaching the market health row, a
part-finished pass reporting itself, and a bulk listing too cheap to divide still quoting 1p. The
sweep's termination has its own set: a no-seller answer reading as checked but not as priced and not
counting toward what the table can price, a later real price replacing it and a later empty book not
undoing one, that answer surviving a same-dump refresh and dying to a newer one, and -- the one that
guards against blacklisting a relic over an outage -- a per-name pass keeping `NoSellers` and
`Unavailable` distinct.
Frontend tests cover the value sort by unit price with unpriced and zero-priced entries present, the
worth arithmetic and its count, the tradeable filter, the dump's date on the register, that a
checked price is visibly distinguished from a dump price, that the refresh control offers every
priceable item on the page including ones with no price yet and neither an unowned one nor one no
name rule reaches, that a pass reported by the backend appears on the register and in the worth
cell's note, that the control is refused while a background pass is spending requests, and that the
view poll keeps running through a live page refresh.

## Out of Scope

Price history and trends, buy-order prices, sell recommendations, per-item manual refresh, relic
refinement pricing, and extending the catalog index to non-prime weapon components.
