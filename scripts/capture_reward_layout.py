#!/usr/bin/env python3
"""Capture reward-card index tags and nearby candidate names."""

from __future__ import annotations

import importlib.util
import json
import os
import time
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "capture_reward_order", Path(__file__).with_name("capture_reward_order.py")
)
capture = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(capture)


def main() -> None:
    started = time.monotonic()
    pid = capture.process_id()
    paths = capture.current_projection_paths()
    names = capture.candidate_names(paths)
    mappings = capture.readable_writable_maps(pid)
    memory = os.open(f"/proc/{pid}/mem", os.O_RDONLY)
    try:
        links = capture.scan_layout_links(memory, mappings, names)
    finally:
        os.close(memory)
    timestamp = int(time.time() * 1000)
    destination = Path(f"/tmp/tennoscope-layout-{timestamp}.json")
    destination.write_text(
        json.dumps(
            {
                "elapsed_ms": round((time.monotonic() - started) * 1000),
                "projection_paths": paths,
                "candidate_names": names,
                "layout_links": links,
            },
            indent=2,
        )
    )
    print(
        f"LAYOUT_CAPTURE_SAVED path={destination} links={len(links)} "
        f"elapsed_ms={round((time.monotonic() - started) * 1000)}",
        flush=True,
    )


if __name__ == "__main__":
    main()
