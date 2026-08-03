# warframe.market Account Link Design

Phase 1 of three. This phase links a warframe.market account, shows the orders it holds, and
reconciles them against the collection. Posting and editing orders is phase 2; marking an order
sold from an observed in-game trade is phase 3.

## Goal

Answer "what am I selling, and is any of it wrong" inside TennoScope, instead of in a browser tab
next to it.

The collection already knows what the player owns and what it is worth. warframe.market knows what
the player has listed. Nothing joins the two, so the listings that outlive their items -- sold in
game, never taken down -- are found by the next person who whispers about one.

## Accounts

TennoScope has no accounts. It does not gain one here.

Linking a warframe.market account is opt-in, off by default, and every existing feature works
without it. What the link changes is narrow and worth stating plainly, because it is the one place
in the application where player data leaves the device: an authenticated request carries the
account's token, and phase 2 will send order contents the player has chosen to publish. Inventory,
mastery, and reward history remain local and are never uploaded.

The consent screen says this before the first request, not in a settings page reached later.

## Authentication

### What the API allows

warframe.market has no third-party authentication story yet. Measured 2026-08-01 against the
published documentation and the live API:

| Route | Result |
| --- | --- |
| `POST /v2/auth/signin` | First-party only. Requires `X-Firebase-AppCheck`; without it the API answers `app.auth.appCheckMissing` |
| OAuth 2.0 | Documented as not publicly available. Client registration is closed to third parties |
| `POST /v1/auth/signin` | The remaining route, and the one the vendor's own documentation directs integrations to |

The v1 flow posts credentials with `auth_type: "header"` and a seed `Authorization: JWT` header,
and reads the issued token out of the **response** `Authorization` header rather than the body. One
token then authenticates both versions: `JWT <token>` on v1 routes, `Bearer <token>` on v2. Tokens
obtained this way currently carry scope `all`, so v2 endpoint scope requirements do not apply.

Every live third-party client converges on this flow. It is undocumented in the sense that matters:
it is not a stable contract, and it can be withdrawn without notice.

### Two ways in, of equal standing

**Sign in.** Email and password, exchanged once for a token. The password is a parameter to one
function. It is not copied, not stored, and not present in any error value.

**Paste a token.** The player copies the `JWT` cookie from a signed-in browser session. This exists
for two independent reasons: some players will not type a market password into a desktop
application, and the v1 signin route may stop working. It is not a degraded path, and the interface
does not present it as one.

A pasted token is validated with one `GET /v2/me` before it is stored, so a bad paste fails at the
moment of pasting.

### Refresh

Authenticated responses carry a renewed token in the `Authorization` header. Every authenticated
call updates the stored credential from it, so an account in regular use never expires. An account
left alone long enough does; the token's documented lifetime is about sixty days.

### Storage

The token is stored in the OS keyring where one is available, and in the local database, file
permissions `0600`, where one is not. A Linux session without a running secret service is ordinary
rather than exceptional -- minimal window managers frequently have none -- so a keyring-only design
would need this fallback anyway.

Which backend holds the credential is reported in the health panel. The difference is real: a
database file is readable by anything running as the user and is swept up by backup tools, and a
player who would rather not have that can install a secret service or decline to link.

Held in memory as `Zeroizing<String>`, matching how the acquisition path already handles account
nonces. It is never returned across the Tauri boundary, never formatted into an error, and never
written to the log.

A token is a credential. Anyone holding it can post and delete orders on the account.

## Architecture

### A new crate

`warframe-market` joins the workspace as a sibling of `warframe-acquisition`. The separation is not
tidiness: acquisition reads the game, this calls a website, and they fail for unrelated reasons and
must be diagnosed apart.

**`auth`** owns the credential lifecycle -- `sign_in`, `link_token`, refresh on use, `sign_out` --
and hides the token from its callers.

