#!/usr/bin/env python3
"""Read the relic reward cards off a screenshot and match them to the run's candidate pool.

This is deliberately not general OCR. The relic pool is known from EE.log before the screen even
renders, so each card only has to be matched to the nearest of ~24 known names. That turns a hard
recognition problem into a closed-set one, where a garbled read still lands on the right item.

Geometry is calibrated per resolution in CARD_BOXES; run with --dump to crop and print without
matching, which is how a new layout gets calibrated.

Usage: ocr_reward_cards.py <screenshot.png> [--dump]
"""

from __future__ import annotations

import difflib
import re
import subprocess
import sys
from pathlib import Path

# (x, y, width, height) of each card's title text, keyed by screenshot size. Calibrated from
# /tmp/tennoscope-screen-20260726-031440-1.png: four 242px-pitch cards starting at x=478, titles at
# y=430. The box is tall enough for a title that wraps to two lines, which means it also catches a
# few pixels of the divider below; the closed-set match absorbs that trailing noise.
CARD_BOXES: dict[tuple[int, int], list[tuple[int, int, int, int]]] = {
    (1920, 1080): [(478 + slot * 242, 430, 240, 48) for slot in range(4)],
}

PROJECTION_PREFIX = "/Lotus/Types/Game/Projections/"
LOG_PATH = Path(
    "$WINEPREFIX/drive_c/users/steamuser/"
    "AppData/Local/Warframe/EE.log"
)
CATALOG = Path("$XDG_DATA_HOME/org.warframehelper.app/catalog/relic-generation.json")


def png_size(path: Path) -> tuple[int, int]:
    import struct

    header = path.open("rb").read(24)
    return struct.unpack(">II", header[16:24])


def candidate_names() -> list[str]:
    """Every reward the relics in the current squad can produce."""
    import json

    lines = LOG_PATH.read_text(errors="replace").splitlines()
    opened = max(
        index for index, line in enumerate(lines) if "OpenVoidProjectionRewardScreen" in line
    )
    closed = max(
        (i for i, line in enumerate(lines[:opened]) if "Relic reward screen shut down" in line),
        default=0,
    )
    paths = set()
    for line in lines[closed:opened]:
        start = line.find(PROJECTION_PREFIX)
        if start < 0:
            continue
        remainder = line[start:]
        end = next(
            (i for i, ch in enumerate(remainder) if ch == ")" or ch.isspace()), len(remainder)
        )
        paths.add(remainder[:end])
    catalog = json.loads(CATALOG.read_text())
    return sorted(
        {
            reward["item"]["name"]
            for relic in catalog["catalog"]
            if relic.get("uniqueName") in paths
            for reward in relic.get("rewards", [])
        }
    )


def read_card(image: Path, box: tuple[int, int, int, int]) -> str:
    x, y, width, height = box
    crop = Path("/tmp/tennoscope-ocr-crop.png")
    subprocess.run(
        ["convert", str(image), "-crop", f"{width}x{height}+{x}+{y}", "+repage",
         "-colorspace", "gray", "-resize", "300%", str(crop)],
        check=True,
    )
    text = subprocess.run(
        ["tesseract", str(crop), "-", "--psm", "6"], capture_output=True, text=True, check=False
    ).stdout
    return re.sub(r"\s+", " ", text).strip()


def match(text: str, candidates: list[str]) -> tuple[str | None, float]:
    if not text:
        return None, 0.0
    best = difflib.get_close_matches(text, candidates, n=1, cutoff=0.0)
    if not best:
        return None, 0.0
    score = difflib.SequenceMatcher(None, text.lower(), best[0].lower()).ratio()
    return best[0], score


def main() -> int:
    image = Path(sys.argv[1])
    dump = "--dump" in sys.argv
    size = png_size(image)
    boxes = CARD_BOXES.get(size)
    if not boxes:
        print(f"no calibrated geometry for {size[0]}x{size[1]} - add it to CARD_BOXES", file=sys.stderr)
        return 1
    candidates = [] if dump else candidate_names()
    print(f"{len(candidates)} candidates in the relic pool")
    for slot, box in enumerate(boxes, start=1):
        text = read_card(image, box)
        if dump:
            print(f"  slot{slot} box={box} raw={text!r}")
            continue
        name, score = match(text, candidates)
        print(f"  slot{slot} raw={text!r:40} -> {name!r} ({score:.2f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
