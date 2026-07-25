#!/usr/bin/env python3
"""Dump every VoidProjections reward artefact in a live Warframe process.

This is the one-run evidence instrument: it makes no assumptions about which region holds the
per-player response records, so a single relic screen answers where remote players' rewards live.

It collects three things across every readable mapping:

  * response records  -- `18 <24-hex account id> ...`, the wire format confirmed from a live
    capture (see crates/warframe-acquisition/tests/fixtures/void-response-record.bin)
  * reward paths      -- every `/Lotus/StoreItems/...` and `/Lotus/Types/Recipes/...` string
  * squad identities  -- every occurrence of the account ids of the current reward screen

Usage: scan_response_records.py [pid]
Output: /tmp/tennoscope-sweep-<pid>-<stamp>.json plus a .bin of the raw records.
"""

from __future__ import annotations

import json
import re
import struct
import subprocess
import sys
import time
from pathlib import Path

LOG_PATH = Path(
    "$WINEPREFIX/drive_c/users/steamuser/"
    "AppData/Local/Warframe/EE.log"
)
RECORD = re.compile(rb"\x18([0-9a-f]{24})")
REWARD_PATH = re.compile(rb"/Lotus/(?:StoreItems|Types)/[A-Za-z0-9/_\-]{8,120}")
RECORD_BYTES = 768
CHUNK = 8 * 1024 * 1024
OVERLAP = 1024


def process_id() -> int:
    if len(sys.argv) > 1:
        return int(sys.argv[1])
    return int(subprocess.check_output(["pgrep", "-f", "Warframe.x64.exe"], text=True).split()[0])


def squad_identities() -> list[str]:
    """Account ids named since the most recent reward screen opened."""
    lines = LOG_PATH.read_text(errors="replace").splitlines()
    opened = [i for i, line in enumerate(lines) if "OpenVoidProjectionRewardScreen" in line]
    identities: list[str] = []
    for line in lines[opened[-1] :] if opened else []:
        if "Relic reward screen shut down" in line:
            break
        for identity in re.findall(r"\b[0-9a-f]{24}\b", line):
            if identity not in identities:
                identities.append(identity)
    return identities


def readable_regions(pid: int) -> list[tuple[int, int, str]]:
    regions = []
    with open(f"/proc/{pid}/maps", encoding="utf-8", errors="replace") as maps:
        for line in maps:
            fields = line.split()
            if "r" not in fields[1]:
                continue
            start, end = (int(value, 16) for value in fields[0].split("-"))
            regions.append((start, end, fields[5] if len(fields) > 5 else "[anon]"))
    return regions


# A reward can be resident under the canonical item path, the StoreItems alias, or both. The
# 2026-07-26 client run had Paris Prime Upper Limb only as /Lotus/Types/Recipes/..., so filtering
# on the StoreItems alias alone silently dropped a quarter of the squad.
REWARD_NAMESPACES = ("/Lotus/Types/Recipes/", "/Lotus/StoreItems/Types/Recipes/")
# Where a pointer to a path's string object aims. Observed: 24 bytes ahead of the path bytes, which
# is the object header; the rest are kept as cheap insurance against a different string layout.
POINTER_OFFSETS = (0, -1, -2, -8, -16, -24, -32)


