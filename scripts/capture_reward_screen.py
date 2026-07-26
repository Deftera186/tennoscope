#!/usr/bin/env python3
"""Grab frames of the Warframe window while the relic reward screen is up.

Warframe runs under Proton as an XWayland client, so the window is reachable through plain X11 and
needs no compositor portal. The reward screen lives for 15 seconds; a handful of frames across it is
enough to pin the card geometry and to feed a closed-set OCR match of the three remote cards.

Usage: capture_reward_screen.py [seconds ...]
"""

from __future__ import annotations

import re
import subprocess
import sys
import time
from pathlib import Path

WINDOW = re.compile(r'(0x[0-9a-f]+) "Warframe":')
DEFAULT_DELAYS = (0.4, 1.5, 3.0, 6.0, 10.0)


def window_id() -> str:
    tree = subprocess.check_output(["xwininfo", "-root", "-tree"], text=True)
    for line in tree.splitlines():
        found = WINDOW.search(line)
        # Skip the 1x1 IME helpers that share the class name.
        if found and re.search(r"\s\d{3,}x\d{3,}", line):
            return found.group(1)
    raise SystemExit("no Warframe window found")


def main() -> int:
    delays = [float(value) for value in sys.argv[1:]] or list(DEFAULT_DELAYS)
    window = window_id()
    stamp = time.strftime("%Y%m%d-%H%M%S")
    started = time.monotonic()
    for index, delay in enumerate(delays):
        remaining = delay - (time.monotonic() - started)
        if remaining > 0:
            time.sleep(remaining)
        out = Path(f"/tmp/tennoscope-screen-{stamp}-{index}.png")
        subprocess.run(["import", "-window", window, str(out)], check=False)
        print(f"SCREEN_SAVED {out} at +{delay}s", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
