"""Writers and reviewers on one long tracked report, through ``Toolbox.dispatch``: how long
each call waits, works and holds the document, and how the process's memory grows.

    python tools/tracked_report_bench.py out.json [--minutes 30] [--writers 12] [--reviewers 2]
        [--doc report.docx] [--workers 2] [--think 0.5] [--edits N] [--stop-mb 4096]

The document is ``tools/tracked_report.py``'s made-up report (made here unless ``--doc``
names one), opened with tracking on.  Each writer edits its own chapters, one call at a time,
with a pause of up to ``2 * think`` seconds between calls, in turn:

* ``word_edit_table`` ``insert_row`` in one of its tables;
* ``word_format`` ``paragraph_style`` on 6 to 18 of a table's cell paragraphs;
* a ``batch`` of 2 to 14 such ops (rows inserted, cells set, cells styled).

Each reviewer alternates a whole-document ``describe`` and a ``check`` of one chapter.  The
run ends after ``--minutes`` or ``--edits`` changing calls, whichever comes first.

Per call: ``wall`` (the dispatch), ``cpu`` (this thread's), and the document lock's ``wait``
(seconds spent taking it) and ``hold`` (seconds it was held) as measured here around
``DocumentEntry.lock``; and the result's own ``timing`` where it reports one.  After every
changing call, and every ``--sample`` seconds: this process's resident memory, its worker processes', and (Linux)
proportional set sizes and the glibc arena count.  A run whose processes together pass
``--stop-mb`` stops (the memory left as found, the calls so far written out).  The JSON holds
every call, every sample, and per call kind the p50/p99 of wall, wait, hold and cpu.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import random
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from tracked_report import build  # noqa: E402

CLOCK = lambda: dt.datetime(2026, 11, 16, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
STYLES = ("Table Text Bold", "Table Number", "Table Text")


class TimedLock:
    """A re-entrant lock that counts, per thread, the seconds spent waiting for it and the
    seconds it was held (outermost acquire to last release) -- the bench's own measure, the
    same before and after any change to the library."""

    def __init__(self, inner) -> None:
        self._inner = inner
        self._local = threading.local()

    def _state(self):
        local = self._local
        if not hasattr(local, "depth"):
            local.depth, local.since, local.wait, local.hold = 0, 0.0, 0.0, 0.0
        return local

    def acquire(self, blocking: bool = True, timeout: float = -1) -> bool:
        local = self._state()
        began = time.perf_counter()
        got = self._inner.acquire(blocking, timeout)
        if got:
            now = time.perf_counter()
            local.wait += now - began
            if local.depth == 0:
                local.since = now
            local.depth += 1
        return got

    def release(self) -> None:
        local = self._state()
        local.depth -= 1
        if local.depth == 0:
            local.hold += time.perf_counter() - local.since
        self._inner.release()

    def _is_owned(self) -> bool:         # threading.RLock's own test, which the library reads
        return self._inner._is_owned()

    __enter__ = acquire

    def __exit__(self, *exc) -> None:
        self.release()

    def take(self) -> tuple[float, float]:
        """This thread's (wait, hold) since the last take."""
        local = self._state()
        out = (local.wait, local.hold)
        local.wait = local.hold = 0.0
        return out


def _children(pid: int) -> list[int]:
    try:
        out = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout
    except OSError:
        return []
    found = [int(x) for x in out.split()]
    return found + [g for c in found for g in _children(c)]


def _memory(pid: int) -> dict:
    """Resident (and, on Linux, proportional) memory of ``pid`` in MB; Linux also the
    count of mappings of 64 MiB or so (glibc's thread arenas) and of large anonymous ones."""
    rollup = Path(f"/proc/{pid}/smaps_rollup")
    if rollup.exists():
        out = {}
        for line in rollup.read_text().splitlines():
            parts = line.split()
            if parts and parts[0] in ("Rss:", "Pss:"):
                out[parts[0][:-1].lower()] = round(int(parts[1]) / 1024, 1)
        return out
    try:
        rss = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout
        return {"rss": round(int(rss.strip()) / 1024, 1)} if rss.strip() else {}
    except (OSError, ValueError):
        return {}


def _maps(pid: int) -> dict:
    """Linux: large anonymous mappings of ``pid`` by size (arenas are 64 MiB reserved)."""
    path = Path(f"/proc/{pid}/smaps")
    if not path.exists():
        return {}
    big, arenas, current = [], 0, None
    for line in path.read_text().splitlines():
        head = line.split()
        if "-" in head[0] and len(head) >= 5 and ":" not in head[0]:
            start, end = (int(x, 16) for x in head[0].split("-"))
            current = {"size": (end - start) >> 20, "anon": len(head) < 6, "rss": 0}
        elif head[0] == "Rss:" and current is not None:
            current["rss"] = int(head[1]) >> 10
            if current["anon"] and current["size"] >= 16:
                if current["size"] in (64, 65) or (current["size"] < 65 and current["size"] >= 60):
                    arenas += 1
                else:
                    big.append(current["rss"])
    return {"arenas": arenas, "big_maps": len(big), "big_maps_rss_mb": sum(big)}