def pointer_sites(
    pid: int, regions: list[tuple[int, int, str]], paths: dict[str, list[int]]
) -> list[dict]:
    """Every 8-aligned word that points at a reward-namespace path string.

    Client-mode memory holds no identity-to-reward record, so the only thing that can order the
    four cards is whatever structure references the four path strings. Recording the referencing
    sites lets a labelled run confirm or kill the ordered-array hypothesis offline.
    """
    import numpy as np

    wanted: dict[int, str] = {}
    for path, addresses in paths.items():
        if not any(namespace in path for namespace in REWARD_NAMESPACES):
            continue
        for address in addresses:
            for offset in POINTER_OFFSETS:
                wanted[address + offset] = path
    if not wanted:
        return []
    allowed = np.array(sorted(wanted), dtype=np.uint64)

    sites: list[dict] = []
    with open(f"/proc/{pid}/mem", "rb", buffering=0) as mem:
        for start, end, mapping in regions:
            address = start - (start % 8) + (8 if start % 8 else 0)
            while address < end:
                try:
                    mem.seek(address)
                    block = mem.read(min(CHUNK, end - address) & ~7)
                except OSError:
                    break
                if len(block) < 8:
                    break
                words = np.frombuffer(block, dtype=np.uint64)
                for index in np.flatnonzero(np.isin(words, allowed)):
                    value = int(words[index])
                    sites.append(
                        {
                            "site": address + int(index) * 8,
                            "value": value,
                            "path": wanted[value],
                            "mapping": mapping,
                        }
                    )
                address += len(block)
    sites.sort(key=lambda site: site["site"])
    return sites


def main() -> int:
    pid = process_id()
    identities = squad_identities()
    needles = [identity.encode() for identity in identities]
    regions = readable_regions(pid)
    started = time.monotonic()
    print(f"pid={pid} regions={len(regions)} squad={identities}", file=sys.stderr)

    stamp = time.strftime("%Y%m%d-%H%M%S")
    record_blob = Path(f"/tmp/tennoscope-sweep-{pid}-{stamp}.bin")
    records: list[dict] = []
    paths: dict[str, list[int]] = {}
    identity_hits: dict[str, int] = {identity: 0 for identity in identities}
    identity_sites: dict[str, list[int]] = {identity: [] for identity in identities}
    scanned = 0

    with open(f"/proc/{pid}/mem", "rb", buffering=0) as mem, record_blob.open("wb") as blob:
        for start, end, mapping in regions:
            address = start
            while address < end:
                try:
                    mem.seek(address)
                    block = mem.read(min(CHUNK, end - address))
                except OSError:
                    break
                if not block:
                    break
                scanned += len(block)

                for match in RECORD.finditer(block):
                    at = address + match.start()
                    raw = block[match.start() : match.start() + RECORD_BYTES]
                    if len(raw) < RECORD_BYTES:  # straddles the chunk edge, re-read it whole
                        mem.seek(at)
                        raw = mem.read(RECORD_BYTES)
                    blob.write(struct.pack("<QI", at, len(raw)) + raw)
                    path = REWARD_PATH.search(raw)
                    records.append(
                        {
                            "address": at,
                            "identity": match.group(1).decode(),
                            "mapping": mapping,
                            "name": raw[26 : 26 + raw[25]].decode("utf-8", "replace"),
                            "path": path.group().decode() if path else None,
                        }
                    )

                for match in REWARD_PATH.finditer(block):
                    paths.setdefault(match.group().decode(), []).append(address + match.start())

                for identity, needle in zip(identities, needles):
                    at = block.find(needle)
                    while at >= 0:
                        identity_hits[identity] += 1
                        identity_sites[identity].append(address + at)
                        at = block.find(needle, at + 1)

                address += max(len(block) - OVERLAP, 1)

    summary = {
        "pid": pid,
        "elapsed_s": round(time.monotonic() - started, 1),
        "scanned_bytes": scanned,
        "squad": identities,
        "identity_hits": identity_hits,
        "identity_sites": identity_sites,
        "records": records,
        "records_with_reward_path": [record for record in records if record["path"]],
        "reward_paths": {path: hits[:8] for path, hits in sorted(paths.items())},
        "pointer_sites": pointer_sites(pid, regions, paths),
        "record_blob": str(record_blob),
    }
    out = Path(f"/tmp/tennoscope-sweep-{pid}-{stamp}.json")
    out.write_text(json.dumps(summary, indent=1))
    print(
        f"scanned={scanned / 1e9:.2f}GB in {summary['elapsed_s']}s "
        f"records={len(records)} with_path={len(summary['records_with_reward_path'])} "
        f"paths={len(paths)} -> {out}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
