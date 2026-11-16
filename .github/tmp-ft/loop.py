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

shard = int(sys.argv[2])
budget = float(sys.argv[3])
WATCHDOG = float(os.environ.get("WATCHDOG", "420"))
os.makedirs("out", exist_ok=True)
faulthandler.enable()

MODES = (("plain", False, False), ("more", True, False), ("more-tracked", True, True))
THREADS = (4, 8, 16, 24)


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


def hang(info):
    path = f"out/hang-shard{shard}-seed{info['seed']}.txt"
    with open(path, "w") as f:
        f.write(json.dumps(info) + "\n\n== python stacks 1\n")
        f.flush()
        first = cpu_by_thread()
        faulthandler.dump_traceback(f, all_threads=True)
        f.flush()
        time.sleep(20)
        second = cpu_by_thread()
        f.write("\n== cpu ticks per thread over 20 s (name, before, after)\n")
        for tid, (name, after) in sorted(second.items()):
            f.write(f"{tid} {name} {first.get(tid, (name, None))[1]} {after}\n")
        f.write("\n== python stacks 2\n")
        f.flush()
        faulthandler.dump_traceback(f, all_threads=True)
        f.flush()
        f.write("\n== native stacks\n")
        f.flush()
        try:
            found = subprocess.run(["sudo", "-n", "gdb", "-p", str(os.getpid()), "-batch", "-ex", "set pagination off",
                                    "-ex", "thread apply all bt 40"], capture_output=True, text=True, timeout=240)
            f.write(found.stdout[-400_000:] + found.stderr[-20_000:])
        except Exception as exc:  # noqa: BLE001
            f.write(f"gdb: {exc!r}\n")
    print(f"HANG {json.dumps(info)} -> {path}", flush=True)
    os._exit(3)


print(json.dumps({"lxml": lxml.etree.LXML_VERSION, "libxml2": lxml.etree.LIBXML_VERSION,
                  "gil": sys._is_gil_enabled(), "python": sys.version}), flush=True)
started = time.time()
runs = 0
fails = []
i = 0
while time.time() - started < budget:
    name, more, tracked = MODES[i % 3]
    threads = THREADS[(i // 3) % 4]
    seed = 1000 * shard + i
    i += 1
    info = {"seed": seed, "mode": name, "threads": threads}
    done = threading.Event()
    watch = threading.Thread(target=lambda: done.wait(WATCHDOG) or hang(info), daemon=True)
    watch.start()
    logged.found.clear()
    t = time.time()
    found = block_stress.run(seed, threads=threads, calls=8 if threads <= 8 else 6, chapters=4,
                             first_fill=bool(seed % 2), tracked=tracked, more=more)
    done.set()
    runs += 1
    row = {**info, "s": round(time.time() - t, 1), "same": found["same"], "drafted": found["drafted"],
           "conflicts": found["conflicts"], "calls": found["calls"], "logged": len(logged.found)}
    print(json.dumps(row), flush=True)
    if not found["same"] or logged.found:
        fails.append(row)
        with open(f"out/fail-shard{shard}-seed{seed}.json", "w") as f:
            json.dump({**row, **found, "logged": logged.found}, f, default=str, indent=1)
summary = {"shard": shard, "runs": runs, "fails": len(fails), "failed": fails}
print("SUMMARY " + json.dumps(summary), flush=True)
with open(f"out/summary-shard{shard}.json", "w") as f:
    json.dump(summary, f)
sys.exit(1 if fails else 0)
