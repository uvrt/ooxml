"""TEMPORARY (lane L): loop block_stress on a free-threaded Python for a time budget --
plain, --more and --more --tracked, 4 to 24 threads on one document, drafts on -- and
record every run: same or not, and why.  A run still going after WATCHDOG seconds is a
hang: every thread's Python stack twice (with each thread's CPU time between, to tell a
deadlock from a livelock), the native stacks (gdb), then out.  An exception logged by a
tool call (the lxml proxy assertion) is counted with its run."""
import faulthandler
import json
import logging
import os
import subprocess
import sys
import threading
import time
import traceback

sys.path.insert(0, sys.argv[1])
import block_stress  # noqa: E402
import lxml.etree  # noqa: E402
import difflib  # noqa: E402
import io  # noqa: E402
import signal  # noqa: E402
import zipfile  # noqa: E402

# What a failed run is made of: the calls, the commits, both final documents.
SEEN = {}
_order, _replay, _repeated = block_stress._order, block_stress.replay, block_stress.repeated_ids


def order(calls, commits):
    SEEN["calls"], SEEN["commits"] = calls, commits
    return _order(calls, commits)


def replay(data, order_, *a, **k):
    out = _replay(data, order_, *a, **k)
    SEEN["serial"] = out["final"]
    SEEN["order"] = [item["call"] for item in order_]
    return out


def repeated(data):
    SEEN["final"] = data
    return _repeated(data)


block_stress._order, block_stress.replay, block_stress.repeated_ids = order, replay, repeated


def part_diffs(a, b):
    za, zb = zipfile.ZipFile(io.BytesIO(a)), zipfile.ZipFile(io.BytesIO(b))
    na, nb = set(za.namelist()), set(zb.namelist())
    out = {"only_concurrent": sorted(na - nb), "only_serial": sorted(nb - na), "parts": {}}
    for name in sorted(na & nb):
        x, y = za.read(name), zb.read(name)
        if x != y:
            xs = x.decode("utf-8", "replace").replace("><", ">\n<").splitlines()
            ys = y.decode("utf-8", "replace").replace("><", ">\n<").splitlines()
            out["parts"][name] = list(difflib.unified_diff(xs, ys, "concurrent", "serial", n=6, lineterm=""))[:400]
    return out

shard = int(sys.argv[2])
budget = float(sys.argv[3])
os.makedirs("out", exist_ok=True)
faulthandler.enable()
# The watchdog outside the process (the workflow's) sends SIGUSR1: every thread's Python
# stack, written by faulthandler's own signal handler (no eval loop needed).
faulthandler.register(signal.SIGUSR1, file=open("out/stacks.txt", "a"), all_threads=True)

MODES = tuple(m for m in (("plain", False, False), ("more", True, False), ("more-tracked", True, True))
              if m[0] in os.environ.get("MODES", "plain,more,more-tracked").split(","))
THREADS = tuple(int(t) for t in os.environ.get("THREADS", "4,8,16,24").split(","))


class Logged(logging.Handler):
    def __init__(self):
        super().__init__(logging.ERROR)
        self.found = []

    def emit(self, record):
        text = record.getMessage()
        if record.exc_info:
            text += "\n" + "".join(traceback.format_exception(*record.exc_info))
        self.found.append(text)


logged = Logged()
logging.getLogger().addHandler(logged)


def cpu_by_thread():
    out = {}
    for tid in os.listdir("/proc/self/task"):
        try:
            with open(f"/proc/self/task/{tid}/stat") as f:
                fields = f.read().rsplit(")", 1)[1].split()
            with open(f"/proc/self/task/{tid}/comm") as f:
                name = f.read().strip()
            out[tid] = (name, int(fields[11]) + int(fields[12]))   # utime + stime, ticks
        except OSError:
            pass
    return out


print(json.dumps({"lxml": lxml.etree.LXML_VERSION, "libxml2": lxml.etree.LIBXML_VERSION,
                  "gil": sys._is_gil_enabled(), "python": sys.version}), flush=True)
started = time.time()
runs = 0
fails = []
i = 0
while time.time() - started < budget:
    name, more, tracked = MODES[i % len(MODES)]
    threads = THREADS[(i // len(MODES)) % len(THREADS)]
    seed = 1000 * shard + i
    i += 1
    info = {"seed": seed, "mode": name, "threads": threads}
    with open("out/current.json", "w") as f:
        json.dump({**info, "pid": os.getpid(), "at": time.time()}, f)
    SEEN.clear()
    logged.found.clear()
    t = time.time()
    found = block_stress.run(seed, threads=threads, calls=8 if threads <= 8 else 6, chapters=4,
                             first_fill=bool(seed % 2), tracked=tracked, more=more,
                             mode=os.environ.get("BLOCK_MODE", "always"))
    runs += 1
    row = {**info, "block_mode": os.environ.get("BLOCK_MODE", "always"), "s": round(time.time() - t, 1), "same": found["same"], "drafted": found["drafted"],
           "conflicts": found["conflicts"], "calls": found["calls"], "logged": len(logged.found)}
    print(json.dumps(row), flush=True)
    if not found["same"] or logged.found:
        fails.append(row)
        with open(f"out/fail-shard{shard}-seed{seed}.json", "w") as f:
            json.dump({**row, **found, "logged": logged.found}, f, default=str, indent=1)
        detail = {"order": SEEN.get("order"), "commits": SEEN.get("commits"),
                  "calls": [{k: c[k] for k in ("call", "thread", "name", "arguments", "version", "start", "end", "block")}
                            for c in SEEN.get("calls") or []]}
        if SEEN.get("final") is not None and SEEN.get("serial") is not None and SEEN["final"] != SEEN["serial"]:
            detail["bytes"] = part_diffs(SEEN["final"], SEEN["serial"])
            with open(f"out/final-shard{shard}-seed{seed}-concurrent.docx", "wb") as f:
                f.write(SEEN["final"])
            with open(f"out/final-shard{shard}-seed{seed}-serial.docx", "wb") as f:
                f.write(SEEN["serial"])
        with open(f"out/detail-shard{shard}-seed{seed}.json", "w") as f:
            json.dump(detail, f, default=str, indent=1)
summary = {"shard": shard, "runs": runs, "fails": len(fails), "failed": fails}
print("SUMMARY " + json.dumps(summary), flush=True)
with open(f"out/summary-shard{shard}.json", "w") as f:
    json.dump(summary, f)
sys.exit(1 if fails else 0)
