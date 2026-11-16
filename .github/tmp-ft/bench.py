"""TEMPORARY (lane L): writers on one made-up report, real docx-agent tool calls -- after
lanes F-K's bench.  Each writer, per turn: an edit (a batch: a Markdown insert before its
chapter's end and a text set in its chapter) then a check of its chapter.  Reports wall and
CPU per call, edits per second, the process's CPU and cores used, the document's lock held
by edits on drafts, conflicts and redos, memory, the collector's pauses.

    python bench.py doc.docx out.json --writers N --turns T [--no-worth]
"""
import argparse
import datetime as dt
import gc
import json
import os
import random
import resource
import statistics
import sys
import threading
import time

from ooxml_edit.tools import Toolbox
from ooxml_edit.tools.limits import Limits

from docx_agent.tools import FORMAT, GROUPS, TOOLS

CLOCK = lambda: dt.datetime(2026, 11, 16, 9, 0, tzinfo=dt.timezone.utc)  # noqa: E731
WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger "
         "window garden timber bridge market evening canvas thread pebble summit").split()
INCLUDE = ["validate", "layout", "numbers", "reflow", "fields"]
GC = {"n": [0, 0, 0], "s": [0.0, 0.0, 0.0], "max": 0.0}
PHASES: dict = {}
_PHASE_LOCK = None


def timed_phases():
    """Wall and CPU time per phase of a change's critical section and of a draft's work, summed
    over the run (the thread's own CPU)."""
    global _PHASE_LOCK
    import threading as _t

    _PHASE_LOCK = _t.Lock()
    from docx_agent import blocks as _blocks
    from ooxml_edit.tools import session as _session

    def wrap(owner, name, label):
        original = getattr(owner, name)

        def timed(*args, **kwargs):
            w, c = time.perf_counter(), time.thread_time()
            try:
                return original(*args, **kwargs)
            finally:
                dw, dc = time.perf_counter() - w, time.thread_time() - c
                with _PHASE_LOCK:
                    item = PHASES.setdefault(label, [0, 0.0, 0.0])
                    item[0] += 1
                    item[1] += dw
                    item[2] += dc
        setattr(owner, name, timed)

    wrap(_blocks.Blocks, "draft", "draft: copy made or reused")
    wrap(_blocks.Blocks, "prepared", "draft: patch made")
    wrap(_blocks.Blocks, "check", "lock: check")
    wrap(_blocks.Blocks, "apply", "lock: apply")
    wrap(_blocks.Blocks, "settle", "lock: settle")
    wrap(_session.DocumentEntry, "commit", "lock: commit (read copy kept)")
    wrap(_session.DocumentEntry, "snapshot", "lock: snapshot")


def sentence(rng, n=12):
    return " ".join(rng.choice(WORDS) for _ in range(n)).capitalize() + "."


def summary(values):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    pick = lambda q: values[min(len(values) - 1, int(q * len(values)))]  # noqa: E731
    return {"n": len(values), "p50": round(statistics.median(values), 4), "p90": round(pick(0.9), 4),
            "p99": round(pick(0.99), 4), "max": round(values[-1], 4), "mean": round(sum(values) / len(values), 4)}


