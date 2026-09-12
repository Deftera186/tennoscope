# Native Wayland support for reward capture and the overlay

Date: 2026-08-27
Status: revised and approved in chat on 2026-08-29; written review pending
Bug: ordinary KDE Wayland users were forced through ScreenCast setup even when Warframe was an
XWayland client that TennoScope could capture directly. Native-Wayland KDE capture also exposed an
opaque monitor chooser and a persistent screen-sharing indicator.
Outcome: capture follows the game protocol, not the desktop session. X11/XWayland games use their
X11 drawable. Native-Wayland games use wlroots screencopy where available, KWin ScreenShot2 on KDE,
and an explicitly authorized portal session only as the final fallback. Screen capture is never a
global first-run requirement and reward polling never negotiates with the portal.

## The defect

From the user's report (`2026-08-22-231838279`):

| Time | Poller failure | Count |
|---|---|---|
| 14:51:51 → 15:49:45 | `no Warframe window found` | 449 |
| 22:15:54 → 22:41:24 | `blank` / `did not match the relic pool` | 507 |
| 23:06:04 → 23:14:25 | `no Warframe window found` | 250 |

The 22:14 session worked — three `reward: published cards=[...]` lines. The 23:04 session
never located the window once, and the attached EE.log covers exactly that session:
`-windowMode:2`, `Borderless mode. Desktop resolution is 1920x1080`.

EE.log parsing was not at fault. The second reward screen (`728.962`, ≈23:13:54) has a full
squad roster, `gets reward /Lotus/.../DualZorenPrimeBlueprint`, and two
`ProjectionRewardChoice.lua: Missing icon data!` lines, i.e. `expected_choices = 2`. The
attached video at 4s shows exactly those two cards. Every log-side input was present.

### Root cause, confirmed by experiment

`warframe_window_rect()` (`reward_ocr.rs:423-448`) has two mechanisms and both read X11:
`xcap::Window::all()` over `_NET_CLIENT_LIST_STACKING`, then `xwininfo -root -tree`. With
`PROTON_ENABLE_WAYLAND=1` the game is a native Wayland surface and appears in neither.

Verified directly, on both compositors, with a native-Wayland GTK client titled `Warframe`:

- sway 1.12: absent from `xwininfo -root -tree`
- KWin 6 (nested): absent from `xwininfo -root -tree`

Wayland does not expose another client's absolute coordinates to anyone. This is deliberate,
so there is no Wayland equivalent of `warframe_window_rect()` to port.

### The second failure, which the report misattributes

`try_publish_player_records` (`lib.rs:1703`) calls `visual_choices`, which retries until its
8s deadline and returns `None` (`reward_source.rs:287`). `lib.rs:1656-1660` reports that as
`record_capture_degraded("Structured reward records were incomplete")`. The log parsing was
complete; capture failed. **The health message names the wrong subsystem**, which is why the
report's `Reward observer: degraded` line points investigators away from the actual fault.

## Decision: select capture by game protocol and compositor capability

The app remains an X11 GTK application. The overlay still uses its proven XWayland
override-redirect path; no `gtk-layer-shell`, process split, or WebKitGTK backend change is needed.
Capture selection is independent of that UI backend:

```
Warframe visible through X11          -> X11 rect + X11 drawable
Native Wayland + wlroots screencopy   -> Wayland output rect + screencopy frame
Native Wayland + KWin ScreenShot2     -> Wayland output rect + KWin raw frame
Native Wayland + neither direct API   -> portal stream rect + PipeWire frame
```

X11 wins whenever it finds Warframe, including on a KDE or sway Wayland session. This is the normal
Proton configuration without `PROTON_ENABLE_WAYLAND=1`; it must not show capture setup, a chooser,
or a sharing indicator. A desktop being Wayland is not evidence that the game is native Wayland.

Only a running game that is absent from X11 needs native-Wayland capture. The monitor thread already
proves that Warframe is running before reward capture is attempted, so absence from X11 at that
point is the game-protocol signal. Inventory and log monitoring never depend on visual capture.

## Direct KDE capture: KWin ScreenShot2