**`orders`** owns `list_mine` against `GET /v2/orders/my`, which returns every order on the
account, visible and hidden, in one request. It also owns the two writes the fixes below need:
`delete` against `DELETE /v2/order/{id}`, and `set_quantity` against `PATCH /v2/order/{id}`.

Phase 1 is therefore not read-only, and the distinction that matters is not reads against writes.
It is that every write here *reduces* an existing listing -- taking one down, or lowering it to what
is owned -- and each is one button the player presses on one row. Phase 2 adds `create`, which
publishes something new.

**`credential_store`** owns the keyring-or-database decision behind `load` and `store`, and reports
which backend answered.

### The existing price path is untouched

`warframe-acquisition::market` remains the anonymous price path. It already holds the shared
request pacing that keeps every caller inside the documented three requests per second, and that
limit belongs to the client rather than to any one caller.

The pacing therefore moves into a small shared component that both crates use. An authenticated
crate with its own limiter would leave two lanes each politely pacing at three per second and
jointly arriving at six.

### Reconciliation belongs to app-core

Joining an inventory snapshot to an order list is the role `app-core` already fills: it accepts
validated observations and produces immutable views. The market crate has no concept of a
collection, and gains none.

### Presentation

Five commands: `market_status`, `market_sign_in`, `market_link_token`, `market_sign_out`,
`refresh_orders`, plus the two the fixes need: `remove_order` and `set_order_quantity`. None
returns the token.

## Reconciliation

Each sell order is labelled with one of four states.

| State | Meaning | Offered fix |
| --- | --- | --- |
| `Ok` | The collection holds at least the listed quantity | None |
| `Missing` | The collection holds none | Take the order down |
| `Overshoot { owned }` | The collection holds fewer than listed | Lower the quantity to `owned` |
| `Unverifiable` | The comparison cannot be made | None |

Every fix is a button the player presses. Nothing is removed or altered automatically.

That restraint is the point rather than caution for its own sake. The application's stated failure
posture is to keep the last coherent inventory when the reader breaks, so a snapshot can be stale
or absent while looking exactly like a current one. An automatic remover reading a stale snapshot
would delete live orders that were never wrong.

`Unverifiable` is what makes the rest trustworthy. **A mismatch is claimed only when the snapshot is
coherent and newer than the order.** Four cases produce it:

- no snapshot has been taken yet;
- the snapshot predates the order's last update;
- the order names something the collection cannot match by identity -- ranked mods and Arcanes,
  where the market's rank and subtype do not map onto an inventory row; and
- the order is a buy order, where owning none is the ordinary state.

An `Unverifiable` row carries no flag and no fix button. It is an ordinary row, because there is
nothing to say about it. A degraded game reader produces a screen of ordinary rows rather than a
screen of accusations, each with a delete button beside it.

## Refresh

Orders are fetched when the section opens, when the player asks, and after any write the
application itself performs. There is no polling.

`GET /v2/orders/my` returns everything in one request, so a fetch is cheap -- but a timer spends
requests continuously to discover changes that, from phase 2 onward, almost always originate in this
application, which already knows about them. What a timer would catch and this does not is an order
changed on the website in another tab, and the manual refresh covers that.

The 2.5-second view rebuild reads the last fetched list. It never reaches the network.

## Interface

### Orders

A new top-level section after Relics. Orders are live editable state rather than history, which is
why they do not belong in Activity, and a central list is the feature itself rather than a
convenience over per-item badges.

**Unlinked** is an explanation rather than an empty list: what linking does, that it is optional,
that order data will leave the device, and the two ways in.

**Linked** is one list. Each row carries item, price, quantity, direction, and visibility. Rows
needing attention sort to the top and state their case in the row -- "you no longer own this", "you
own 1 of 3 listed" -- with the fix inline. A header states total listed value and when the list was
last fetched.

Status reads "Linked". Not the email address: one account is linked, so the identity answers no
question the player has. If multiple accounts or platform variants ever exist, the market username
is the right thing to show, and `GET /v2/me` already returns it.

