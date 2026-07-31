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
| `closed` statistics present | 2,433 of 3,836 items | The preferred measurement, with `sell` as the fallback: coverage stops mattering once it falls back |
| `sell`/`closed` median ratio, matched on `(subtype, rank)` | median 1.17x, 144 pairs at ≥2x, 24 at ≥4x | The bulk-lot inflation is visible in the file itself, and only in `sell` |
| `closed` records quoting >1.5x their own ask | 15 of 3,179 pairs; 1 survives a `volume >= 3` floor | The floor is necessary and not sufficient. `Vitality` unranked: 115p closed on 4 trades against a 1p ask on 3,186 listings. Taking the lower of the two removes all 15 |
| Relics with a usable `closed` record | 163 of 772 on one day, 425 across four | One day prices 45% of a real collection's relics; 28 days unioned price 96% |
| A real collection's relic holding | 71 rows, 202 copies, 391p | 1.4% of the collection, against ~70 requests and 22s per launch to sweep it |
| Newest dump on 2026-07-29 | `price_history_2026-07-27.json` | The fetcher walks back from today; the data is a day or two old by construction |
| Three name rules, no manifest | 2,940 of 3,835 dump items reachable from the 15,972 catalog names | Name matching alone is enough |
| A fourth rule appending ` Blueprint` | 25 firings against a real 1,106-item collection, 25 of them wrong, 0 right | Rejected: it prices built equipment at its blueprint's listing |
| Prime parts in that collection | 75 of 75 resolved by rule 1 | The rejected rule had nothing left to reach |
| `/v2/items` manifest on top | 257 more, 228 of them sets | Rejected: its entire contribution is items that must not be priced |
| Mirage Prime Systems, dump median sell | 20p, against 19p live from an online seller | The daily median tracks the live number closely |
| Mirage Prime Systems, dump minimum sell | 10p | Rejected: the day's cheapest listing includes offline lowballers |
| Relic sell orders with `perTrade > 1` | 5–29% of orders, typically `perTrade: 6` | Prices are per unit after dividing by trade size |
| `lith_t11_relic` intact, all 4 online sellers listing 6-packs at 28–30p (4.67–5.00p/unit), `statistics_live` for that hour | min 28, max 30, median 29.5, volume 330 | `statistics_live` quotes the lot, not the unit (measured 2026-07-30, `volume` matching `sum(quantity)` on single-seller cases confirms the same order set) |
| The same relic's `statistics_closed` for that day | intact median 4.5, radiant median 6.0 | `statistics_closed` is per unit, and lands inside the measured 4.67–5.00p/unit band. The bulk-lot fault is in `statistics_live` alone |
| Relic subtypes with ≥3 online sellers, `median(lot) / median(unit)` | 16 of 31 unaffected, 13 at ≥1.5x, 5 at ≥4x, worst 6.0x | The inflation is heavy-tailed, not a constant that could be divided back out |
| Dump items with more than one `sell` record | 60, of which 39 are fish, 13 crafted blueprints, 8 riven veils | The extra records are `subtype`s; the cheapest is taken, because a fish's size is not in the inventory |
| Refinement tiers with an ingame seller, over 80 relics | intact 85%, radiant 39%, exceptional 0%, flawless 0% | A tier is priced from its own subtype where anyone is selling it, and from intact where nobody is |
| Radiant against intact where both are quotable | median 1.46x, worst 17x (Requiem I–IV, 1p intact against 17p radiant) | Pricing every tier at intact is not a rounding error |
| Listings quoted at one rank above 0 and no other | 7 of 3,835, all 80–300p (`Scan Matter` 240p, `Sword Alone` 300p) | An unranked copy has no price from such a listing rather than the maxed one, which valued a 0/3 `Scan Matter` at 240p |
| A real collection's priced holding | 1,215 rows, 7,982 copies, 30,817p | The total is dominated by bulk commons, not by anything concentrated: the largest single stack is 364p |
| The same collection capped at a month of completed trades | 24,086p, 89 rows capped | What the market will actually take is 22% under the market rate |
| That capped total by unit price | 14,936p of it at 1–5p, 6,708p at 11p+ | The cap does not answer "this is too high" on its own: the cheap end is cheap *and* genuinely traded. A price floor at 6p would take it to 9,150p and delete `Intruder` ×104 at a true 3p |
| `closed` volume for one item on one day against 28 | `Intruder` 0 on the 30th, 159 across the month; `Quickdraw` 2 across the month | A day's dump is a sparse sample of appetite. Reading one day understated the sellable total by a quarter and moved it ±11%; carrying the last day that *saw* a trade overstated `Quickdraw` at 30/month against a true 2 |

