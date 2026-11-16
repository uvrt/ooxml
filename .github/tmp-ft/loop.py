"""TEMPORARY (lane J): loop block_stress's --more --tracked mix on a free-threaded Python,
for a time budget; every run that differs is written to out/ as JSON: its seed, the first
differences, and a log of each change applied from a draft (the parts its patch carried,
the document's relationships before and after) and each commit."""
import base64
import json
import os
import sys
import threading
import time

sys.path.insert(0, sys.argv[1])
import block_stress  # noqa: E402
from docx_agent import blocks  # noqa: E402
from ooxml_edit.tools import DocumentEntry  # noqa: E402

seeds = [int(s) for s in sys.argv[2].split(",")]
budget = float(sys.argv[3])
os.makedirs("out", exist_ok=True)
LOG: list = []
LOCK = threading.Lock()
RELS = "word/_rels/document.xml.rels"


def _rels(document):
    data = document.package.read(RELS)
    return data.decode("utf-8", "replace") if data else None


real_apply = blocks.Blocks.apply


def apply(self, document, plan):
    before = _rels(document)
    version = document.history.version
    out = real_apply(self, document, plan)
    with LOCK:
        LOG.append({"kind": "apply", "thread": threading.current_thread().name, "version_before": version,
                    "call": getattr(block_stress_recorder_local(), "call", None),
                    "parts": [(p, None if b is None else len(b), None if a is None else len(a)) for p, b, a in plan.parts],
                    "patch_parts": [(p, None if b is None else b.decode("utf-8", "replace") if p == RELS else len(b),
                                     None if a is None else a.decode("utf-8", "replace") if p == RELS else len(a))
                                    for p, b, a in plan.patch.parts],
                    "merged": plan.merged, "relinked": plan.relinked, "moved": plan.moved,
                    "rels_before": before, "rels_after": _rels(document), "patch_version": plan.patch.version})
    return out


blocks.Blocks.apply = apply
_recorder = {}


def block_stress_recorder_local():
    rec = _recorder.get("r")
    return rec.local if rec is not None else threading.local()


real_init = block_stress.Recorder.__init__


def init(self):
    real_init(self)
    _recorder["r"] = self


block_stress.Recorder.__init__ = init
real_note = DocumentEntry.note_change


def note_change(entry, parts, before, *more):
    real_note(entry, parts, before, *more)
    rec = _recorder.get("r")
    with LOCK:
        LOG.append({"kind": "commit", "call": getattr(rec.local, "call", None) if rec else None,
                    "before": before, "after": entry.version, "parts": sorted(parts) if parts else parts,
                    "rels": _rels(entry.document) if entry.document is not None else None})


DocumentEntry.note_change = note_change
started = time.time()
runs = fails = 0
while time.time() - started < budget:
    for seed in seeds:
        LOG.clear()
        found = block_stress.run(seed, threads=4, calls=8, chapters=3, first_fill=bool(seed % 2), tracked=True,
                                 more=True)
        runs += 1
        if not found["same"]:
            fails += 1
            with open(f"out/fail-{fails}-seed{seed}.json", "w") as f:
                json.dump({"seed": seed, "run": runs, **found, "log": list(LOG)}, f, default=str, indent=1)
        print(json.dumps({"run": runs, "seed": seed, "same": found["same"]}), flush=True)
print(f"RUNS {runs} FAILS {fails}", flush=True)
with open("out/summary.txt", "w") as f:
    f.write(f"runs {runs} fails {fails}\n")
sys.exit(1 if fails else 0)
