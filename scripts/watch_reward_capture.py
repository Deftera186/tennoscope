#!/usr/bin/env python3
"""Run bounded player/reward memory captures as soon as rewards are rendered."""

from __future__ import annotations

import subprocess
import time
from pathlib import Path

import _paths


TRIGGER = "ProjectionRewardChoice.lua: Got rewards"
CAPTURE = Path(__file__).with_name("capture_reward_players.py")
ORDER_CAPTURE = Path(__file__).with_name("capture_reward_order.py")
SWEEP = Path(__file__).with_name("scan_response_records.py")
SCREEN = Path(__file__).with_name("capture_reward_screen.py")


def main() -> None:
    log_path = _paths.ee_log()
    position = log_path.stat().st_size
    print(f"REWARD_CAPTURE_ARMED offset={position}", flush=True)
    while True:
        with log_path.open("r", errors="replace") as log:
            log.seek(position)
            while line := log.readline():
                position = log.tell()
                if TRIGGER not in line:
                    continue
                print("REWARD_CAPTURE_TRIGGERED", flush=True)
                # Frames first: the reward screen is only up for 15s, while the memory passes can
                # run on into its afterlife.
                screen = subprocess.Popen(["python3", str(SCREEN)])
                sweep = subprocess.Popen(["python3", str(SWEEP)])
                player_capture = subprocess.Popen(["python3", str(CAPTURE)])
                order_capture = subprocess.Popen(["python3", str(ORDER_CAPTURE)])
                player_capture.wait()
                order_capture.wait()
                time.sleep(0.5)
                subprocess.run(["python3", str(CAPTURE)], check=False)
                screen.wait()
                sweep.wait()
                print("REWARD_CAPTURE_COMPLETE", flush=True)
        time.sleep(0.05)


if __name__ == "__main__":
    main()