The rejected manifest is the load-bearing negative result. Its unique contribution is warframe.market
*sets* -- `Vectis Prime Set`, `Xiphos Set` -- which exist as market listings but never as inventory
items. A player owns the parts, or owns the built weapon, and neither is a sellable set. Pricing a
built and mastered Vectis Prime at 144p states something false about what can be traded, so the
257 items the manifest would add are the 257 items most worth leaving alone. Name matching declines
them naturally, because no dump key is a bare `Vectis Prime`.

## Price Source

One request per day to `https://relics.run/history/price_history_<date>.json`, the warframe.market
price history dump, named to us by the warframe.market team.

The file is keyed by English item name and holds, per item, a record per order type and per
`subtype` and rank. The collection reads the `closed` record's `median` -- the middle of that day's
*completed trades* -- and falls back to the `sell` record's, the middle of what sellers were asking,
wherever no trustworthy closed record exists, and where both exist takes the lower of them.
`min_price` is used by neither: it is the day's
cheapest listing and includes sellers who are offline, which is how the same item reads 10p on a
number nobody could trade at and 19p on one they could.

Closed is preferred because it is the only per-unit measurement in the file. warframe.market's
`statistics_live` -- the source of the `sell` and `buy` records -- quotes a bulk listing's whole lot,
so a six-pack enters the day's median at six times what one item costs. `statistics_closed` is not
affected, and it is the more-used endpoint on the website for exactly that reason. Measured
2026-07-30 on `lith_t11_relic` intact: 30p asked against 4.5p traded, where the four online sellers'
per-unit asks were 4.67-5.00p. The same correction lands on everything else sold in stacks --
`Star Crimzian` 5p to 1p, `Proof Fragment` 5p to 1p, gems, fish, imprints.

This was the design's largest error, and it was an error of framing rather than of measurement.
Closed was rejected here on the grounds that it covered only 2,489 of 3,835 items and "a collection
where a third of the entries have no price is not a valuation" -- which treats closed as a
*replacement* for the ask when it is a *preference* over it. With a fallback, coverage stops
mattering: 2,433 items take the traded price and the remaining 1,400-odd keep the ask they already
had. Nothing lost coverage; 1,442 of 3,059 non-relic items gained a truer one.

Neither number survives being trusted alone, which is why the price is the lower of the two rather
than closed outright. The ask reads high on anything sold in stacks. The trade reads high when it is
a thin sample: `Vitality` unranked closed at 115p on four trades against a 1p ask backed by 3,186
listings, and a `volume >= 3` floor -- which the evidence table originally claimed removed every
such case -- passes it. That claim came from reading a rounded `0.000` as zero when the true count
was 1 in 2,455. Taking the lower of the two removes all fifteen, in both directions, without a
tuned constant: a lot-inflated ask always loses to the trade, and a freak trade always loses to the
ask. The volume floor stays, because it is the only guard against a thin trade reading *low*, where
the ask cannot help.

Left uncaught, that one record cost 3,955p of a real 27,150p collection: `Vitality`'s rank-0 group
took the 115p trade, its rank-10 group fell back to a 35p ask, and the cross-group minimum then
priced 113 unranked copies at a rank nobody in the collection held.

