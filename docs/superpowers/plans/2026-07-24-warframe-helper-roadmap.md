# Warframe Helper Delivery Roadmap

The product specification is too broad for one safe implementation plan. Deliver it as five independently testable phases:

1. **Application foundation and fake-session vertical slice** — Rust workspace, domain interface, SQLite store, Tauri commands, collection browser, advisor overlay, and diagnostics driven by deterministic fake observations.
2. **Catalog and market data** — versioned item catalog import, stable identities, cached price observations, offline behavior, and freshness display.
3. **Linux game acquisition** — Wine/Proton process discovery, `EE.log` adapter, signed reader definitions, read-only memory adapter, coherent snapshot validation, and automatic synchronization.
4. **Screen observation and live overlay** — PipeWire portal and X11 capture adapters, English reward recognition, confidence handling, click-through positioning, and multi-monitor behavior.
5. **Distribution and release engineering** — Arch, Gentoo, Debian/Ubuntu, Fedora, AppImage, signed reader-definition publishing, and the X11/Wayland release matrix.

Each phase must leave the application runnable. Later phases replace fake adapters through existing interfaces rather than changing domain or presentation contracts.

