# Flathub: verdict deferred

Spike date: 2026-09-23. No manifest was written on purpose: there is no `packaging/flatpak/`.

## Why not

The sandbox denies exactly what the app is:

- **Full cannot be confined.** Full reads `/proc/<pid>/maps` + `/proc/<pid>/mem` of the
  running game. Flatpak isolates the PID namespace and grants no ptrace capability, and
  no `--socket`, `--share` or `--device` flag restores it. There is no portal for
  "read another process's memory", by design. Overlay and Companion are the modes that
  stay inside their boundary; Full is the one that would break.
- **Overlay would need its own spike.** It reads the host's `EE.log` and the game's own
  X11 drawable, neither of which a sandbox hands over whole. The KWin rung authorizes a
  caller by matching the desktop entry's `Exec` against the running executable, which is
  why an AppImage is refused outright; whether a Flatpak export satisfies the same
  comparison was not tested.
- **Policy risk.** Flathub reviews host-dependent apps case by case, and an app whose
  headline features silently no-op inside the sandbox is what that review screens out.

Companion is the mode that needs none of it: catalog, saved collection, prices and ducats,
with nothing read from Warframe. It could ship under the same name, which would confuse
every installer who came for the overlay, or as a separate `...tennoscope-companion` ID,
which doubles the maintenance for the least capable edition. Neither earns its place while
COPR, the APT repository, the Gentoo overlay and the Arch recipes already give every
supported platform a route. The winget manifest is submitted upstream but not merged, and
there is no AUR package.

## Revisit if

- A companion-only edition becomes an explicit product decision (then: MetaInfo,
  bare-Exec desktop, offline vendoring via flatpak-builder-tools, sandbox behavior
  spike with `--socket=wayland --share=network` only).
- Flathub policy or portals ever cover process inspection (don't hold breath).
