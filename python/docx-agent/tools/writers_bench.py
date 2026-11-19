"""Writers on one long document through docx-agent's toolbox, the Rust session serving the
calls it can: per-edit wall (p50, p99, max), the time each call held the document and waited
for its lock, which calls the Rust session served and which fell back, the bridge's calls
and the memory.  The 3k-page bench of the heavy gates (.github/workflows/gates.yml), as lanes
P2c6 and P2c7 ran it.

    python tools/writers_bench.py long.docx [writers=24] [edits=5] [mix=rust|mixed]

Each writer, in its own thread, edits paragraphs of its own: ``word_set_text``, then
``replace_text`` of the text it set; ``mix=mixed`` makes every fourth edit a
``word_insert_text`` (a call the session hands back to Python).  One JSON line on stdout.
"""

from __future__ import annotations

import json
import resource
import sys
import threading
import time

TOOLS = ("word_set_text", "replace_text", "word_insert_text")


def rss_mb() -> float:
    """The process's peak resident memory (macOS reports bytes, Linux KiB)."""
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak / 2**20 if sys.platform == "darwin" else peak / 2**10


def percentile(values: list[float], p: float) -> float:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(p * len(ordered)))]


def main() -> None:
    from ooxml_edit.tools import Toolbox

    from docx_agent.rust import edits, reads
    from docx_agent.tools import FORMAT, GROUPS, TOOLS as ALL

    path = sys.argv[1]
    writers = int(sys.argv[2]) if len(sys.argv) > 2 else 24
    per = int(sys.argv[3]) if len(sys.argv) > 3 else 5
    mix = sys.argv[4] if len(sys.argv) > 4 else "rust"
    box = Toolbox(ALL, formats=[FORMAT], groups=GROUPS)
    session = box.session()
    before = rss_mb()
    started = time.perf_counter()
    with open(path, "rb") as handle:
        doc = session.open(handle.read(), name="long.docx")
    opened = time.perf_counter() - started
    after_open = rss_mb()
    ids = [p.id for p in session.entry(doc).document.paragraphs() if p.text.strip() and p.id.startswith("p:")]
    print("paragraphs with ids", len(ids), "open s", round(opened, 2), file=sys.stderr)
    calls: list[tuple[str, float, float, float]] = []      # tool, wall, held, lock wait
    errors: list = []
    lock = threading.Lock()

    def writer(k: int) -> None:
        view = session.view(context={"agent": f"w{k}"})
        mine = ids[k * 37 % len(ids):][:per] or ids[:per]
        for n in range(per):
            target = mine[n % len(mine)]
            if mix == "mixed" and n % 4 == 3:
                tool, arguments = "word_insert_text", {"doc": doc, "target": target, "text": f" w{k}n{n}"}
            elif n % 2 == 1:
                tool, arguments = "replace_text", {"doc": doc, "find": f"Writer {k} edit {n - 1}",
                                                   "replace": f"Writer {k} replaced {n}"}
            else:
                tool, arguments = "word_set_text", {"doc": doc, "target": target, "text": f"Writer {k} edit {n}"}
            at = time.perf_counter()
            result = box.dispatch(view, tool, arguments)
            wall = time.perf_counter() - at
            timing = result.timing or {}
            with lock:
                calls.append((tool, wall, timing.get("held", 0.0), timing.get("lock_wait", 0.0)))
                if not result.ok:
                    errors.append((tool, result.error.to_json() if result.error else None))

    threads = [threading.Thread(target=writer, args=(k,)) for k in range(writers)]
    cpu = time.process_time()
    started = time.perf_counter()
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    wall = time.perf_counter() - started
    cpu = time.process_time() - cpu
    walls = [w for _, w, _, _ in calls]

    def by_tool(index: int, reduce) -> dict:
        return {tool: (round(reduce([c[index] for c in calls if c[0] == tool]) * 1000, 1)
                       if any(c[0] == tool for c in calls) else None) for tool in TOOLS}

    rust = reads._session()
    print(json.dumps({
        "writers": writers, "edits": len(calls), "mix": mix, "wall_s": round(wall, 2), "cpu_s": round(cpu, 2),
        "p50_ms": round(percentile(walls, 0.5) * 1000, 1), "p99_ms": round(percentile(walls, 0.99) * 1000, 1),
        "max_ms": round(max(walls) * 1000, 1),
        "by_tool_p99_ms": by_tool(1, lambda v: percentile(v, 0.99)),
        "held_max_ms": by_tool(2, max), "lock_wait_max_ms": by_tool(3, max),
        "errors": len(errors), "first_error": errors[:1],
        "served": edits.served(), "fallbacks": edits.fallbacks(), "bridge_calls": dict(rust.bridge_calls()),
        "store": dict(rust.store_stats(session.entry(doc).handle.doc)),
        "open_s": round(opened, 2), "rss_open_mb": round(after_open - before, 1), "rss_max_mb": round(rss_mb(), 1),
    }))


if __name__ == "__main__":
    main()
