# Security Policy

## Reporting a vulnerability

Report privately through GitHub's [private vulnerability
reporting](https://github.com/Deftera186/tennoscope/security/advisories/new). Please do not open a
public issue for anything in the categories below.

This is a single-maintainer hobby project. Expect an acknowledgement within a week and a fix when
one is possible; there is no paid support and no bounty.

## What counts as a vulnerability here

TennoScope has no server, no account, and no network service, so the usual web threat model does
not apply. What matters instead:

- **Leaking session credentials.** The account identifier and nonce read from game memory are live
  session credentials. Any path that writes them to disk, to a log, into the database, into a
  crash dump, or over the network is a vulnerability, not a bug.
- **Leaking player data.** Anything that publishes another player's handle or account identifier,
  including in a debug capture, a test fixture, or a bundled screenshot.
- **Writing to game memory.** Every memory path in this project is read-only by design. A write,
  or anything that could be turned into one, is a vulnerability.
- **Escalation.** Anything that requires or encourages running as root, a setuid binary, or broad
  capabilities. The documented answer to a `ptrace_scope` failure is a user decision, never a
  privilege grab by the application.
- **Running the wrong external program.** The reward reader spawns exactly two and nothing else:
  `tesseract` on every platform, and `xwininfo -root -tree` on Linux, which is how Warframe is
  found when Wine's virtual desktop hides it from the EWMH list. Both are spawned directly, never
  through a shell, with a fixed argument list, so there is no shell quoting to get wrong. What is
  in scope is a path that decides which binary runs: on Windows `tesseract` has to resolve to the
  copy the installer bundles, not to whatever a `PATH` lookup turns up.
- **Catalog integrity.** The item catalog is fetched over the network and cached. A path that
  accepts an unvalidated or partial generation is in scope.

## What does not count

- Requiring `kernel.yama.ptrace_scope=0` on some systems. That is documented, and the trade-off is
  the user's to make.
- The account-policy risk of reading game memory at all. That is the disclosed premise of the
  project, not a defect. The [README](README.md#you-decide-what-it-may-touch) is where that
  premise is stated.
- The Python research instruments in [`scripts/`](scripts), the ones
  [`scripts/README.md`](scripts/README.md) lists. They need a live Wine session, no CI job runs
  them, and they are kept as the evidence behind [`docs/research/`](docs/research). That
  exclusion is the file, not the language: every other script in `scripts/` either lands in
  a shipped artifact or gates one, so it is in scope like any other code here. The two
  exceptions are `check-windows.sh`, which no workflow cross-compiles through, and
  `test-build-apt-repo.sh`, which covers the APT repository generator outside CI.

## Supported versions

The latest release only; nothing older is promised compatible.

The mechanism is narrow by construction.
[`plugins.updater.endpoints`](app/src-tauri/tauri.conf.json) is one URL,
`releases/latest/download/latest.json`, and `build-update-feed.sh` writes one entry per artifact
it is handed, so a build can only ever be offered the newest stable release and never an
intermediate one. Anything that arrived through a package manager (deb, rpm, the APT repository,
COPR, the Arch recipes, winget, the Gentoo overlay) is updated by that package manager instead.