KWin 6.7.4 exposes `org.kde.KWin.ScreenShot2` version 5 at
`/org/kde/KWin/ScreenShot2`. `CaptureScreen` accepts a compositor output name, an options map, and a
Unix pipe file descriptor. It replies with `type`, `format`, `width`, `height`, `stride`, and `scale`,
then writes the raw `QImage` rows to the pipe. It does not create a ScreenCast session, open a
chooser, or light KDE's sharing indicator.

Add `reward_capture/kwin.rs` with one responsibility: turn KWin output captures into the existing
`(WindowRect, MonitorFrame)` contract. It will:

1. Probe the DBus service and require ScreenShot2 version 4 or newer. Version 4 introduced the
   returned screen name and scale needed to validate the result; version 5 adds caller-window
   filtering.
2. Discover each Wayland output's compositor name and logical geometry through `wl_output` plus
   `zxdg_output_manager_v1`. KWin's `CaptureScreen` name is the same compositor output name.
3. Call `CaptureScreen` with cursor excluded, native resolution disabled, and caller windows hidden
   where API version 5 supports it. Logical-size frames keep the existing crop coordinates valid.
4. Read the pipe concurrently with the delayed DBus reply so KWin cannot block on a full pipe.
5. Validate the result type, supported `QImage::Format`, non-zero dimensions, stride, scale, checked
   row length, and checked total buffer size before allocating or copying.
6. Convert rows directly into one `RgbaImage`, honoring stride and channel order. No encoded-image
   round trip and no whole-workspace capture.
7. Return every successfully captured output as an OCR candidate. One failed output does not discard
   usable candidates; all failures return one stable capture error.

KWin renders screen captures as `QImage::Format_RGBX8888` today. The decoder will explicitly support
the compatible 32-bit formats KWin can report and reject unknown formats rather than reinterpret
bytes. Framebuffer dimensions and `scale` are checked against the output's logical geometry; logical
geometry remains the `MonitorFrame` coordinate system.

### KWin authorization and packaging

ScreenShot2 is a restricted KDE DBus interface. Every installed desktop entry must declare:

```
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
```

Update both `packaging/tennoscope.desktop` and Tauri-generated Linux package metadata so Arch,
AppImage, deb, and rpm installations carry the same permission declaration. The executable identity
must remain `tennoscope`; no helper process is introduced because KWin authorizes the DBus caller.

Development binaries may not match an installed desktop entry and can therefore be rejected by
KWin. The live harness must report that authorization failure explicitly and document how to run a
packaged or locally registered TennoScope binary. It must not disable KWin's permission checks.

## Portal fallback

Portal ScreenCast + PipeWire remains the compatibility fallback for native-Wayland compositors that
offer neither wlroots screencopy nor KWin ScreenShot2. Existing stream conversion, restore-token
storage, dedicated worker ownership, frame deadlines, and explicit close behavior remain.

The fallback is demand-driven:

- A saved grant is capability state, not global setup completion.
- Runtime capture never restores or negotiates a portal session. A restore token may legally be
  ignored by the portal and open its chooser, so replaying it from gameplay cannot satisfy the
  no-surprise contract.
- Restore-token replay and first authorization remain reachable only from the explicit
  `Allow desktop capture` Settings action. A valid token may make that requested action silent; a
  stale token may open the chooser the player just asked for.
- The chooser asks for every monitor where Warframe may run because Wayland exposes no other
  client's window geometry. Portal stream positions and sizes become OCR candidates.
- The live session exists only while the game needs it. Dropping it clears the desktop sharing
  indicator.

The existing direct PipeWire fd read is retained. `StreamFlags::MAP_BUFFERS` proved unsafe with this
stack: the negotiated `MemFd` pointer was non-null but unmapped and faulted on its first byte. Reads
therefore continue through the fd while honoring chunk offset, size, and row stride.

## Setup and Settings UX

Screen capture is removed from first-run setup. Accepting the local read-only risk disclosure always
starts normal monitoring. Capture availability cannot block collection, log, or market features.

Settings describes the default rather than exposing implementation jargon:

