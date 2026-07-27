"""Locate the running game's EE.log and TennoScope's cached relic catalog.

Mirrors the discovery the app does in `app/src-tauri/src/lib.rs`: the Wine/Proton prefix is read
out of the live process's own mappings rather than guessed, because prefixes live wherever the
launcher put them and the two Warframe builds disagree about where inside the prefix EE.log goes.

Both lookups can be overridden -- `TENNOSCOPE_EE_LOG` and `TENNOSCOPE_CATALOG` -- so a script can
be pointed at an archived log or catalog with no game running.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

# The two places the game has been observed to write EE.log inside a prefix, relative to the
# Windows user directory.
LOG_RELATIVE = (
    "AppData/Local/Warframe/EE.log",
    "Local Settings/Application Data/Warframe/EE.log",
)


def process_id(pid: int | None = None) -> int:
    """The Warframe pid. Wine truncates the process name, so match on the prefix."""
    if pid is not None:
        return pid
    out = subprocess.run(
        ["pgrep", "-f", "Warframe.x64.ex"], capture_output=True, text=True
    ).stdout.split()
    if not out:
        raise SystemExit("no Warframe process found")
    return int(out[0])


def wine_prefixes(pid: int) -> list[Path]:
    prefixes = []
    if env := os.environ.get("WINEPREFIX"):
        prefixes.append(Path(env))
    try:
        maps = Path(f"/proc/{pid}/maps").read_text(errors="replace")
    except OSError:
        maps = ""
    for line in maps.splitlines():
        start = line.find("/")
        if start >= 0 and "/drive_c/" in line[start:]:
            prefixes.append(Path(line[start:].rsplit("/drive_c/", 1)[0]))
    return sorted(set(prefixes))


def ee_log(pid: int | None = None) -> Path:
    """EE.log for the running game, or `TENNOSCOPE_EE_LOG` when set."""
    if override := os.environ.get("TENNOSCOPE_EE_LOG"):
        return Path(override)
    for prefix in wine_prefixes(process_id(pid)):
        users = prefix / "drive_c/users"
        if not users.is_dir():
            continue
        for user in users.iterdir():
            for relative in LOG_RELATIVE:
                if (candidate := user / relative).is_file():
                    return candidate
    raise SystemExit("EE.log not found; set TENNOSCOPE_EE_LOG to point at it")


def catalog() -> Path:
    """The relic generation TennoScope caches, or `TENNOSCOPE_CATALOG` when set."""
    if override := os.environ.get("TENNOSCOPE_CATALOG"):
        return Path(override)
    data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
    path = data / "org.warframehelper.app/catalog/relic-generation.json"
    if not path.is_file():
        raise SystemExit(f"no cached catalog at {path}; run TennoScope once first")
    return path
