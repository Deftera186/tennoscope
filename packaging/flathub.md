# Flathub — verdict: deferred

Spike date: 2026-09-23. No manifest written on purpose.

## Why not

The sandbox denies exactly what the app is:

- **Process memory reads are impossible confined.** Overlay and Full access
  read `/proc/<pid>/maps` + `/proc/<pid>/mem` of the running game. Flatpak
  isolates the PID namespace and grants no ptrace capability; no `--socket`,
  `--share`, or `--device` flag restores it. There is no portal for "read
  another process's memory", by design.
- **Capture degrades to portal-only.** The KWin ScreenShot2 fast path needs a
  desktop entry the sandbox cannot provide; only the portal fallback survives,
  which the app already has but has never been the primary path.
- **Policy risk.** Flathub reviews host-dependent apps case-by-case; an app
  whose headline features silently no-op inside the sandbox is exactly what
  the policy screens out.

What *could* ship is Companion mode only (catalog, prices, no game reads) —
under the same name that would confuse every installer, and as a separate
`...tennoscope-companion` ID it doubles the maintenance for the least
capable edition. Neither earns its place while COPR, APT, the Gentoo overlay,
and the native Arch PKGBUILD cover their platforms; the winget manifest is
submitted but not merged, and there is no AUR package.

## Revisit if

- A companion-only edition becomes an explicit product decision (then: MetaInfo,
  bare-Exec desktop, offline vendoring via flatpak-builder-tools, sandbox
  behavior spike with `--socket=wayland --share=network` only).
- Flathub policy or portals ever cover process inspection (don't hold breath).