Preferring closed does change every price, not only the bulk-listed ones. Where neither number is a
lot, closed is still a median 0.83x sell, because closed is what a buyer paid and sell is what a
seller wants. That is the right direction for this feature: the collection asks "what is this pile
worth", which is a question about what it would fetch, not about what could be asked for it. The
reward overlay is untouched and still asks the live order book what the cheapest seller wants right
now, because that is a different question.

Two guards make the preference safe, and both are load-bearing:

- **Volume.** A closed record standing on one or two trades is one player's odd deal, not a price: `Vitality` closed at 115p against a 1p ask on four trades, `Pressure Point` at 50p against 1p on one. Across the 2026-07-30 dump, every one of the 15 records quoting more than 1.5x its own ask sat at volume 4 or below; requiring three trades leaves none at all. Below the floor the record is ignored and the ask stands.
- **The rank and subtype are matched, never crossed.** Closed is preferred *within* one `(subtype, mod_rank)` group and falls back to that same group's ask. `Serration` on 2026-07-30 carries a closed record at rank 10 and none at rank 0; taking the cheapest closed record on the listing would price every unranked copy at the maxed 20p. That is precisely the fault `CACHE_SCHEMA` was bumped for the first time, and preferring closed is the change most likely to restore it.

Sixty items carry more than one `sell` record, one per `subtype`, and the cheapest is taken. Thirty-
nine of them are fish, whose subtype is a size the inventory does not record: a `Tromyzon` is a
`Tromyzon` whether it is `basic` at 2p or `magnificent` at 10p. Taking whichever record the file
listed first meant valuing an unknown at its best case; taking the lowest states the least the
player is certainly holding. The rest are relic tiers, riven veils and crafted blueprints.

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

Rule 3 recovers 772 relic names that no other rule reaches, and prices the ones that traded. A
relic is the worst case for the bulk-lot fault -- sellers list them six at a time, so the ask runs
at six times what one relic costs -- so a relic is priced from its `closed` records *only*, with no
fallback to the ask. There is no honest way to divide the ask back down: the inflation is
heavy-tailed rather than a constant, measured 16 of 31 relic subtypes unaffected against 5 at 4x or
worse.

That prices 139 relic names outright, and 163 of 772 carry a usable closed record on a given day --
45% of a real collection's relic rows. The dumps do not disagree about the rest; they are sparse,
and each day's file prices a different subset. So `PriceTable::adopt` carries a relic's dump price
forward into the tables that follow it, up to 30 days, and coverage measured against that same
collection goes 45% at one dump, 76% at three, 86% at seven, 96% at twenty-eight. Every one of those
days is a download the app already makes, so the union costs nothing.

**This is what removed the startup sweep.** The sweep spent about 70 requests and 22 seconds of
every launch to refine a holding worth 391p -- 1.4% of the collection -- and the union now answers
96% of it for free. The live path remains, but only where the player points it: the page refresh.
A relic no dump in the last month saw trade shows the honest dash it always did, and one click
prices it.

The 30-day window is what lets the register state its own provenance. Relics are cheap enough to
carry: a typical closed median is 4.2p and 90% are under 7p, so the median 20% drift across a month
is 0.8p on an item. Past a month the number stops being a stale reading and becomes a guess, and a
relic still uncarried by then is one nobody has traded in a month.

Only relics are carried. Everything else is priced from an ask that is quoted fresh in every file,
so carrying those would keep a month-old number alive that the very next download already answered.
A checked price still outranks both: a live order book beats a day-old trade, which beats a
month-old one.

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

That attempt usually returns the file we already had, so the refreshed table adopts from the cached
one: its carried relic prices always, and its checked prices when the two dates match. Without that,
the ordinary launch would overwrite the cache with a table that has neither -- discarding both a
month of accumulated relic coverage and every price the player spent a request on, every launch, on
the days the lag makes ordinary.

The response is capped, as every remote read in this codebase is, because the body is streamed into
memory and an uncapped `read_to_end` against an untrusted host is an out-of-memory waiting for a bad
day. The measured file is 3.9 MB and the cap is 32 MB.