> Screen capture is automatic. Desktop sharing is only needed when Warframe is launched with
> `PROTON_ENABLE_WAYLAND=1` and this compositor has no direct capture API.

No capture control appears for X11/XWayland, wlroots direct capture, or KWin direct capture. The
portal action appears only when the running native-Wayland game has no direct backend or diagnostics
have established that the portal is the remaining backend. Before opening it, the UI states all
observable consequences:

- the desktop will open its own screen chooser;
- select every display where Warframe may run;
- KDE/GNOME may show an active screen-sharing indicator while reward capture is available;
- TennoScope releases the session when the game exits.

The action is named `Allow desktop capture`, not `Captured screens` or `Start screen capture`.
Installed builds identify as TennoScope through their desktop entry. A development launch may be
identified by its terminal on portal implementations; direct KWin capture avoids that path.

## Runtime structure

`reward_capture/mod.rs` owns backend precedence and the common `MonitorFrame` contract:

- `reward_capture/x11.rs` — X11 window discovery and drawable capture
- `reward_capture/direct.rs` — wlroots screencopy
- `reward_capture/kwin.rs` — KWin DBus capture and raw `QImage` conversion
- `reward_capture/portal/` — explicit portal authorization and PipeWire fallback

`reward_ocr.rs` keeps geometry, cropping, OCR, and matching. The held `GameCapture` feeds the same
`visible_region` / `window_frame_from_monitor` path from every backend, preserving multi-monitor and
fractional-scale invariants without rebuilding connections per poll.

Backend availability is capability-based. Desktop-name environment variables may prioritize a KWin
probe for diagnostics, but cannot prove availability. Selection order is X11, wlroots, KWin, portal.
If a direct native backend fails during capture, diagnostics name that backend. Portal capture is
used only when the explicit Settings action has installed a live worker session; direct-backend
failure or missing portal permission during gameplay never opens the chooser.

## Error handling and diagnostics

Capture errors name the selected backend and remediation. Examples:

- `KWin screen capture was not authorized; run the installed TennoScope build`
- `KWin returned an unsupported screenshot format`
- `Desktop capture permission is required for native Wayland Warframe`

The report header adds `kwin` as a frame-backend label. Capture-shape logging remains Info only when
the source or geometry changes, Debug otherwise. Routine OCR misses remain Debug; backend failures
remain reportable. No error from visual capture degrades unrelated inventory or EE.log health.

## Dependencies

Use the `zbus` version already locked transitively through `ashpd`, promoted to a direct Linux-only
dependency with its current Tokio integration. Use existing `wayland-client` and
`wayland-protocols` for output discovery. Unix pipe creation and fd ownership use the standard
library plus the already locked low-level Unix crate where required. No Qt, KDE Frameworks,
external screenshot tool, helper executable, or new system library is added.

## Correctness fixes found along the way

**OCR match is fragile, not just on Wayland.** `best_match` (`reward_ocr.rs:720-735`)
normalises the whole OCR blob before scoring. On the real frame, slot 1 reads as three noise
fragments plus `Forma Blueprint` and scores **0.636** against `MATCH_FLOOR = 0.6`
(`reward_ocr.rs:87`). One more speck of noise and a correct read is discarded.

Fix: score the full text *and* each line-group *and* trailing suffix runs, keep the best.
Validated against the real strings:

| Read | Current | Proposed |
|---|---|---|
| `Dual Zoren Prime\n\nBlueprint` (wrapped name) | 1.000 | 1.000 |
| `&\n\nvr\n\ni STrTl\n\nForma Blueprint` | 0.636 | 1.000 |
| pure noise, three variants | rejected | rejected |

The full-text candidate is what keeps wrapped names working; the guard cases confirm the
floor still rejects garbage.

**`xwininfo` fails silently.** `unwrap_or_default()` (`reward_ocr.rs:465-472`) turns a missing
binary into an empty string, indistinguishable from "no match". Log the distinction.

**The Linux notice is Windows-gated.** `borderless_notice` (`overlay_window.rs:104-112`)
returns `None` whenever `!cfg!(windows)`, so a Linux user whose game cannot be found is told
nothing at all. Give it a Linux message.