### Collection

An item with a live order shows it inline -- `listed 12p`. This is what phase 2's sell action
attaches to.

### Health

A `market_account` row beside the existing backend rows: linked or not, which credential backend is
in use, and the last successful fetch.

### Visual design

The interface is designed with the `impeccable` skill during implementation rather than specified
here, so that the design is made against the real screen instead of against a sketch in this
document.

Two requirements are not the skill's to decide. A flagged row must read as different at a glance
without shouting, since the flags are the reason the screen exists. And the section must match the
application's existing visual language rather than arriving as a differently-styled panel.

### What is never shown

The list holds only the player's own orders, so no other player is named anywhere in this phase.
The email address is not displayed after linking. No token, order, or account detail reaches the
log, the diagnostics output, or the repository.

## Failure handling

| Condition | Response |
| --- | --- |
| Credentials rejected | Reported as such, form retained. Which field failed is not logged |
| v1 signin route unavailable | Reported as the route having changed, pointing at the paste-token path, rather than as a generic outage |
| Token expired or rejected | Status becomes `NeedsRelink`; requests stop rather than retrying a credential that cannot work |
| Unreachable, or `429` | Last fetched list retained with its age shown; the health row carries the detail |
| No keyring and the database fallback is declined | Linking fails without holding the token anywhere unstated |

## Testing

Test-driven throughout, at interfaces, with deterministic adapters -- the convention the workspace
already follows.

**`auth`,** against a fake transport: a successful exchange, a rejection, and the renewed token read
from a response header. Plus one test asserting that no token and no password appears in an error
value, a `Debug` rendering, or the log. That case is a test rather than a comment because it is what
keeps the rule true through later edits.

**`credential_store`,** both backends, including a keyring-absent run falling back and reporting it.

**Reconciliation,** table-driven across every state: exact match, missing, overshoot, absent
snapshot, stale snapshot, ranked mod, buy order. The absent and stale snapshot cases carry the most
weight, since they are what stops a broken game reader from producing false accusations.

**Interface,** across the three states -- unlinked, linked and clean, linked with mismatches -- in
the existing vitest setup.

**Live,** ignored by default alongside the existing `live_*` tests. It is read-only: `GET /v2/me`
and `GET /v2/orders/my`. It creates, modifies and deletes nothing, and a maintainer running it
against a real account changes nothing on that account. The two writes this phase adds are
deliberately excluded, because there is no way to exercise a real deletion against a real account
without destroying something the account holder wanted. They are covered against a fake transport
instead.

What no test covers is whether the v1 signin route works on a given day. It is undocumented and
outside this project's control, which is the reason the paste-token path exists as an equal.

## Acceptance

1. An account links by sign-in, and by pasted token, and the token survives a restart.
2. The credential backend in use is visible in the health panel.
3. Orders load and list, with total value and fetch age.
4. An order for an unowned item is flagged and removable in one action.
5. An order exceeding the owned quantity is flagged and correctable in one action.
6. An absent or stale snapshot produces `Unverifiable` throughout, and no flag.
7. Unlinking removes the stored credential, and the application keeps working.
8. No token, password, or account identifier appears in the log or diagnostics.

## Phases to follow

**Phase 2** posts new sell orders, reached by selecting an item in the collection. The ownership
check specified here gates it: the application does not list what the player does not hold.

**Phase 3** marks an order sold from an observed in-game trade. Trades are visible in `EE.log`, so
this extends the log machine that already drives reward detection rather than adding a capture
pipeline. `The trade was successful!` is emitted only after both parties accept and the exchange
settles, and is distinct from the prompt that opens a trade. The risk in that phase is identifying
the item rather than detecting the trade: the markers are English-only, the log does not carry an
Arcane's rank, and multi-item trades resolve poorly. The line shapes must be confirmed against a
captured trade before that phase is planned.