## Live Prices For Live Decisions

The dump is a day old by construction, and there is a moment where that is the wrong number: when
the player has stopped valuing the collection and started trading out of it. The week an item is
unvaulted its price moves faster than a daily file can follow, and that is precisely the week
somebody goes looking.

So the dump seeds everything and warframe.market answers for the rest. **One trigger, and the player
pulls it:** the page refresh. It prices what is on screen, at most forty-eight items, paced at three
requests a second, so a full page takes about sixteen seconds. It offers every item the player owns
rather than only the ones already priced: an item with no price is exactly the one somebody would
want to ask about, and unresolvable names are dropped by the backend before any request is made.
Its results are written into the price table, for the reason below.

There used to be a second trigger -- a relic sweep at every launch and after every inventory refresh,
about 70 requests and 22 seconds, because the dump's relic ask was unusable and nothing else was on
offer. It is gone. The `closed` statistics gave relics a real dump price, and unioning the daily
dumps (see Price Source) covers 96% of a real collection's relics for no request at all. The last
few percent were never worth 22 seconds of every launch against a 391p holding, and a player who
wants them has a button.

This is not a sweep of the collection. Nothing is fetched for an item the player does not own, and
nothing at all is fetched without a click, which is what keeps a feature that could have cost a
thousand requests down to the page somebody is looking at.

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
the cache rather than in any caller. Three call paths share it -- the pool warm, the page refresh
and the reward screen's fill -- and any two can overlap; each politely
waiting 334ms of its own would still have put six or nine requests a second on the API. Every
request claims a slot from one shared clock before it leaves, so a caller in a hurry (the reward
fill, which has fifteen seconds of screen and skips its own extra delay) can spend the budget sooner
but cannot exceed it. One live pass runs at a time for the same reason: two overlapping passes over
the same page would spend the same requests twice.

A checked price outlives that cache. It is written into the price table beside the dump's own
prices, because re-spending requests to learn a number we already had is the behaviour the API rules
ask clients not to have, and because a price the player deliberately asked for is the best number
the app has about that item -- letting it expire back to a day-old figure discards a request they
spent. The table therefore holds three price maps, read in order of freshness: what was checked
live, then today's dump, then the newest earlier dump that priced the relic. Consulting them the
other way round would shadow a live number with the one it was fetched to replace. A live order book
beats a day-old completed trade, which beats a month-old one.

A checked price lives exactly as long as the dump it was checked against: a refresh that brings back
the same dump -- which is the ordinary case, since the dumps lag two days -- carries the checked
prices across, and a genuinely newer dump clears them, after which the page offers to re-price what
is on screen. That is the whole freshness policy for a checked price and the only bound on how stale
one can get.

Carried relic prices answer to a different clock on purpose, and it is the only other one: 30 days
from the dump that produced each, tracked per price. They are not the market's answer about right
now, they are an older file's, and unlike a checked price nobody spent a request on them -- so they
can afford a longer life, and they have to carry a date to have an honest one.

"Nobody is selling this" travels the same road, because it is an answer and not a failed request.
The table records it beside the prices and a later pass skips it on the same terms. Filtering on
"has a price" instead meant every item with an empty order book failed the test again on the next
pass, and the one after -- the same requests, the same answer, for a set of items a real collection
is never short of. An *unreachable*
endpoint is deliberately not recorded: an outage is a reason to try again, and treating it as an
answer would blacklist a relic until tomorrow's dump over a router that rebooted mid-pass.

Two callers write that table -- the page refresh and the daily dump download that replaces it -- so
the read-modify-write that folds prices into it happens under the runtime lock rather than beside
it, against whatever the runtime is serving at that moment rather
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
priced from, and a card whose number came from a live check -- the page refresh, or a warmed relic
pool persisted into the table -- says it was checked live. The distinction is
on the card, not in a tooltip, because the whole reason the live path exists is that the difference
matters.

