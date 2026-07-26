#!/usr/bin/env python3
"""Capture bounded Warframe reward-string/object evidence during the reward screen."""

from __future__ import annotations

import json
import os
import re
import struct
import subprocess
import time
from pathlib import Path


LOG_PATH = Path(
    "$WINEPREFIX/pfx/drive_c/users/steamuser/"
    "AppData/Local/Warframe/EE.log"
)
CATALOG_PATH = Path(
    "$XDG_DATA_HOME/org.warframehelper.app/catalog/relic-generation.json"
)
PROJECTION_PREFIX = "/Lotus/Types/Game/Projections/"
PRIMARY_ADDRESS_MIN = 0x1300_0000
PRIMARY_ADDRESS_MAX = 0x2800_0000
REFERENCE_ADDRESS_MIN = 0x1800_0000
REFERENCE_ADDRESS_MAX = 0x6000_0000
READ_CHUNK = 4 * 1024 * 1024
LAYOUT_RADIUS = 4096


def process_id() -> int:
    output = subprocess.check_output(
        ["pgrep", "-f", "Warframe.x64.exe"], text=True
    ).splitlines()
    return int(output[0])


def current_projection_paths() -> list[str]:
    lines = LOG_PATH.read_text(errors="replace").splitlines()
    open_index = max(
        index
        for index, line in enumerate(lines)
        if "OpenVoidProjectionRewardScreen" in line
    )
    previous_close = max(
        (
            index
            for index, line in enumerate(lines[:open_index])
            if "Relic reward screen shut down" in line
        ),
        default=0,
    )
    paths: list[str] = []
    for line in lines[previous_close:open_index]:
        start = line.find(PROJECTION_PREFIX)
        if start < 0:
            continue
        remainder = line[start:]
        end = next(
            (
                index
                for index, character in enumerate(remainder)
                if character == ")" or character.isspace()
            ),
            len(remainder),
        )
        path = remainder[:end]
        if path not in paths:
            paths.append(path)
    return paths


def candidate_names(paths: list[str]) -> list[str]:
    catalog = json.loads(CATALOG_PATH.read_text())
    wanted = set(paths)
    return sorted(
        {
            reward["item"]["name"]
            for relic in catalog["catalog"]
            if relic.get("uniqueName") in wanted
            for reward in relic.get("rewards", [])
        }
    )


def readable_writable_maps(pid: int) -> list[tuple[int, int, str]]:
    mappings = []
    for line in Path(f"/proc/{pid}/maps").read_text().splitlines():
        parts = line.split()
        if len(parts) < 2 or not parts[1].startswith("rw"):
            continue
        start, end = (int(value, 16) for value in parts[0].split("-"))
        mappings.append((start, end, line))
    return mappings