def gc_watch():
    started = {}

    def cb(phase, info):
        g = info["generation"]
        if phase == "start":
            started[g] = time.perf_counter()
        elif g in started:
            took = time.perf_counter() - started.pop(g)
            GC["n"][g] += 1
            GC["s"][g] += took
            GC["max"] = max(GC["max"], took)
    gc.callbacks.append(cb)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("doc")
    ap.add_argument("out")
    ap.add_argument("--writers", type=int, default=1)
    ap.add_argument("--turns", type=int, default=4)
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--no-worth", action="store_true", help="drafts at any size (DRAFT_BODY_LIMIT off)")
    args = ap.parse_args()
    gc_watch()
    timed_phases()
    if args.no_worth:
        import docx_agent.blocks as blocks
        blocks.DRAFT_BODY_LIMIT = 1 << 40
    data = open(args.doc, "rb").read()
    out = {"args": vars(args), "gil": getattr(sys, "_is_gil_enabled", lambda: True)(),
           "env": {k: v for k, v in os.environ.items() if k.startswith(("DOCX_AGENT", "OOXML", "PYTHON_GIL"))},
           "cpus": os.cpu_count()}
    calls, errors = [], []
    lock = threading.Lock()
    limits = Limits(render_timeout=3600, layout_timeout=3600, batch_timeout=3600, worker_queue_timeout=3600)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=args.workers, limits=limits) as box:
        session = box.session(clock=CLOCK)
        t0 = time.perf_counter()
        doc = session.open(data, "report.docx")
        out["open_s"] = round(time.perf_counter() - t0, 3)
        document = session.entry(doc).document
        paragraphs = document.paragraphs()
        heads = [p.id for p in paragraphs if p.text.startswith("Chapter ") and (p.style or "").lower().startswith("heading")]
        ends = [p.id for p in paragraphs if p.text.startswith("End of chapter ")]
        position = {p.id: k for k, p in enumerate(paragraphs)}
        bodies = [[p.id for p in paragraphs[position[h]:position[e]] if len(p.text) > 60 and not (p.style or "").startswith(("Heading", "List"))]
                  for h, e in zip(heads, ends)]
        out["chapters"], out["paragraphs"] = len(heads), len(paragraphs)
        w0, c0 = time.perf_counter(), time.thread_time()
        r = box.dispatch(session, "check", {"doc": doc, "range": [f"{heads[0]}..{ends[0]}"], "include": INCLUDE})
        out["first_check"] = {"wall": round(time.perf_counter() - w0, 3), "cpu": round(time.thread_time() - c0, 3), "ok": r.ok}
        d = box.dispatch(session, "describe", {"doc": doc}).to_json()
        out["pages"] = (d.get("data") or {}).get("pages")

        def call(view, label, kind, tool, arguments):
            c, w = time.thread_time(), time.perf_counter()
            result = box.dispatch(view, tool, arguments)
            entry = {"label": label, "kind": kind, "wall": time.perf_counter() - w, "cpu": time.thread_time() - c,
                     "ok": result.ok, "version": result.version, "block": getattr(result, "block", None) or None}
            if not result.ok:
                entry["error"] = json.dumps(result.to_json().get("error"))[:300]
            with lock:
                calls.append(entry)

        go = threading.Barrier(args.writers)

        def writer(w):
            rng = random.Random(1000 + 7919 * w)
            view = session.view(context={"agent": f"w{w}"})
            ch = w % len(heads)
            mine = bodies[ch]
            go.wait()
            for turn in range(args.turns):
                label = f"w{w}:t{turn}"
                try:
                    md = (f"### Note {w + 1}.{turn + 1} {sentence(rng, 2)[:-1]}\n\n"
                          + " ".join(sentence(rng, rng.randint(10, 20)) for _ in range(3)) + "\n\n"
                          + "\n".join(f"- {sentence(rng, rng.randint(4, 9))}" for _ in range(3)))
                    ops = [{"tool": "word_insert_markdown", "arguments": {"doc": doc, "at": f"before:{ends[ch]}", "markdown": md}},
                           {"tool": "word_set_text", "arguments": {"doc": doc, "target": mine[(turn + w) % len(mine)],
                                                                   "text": " ".join(sentence(rng, rng.randint(10, 20)) for _ in range(3))}}]
                    call(view, label + ":edit", "edit", "batch", {"ops": ops})
                    call(view, label + ":check", "check", "check",
                         {"doc": doc, "range": [f"{heads[ch]}..{ends[ch]}"], "include": INCLUDE})
                except Exception as exc:  # noqa: BLE001
                    with lock:
                        errors.append(f"{label}: {exc!r}"[:300])

        threads = [threading.Thread(target=writer, args=(w,)) for w in range(args.writers)]
        c0, w0 = os.times(), time.perf_counter()
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        wall = time.perf_counter() - w0
        c1 = os.times()
    edits = [c for c in calls if c["kind"] == "edit" and c["ok"]]
    checks = [c for c in calls if c["kind"] == "check" and c["ok"]]
    blocks_ = [c["block"] for c in calls if c["kind"] == "edit" and c["block"]]
    cpu = (c1.user + c1.system) - (c0.user + c0.system)
    out.update({
        "wall": round(wall, 3), "cpu": round(cpu, 3), "cores_used": round(cpu / wall, 2) if wall else None,
        "edits": len(edits), "edits_per_s": round(len(edits) / wall, 3) if wall else None,
        "edit": summary([c["wall"] for c in edits]), "edit_cpu": summary([c["cpu"] for c in edits]),
        "check": summary([c["wall"] for c in checks]), "check_cpu": summary([c["cpu"] for c in checks]),
        "drafted": sum(1 for b in blocks_ if b.get("tries")),
        "to_lock": sum(1 for b in blocks_ if b.get("lock")),
        "lock_reasons": sorted({b["lock"] for b in blocks_ if b.get("lock")}),
        "conflicts": sum(len(b.get("conflicts") or []) for b in blocks_),
        "redos": sum(max(0, b.get("tries", 0) - 1) for b in blocks_),
        "held_s": round(sum(b.get("held", 0.0) for b in blocks_), 3),
        "held": summary([b.get("held") for b in blocks_ if b.get("held") is not None]),
        "draft_cpu": summary([b.get("cpu") for b in blocks_ if b.get("cpu") is not None]),
        "peak_rss_mb": round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024),
        "gc": {"runs": GC["n"], "pause_s": [round(x, 3) for x in GC["s"]], "max_pause_s": round(GC["max"], 4)},
        "failed": [c for c in calls if not c["ok"]][:10], "errors": errors[:10],
        "phases": {k: {"n": v[0], "wall_s": round(v[1], 3), "cpu_s": round(v[2], 3)} for k, v in PHASES.items()}})
    out["calls"] = calls
    with open(args.out, "w") as f:
        json.dump(out, f, indent=1, default=str)
    print(json.dumps({k: v for k, v in out.items() if k != "calls"}, default=str), flush=True)


if __name__ == "__main__":
    sys.exit(main())