## Diagnostics

The report actively misled here, in three separate ways.

**Wrong subsystem named.** Plumb the capture failure reason through `visual_choices` so the
health message says what failed — `Screen capture failed: no Warframe window found` — instead
of blaming log parsing. `try_publish_player_records` returning `false` must not collapse two
unrelated causes into one message.

**Real failures buried.** report.txt's "Recent warnings and errors" (`report.rs:177`) showed
the 22:15-22:41 `blank` / `did not match` warnings, which are *normal*: the poller runs every
2s for a whole fissure run and is usually looking at gameplay, not a reward screen. 507 lines
of routine noise crowded out the 250 real ones. Demote routine per-poll failures to Debug and
emit a WARN only when a failure reason persists past a threshold, so a warning in the report
means something is actually wrong.

**The decisive line was filtered out.** `[DEBUG-capture]` (`reward_ocr.rs:297`) is what
distinguishes "no window" from "wrong monitor" from "captured an XWayland helper", and stable
builds cap the file target at Info (`lib.rs:2766`). Logging it per poll at Info would flood a
5 MiB rotation, so log it at Info *only when the capture configuration changes* — memoized on
rect, monitor origin and region. Reports then carry the geometry without the flood, which is
what the original comment at `lib.rs:2780-2786` was worried about.

Add the session type, chosen capture backend, and stream geometry to the report header. Every
one of those was something I had to infer for this bug.

## Testing

Unit tests without a display server:

- backend precedence: X11 beats every native backend; wlroots beats KWin; KWin beats portal;
- setup state never blocks first run and exposes the portal action only for the actual fallback;
- KWin capability parsing rejects absent service and API versions below 4;
- raw KWin rows convert supported channel orders while honoring padded stride;
- truncated rows, zero dimensions, overflow, unknown type/format, and inconsistent logical scale are
  rejected before image construction;
- multi-output logical geometry survives negative origins and fractional scaling;
- partial KWin output failure keeps successful OCR candidates;
- report labels distinguish `x11`, `wayland`, `kwin`, and `portal`.

Frontend tests assert the exact user contract:

- ordinary KDE Wayland/XWayland startup never invokes capture authorization;
- capture setup is absent from first run;
- direct KWin support has no `Captured screens` control;
- portal fallback explains the system chooser and sharing indicator before `Allow desktop capture`;
- declining or failing capture leaves the rest of the application usable.

The ignored live capture harness remains the integration boundary because CI has no compositor. Run
it on KDE in two modes:

1. Warframe without `PROTON_ENABLE_WAYLAND=1`: capture succeeds through `x11`; no chooser or sharing
   indicator appears.
2. Warframe with `PROTON_ENABLE_WAYLAND=1`: capture succeeds through `kwin`; no chooser or sharing
   indicator appears.

For each mode, the report header and capture-shape line must name the observed backend and monitor
geometry. A live reward screen is the final behavioral check: OCR publishes the visible cards and
the overlay lands on the monitor containing the game.

## Risks and boundaries

- ScreenShot2 is KDE-specific and restricted. A package missing desktop metadata fails closed and
  falls back only to an already authorized portal session; it never weakens KWin permissions.
- KWin emits raw Qt image formats. The decoder accepts a deliberately small audited set; a new
  format produces a diagnostic rather than corrupted OCR input.
- Native-Wayland games still expose no window rectangle. All direct native backends assume Warframe
  is Borderless or Fullscreen on one output. Windowed native-Wayland mode is unsupported and gets an
  explicit message.
- Portal chooser and sharing UI remain unavoidable on compositors without a direct API. They are no
  longer imposed on KDE, wlroots, or any XWayland game.
- GNOME portal behavior remains unverified on this hardware. This design does not claim prompt-free
  GNOME capture.
- The overlay remains XWayland override-redirect. It is proven on sway and nested KWin; the live KDE
  verification above is required before claiming end-to-end KDE native-Wayland support.
- Keyboard handling is out of scope. TennoScope does not grab or inject input, and the reported
  gameplay-key issue occurred with TennoScope absent and Warframe running through XWayland.
