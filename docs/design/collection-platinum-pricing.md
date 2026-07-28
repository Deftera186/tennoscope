# Collection Platinum Pricing Design

## Goal

Put a platinum price on every tradeable item in the collection, so the question "what is this pile
worth, and what should I sell" is answered in the browser rather than in a second window against a
website.

The reward overlay already prices four cards live against warframe.market. The collection is a
different problem wearing similar clothes: hundreds of items rather than four, and a valuation
rather than a decision made inside fifteen seconds. It gets a different source, and the difference
is the whole design.

## Evidence

Measured on 2026-07-29, and the source of the constants and rules below.

| Measurement | Value | Consequence |
| --- | --- | --- |
| `relics.run/history/price_history_<date>.json` | 3,835 items, 3.9 MB, one request | The entire collection is priced by a single daily download |
| `sell` statistics present | 3,831 of 3,835 items | The field the collection reads |
| `closed` statistics present | 2,489 of 3,835 items | Rejected as primary: a third of the collection would show nothing |
| Newest dump on 2026-07-29 | `price_history_2026-07-27.json` | The fetcher walks back from today; the data is a day or two old by construction |
| Four name rules, no manifest | 3,365 of 3,835 items resolved | Name matching alone is enough |
| `/v2/items` manifest on top | 257 more, 228 of them sets | Rejected: its entire contribution is items that must not be priced |
| Mirage Prime Systems, dump median sell | 20p, against 19p live from an online seller | The daily median tracks the live number closely |
| Mirage Prime Systems, dump minimum sell | 10p | Rejected: the day's cheapest listing includes offline lowballers |

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

The dump is keyed by warframe.market's English names and the collection by WFCD's. Four rules close
the gap, in order, with no network call between them:

1. The name as it stands.
2. The name with ` Blueprint` appended, reconciling `Zephyr Prime Chassis` with `Zephyr Prime Chassis Blueprint`.
3. The name with ` Blueprint` removed, for the mirror case.
4. A relic's refinement suffix -- `Intact`, `Exceptional`, `Flawless`, `Radiant` -- replaced by ` Relic`, reconciling `Axi A1 Radiant` with `Axi A1 Relic`.

Rule 4 recovers 772 relics that no other rule reaches, at the cost of one known imprecision: the
dump prices a relic without regard to refinement, so all four tiers of `Axi A1` read the same. A
radiant relic is worth more than an intact one and this design does not say so. That is a real
loss, accepted because the alternative is pricing no relics at all.

An unresolved name means no price. It is not an error, and it is not evidence that the item is
untradeable -- the remaining gap is largely non-prime weapon components, which the catalog does not
index today. Extending that index is a separate change.

## Cache And Refresh

Following the pattern `catalog_cache.rs` established: atomic writes, serde, a file under the
application data directory.

What is cached is the resolved map, not the download. The 3.9 MB dump reduces to a few thousand
name-and-price pairs, so the cached file is small, loads instantly at startup, and prices the
collection before any network call is attempted. The dump date is stored with it. A fetch is
attempted once per day, and when it fails the cached prices stay and are described by their date
rather than discarded.

The response is capped, as every remote read in this codebase is, because the body is streamed into
memory and an uncapped `read_to_end` against an untrusted host is an out-of-memory waiting for a bad
day. The measured file is 3.9 MB and the cap is 32 MB.

## Two Numbers, On Purpose

The overlay keeps its live per-item path against warframe.market's own API, unchanged. Its question
is "what is the cheapest online seller asking, right now", because the answer is acted on in the
next fifteen seconds and a day-old median can be badly wrong the week an item is unvaulted. The
collection's question is "what is this worth", and there a stable median beats a live quote that one
lowballer can move.

They are different numbers and the interface says so rather than hoping nobody notices: the
collection labels its prices with the date of the dump they came from.

## Application View

`CollectionItemView` gains `platinum: Option<u32>`, and nothing else. A separate `tradeable` flag
would be a second name for "has a price", since with a single dump those are the same fact; the
`Tradeable` filter reads the price field directly. `AppCore` holds an optional price table -- one
cheap `Arc` clone -- that `current_view()` reads while building items.

Nothing else is needed to deliver prices to the interface. The frontend already polls the view every
2.5 seconds, so prices appear when the table loads, through plumbing that already exists. There is
no queue, no worker thread, no per-page scheduling and no rate limiter, because there is nothing
left to schedule when every price arrives in one file.

## Presentation

A priced card shows `20p`, and `20p · 60p total` once more than one is owned. Both numbers earn
their place: the unit price is what gets quoted in trade chat, the stack total is what decides
whether the pile is worth clearing.

Unpriced and untradeable are not distinguished, because with a single dump this design genuinely
cannot tell them apart -- an item absent from the file may be untradeable or may be one the name
rules failed to reach. Claiming to know which would be a guess dressed as a fact. Unpriced items say
only that there is no price, and the market health row carries the dump's date so a collection full
of dashes is legible as a stale or failed download rather than as a worthless account.

Three affordances follow the prices. A `Value` sort orders by stack value with unpriced entries
sinking to the bottom. A `Tradeable` filter narrows to items that have a price. A fourth cell in the
assay band sums price times quantity across priced items and carries the count it was computed from,
because a partial sum presented as a total is a lie the reader cannot detect.

The interface work is done through the `impeccable` skill.

## Boundaries

- `warframe-acquisition` owns the download, the name rules, the cache and the price table. It knows nothing about collections or presentation.
- `app-core` joins prices onto collection items and publishes them in the immutable view.
- The Tauri shell owns when the refresh runs.
- React owns the price line, the sort, the filter and the worth cell, and formats nothing it was not given.

## Failure Handling

An unreachable dump leaves the cached prices in place, dated. A first run with no cache and no
network shows no prices and a market health row that says why. A malformed dump is rejected whole
rather than partially applied, so a truncated download cannot silently halve the collection's worth.

A response over the size cap is reported as its own outcome rather than as an absent price. The same
correction applies to the overlay's live lookup, where today a `None` conflates four different
facts -- priced, no online seller, endpoint unreachable, and response over the cap. That last one is
the dangerous member: it is the failure that arrives the day warframe.market widens its payload, and
as an `Option` it presents as "every item is worthless" with nothing anywhere saying otherwise.

No pricing failure can affect inventory synchronization.

## Verification

Automated tests cover each name rule and a name no rule reaches; the dump parser against a trimmed
fixture, including an item whose `sell` record is absent and one whose body is one byte over the
cap; date walk-back, proving a missing file for today falls through to an older one and records the
date it used; the cache round-tripping through disk and pricing a collection before any network call;
malformed input rejected whole; and each live-lookup outcome distinctly, including oversize reaching
the market health row. Frontend tests cover the value sort with unpriced entries present, the worth
arithmetic and its count, and the tradeable filter.

## Out of Scope

Price history and trends, buy-order prices, sell recommendations, per-item manual refresh, relic
refinement pricing, and extending the catalog index to non-prime weapon components.
