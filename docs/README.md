# Documentation

## [`design/`](design)

The design decisions behind the shipped code, written before the code and left as the record of
*why* each subsystem is shaped the way it is.

| Document | Subject |
| --- | --- |
| [architecture.md](design/architecture.md) | Overall shape: Rust workspace, Tauri shell, local-only persistence, Linux-first constraints. |
| [product-pass.md](design/product-pass.md) | Turning the vertical slice into TennoScope: identity, artwork, pagination, freshness. |
| [memory-reward-recognition.md](design/memory-reward-recognition.md) | Recognising the squad's reward choices from game memory, gated on the log's reward lifecycle. |
| [player-reward-records.md](design/player-reward-records.md) | Resolving rewards from the transient per-player response records. |
| [persistent-reward-ui.md](design/persistent-reward-ui.md) | Resolving reward order from the persistent reward-screen object graph. |

The three reward documents are successive attempts at the same problem. All three are kept: the
memory path is still live code, and the shipped answer — reading the screen — only makes sense
against what it replaced.

## [`research/`](research)

Findings from live investigation, with the evidence they rest on. These are the load-bearing
documents: where a constant in the code looks arbitrary, its justification is usually here.

| Document | Subject |
| --- | --- |
| [relic-reward-resolution.md](research/relic-reward-resolution.md) | How relic rewards are actually resolved, and what each rejected approach failed at. |
| [relic-reward-source-options.md](research/relic-reward-source-options.md) | The candidate sources for reward data, compared. |
| [memory-reward-live-validation.md](research/memory-reward-live-validation.md) | Live validation of the memory path across runs. |
| [2026-07-24-live-warframe-acquisition-spike.md](research/2026-07-24-live-warframe-acquisition-spike.md) | The first end-to-end inventory acquisition against a live session. |
| [warframe-acquisition-existing-implementations.md](research/warframe-acquisition-existing-implementations.md) | Prior art: what other tools do and where this one diverges. |

The instruments that produced this evidence are in [`../scripts`](../scripts).

Captures under `research/evidence/` are published with account identifiers and display names
removed; see [SECURITY.md](../SECURITY.md) for what this project treats as sensitive.
