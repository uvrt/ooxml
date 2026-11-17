"""Writers and reviewers on one long tracked report, through ``Toolbox.dispatch``: how long
each call waits, works and holds the document, and how the process's memory grows.

    python tools/tracked_report_bench.py out.json [--minutes 30] [--writers 12] [--reviewers 2]
        [--doc report.docx] [--workers 2] [--think 0.5] [--edits N] [--stop-mb 4096]
        [--views] [--view-pages 3] [--close] [--mix table|full] [--window 0.2]

The document is ``tools/tracked_report.py``'s made-up report (made here unless ``--doc``
names one), opened with tracking on.  Each writer edits its own chapters, one call at a time,
with a pause of up to ``2 * think`` seconds between calls, in turn:

* ``word_edit_table`` ``insert_row`` in one of its tables;
* ``word_format`` ``paragraph_style`` on 6 to 18 of a table's cell paragraphs;
* a ``batch`` of 2 to 14 such ops (rows inserted, cells set, cells styled).

Each reviewer alternates a whole-document ``describe`` and a ``check`` of one chapter.  The
run ends after ``--minutes`` or ``--edits`` changing calls, whichever comes first.

``--mix full``: the many-writers measure (round 59), on the report with its fields
(``tracked_report.py --fields``: a table of contents, a bookmark on each chapter's heading and
a page reference to it in the chapter before; ``--add-fields`` adds them through the tools
instead, to a ``--doc`` without them), each writer, in turn, makes one of the edits above or a ``word_insert_text`` in one of
its chapter's paragraphs, and an occasional ``word_fields`` update of the whole document
(about one turn in 40, one at a time); after each edit it checks its chapter, as an
application would, and every few turns reads (a ``describe``, or a ``read_batch`` of a
``word_read`` of its chapter, a ``word_inspect`` of one of its tables and a ``find_text``).
``--reviewers`` defaults to 0 there.  The report then compares, per call kind, the first
and the last ``--window`` of the changing calls (by count: 0.2 is the first and last
fifth): median and slowest.

Every run also reports this process's CPU (seconds and percentage of the run, and per
sample), the time Python's garbage collector ran (``gc.callbacks``: per sample, per
generation, the longest pause), and per sample the sizes of what the session keeps beside
the document (owners, refs by version, the call log, the changes kept, the history).

``--figures``: the report with a chart, a picture and a floating shape in each chapter
(``tracked_report.py --figures``).

``--views``: an application's live view beside them (:mod:`docx_agent.liveview`): for each
version committed, ``view`` since the last token and ``page_svg`` of up to ``--view-pages``
of the pages it changed.  ``--close``: the document closed at the end (the bench's own references to it dropped
first), and this process's and the workers' memory sampled every 0.1 s for 3 s after
(``after_close``); ``before_open`` is this process's before the document was opened.

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
import gc
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
NOTE_WORDS = ("added", "since", "revised", "noted", "as", "agreed", "in", "the", "draft")


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
    """Per chapter: its heading's id, its end paragraph's, its body paragraphs' and its
    tables with their cell paragraphs (rows after the header)."""
    out: list[dict] = []
    for block in document.blocks():
        text = document.paragraph(block["id"]).text if block.get("kind") == "paragraph" else ""
        if block.get("kind") == "paragraph" and text.startswith("Chapter "):
            out.append({"head": block["id"], "end": None, "tables": [], "body": []})
        elif block.get("kind") == "paragraph" and text.startswith("End of chapter ") and out:
            out[-1]["end"] = block["id"]
        elif block.get("kind") == "paragraph" and out and out[-1]["end"] is None and len(text) > 200:
            out[-1]["body"].append(block["id"])
        elif block.get("kind") == "table" and out:
            table = document.table(block["id"])
            cells = [table.cell(r, c).paragraphs[0].id for r in range(1, 10) for c in range(4)]
            out[-1]["tables"].append({"id": block["id"], "cells": cells})
    return out


class GcClock:
    """The time Python's garbage collector runs (``gc.callbacks``): in all, per generation,
    the count and the longest pause.  A collection holds the interpreter lock, so every
    thread waits for it."""

    def __init__(self) -> None:
        self.total = 0.0
        self.count = 0
        self.longest = 0.0
        self.by_gen = [0.0, 0.0, 0.0]
        self._since = 0.0

    def __call__(self, phase: str, info: dict) -> None:
        if phase == "start":
            self._since = time.perf_counter()
            return
        took = time.perf_counter() - self._since
        self.total += took
        self.count += 1
        self.longest = max(self.longest, took)
        self.by_gen[min(2, info.get("generation", 2))] += took

    def to_json(self) -> dict:
        return {"total_s": round(self.total, 2), "count": self.count, "longest_s": round(self.longest, 3),
                "by_generation_s": [round(x, 2) for x in self.by_gen]}


def _sizes(entry) -> dict:
    """What the session keeps beside one document, counted: what grows with the run."""
    if entry is None:
        return {}
    out = {}
    for name in ("owners", "ref_states", "ref_owners", "refs", "log", "keys", "changes_kept", "own_undos",
                 "part_stamps", "drafts_in_flight"):
        value = getattr(entry, name, None)
        if value is not None:
            out[name] = len(value)
    history = entry.history
    for name in ("_undo", "_redo", "_undone", "_watches"):
        value = getattr(history, name, None)
        if value is not None:
            out["history" + name] = len(value)
    committed = entry.committed
    if committed is not None:
        out["committed_owners"] = len(getattr(committed, "owners", ()) or ())
    return out


def windows(calls: list[dict], share: float) -> dict:
    """Per call kind, the median and slowest wall of the calls made in the first and the
    last ``share`` of the run, the run measured in changing calls (the ``edits`` each call
    found made when it began)."""
    total = max((row.get("edits_at", 0) for row in calls), default=0)
    if total <= 0:
        return {}
    first_end, last_start = total * share, total * (1 - share)
    out: dict = {}
    for row in calls:
        if "wall" not in row:
            continue
        kind = "batch" if row["kind"].startswith("batch") else row["kind"]
        at = row.get("edits_at", 0)
        span = "first" if at < first_end else "last" if at >= last_start else None
        if span:
            out.setdefault(kind, {}).setdefault(span, []).append(row["wall"])
    for kind, spans in out.items():
        for span, walls in list(spans.items()):
            walls.sort()
            spans[span] = {"n": len(walls), "p50": round(statistics.median(walls), 3), "max": round(walls[-1], 3)}
        if "first" in spans and "last" in spans and spans["first"]["p50"] > 0:
            spans["ratio_p50"] = round(spans["last"]["p50"] / spans["first"]["p50"], 2)
    return out


def add_fields(box, session, doc: str, chapters: list[dict]) -> None:
    """``--mix full``: a table of contents before the first chapter, a bookmark on each
    chapter's heading, and in each chapter's end paragraph a page cross-reference to the
    next chapter's heading -- the fields a ``word_fields`` update lays the document out for."""
    ops = [{"tool": "word_fields", "arguments": {"doc": doc, "action": "insert_toc", "before": chapters[0]["head"],
                                                 "levels": "1-2"}}]
    for k, chapter in enumerate(chapters):
        ops.append({"tool": "word_links", "arguments": {"doc": doc, "action": "add_bookmark",
                                                        "target": chapter["head"], "bookmark": f"chapter{k + 1}"}})
    for k, chapter in enumerate(chapters[:-1]):
        if chapter["end"]:
            ops.append({"tool": "word_fields", "arguments": {"doc": doc, "action": "insert_cross_reference",
                                                             "at": f"{chapter['end']}@0", "kind": "page",
                                                             "bookmark": f"chapter{k + 2}"}})
    r = box.dispatch(session, "batch", {"ops": ops})
    assert r.ok, str(r.to_json())[:2000]


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out")
    ap.add_argument("--doc")
    ap.add_argument("--minutes", type=float, default=30.0)
    ap.add_argument("--edits", type=int, default=0, help="stop after this many changing calls (0: no limit)")
    ap.add_argument("--writers", type=int, default=12)
    ap.add_argument("--reviewers", type=int, default=None, help="default 2; 0 with --mix full")
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--think", type=float, default=0.5)
    ap.add_argument("--sample", type=float, default=10.0)
    ap.add_argument("--stop-mb", type=float, default=4096.0)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--views", action="store_true", help="a live view call per committed version")
    ap.add_argument("--view-pages", type=int, default=3)
    ap.add_argument("--close", action="store_true", help="close the document at the end; sample the workers")
    ap.add_argument("--mix", choices=("table", "full"), default="table",
                    help="table: the table edits alone; full: every kind of writer call (see above)")
    ap.add_argument("--window", type=float, default=0.2,
                    help="the share of the changing calls compared at the start and the end")
    ap.add_argument("--profile", help="a directory: cProfile calls there, one file per call (one call at a time)")
    ap.add_argument("--add-fields", action="store_true", help="--mix full: add the fields through the tools first")
    ap.add_argument("--python-reads", action="store_true",
                    help="answer the reading tools by the Python path (before lane P1c), not the Rust session")
    ap.add_argument("--figures", action="store_true", help="the report with charts, pictures and shapes "
                    "(tracked_report.py --figures)")
    args = ap.parse_args()
    if args.python_reads:
        from docx_agent.rust import reads as _reads

        _reads.use_session(None)
    if args.reviewers is None:
        args.reviewers = 0 if args.mix == "full" else 2

    from ooxml_edit.tools import Toolbox
    from ooxml_edit.tools.limits import Limits

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    data = Path(args.doc).read_bytes() if args.doc else build(fields=args.mix == "full", figures=args.figures)[0]
    out: dict = {"args": vars(args), "python": sys.version, "started": time.time(), "calls": [], "samples": [],
                 "by_edit": []}
    calls, lock = out["calls"], threading.Lock()
    stop = threading.Event()
    edits = [0]
    limits = Limits(render_timeout=3600, layout_timeout=3600, batch_timeout=3600, worker_queue_timeout=3600)
    pid = os.getpid()
    clock = GcClock()
    gc.callbacks.append(clock)
    held: dict = {"entry": None}

    def sample(t0: float) -> dict:
        kids = _children(pid)
        me = _memory(pid)
        theirs = [_memory(k) for k in kids]
        return {"t": round(time.perf_counter() - t0, 1), "cpu_s": round(time.process_time(), 1),
                "gc_s": round(clock.total, 2), "sizes": _sizes(held["entry"]),
                "workers_each_mb": [m.get("rss") for m in theirs],
                "rss_mb": me.get("rss"), "pss_mb": me.get("pss"),
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
        out["before_open"] = sample(t0)
        doc = session.open(data, "report.docx")
        entry = session.entry(doc)
        held["entry"] = entry
        timed = TimedLock(entry.lock)
        entry.lock = timed
        r = box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Bench Writer"})
        assert r.ok, r.to_json()
        timed.take()
        chapters = chapters_of(entry.document)
        if args.mix == "full" and args.add_fields:
            began = time.perf_counter()
            add_fields(box, session, doc, chapters)
            timed.take()
            out["fields_s"] = round(time.perf_counter() - began, 2)
        out["opened_s"] = round(time.perf_counter() - t0, 2)
        out["chapters"] = len(chapters)
        deadline = time.perf_counter() + args.minutes * 60

        profiling = threading.Lock()

        def call(view, who: str, kind: str, tool: str, arguments: dict) -> None:
            timed.take()
            edits_at = edits[0]
            profile = None
            if args.profile and profiling.acquire(blocking=False):
                # One call at a time: a profile is the process's (sys.monitoring, Python 3.12),
                # so it holds every thread's work done meanwhile.
                import cProfile

                profile = cProfile.Profile()
                profile.enable()
            c, w = time.thread_time(), time.perf_counter()
            result = box.dispatch(view, tool, arguments)
            took = time.perf_counter() - w
            if profile is not None:
                profile.disable()
                profiling.release()
                Path(args.profile).mkdir(parents=True, exist_ok=True)
                profile.dump_stats(Path(args.profile) / f"{edits_at:04d}-{who}-{kind}.prof")
            wait, hold = timed.take()
            j = result.to_json()
            row = {"who": who, "kind": kind, "at": round(w - t0, 2), "edits_at": edits_at, "wall": round(took, 3),
                   "cpu": round(time.thread_time() - c, 3), "wait": round(wait, 3), "hold": round(hold, 3),
                   "ok": j.get("ok", True),
                   # ooxml-edit 0.121.0: where the call's time went (Result.timing), else none.
                   "timing": getattr(result, "timing", None)}
            if not row["ok"]:
                row["error"] = str(j.get("error"))[:300]
            changing = kind not in ("describe", "check", "read_batch")
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
                    out["by_edit"].append({"edits": count, "version": entry.version, **memory})

        fields_free = threading.Lock()

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
                if args.mix == "full":
                    shape = ("insert_row", "format", "batch", "insert_text")[turn % 4]
                    if rng.random() < 1 / 40 and fields_free.acquire(blocking=False):
                        shape = "fields"
                else:
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
                    if shape == "fields":
                        try:
                            call(view, f"w{w}", "fields", "word_fields", {"doc": doc, "action": "update"})
                        finally:
                            fields_free.release()
                    elif shape == "insert_text":
                        call(view, f"w{w}", "insert_text", "word_insert_text",
                             {"doc": doc, "target": rng.choice(chapter["body"]),
                              "text": " " + " ".join(rng.choice(NOTE_WORDS) for _ in range(rng.randint(2, 9)))})
                    elif shape == "insert_row":
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
                    if args.mix == "full" and not done():
                        # As an application does: its chapter checked after each edit, and
                        # every few turns a read.
                        span = [f"{chapter['head']}..{chapter['end']}"]
                        call(view, f"w{w}", "check", "check", {"doc": doc, "range": span})
                        if turn % 3 == 0 and not done():
                            if turn % 6 == 0:
                                call(view, f"w{w}", "describe", "describe", {"doc": doc})
                            else:
                                call(view, f"w{w}", "read_batch", "read_batch", {"ops": [
                                    {"tool": "word_read", "arguments": {"doc": doc, "range": span[0]}},
                                    {"tool": "word_inspect", "arguments": {"doc": doc, "target": table["id"]}},
                                    {"tool": "find_text", "arguments": {"doc": doc, "text": "End of chapter", "range": span[0]}}]})
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

        def viewer() -> None:
            from docx_agent.liveview import page_svg, view

            token, seen = None, None
            while not done():
                if entry.version == seen:
                    stop.wait(0.2)
                    continue
                c, w = time.thread_time(), time.perf_counter()
                try:
                    shown = view(session, doc, since=token)
                    for number in shown.changed[:args.view_pages]:
                        page_svg(session, doc, number, shown.token)
                    token, seen = shown.token, shown.version
                    row = {"who": "v", "kind": "view", "at": round(w - t0, 2),
                           "wall": round(time.perf_counter() - w, 3), "cpu": round(time.thread_time() - c, 3),
                           "wait": 0.0, "hold": 0.0, "ok": True, "version": shown.version,
                           "pages": shown.page_count, "changed": len(shown.changed)}
                except Exception as error:  # noqa: BLE001
                    seen = entry.version
                    row = {"who": "v", "kind": "view", "ok": False, "error": repr(error)[:300]}
                with lock:
                    calls.append(row)

        c0 = time.process_time()
        threads = ([threading.Thread(target=writer, args=(w,)) for w in range(args.writers)]
                   + [threading.Thread(target=reviewer, args=(k,)) for k in range(args.reviewers)]
                   + ([threading.Thread(target=viewer)] if args.views else []))
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
        if args.close:
            # The bench's own references go first (what an application that closes a
            # document holds of it: its id), so what is left is what the library keeps.
            entry = timed = None
            held["entry"] = None
            began = time.perf_counter()
            session.close(doc)
            closed = time.perf_counter()
            out["close_s"] = round(closed - began, 2)
            after: list = []
            while time.perf_counter() - closed < 3.0:
                after.append({"t": round(time.perf_counter() - closed, 2), "rss_mb": _memory(pid).get("rss"),
                              "workers_rss_mb": [_memory(k).get("rss") for k in _children(pid)]})
                time.sleep(0.1)
            out["after_close"] = after

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
    gc.callbacks.remove(clock)
    out["gc"] = clock.to_json()
    out["windows"] = windows(calls, args.window)
    out["process_cpu_pct"] = round(100 * out["process_cpu_s"] / out["elapsed_s"], 1) if out["elapsed_s"] else None
    changing = sum(1 for r in calls if "wall" in r and r["kind"] not in ("describe", "check", "read_batch", "view"))
    out["cpu_per_edit_s"] = round(out["process_cpu_s"] / changing, 2) if changing else None
    # Lane L4: the layouts made in Python instead of Rust here, by reason (this process's).
    from docx_agent import pages

    out["layout_fallbacks"] = pages.fallbacks()
    from docx_agent.rust import reads as _reads

    out["read_fallbacks"] = _reads.fallbacks()
    out["reads_served"] = _reads.served()
    Path(args.out).write_text(json.dumps(out, indent=1))
    print(json.dumps({"layout_fallbacks": out["layout_fallbacks"], "by_kind": out["by_kind"], "windows": out["windows"], "memory": out["memory"],
                      "elapsed_s": out["elapsed_s"], "process_cpu_s": out["process_cpu_s"],
                      "process_cpu_pct": out["process_cpu_pct"], "cpu_per_edit_s": out["cpu_per_edit_s"],
                      "gc": out["gc"], "failed": out["failed"],
                      "calls": len(calls)}, indent=1))


if __name__ == "__main__":
    main()