The dump's own two measurements -- the traded median and the ask it falls back to -- are
deliberately *not* told apart on the card. That is a real corner cut: an item priced from 14
completed trades and one priced from an untested asking price both read as "from the 2026-07-30
dump". It stays cut because the distinction is not one a player can act on -- both are the same
day-old file, and the live check is the affordance for wanting better -- while a third provenance
state on every card is a visible cost on all 3,209 of them. Add it if the ask fallback ever proves
to be misleading in a way the volume floor does not catch.

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

## What The Market Will Take

Every unit price above can be right and the collection's total still be a number nobody could
realise. warframe.market is not an order book: a trade is arranged in chat and completed by two
players standing in a dojo, one item at a time. 7,982 copies at a correct 1–5p each is 21,837p that
would take years of evenings to collect, and 182 Quickdraws is not 364p of platinum, it is 364p of
platinum that the two people who wanted a Quickdraw this month already have.

So the collection's headline worth is each stack at its unit price times the *smaller* of what is
owned and what the whole game completes in a month. The market rate stays beside it, unchanged.

The cap is a volume, not a price threshold. A threshold -- "commons under 5p don't count" -- needs an
invented constant and is wrong in both directions: it writes off a 3p mod the game trades five times
a day, and leaves a 240p one nobody has bought in a month at full value. Volume is measured rather
than chosen, comes from the `closed` records already being parsed for prices, and separates exactly
those two cases. It is still an optimistic bound -- it assumes the player personally makes every
trade in the game for that item -- and that is the right direction for a bound to be wrong in.

Appetite is averaged across the dumps seen, not read off today's file, because a day's dump samples
the market as thinly for volume as it does for price. Both plainer readings are biased and both were
measured on a real account: today's count alone understated the sellable total by about a quarter and
swung it a tenth with whichever listings happened to trade that morning, since `Intruder` completed
159 trades in twenty-eight days and has no `closed` record at all on the 30th. Carrying the last
count *seen* overstates by the same mechanism inverted -- it conditions on a day where a trade
happened, and read `Quickdraw` at 30 a month against a true 2.

The average is kept as one running figure per item rather than a month of daily counts, because the
cache is a file rewritten on every checked price and several thousand counts per item would cost more
than the figure they support is worth. Each dump gets an equal share until a month of them have been
seen and a thirtieth after that. The equal share is what makes it converge: weighting today at a flat
thirtieth from the first day leaves the very first dump 40% of the estimate a month later, which is
how the same real account read `Quickdraw` at 15 a month. A residual overstatement remains, since an
item's average begins on the first day it was seen to trade and so discards the leading zeros -- it
puts `Quickdraw` at 3 rather than 2, which changes no decision anybody makes.

The counts expire on the same thirty-day boundary as the carried relic prices, dated by the last
dump that saw a trade rather than the last one processed, so an item nobody has bought in a month
drops out entirely instead of decaying towards zero forever.

The cap is not enough on its own, and what is left over is not a measurement problem. 14,936p of one
real account's 24,086p is items priced at 1–5p, and the volume cap barely touches them because the
market genuinely does complete those trades: `Redirection` 68 a month, `Intruder` 104. The tempting
second cut -- write off anything under some price -- is not something this design can decide. It
would be the only invented constant where everything else is measured, and any constant it picked
would be wrong for somebody: at a 6p floor the account loses `Intruder` ×104 at a true 3p, which is
312p that demonstrably moves, and for a player who will not spend an evening on 3p mods that 312p is
correctly gone.

So the floor exists and belongs to the player. A slider in Settings, 0 to 20 platinum, applied to the
sellable figure and never to the market rate: a stack whose copies are worth less than the floor
stops counting. Zero is the default, which is the measured answer with nothing invented on top of it.
The slider stops at 20 because above roughly that point the figure stops answering -- every floor
from 21p up lands within a few percent of the last, since all that is left by then is the few dozen
items anybody would trade one at a time (3p → 21,369p, 6p → 9,150p, 11p → 6,708p, 16p → 5,459p,
21p → 4,309p). The floor is a display preference over a figure the frontend already computes, so it
lives in the window's own storage rather than in SQLite; putting it in the database would mean a
migration, an IPC pair and a round trip to move a slider.