def scan_exact_strings(
    memory: int,
    mappings: list[tuple[int, int, str]],
    names: list[str],
) -> list[dict[str, object]]:
    encoded: dict[bytes, tuple[str, str]] = {}
    for name in names:
        encoded[name.encode()] = (name, "display_utf8")
        encoded[name.encode("utf-16-le")] = (name, "display_utf16")
        compact = name.replace(" ", "")
        encoded[compact.encode()] = (name, "compact_utf8")
    matcher = re.compile(
        b"|".join(
            re.escape(value) for value in sorted(encoded, key=len, reverse=True)
        )
    )
    hits: list[dict[str, object]] = []
    # Reward labels are short-lived UI allocations. Newer/high mappings have
    # consistently contained the live strings, so inspect them first.
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= PRIMARY_ADDRESS_MIN or start >= PRIMARY_ADDRESS_MAX:
            continue
        position = max(start, PRIMARY_ADDRESS_MIN)
        limit = min(end, PRIMARY_ADDRESS_MAX)
        carry = b""
        while position < limit:
            request = min(READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            blob = carry + data
            base = position - len(carry)
            for match in matcher.finditer(blob):
                name, representation = encoded[match.group()]
                terminator_len = 2 if representation == "display_utf16" else 1
                if (
                    match.end() + terminator_len > len(blob)
                    or blob[match.end() : match.end() + terminator_len]
                    != b"\0" * terminator_len
                ):
                    continue
                address = base + match.start()
                try:
                    context = os.pread(memory, 128, max(0, address - 48))
                except OSError:
                    context = b""
                hits.append(
                    {
                        "address": address,
                        "name": name,
                        "representation": representation,
                        "mapping": mapping,
                        "context": context.hex(),
                    }
                )
            carry = blob[-128:]
            position += len(data)
    return hits


def scan_references(
    memory: int,
    mappings: list[tuple[int, int, str]],
    hits: list[dict[str, object]],
) -> list[dict[str, object]]:
    patterns: dict[bytes, list[tuple[int, int, str]]] = {}
    for hit in hits:
        if hit.get("representation") != "display_utf8":
            continue
        address = int(hit["address"])
        name = str(hit["name"])
        # Warframe's live UI string object begins 24 bytes before its text.
        # Searching speculative deltas generated many random byte matches and
        # made the capture outlive the reward screen.
        delta = -24
        encoded = struct.pack("<I", (address + delta) & 0xFFFF_FFFF)
        patterns.setdefault(encoded, []).append((address, delta, name))
    if not patterns:
        return []
    references: list[dict[str, object]] = []
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= REFERENCE_ADDRESS_MIN or start >= REFERENCE_ADDRESS_MAX:
            continue
        position = max(start, REFERENCE_ADDRESS_MIN)
        limit = min(end, REFERENCE_ADDRESS_MAX)
        while position < limit:
            request = min(READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            for encoded, targets in patterns.items():
                search_from = 0
                while True:
                    found = data.find(encoded, search_from)
                    if found < 0:
                        break
                    search_from = found + 1
                    reference = position + found
                    # Real UI fields observed so far are naturally aligned.
                    # Rejecting unaligned byte coincidences removes the bulk
                    # of false references without assuming an object size.
                    if reference % 8 != 0:
                        continue
                    for address, delta, name in targets:
                        if abs(reference - address) < 64:
                            continue
                        try:
                            context = os.pread(memory, 1024, max(0, reference - 384))
                        except OSError:
                            context = b""
                        references.append(
                            {
                                "reference": reference,
                                "string_address": address,
                                "delta": delta,
                                "name": name,
                                "mapping": mapping,
                                "context": context.hex(),
                            }
                        )
            position += len(data)
    return references


def scan_layout_links(
    memory: int,
    mappings: list[tuple[int, int, str]],
    names: list[str],
) -> list[dict[str, object]]:
    tags = [f"RewardList.Item{index}".encode() for index in range(4)]
    reward_patterns = {
        name: (name.encode(), name.replace(" ", "").encode()) for name in names
    }
    links: list[dict[str, object]] = []
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= PRIMARY_ADDRESS_MIN or start >= REFERENCE_ADDRESS_MAX:
            continue
        position = max(start, PRIMARY_ADDRESS_MIN)
        limit = min(end, REFERENCE_ADDRESS_MAX)
        carry = b""
        while position < limit:
            request = min(READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            blob = carry + data
            base = position - len(carry)
            for index, tag in enumerate(tags):
                search_from = 0
                while True:
                    found = blob.find(tag, search_from)
                    if found < 0:
                        break
                    search_from = found + 1
                    left = max(0, found - LAYOUT_RADIUS)
                    right = min(len(blob), found + len(tag) + LAYOUT_RADIUS)
                    window = blob[left:right]
                    matches = []
                    for name, patterns in reward_patterns.items():
                        for representation, pattern in zip(
                            ("display_utf8", "compact_utf8"), patterns, strict=True
                        ):
                            match_from = 0
                            while True:
                                reward_at = window.find(pattern, match_from)
                                if reward_at < 0:
                                    break
                                match_from = reward_at + 1
                                matches.append(
                                    {
                                        "name": name,
                                        "representation": representation,
                                        "relative_offset": reward_at + left - found,
                                    }
                                )
                    links.append(
                        {
                            "item_index": index,
                            "tag_address": base + found,
                            "mapping": mapping,
                            "matches": matches,
                            "context": window.hex(),
                        }
                    )
            carry = blob[-(LAYOUT_RADIUS + 64) :]
            position += len(data)
    return links


def scan_object_references(
    memory: int,
    mappings: list[tuple[int, int, str]],
    hits: list[dict[str, object]],
    layout_links: list[dict[str, object]],
) -> list[dict[str, object]]:
    targets: dict[bytes, list[dict[str, object]]] = {}
    for hit in hits:
        address = int(hit["address"])
        representation = str(hit.get("representation", ""))
        object_address = address - 24
        if representation == "compact_utf8":
            try:
                prefix = os.pread(memory, 192, max(0, address - 192))
            except OSError:
                continue
            path_at = prefix.rfind(b"/Lotus/")
            if path_at < 0:
                continue
            object_address = address - 192 + path_at - 24
        elif representation != "display_utf8":
            continue
        metadata = {
            "kind": "reward",
            "name": hit["name"],
            "representation": representation,
            "object_address": object_address,
        }
        targets.setdefault(struct.pack("<I", object_address), []).append(metadata)
    for link in layout_links:
        object_address = int(link["tag_address"]) - 24
        metadata = {
            "kind": "item_tag",
            "item_index": link["item_index"],
            "object_address": object_address,
        }
        targets.setdefault(struct.pack("<I", object_address), []).append(metadata)

    references: list[dict[str, object]] = []
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= REFERENCE_ADDRESS_MIN or start >= REFERENCE_ADDRESS_MAX:
            continue
        position = max(start, REFERENCE_ADDRESS_MIN)
        limit = min(end, REFERENCE_ADDRESS_MAX)
        while position < limit:
            request = min(READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            for encoded, metadata_items in targets.items():
                search_from = 0
                while True:
                    found = data.find(encoded, search_from)
                    if found < 0:
                        break
                    search_from = found + 1
                    reference = position + found
                    if reference % 8 != 0:
                        continue
                    for metadata in metadata_items:
                        if abs(reference - int(metadata["object_address"])) < 64:
                            continue
                        try:
                            context = os.pread(memory, 2048, max(0, reference - 1024))
                        except OSError:
                            context = b""
                        references.append(
                            {
                                **metadata,
                                "reference": reference,
                                "mapping": mapping,
                                "context": context.hex(),
                            }
                        )
            position += len(data)
    return references


def main() -> None:
    started = time.monotonic()
    pid = process_id()
    paths = current_projection_paths()
    names = candidate_names(paths)
    mappings = readable_writable_maps(pid)
    memory = os.open(f"/proc/{pid}/mem", os.O_RDONLY)
    try:
        exact_started = time.monotonic()
        hits = scan_exact_strings(memory, mappings, names)
        exact_elapsed_ms = round((time.monotonic() - exact_started) * 1000)
        reference_started = time.monotonic()
        references = scan_references(memory, mappings, hits)
        reference_elapsed_ms = round((time.monotonic() - reference_started) * 1000)
        layout_started = time.monotonic()
        layout_links = scan_layout_links(memory, mappings, names)
        layout_elapsed_ms = round((time.monotonic() - layout_started) * 1000)
        object_reference_started = time.monotonic()
        object_references = scan_object_references(
            memory, mappings, hits, layout_links
        )
        object_reference_elapsed_ms = round(
            (time.monotonic() - object_reference_started) * 1000
        )
    finally:
        os.close(memory)
    timestamp = int(time.time() * 1000)
    destination = Path(f"/tmp/tennoscope-order-{timestamp}.json")
    destination.write_text(
        json.dumps(
            {
                "captured_unix_ms": timestamp,
                "elapsed_ms": round((time.monotonic() - started) * 1000),
                "exact_elapsed_ms": exact_elapsed_ms,
                "reference_elapsed_ms": reference_elapsed_ms,
                "layout_elapsed_ms": layout_elapsed_ms,
                "object_reference_elapsed_ms": object_reference_elapsed_ms,
                "projection_paths": paths,
                "candidate_names": names,
                "exact_hits": hits,
                "references": references,
                "layout_links": layout_links,
                "object_references": object_references,
            },
            indent=2,
        )
    )
    print(
        f"ORDER_CAPTURE_SAVED path={destination} candidates={len(names)} "
        f"exact_hits={len(hits)} references={len(references)} "
        f"exact_ms={exact_elapsed_ms} reference_ms={reference_elapsed_ms} "
        f"layout_links={len(layout_links)} layout_ms={layout_elapsed_ms} "
        f"object_refs={len(object_references)} "
        f"object_ref_ms={object_reference_elapsed_ms} "
        f"elapsed_ms={round((time.monotonic() - started) * 1000)}",
        flush=True,
    )


if __name__ == "__main__":
    main()
