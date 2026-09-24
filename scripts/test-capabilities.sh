#!/bin/sh
# Guards app/src-tauri/capabilities/default.json: every permission the UI
# calls must be granted, or the call is denied at runtime with no fallback.
# tennoscope#updater-round-1 caught a dropped core:window:allow-close here,
# which silently disabled the window Close button.
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

required="process:default core:default core:window:allow-hide core:window:allow-show core:window:allow-minimize core:window:allow-toggle-maximize core:window:allow-close core:window:allow-start-dragging opener:allow-open-url opener:allow-reveal-item-in-dir clipboard-manager:allow-write-text"

missing=$(REQUIRED="$required" python3 - "$repo_root/app/src-tauri/capabilities/default.json" <<'EOF'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    capability = json.load(handle)
granted = set()
for entry in capability.get("permissions", []):
    granted.add(entry if isinstance(entry, str) else entry.get("identifier", ""))
for want in os.environ["REQUIRED"].split():
    if want not in granted:
        print(want)
EOF
)

if [ -n "$missing" ]; then
  echo "default.json is missing permissions:" >&2
  printf '%s\n' "$missing" >&2
  exit 1
fi

forbidden="updater:default"
present=$(FORBIDDEN="$forbidden" python3 - "$repo_root/app/src-tauri/capabilities/default.json" <<'EOF'
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    capability = json.load(handle)
granted = set()
for entry in capability.get("permissions", []):
    granted.add(entry if isinstance(entry, str) else entry.get("identifier", ""))
for bad in os.environ["FORBIDDEN"].split():
    if bad in granted:
        print(bad)
EOF
)

if [ -n "$present" ]; then
  echo "default.json must not grant updater IPC permissions:" >&2
  printf '%s\n' "$present" >&2
  exit 1
fi

# The opener scope must cover release pages (update notes, nudge fallback).
python3 - "$repo_root/app/src-tauri/capabilities/default.json" <<'EOF'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    capability = json.load(handle)
for entry in capability.get("permissions", []):
    if isinstance(entry, dict) and entry.get("identifier") == "opener:allow-open-url":
        urls = [rule.get("url", "") for rule in entry.get("allow", [])]
        assert any("github.com/Deftera186/tennoscope" in url for url in urls), urls
        break
else:
    raise SystemExit("opener:allow-open-url entry missing")
print("capabilities ok")
EOF
