#!/usr/bin/env python3
"""Capture bounded player-id to relic-reward pointer associations."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import struct
import time
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "capture_reward_order", Path(__file__).with_name("capture_reward_order.py")
)
capture = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(capture)


def current_player_ids() -> list[str]:
    lines = capture.LOG_PATH.read_text(errors="replace").splitlines()
    open_index = max(
        index
        for index, line in enumerate(lines)
        if "OpenVoidProjectionRewardScreen" in line
    )
    ids: list[str] = []
    for line in lines[open_index:]:
        if "ProjectionRewardChoice.lua: Got rewards" in line:
            break
        for player_id in re.findall(r"\b[0-9a-f]{24}\b", line):
            if player_id not in ids:
                ids.append(player_id)
    return ids


def scan_ids(memory: int, mappings: list[tuple[int, int, str]], ids: list[str]) -> list[dict]:
    patterns = {player_id.encode(): player_id for player_id in ids}
    matcher = re.compile(b"|".join(re.escape(value) for value in patterns))
    hits = []
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= capture.PRIMARY_ADDRESS_MIN or start >= capture.REFERENCE_ADDRESS_MAX:
            continue
        position = max(start, capture.PRIMARY_ADDRESS_MIN)
        limit = min(end, capture.REFERENCE_ADDRESS_MAX)
        while position < limit:
            request = min(capture.READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            for match in matcher.finditer(data):
                address = position + match.start()
                hits.append(
                    {
                        "player_id": patterns[match.group()],
                        "address": address,
                        "mapping": mapping,
                    }
                )
            position += len(data)
    return hits


def reference_targets(memory: int, reward_hits: list[dict], id_hits: list[dict]) -> dict[bytes, list[dict]]:
    targets: dict[bytes, list[dict]] = {}
    for hit in id_hits:
        target = int(hit["address"]) - 24
        targets.setdefault(struct.pack("<I", target), []).append(
            {"kind": "player", "value": hit["player_id"], "target": target}
        )
    for hit in reward_hits:
        address = int(hit["address"])
        representation = hit.get("representation")
        target = address - 24
        if representation == "compact_utf8":
            prefix = os.pread(memory, 192, max(0, address - 192))
            path_at = prefix.rfind(b"/Lotus/")
            if path_at < 0:
                continue
            target = address - 192 + path_at - 24
        elif representation != "display_utf8":
            continue
        targets.setdefault(struct.pack("<I", target), []).append(
            {"kind": "reward", "value": hit["name"], "target": target}
        )
    return targets


def scan_references(memory: int, mappings: list[tuple[int, int, str]], targets: dict) -> list[dict]:
    if not targets:
        return []
    matcher = re.compile(b"|".join(re.escape(value) for value in targets))
    references = []
    for start, end, mapping in sorted(mappings, reverse=True):
        if end <= capture.REFERENCE_ADDRESS_MIN or start >= capture.REFERENCE_ADDRESS_MAX:
            continue
        position = max(start, capture.REFERENCE_ADDRESS_MIN)
        limit = min(end, capture.REFERENCE_ADDRESS_MAX)
        while position < limit:
            request = min(capture.READ_CHUNK, limit - position)
            try:
                data = os.pread(memory, request, position)
            except OSError:
                break
            if not data:
                break
            for match in matcher.finditer(data):
                reference = position + match.start()
                if reference % 8:
                    continue
                for metadata in targets[match.group()]:
                    references.append(
                        {**metadata, "reference": reference, "mapping": mapping}
                    )
            position += len(data)
    return references


def nearby_rewards(memory: int, id_hits: list[dict], names: list[str]) -> list[dict]:
    patterns = {
        name: (name.encode(), name.replace(" ", "").encode()) for name in names
    }
    associations = []
    for hit in id_hits:
        address = int(hit["address"])
        try:
            window = os.pread(memory, 128 * 1024, max(0, address - 64 * 1024))
        except OSError:
            continue
        matches = []
        for name, variants in patterns.items():
            for representation, pattern in zip(
                ("display_utf8", "compact_utf8"), variants, strict=True
            ):
                search_from = 0
                while True:
                    found = window.find(pattern, search_from)
                    if found < 0:
                        break
                    search_from = found + 1
                    matches.append(
                        {
                            "name": name,
                            "representation": representation,
                            "relative_offset": found - 64 * 1024,
                        }
                    )
        if matches:
            context_start = max(0, address - 32 * 1024)
            try:
                context = os.pread(memory, 64 * 1024, context_start)
            except OSError:
                context = b""
            associations.append(
                {
                    **hit,
                    "matches": matches,
                    "context_start": context_start,
                    "context": context.hex(),
                }
            )
    return associations


def focused_mapping_slices(
    memory: int, id_hits: list[dict], reward_hits: list[dict]
) -> list[dict]:
    by_mapping_ids: dict[str, list[dict]] = {}
    by_mapping_rewards: dict[str, list[dict]] = {}
    for hit in id_hits:
        by_mapping_ids.setdefault(hit["mapping"], []).append(hit)
    for hit in reward_hits:
        if hit.get("representation") == "compact_utf8":
            by_mapping_rewards.setdefault(hit["mapping"], []).append(hit)
    slices = []
    for mapping, ids in by_mapping_ids.items():
        rewards = by_mapping_rewards.get(mapping, [])
        if len({hit["player_id"] for hit in ids}) < 3 or not rewards:
            continue
        addresses = [int(hit["address"]) for hit in ids + rewards]
        start = max(int(mapping.split("-")[0], 16), min(addresses) - 64 * 1024)
        end = min(int(mapping.split("-")[1].split()[0], 16), max(addresses) + 64 * 1024)
        if end - start > 2 * 1024 * 1024:
            continue
        try:
            data = os.pread(memory, end - start, start)
        except OSError:
            continue
        slices.append(
            {
                "mapping": mapping,
                "start": start,
                "data": data.hex(),
            }
        )
    return slices


def main() -> None:
    started = time.monotonic()
    pid = capture.process_id()
    paths = capture.current_projection_paths()
    names = capture.candidate_names(paths)
    ids = current_player_ids()
    mappings = capture.readable_writable_maps(pid)
    memory = os.open(f"/proc/{pid}/mem", os.O_RDONLY)
    try:
        reward_hits = capture.scan_exact_strings(memory, mappings, names)
        id_hits = scan_ids(memory, mappings, ids)
        associations = nearby_rewards(memory, id_hits, names)
        mapping_slices = focused_mapping_slices(memory, id_hits, reward_hits)
    finally:
        os.close(memory)
    destination = Path(f"/tmp/tennoscope-players-{int(time.time() * 1000)}.json")
    destination.write_text(
        json.dumps(
            {
                "elapsed_ms": round((time.monotonic() - started) * 1000),
                "player_ids": ids,
                "reward_hits": reward_hits,
                "id_hits": id_hits,
                "associations": associations,
                "mapping_slices": mapping_slices,
            },
            indent=2,
        )
    )
    print(
        f"PLAYER_CAPTURE_SAVED path={destination} ids={len(ids)} "
        f"reward_hits={len(reward_hits)} id_hits={len(id_hits)} "
        f"associations={len(associations)} elapsed_ms={round((time.monotonic() - started) * 1000)}",
        flush=True,
    )


if __name__ == "__main__":
    main()
