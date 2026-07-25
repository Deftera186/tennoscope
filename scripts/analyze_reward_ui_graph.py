#!/usr/bin/env python3
"""Summarize genuine 64-bit reward-object references in archived captures."""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path


def true_pointer(reference: dict[str, object]) -> bool:
    context = bytes.fromhex(str(reference.get("context", "")))
    target = int(reference["object_address"])
    return len(context) >= 1032 and context[1024:1032] == struct.pack("<Q", target)


def summarize(path: Path) -> dict[str, object]:
    capture = json.loads(path.read_text())
    references = [
        reference
        for reference in capture.get("object_references", [])
        if reference.get("kind") == "reward"
        and reference.get("name")
        and true_pointer(reference)
    ]
    return {
        "capture": path.name,
        "projection_count": len(capture.get("projection_paths", [])),
        "candidate_count": len(capture.get("candidate_names", [])),
        "references": [
            {
                "name": reference["name"],
                "reference_mapping": reference["mapping"],
                "reference_alignment": int(reference["reference"]) % 8,
            }
            for reference in references
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "captures",
        nargs="*",
        type=Path,
        default=sorted(Path("/tmp").glob("tennoscope-order-*.json")),
    )
    args = parser.parse_args()
    summaries = [summary for path in args.captures if (summary := summarize(path))["references"]]
    print(json.dumps(summaries, indent=2))


if __name__ == "__main__":
    main()
