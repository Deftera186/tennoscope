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
- **Leaking player data.** Anything that publishes another player's handle or account identifier
  — including in a debug capture, a test fixture, or a bundled screenshot.
- **Writing to game memory.** Every memory path in this project is read-only by design. A write,
  or anything that could be turned into one, is a vulnerability.
- **Escalation.** Anything that requires or encourages running as root, a setuid binary, or broad
  capabilities. The documented answer to a `ptrace_scope` failure is a user decision, never a
  privilege grab by the application.
- **Command injection** through the external tools the reward reader shells out to (`xwininfo`,
  `import`, `magick`, `tesseract`).
- **Catalog integrity.** The item catalog is fetched over the network and cached. A path that
  accepts an unvalidated or partial generation is in scope.

## What does not count

- Requiring `kernel.yama.ptrace_scope=0` on some systems. That is documented, and the trade-off is
  the user's to make.
- The account-policy risk of reading game memory at all. That is the disclosed premise of the
  project, not a defect — see the risk section in the [README](README.md).
- Anything in [`scripts/`](scripts). Those are unsupported research instruments, not shipped code.

## Supported versions

The latest release only. This project has not reached 1.0; older versions receive nothing.