def summary(values: list[float]) -> dict | None:
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    pick = lambda q: values[min(len(values) - 1, int(q * len(values)))]  # noqa: E731
    return {"n": len(values), "p50": round(statistics.median(values), 3), "p99": round(pick(0.99), 3),
            "max": round(values[-1], 3), "mean": round(sum(values) / len(values), 3),
            "sum": round(sum(values), 1)}


def chapters_of(document) -> list[dict]:
    """Per chapter: its heading's id, its end paragraph's, and its tables with their cell
    paragraphs (rows after the header)."""
    out: list[dict] = []
    for block in document.blocks():
        text = document.paragraph(block["id"]).text if block.get("kind") == "paragraph" else ""
        if block.get("kind") == "paragraph" and text.startswith("Chapter "):
            out.append({"head": block["id"], "end": None, "tables": []})
        elif block.get("kind") == "paragraph" and text.startswith("End of chapter ") and out:
            out[-1]["end"] = block["id"]
        elif block.get("kind") == "table" and out:
            table = document.table(block["id"])
            cells = [table.cell(r, c).paragraphs[0].id for r in range(1, 10) for c in range(4)]
            out[-1]["tables"].append({"id": block["id"], "cells": cells})
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out")
    ap.add_argument("--doc")
    ap.add_argument("--minutes", type=float, default=30.0)
    ap.add_argument("--edits", type=int, default=0, help="stop after this many changing calls (0: no limit)")
    ap.add_argument("--writers", type=int, default=12)
    ap.add_argument("--reviewers", type=int, default=2)
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--think", type=float, default=0.5)
    ap.add_argument("--sample", type=float, default=10.0)
    ap.add_argument("--stop-mb", type=float, default=4096.0)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    from ooxml_edit.tools import Toolbox
    from ooxml_edit.tools.limits import Limits

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    data = Path(args.doc).read_bytes() if args.doc else build()[0]
    out: dict = {"args": vars(args), "python": sys.version, "started": time.time(), "calls": [], "samples": [],
                 "by_edit": []}
    calls, lock = out["calls"], threading.Lock()
    stop = threading.Event()
    edits = [0]
    limits = Limits(render_timeout=3600, layout_timeout=3600, batch_timeout=3600, worker_queue_timeout=3600)
    pid = os.getpid()

    def sample(t0: float) -> dict:
        kids = _children(pid)
        me = _memory(pid)
        theirs = [_memory(k) for k in kids]
        return {"t": round(time.perf_counter() - t0, 1), "rss_mb": me.get("rss"), "pss_mb": me.get("pss"),
                "workers_rss_mb": round(sum(m.get("rss") or 0 for m in theirs), 1),
                "workers_pss_mb": round(sum(m.get("pss") or 0 for m in theirs), 1) if theirs and
                theirs[0].get("pss") is not None else None, **_maps(pid)}

    def sampler(t0: float) -> None:
        while not stop.is_set():
            row = sample(t0)
            with lock:
                row["calls"] = len(calls)
                out["samples"].append(row)
            print(json.dumps(row), flush=True)
            if (row["rss_mb"] or 0) + row["workers_rss_mb"] > args.stop_mb:
                out["stopped"] = f"memory past {args.stop_mb} MB at {row['t']} s"
                print(out["stopped"], flush=True)
                stop.set()
            stop.wait(args.sample)

    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=args.workers, limits=limits) as box:
        session = box.session(clock=CLOCK)
        t0 = time.perf_counter()
        threading.Thread(target=sampler, args=(t0,), daemon=True).start()
        doc = session.open(data, "report.docx")
        entry = session.entry(doc)
        timed = TimedLock(entry.lock)
        entry.lock = timed
        r = box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Bench Writer"})
        assert r.ok, r.to_json()
        timed.take()
        chapters = chapters_of(entry.document)
        out["opened_s"] = round(time.perf_counter() - t0, 2)
        out["chapters"] = len(chapters)
        deadline = time.perf_counter() + args.minutes * 60

        def call(view, who: str, kind: str, tool: str, arguments: dict) -> None:
            timed.take()
            c, w = time.thread_time(), time.perf_counter()
            result = box.dispatch(view, tool, arguments)
            took = time.perf_counter() - w
            wait, hold = timed.take()
            j = result.to_json()
            data = j.get("data") if isinstance(j.get("data"), dict) else {}
            row = {"who": who, "kind": kind, "at": round(w - t0, 2), "wall": round(took, 3),
                   "cpu": round(time.thread_time() - c, 3), "wait": round(wait, 3), "hold": round(hold, 3),
                   "ok": j.get("ok", True),
                   # ooxml-edit 0.121.0: where the call's time went (Result.timing), else none.
                   "timing": getattr(result, "timing", None)}
            if not row["ok"]:
                row["error"] = str(j.get("error"))[:300]
            changing = kind not in ("describe", "check")
            with lock:
                calls.append(row)
                if changing:
                    edits[0] += 1
                    count = edits[0]
                    if args.edits and edits[0] >= args.edits:
                        stop.set()
            if changing:
                # Memory by version: what a run's growth is measured by.
                memory = sample(t0)
                with lock:
                    out["by_edit"].append({"edits": count, **memory})

        def done() -> bool:
            return stop.is_set() or time.perf_counter() > deadline

        def writer(w: int) -> None:
            rng = random.Random(args.seed * 7919 + w)
            view = session.view(context={"agent": f"writer-{w}"})
            mine = [ch for k, ch in enumerate(chapters) if k % args.writers == w and ch["tables"]]
            turn = 0
            while not done():
                stop.wait(rng.uniform(0, 2 * args.think))
                if done():
                    break
                chapter = mine[turn % len(mine)]
                table = rng.choice(chapter["tables"])
                shape = ("insert_row", "format", "batch")[turn % 3]
                turn += 1

                def row_op() -> dict:
                    return {"doc": doc, "table": table["id"], "action": "insert_row", "index": rng.randint(1, 8)}

                def format_op() -> dict:
                    picked = rng.sample(table["cells"], rng.randint(6, 18))
                    return {"doc": doc, "targets": picked, "paragraph_style": rng.choice(STYLES)}

                def cells_op() -> dict:
                    return {"doc": doc, "table": table["id"], "action": "set_cells",
                            "cells": [{"row": rng.randint(1, 9), "col": rng.randint(1, 3),
                                       "text": str(rng.randint(1, 999))} for _ in range(rng.randint(1, 4))]}

                try:
                    if shape == "insert_row":
                        call(view, f"w{w}", "insert_row", "word_edit_table", row_op())
                    elif shape == "format":
                        call(view, f"w{w}", "format", "word_format", format_op())
                    else:
                        ops = []
                        for _ in range(rng.randint(2, 14)):
                            pick = rng.random()
                            if pick < 0.4:
                                ops.append({"tool": "word_format", "arguments": format_op()})
                            elif pick < 0.7:
                                ops.append({"tool": "word_edit_table", "arguments": row_op()})
                            else:
                                ops.append({"tool": "word_edit_table", "arguments": cells_op()})
                        call(view, f"w{w}", f"batch{len(ops)}", "batch", {"ops": ops})
                except Exception as error:  # noqa: BLE001 -- recorded, the run goes on
                    with lock:
                        calls.append({"who": f"w{w}", "kind": shape, "ok": False, "error": repr(error)[:300]})

        def reviewer(k: int) -> None:
            rng = random.Random(args.seed * 104729 + k)
            view = session.view(roles=["review"], context={"agent": f"reviewer-{k}"})
            turn = 0
            while not done():
                stop.wait(rng.uniform(0, 2 * args.think))
                if done():
                    break
                try:
                    if turn % 2 == 0:
                        call(view, f"r{k}", "describe", "describe", {"doc": doc})
                    else:
                        chapter = rng.choice(chapters)
                        call(view, f"r{k}", "check", "check", {"doc": doc, "range": [f"{chapter['head']}..{chapter['end']}"]})
                except Exception as error:  # noqa: BLE001
                    with lock:
                        calls.append({"who": f"r{k}", "kind": "read", "ok": False, "error": repr(error)[:300]})
                turn += 1

        c0 = time.process_time()
        threads = ([threading.Thread(target=writer, args=(w,)) for w in range(args.writers)]
                   + [threading.Thread(target=reviewer, args=(k,)) for k in range(args.reviewers)])
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        out["elapsed_s"] = round(time.perf_counter() - t0, 1)
        out["process_cpu_s"] = round(time.process_time() - c0, 1)
        stop.set()
        time.sleep(0.1)
        out["final"] = {"rss_mb": _memory(pid).get("rss"), "pss_mb": _memory(pid).get("pss"),
                        "workers_rss_mb": round(sum(_memory(k).get("rss") or 0 for k in _children(pid)), 1)}

    kinds: dict[str, list] = {}
    for row in calls:
        if "wall" in row:
            kind = "batch" if row["kind"].startswith("batch") else row["kind"]
            kinds.setdefault(kind, []).append(row)
    out["by_kind"] = {kind: {m: summary([r[m] for r in rows]) for m in ("wall", "wait", "hold", "cpu")}
                      for kind, rows in kinds.items()}
    samples = [s["rss_mb"] or 0 for s in out["samples"]]
    out["memory"] = {"peak_rss_mb": max(samples, default=None), "final_rss_mb": out["final"]["rss_mb"],
                     "peak_workers_rss_mb": max((s["workers_rss_mb"] for s in out["samples"]), default=None)}
    out["failed"] = sum(1 for r in calls if not r.get("ok", True))
    Path(args.out).write_text(json.dumps(out, indent=1))
    print(json.dumps({"by_kind": out["by_kind"], "memory": out["memory"], "elapsed_s": out["elapsed_s"],
                      "process_cpu_s": out["process_cpu_s"], "failed": out["failed"],
                      "calls": len(calls)}, indent=1))


if __name__ == "__main__":
    main()
