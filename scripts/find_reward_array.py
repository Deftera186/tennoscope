#!/usr/bin/env python3
"""Look for an ordered reward array among a sweep's pointer sites.

Client-mode memory holds no per-player reward record, so the four cards must be ordered by some
structure that references the four reward path strings. If that structure is an array, the pointer
sites for exactly four distinct reward paths will sit within a short span of each other.

Usage: find_reward_array.py <sweep.json> [expected_slots]
"""

from __future__ import annotations

import json
import sys
from collections import Counter

MAX_SPAN = 4096


def main() -> int:
    sweep = json.load(open(sys.argv[1], encoding="utf-8"))
    slots = int(sys.argv[2]) if len(sys.argv) > 2 else 4
    sites = sorted(sweep["pointer_sites"], key=lambda site: site["site"])
    print(f"pointer sites: {len(sites)}  distinct paths: {len({s['path'] for s in sites})}")

    counts = Counter(site["path"] for site in sites)
    print("\nreward-namespace paths referenced:")
    for path, count in counts.most_common():
        print(f"  {count:4}x {path}")

    print(f"\nclusters of {slots} distinct paths within {MAX_SPAN} bytes:")
    found = 0
    for left in range(len(sites)):
        window = [sites[left]]
        for right in range(left + 1, len(sites)):
            if sites[right]["site"] - sites[left]["site"] > MAX_SPAN:
                break
            window.append(sites[right])
        distinct = {site["path"] for site in window}
        if len(distinct) < slots:
            continue
        found += 1
        span = window[-1]["site"] - window[0]["site"]
        print(f"\n  === {len(window)} sites, {len(distinct)} paths, span {span} @ {window[0]['site']:#x}")
        for site in window:
            print(f"      {site['site']:#014x} -> {site['value']:#014x}  {site['path'].split('/')[-1]}")
        if found >= 12:
            print("  ... truncated at 12 clusters")
            break
    if not found:
        print("  none - the four reward strings are not referenced from a common structure")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