## Presentation

A priced card shows `20p`, and `20p · 140p total` once more than one is owned. Both numbers earn
their place: the unit price is what gets quoted in trade chat, the stack total is what decides
whether the pile is worth clearing.

The band's worth cell is two figures and one clause: the market rate as the struck mark, the sellable
total under it at the size of a qualification, and the cap that produced it as the note. Market rate
leads because it is the plain reading of what is owned; the capped figure is the thing that needs
explaining. Both figures carry the game's own platinum icon -- the sellable line sits in the slot the
three cells beside it fill with item counts, and without the icon a bare `16,994 sellable` reads as
one more count. The icon is set to the line's own `1em` there rather than to the figure's, since a
mark sized for a 2rem total beside 0.72rem text is a badge, not a unit. The note names the cap in the
terms the reader has -- copies the market buys in a month -- rather than in the dump's own vocabulary
of completed trades, which describes where the number came from and not what it means. The cell
previously carried five numbers -- the capped total, the trades it would
take, the market rate, how many items were priced and a copy of the live pass's counter -- and read
as an argument about the collection rather than a valuation of it. The pass counter was already on
the register line below. The priced-item count mostly measured how much of a collection is
untradeable, which is not a fact about worth. And the trade count was there to say the total is not
free, a job the price floor now does directly and adjustably. The cap, and the floor when one is set,
is stated in the cell's own note rather than on the register line among the filters, where it was
answering a question the reader had not been given yet. The per-card treatment is deliberately left
alone; how much of one stack the market takes is a fact about 89 cards out of 1,215, and putting a
hallmark on all of them to serve those would cost more attention than the band total's haircut is
worth explaining twice.

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
times quantity across priced items, over the sellable figure that says how much of that sum the
market would take. All three read the best price in hand for each item.

The floor's control is a slider on the Settings page, which until now was four notices and a preview
button under a heading that said "Settings & about". Those are two different kinds of thing on one
page: a preference changes what the app does and is there to be operated, while a disclosure states
what it already does and is there to be read. So they are now two pages in the rail. Settings holds
what is set -- the price floor, and the reward overlay preview, which is a control that moves a
window and not a statement about one -- under a tracked "Preferences" head, each preference on its
own ruled plate. About holds what is stated: the licence line and the four clauses, including the
overlay's OCR and click-through behaviour, which is a fact about the app rather than a knob on it.
The first-run disclosure's footnote points at About accordingly, since About is where it now lives.
The slider
reads out its own effect as it moves -- stacks counted, platinum left -- because a control whose result only appears on
another page is a knob rather than a dial, and the whole reason the floor is the player's to set is
that they can see what each setting costs them. It is drawn in the register's own materials: an
engraved groove filled with platinum to the setting, a struck square rider on it. The fill is a
gradient driven by an inline custom property, the same device the register bar's progress rule uses,
because a native progress fill exists in Gecko and not in WebKit and WebKitGTK is what draws this
window.

One control invokes the live path: the register's refresh, which names its scope and how many items
it will price -- everything on the page the backend can actually ask about, whether or not it has a
number yet. It counts `priceable` rather than everything owned, because counting items the backend
drops before it makes a request promised prices that were never coming. It sits at the end of the
register bar, after the provenance line and the range readout: those two are one statement about
what is on screen, and an action set between them broke a line meant to read as one. There is no
per-item control. It was one click for one request, in a register where the row-level answer is the
same request; the page control subsumes it and one affordance is easier to understand than two.

A pass in flight is visible, because sixteen seconds of silence reads as a broken button -- and
because the worth figure moves the whole time it runs. Two things say so, and they say it once each.
The register bar's own bottom
rule fills with platinum as the pass advances: an engraved hairline is already this interface's
device for dividing the sheet, so a reading struck into one needs no progress bar, no spinner and no
new component. Beside it, in the same voice as the provenance line, sits the count. The worth cell
does not repeat it: the count is four inches away on the same screen, and a figure that says the same
thing twice is how that cell came to hold five numbers.

