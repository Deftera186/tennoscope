# Collection Platinum Pricing Design

## Goal

Put a platinum price on every tradeable item in the collection, so the question "what is this pile
worth, and what should I sell" is answered in the browser rather than in a second window against a
website. The reward overlay already prices four cards against warframe.market; this extends the same
machinery from four names to a whole account.

The constraint that shapes everything: warframe.market publishes no bulk price endpoint, and the
public limit is 3 requests per second. A collection is hundreds of tradeable items. Every design
decision below exists to spend fewer requests.

## Evidence

Measured against the live API on 2026-07-28, and the source of the constants in this document.

| Measurement | Value | Consequence |
| --- | --- | --- |
| `GET /v2/items` | 3,837 tradable items, 1.6 MB, one request | Definitive tradability oracle; untradeable items cost zero requests |
| `GET /v2/orders/item/mirage_prime_systems` | 184,039 bytes, 324 sell orders | The full order book is 38x the data we need |
| `GET /v2/orders/item/mirage_prime_systems/top` | 4,866 bytes | Same answer, a fraction of the bytes |
| `/top` seller status | 4 orders, all `ingame` | The online filter is applied server-side |
| `/top` lowest vs full book lowest-ingame | 19p vs 19p (lowest offline: 10p) | `/top` is exact for our purpose, not an approximation |
| `gameRef` against catalog item IDs | 2,826 of 3,837 resolvable | Slugs are looked up, not guessed |
| Documented public rate limit | 3 requests/second | One worker at 334 ms between requests |

`gameRef` in the item manifest is DE's own item path -- `/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod`
for Serration -- which is the same identifier space as our collection keys. That is what makes exact
resolution possible at all.

warframe.market's client rules require a dedicated descriptive `User-Agent` and ask explicitly that
clients cache rather than re-fetch large collections. Both are honored below.

## Item Manifest

A new `market_manifest.rs` in `warframe-acquisition`, built on the pattern `catalog_cache.rs`
already established for the WFCD catalog: atomic writes, serde, a cached copy under the application
data directory.

`GET /v2/items` is fetched once and stored as `market-items.json` beside the `collections.items`
hash from `GET /v2/versions`. On later starts only the versions response is fetched -- a few hundred
bytes -- and the manifest is re-downloaded only when that hash has changed. When the versions
endpoint is unreachable the cached manifest is used regardless of age: a tradability list that is a
week old is still overwhelmingly correct, and refusing to price anything because a metadata request
failed serves nobody.

The manifest yields a `MarketIndex` whose `slug_for(item_id, name)` resolves in three steps:
`gameRef` first, then exact English name, then a blueprint normalization that reconciles the
catalog's `Zephyr Prime Chassis` with the market's `Zephyr Prime Chassis Blueprint`. A `None` result
means the item is not tradeable. That is a fact rather than a failure, and it is the single largest
saving in the design: no request is ever spent discovering that Orokin Cell and Forma Blueprint
cannot be traded.

The overlay resolves through the same index, keeping today's `market_slug()` derivation as a final
fallback. The overlay reads names off the screen and has no item ID, and warframe.market honors
legacy slug aliases that the derivation happens to produce; nothing that prices today stops pricing.

## Pricing Worker

One thread owns a deque of pending slugs behind a mutex and condvar and issues one request every
334 ms. A single worker is what makes the rate limit true by construction rather than by convention:
one queue, one clock, no coordination between producers.

The queue has two ends and that is the whole scheduling policy. `prioritize()` pushes the items
currently on screen to the front, so paging or filtering puts those forty-eight items next in line.
`enqueue()` appends the background sweep -- every tradeable item held in quantity one or more,
without a fresh price -- to the back. Items already fresh in cache, or already in flight, are never
enqueued twice.

Requests go to `/v2/orders/item/{slug}/top` and carry a `User-Agent` naming this project and a
contact URL, matching the form the API rules ask for. The response cap falls from 8 MB to 64 KB,
which the measured 4.9 KB response sits comfortably inside. A 429 or 509 backs the worker off
exponentially rather than killing it.

## Price Cache

`MarketPriceCache` gains two things. First, `get_fresh(name, max_age)`, with the existing `get()`
becoming `get_fresh(name, PRICE_TTL)`: the overlay keeps its fifteen-minute bar because a reward
screen is a trading decision made in the next fifteen seconds, while the collection reads at twelve
hours because an inventory valuation is not a live quote and re-fetching hundreds of items to shave
hours off their age would be exactly the behavior the API rules ask clients to avoid.

Second, the cache persists. Entries currently carry an `Instant`, which means nothing across a
restart, so they become unix timestamps and the cache round-trips through a JSON file in the
application data directory. Prices are disposable data with an expiry, the same class as the cached
catalog and deliberately not in the SQLite schema that holds the inventory: that database is
validated against a canonical definition and migrated by version, and neither of those protections
is worth anything to a number that expires by lunchtime.

## Application View

`CollectionItemView` gains `platinum: Option<u32>` and `tradeable: bool`. `AppCore` holds an
optional market handle -- two cheap `Arc` clones -- that `current_view()` reads when building items.

A `prioritize_prices(item_ids)` command lets the frontend declare what is on screen. The background
sweep is enqueued after each inventory refresh. Nothing else is needed to deliver prices to the UI:
the frontend already polls the view every 2.5 seconds, so prices appear as they land, through
existing plumbing, with no new events.

## Presentation

A priced card shows `19p`, and `19p · 57p total` once more than one is owned. Both numbers earn
their place: the unit price is what gets quoted in trade chat, the stack total is what decides
whether the pile is worth clearing.

Untradeable and not-yet-priced are shown differently, and this is the one place where the cheaper
option is wrong. Collapsing both into an em dash tells a player that a mod worth 300p is worthless,
because the request for it has not come back yet. Untradeable is permanent and stated as such;
unpriced is a pending state and looks like one.

Three affordances follow the prices. A `Value` sort orders by stack value with unpriced entries
sinking to the bottom. A `Tradeable` filter joins the ownership row so the untradeable half of an
account stops diluting the view. A fourth cell in the assay band sums price times quantity across
priced items, and carries the count it was computed from -- a partial sum presented as a total is a
lie the reader has no way to detect.

The interface work is done through the `impeccable` skill.

## Boundaries

- `warframe-acquisition` owns the manifest, slug resolution, the rate-limited worker and the cache. It knows nothing about collections or presentation.
- `app-core` joins prices onto collection items and publishes them in the immutable view.
- The Tauri shell owns worker lifecycle and translates the frontend's visible-page declaration into slugs.
- React owns the price line, the sort, the filter and the worth cell, and formats nothing it was not given.

## Failure Handling

An unreachable manifest means no prices: market health reports degraded and the collection renders
exactly as it does today. An unreachable orders endpoint leaves individual items unpriced and the
worker continues with the next slug. An untradeable item is not an error anywhere in the pipeline.
No pricing failure can affect inventory synchronization, and no pricing request is ever made for an
item the manifest does not list.

## Verification

Automated tests cover slug resolution across all four outcomes (`gameRef` hit, name hit, blueprint
normalization, untradeable miss); the `/top` parser against a captured fixture containing an offline
seller whose lower price must not win; queue ordering, proving a prioritized page overtakes a
backlog; TTL expiry at both bars against a fake clock; and the cache round-tripping through disk.
Frontend tests cover the value sort with unpriced entries present, the worth arithmetic and its
count, and the tradeable filter.

## Out of Scope

Price history and trends, buy-order prices, sell recommendations, and per-item manual refresh.
