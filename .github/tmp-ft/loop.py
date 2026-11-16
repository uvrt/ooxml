"""TEMPORARY (lane J): loop block_stress's --more --tracked mix on a free-threaded Python,
for a time budget; every run that differs is written to out/ as JSON (its seed, the
first differences and the final bytes of both runs, base64)."""
import base64
import json
import os
import sys
import time

sys.path.insert(0, sys.argv[1])
import block_stress  # noqa: E402

seeds = [int(s) for s in sys.argv[2].split(",")]
budget = float(sys.argv[3])
os.makedirs("out", exist_ok=True)

finals = {}
real_replay = block_stress.replay


def replay(*a, **k):
    found = real_replay(*a, **k)
    finals["serial"] = found["final"]
    return found


block_stress.replay = replay
real_to_bytes = None
started = time.time()
runs = fails = 0
while time.time() - started < budget:
    for seed in seeds:
        found = block_stress.run(seed, threads=4, calls=8, chapters=3, first_fill=bool(seed % 2), tracked=True,
                                 more=True)
        runs += 1
        if not found["same"]:
            fails += 1
            with open(f"out/fail-{fails}-seed{seed}.json", "w") as f:
                json.dump({"seed": seed, "run": runs, **{k: v for k, v in found.items()},
                           "serial_final": base64.b64encode(finals.get("serial", b"")).decode()},
                          f, default=str, indent=1)
        print(json.dumps({"run": runs, "seed": seed, "same": found["same"], "drafted": found["drafted"],
                          "conflicts": found["conflicts"]}), flush=True)
print(f"RUNS {runs} FAILS {fails}", flush=True)
with open("out/summary.txt", "w") as f:
    f.write(f"runs {runs} fails {fails}\n")
sys.exit(1 if fails else 0)