The count is the backend's, not the page's. Reconstructing it in the client -- which of the
requested ids have gone live since the click -- could only ever describe the pass the client
started, and the backend is the party that knows a pass's total. The control itself carries no
number while a pass runs: it is disabled, because every pass spends the same rate-limited budget, and a second copy of the same figures on the disabled thing reads as a
different pass.

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
the cap; the closed-price preference in each of the four ways it can go wrong -- a relic priced per
tier from its `closed` records, a relic carrying only an ask still reading as unpriced, a closed
record below the volume floor refused (and falling back to the ask where there is one, and to
nothing where there is not), and a closed record at one rank not becoming another rank's price,
which is the regression that would restore the maxed-price fault `CACHE_SCHEMA` was first bumped
for; a refinement tier that did not trade borrowing the intact tier's traded price; date walk-back, proving a missing file for today falls through to an older one and records
the date it used; a cached dump dated today or yesterday not being downloaded again while an older
one is; a refresh of the same dump keeping the prices checked against it -- relic and dump-priced
item alike -- and a newer dump discarding them, the dump-priced one falling back to the new dump
rather than to the stale number; a cache written under the map's former name still carrying its
prices; the cache round-tripping through disk and pricing a collection before any network call;
malformed input rejected whole; a checked price taking precedence over the dump's for the same item
and being marked as such; a persisted checked price still reading as checked live once the
fifteen-minute cache has dropped it, for a relic and for an item the dump prices; an item at
quantity 0 carrying no price; two concurrent warms unable to put requests closer together than the
shared floor; and each live-lookup outcome distinctly, including oversize reaching the market health
row, a part-finished pass reporting itself, and a bulk listing too cheap to divide still quoting 1p.
A no-seller answer has its own set: reading as checked but not as priced and not counting toward
what the table can price, a later real price replacing it and a later empty book not undoing one,
that answer surviving a same-dump refresh and dying to a newer one, and -- the one that guards
against blacklisting a relic over an outage -- a per-name pass keeping `NoSellers` and `Unavailable`
distinct.

Carrying relic prices has its own: a relic priced by yesterday's dump staying priced through today's
and tomorrow's, that carried price reading as a dump price rather than a checked one, today's dump
winning over a carried one, a non-relic never being carried at all, and the 30-day window measured
on both sides of its edge.

The appetite has its own too: that it is counted from completed trades and not from the live
listings sitting beside them, that the volume floor guarding *prices* deliberately does not apply to
it -- one trade is a poor median and a perfectly real trade -- that a rank-only quote leaves an
unranked copy unpriced while the maxed copy keeps its price and the name still resolves, that a
quiet day averages into the rate rather than replacing it or wiping it, and the same 30-day window
measured on both sides of its edge.

Frontend tests cover the value sort by unit price with unpriced and zero-priced entries present, the
worth arithmetic, the cap at what the market takes including a correct price on a stack nobody trades
being worth nothing, the floor applied per copy rather than per stack and counting its own value, the
band leading with the market rate and carrying the sellable figure under it, the slider moving both
its own readout and the band's second figure while leaving the market rate alone, the floor surviving
the window that set it and a storage that refuses to answer at all, the tradeable filter, the dump's
date on the register, that a checked price is visibly distinguished from a dump price, that the
refresh control offers every priceable item on the page including ones with no price yet and neither
an unowned one nor one no name rule reaches, that a pass reported by the backend appears on the
register, that the control is refused while a background pass is spending requests, that Settings
carries the controls and About the notices with neither holding the other's, and that the view
poll keeps running through a live page refresh.

## Out of Scope

Price history and trends, buy-order prices, sell recommendations, per-item manual refresh, relic
refinement pricing, and extending the catalog index to non-prime weapon components.
