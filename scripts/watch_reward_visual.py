#!/usr/bin/env python3
"""Fire the memory captures the moment the reward screen is visible.

Every memory capture so far was triggered by "ProjectionRewardChoice.lua: Got rewards" in EE.log.
Warframe flushes that log seconds after the fact -- measured at ~7.5s on 2026-07-27 -- so the
captures were starting at or after the point where the fifteen-second screen closed, and running for
another 7-50 seconds beyond that.

That makes every conclusion drawn from them suspect. The four reward path strings kept turning up
because interned strings outlive the screen; the per-player structures that would tie a reward to a
player are exactly the kind of thing freed when the screen tears down. "No such record exists on a
client" and "we were looking after it was freed" produce identical evidence.

Detecting the screen visually removes the lag, so this is the first capture that is genuinely inside
the window.
"""

from __future__ import annotations

import re
import subprocess
import time
from pathlib import Path

WINDOW = re.compile(r'(0x[0-9a-f]+) "Warframe":')
HEADER_CROP = "620x60+270+35"
PROBE_INTERVAL = 0.6
CAPTURE = Path(__file__).with_name("capture_reward_players.py")
SWEEP = Path(__file__).with_name("scan_response_records.py")


def window_id() -> str | None:
    try:
        tree = subprocess.check_output(["xwininfo", "-root", "-tree"], text=True)
    except subprocess.CalledProcessError:
        return None
    for line in tree.splitlines():
        found = WINDOW.search(line)
        if found and re.search(r"\s\d{3,}x\d{3,}", line):
            return found.group(1)
    return None


def reward_screen_visible(window: str) -> bool:
    """True when the reward screen's header is on screen."""
    subprocess.run(["import", "-window", window, "ppm:/tmp/tennoscope-probe.ppm"], check=False)
    subprocess.run(
        ["magick", "/tmp/tennoscope-probe.ppm", "-crop", HEADER_CROP, "+repage",
         "-colorspace", "gray", "-resize", "200%", "/tmp/tennoscope-probe.png"],
        check=False,
    )
    text = subprocess.run(
        ["tesseract", "/tmp/tennoscope-probe.png", "-", "--psm", "7"],
        capture_output=True, text=True, check=False,
    ).stdout
    return "FISSURE" in text.upper()


def main() -> None:
    print("VISUAL_TRIGGER_ARMED", flush=True)
    while True:
        window = window_id()
        if not window:
            time.sleep(2)
            continue
        if not reward_screen_visible(window):
            time.sleep(PROBE_INTERVAL)
            continue

        print(f"REWARD_SCREEN_VISIBLE {time.strftime('%H:%M:%S')}", flush=True)
        # Screenshot first so the frame that proved the screen was up is kept alongside the memory.
        subprocess.run(
            ["import", "-window", window, f"/tmp/tennoscope-visible-{time.strftime('%H%M%S')}.png"],
            check=False,
        )
        players = subprocess.Popen(["python3", str(CAPTURE)])
        sweep = subprocess.Popen(["python3", str(SWEEP)])
        players.wait()
        sweep.wait()
        print("VISUAL_TRIGGER_COMPLETE", flush=True)
        # Do not re-fire on the same screen.
        while reward_screen_visible(window):
            time.sleep(1)


if __name__ == "__main__":
    main()
