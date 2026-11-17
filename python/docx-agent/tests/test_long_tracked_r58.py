"""Round 58: a long report with many tracked changes -- what one edit costs, what the process
keeps, and where a call's time went.

* Tracked formatting reads only what it formats (``Document._track_properties``'s
  ``within``): the bytes are those of tracking the whole part, edit for edit.
* ``describe``'s page count, ``render``'s range pages and ``save_document``'s coverage come
  from the worker holding the layout: the layout never comes to the caller for them.
* The layouts a document keeps in this process, and a live view's tokens, are bounded by
  what they cost (``LAYOUT_MEMORY``), the newest kept.
* A layout past its deadline is finished and kept by its worker, not stopped with it.
* ``describe``'s styles and revision authors are counted as before, in one pass.
* Every result says where its time went (``Result.timing``).

Made-up documents only (``tools/tracked_report.py``)."""

from __future__ import annotations

import sys
import threading
from collections import Counter
from pathlib import Path

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402
from ooxml_edit.tools import timing as timing_module  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent import blocks as blocks_module  # noqa: E402
from docx_agent import layout as layout_module  # noqa: E402
from docx_agent.revisions import track  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

from tracked_report import build  # noqa: E402

pytestmark = pytest.mark.any_faces


@pytest.fixture(scope="module")
def report() -> bytes:
    return build(chapters=2, seed=3)[0]


def _cells(document: Document, table: int = 0) -> list[str]:
    found = document.table(document.tables()[table].id)
    return [found.cell(r, c).paragraphs[0].id for r in range(1, 10) for c in range(4)]


# -- tracked formatting reads what it formats ---------------------------------------------------


def _whole_part(monkeypatch) -> None:
    """Tracking as before round 58: every paragraph and run of the part read and compared."""
    from docx_agent.revisions.mode import TrackingOps

    method = TrackingOps._track_properties

    def whole(self, parts, within=None):
        return method(self, parts)

    monkeypatch.setattr(TrackingOps, "_track_properties", whole)


@pytest.mark.parametrize("edit", ["style", "paragraph", "range", "run", "clear", "keep"])
def test_tracked_formatting_writes_what_tracking_the_whole_part_wrote(report, monkeypatch, edit):
    def run(document: Document) -> bytes:
        cells = _cells(document)
        body = [p.id for p in document.paragraphs() if len(p.text) > 300]
        with document.tracking(author="Writer", date="2026-11-16T09:00:00Z"):
            if edit == "style":
                for cell in cells[:8]:
                    document.paragraph(cell).set_style("Table Text Bold")
            elif edit == "paragraph":
                document.paragraph(body[0]).format(alignment="center", bold=True)
            elif edit == "range":
                document.format_range(document.range(f"{body[1]}@5:40"), italic=True, color="1F4E79")
            elif edit == "run":
                document.format_run(body[2], 0, underline="single")
            elif edit == "clear":
                document.clear_formatting(cells[3])
            else:
                document.keep_blocks_together(f"{body[3]}..{body[4]}")
        return document.to_bytes()

    scoped = run(Document.open(report))
    _whole_part(monkeypatch)
    whole = run(Document.open(report))
    assert scoped == whole


def test_outermost_keeps_each_root_once():
    document = Document.open(build(chapters=1, seed=4)[0])
    table = document.tables()[0]
    element = document._table_entry(table.id)[1].element
    cell_paragraph = element.find(".//{*}p")
    other = document.paragraphs()[0]._element
    roots = track.outermost([cell_paragraph, element, other, element])
    assert roots == [element, other]


# -- the caller never reads the whole layout for small facts ------------------------------------


def _box(**options):
    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, **options)


@pytest.mark.parametrize("workers", [1])
def test_describe_render_and_save_read_their_facts_where_the_layout_is_held(report, workers):
    expected = Document.open(report).layout()
    with _box(workers=workers) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        document = session.entry(doc).document
        described = box.dispatch(session, "describe", {"doc": doc})
        assert described.ok, described.to_json()
        assert described.data["pages"] == expected.pages_known
        table = document.tables()[1].id
        rendered = box.dispatch(session, "render", {"doc": doc, "range": [table]})
        assert rendered.ok, rendered.to_json()
        assert rendered.data["range"][0]["pages"] == expected.pages_of(table)
        saved = box.dispatch(session, "save_document", {"doc": doc, "name": "out.docx", "format": "docx"})
        assert saved.ok, saved.to_json()
        assert saved.data["coverage"] == expected.coverage_facts()
        # No whole Python layout was read here: lane L4b reads a version the Rust layout
        # lays out here, by the document's engine (no worker holds one); the rest in a worker.
        from docx_agent import pages

        assert all(pages.is_rust(kept.raw) for kept in document._layouts.values())


def test_layouts_kept_here_stay_within_their_memory(monkeypatch):
    document = Document.open(build(chapters=1, seed=5)[0])
    document.layout()
    cost = layout_module.layout_cost(next(iter(document._layouts.values())).raw)
    # Room for one layout: the newest only is kept (lane L4b: the room is the document's
    # account of the process's memory budget where the Rust layout lays it out; a quarter
    # of LAYOUT_MEMORY for docx2svg's Python layouts).
    monkeypatch.setattr(layout_module, "LAYOUT_MEMORY", 4 * cost)
    if layout_module._layout_account(document) is not None:
        import ooxml_core

        monkeypatch.setattr(document, "_layout_account", ooxml_core.MemoryBudget(cost + cost // 2).account())
    for k in range(3):
        document.paragraphs()[3].set_text(f"Changed {k}.")
        document.layout()
    assert len(document._layouts) == 1


def test_the_bodies_read_stay_within_their_bytes(monkeypatch):
    monkeypatch.setattr(blocks_module, "_BODIES", {})
    monkeypatch.setattr(blocks_module, "_BODIES_BYTES", 10)
    for k in range(3):
        blocks_module.read_body(b"<x>%d</x>" % k)
    assert list(blocks_module._BODIES) == [b"<x>2</x>"]


# -- a layout past its deadline is kept ---------------------------------------------------------


def test_a_layout_past_its_deadline_is_finished_by_its_worker():
    from docx_agent.tools._base import Engine, engage

    seen = []

    def run(fn, *args, **kwargs):
        seen.append(kwargs)
        return fn(*args)

    engine = Engine(run, 30.0, overrun=True)
    engine.lay_out(Document.new().to_bytes(), layout_module.layout_options(Document.new(), {}))
    assert seen[-1].get("overrun") is True
    with _box(workers=1) as box:
        session = box.session()
        doc = session.open(Document.new().to_bytes(), "a.docx")

        class Call:
            limits = box.limits
            toolbox = box
            entries = {doc: session.entry(doc)}

        engage(Call)
        assert session.entry(doc).document.converter.overrun is True


# -- describe's counts --------------------------------------------------------------------------


def test_describe_counts_styles_and_authors_as_each_paragraph_and_revision_says(report):
    from docx_agent.tools.read import _styles_in_use

    document = Document.open(report)
    counts = Counter()
    for story in document.stories:
        for paragraph in story.paragraphs:
            counts[paragraph.style_name] += 1
    assert _styles_in_use(document) == [{"name": name, "paragraphs": n} for name, n in counts.most_common(40)
                                        if name]
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        described = box.dispatch(session, "describe", {"doc": doc})
    assert described.data["revisions"]["by_author"] == dict(Counter(r.author for r in document.revisions()))


# -- where a call's time went -------------------------------------------------------------------


def test_every_result_says_where_its_time_went(report):
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        result = box.dispatch(session, "word_set_tracking", {"doc": doc, "on": True, "author": "Writer"})
        timing = result.timing
        assert set(timing) == {"total", "lock_wait", "work", "cpu", "held"}
        assert timing["total"] == pytest.approx(timing["lock_wait"] + timing["work"], abs=1e-3)
        assert 0 < timing["held"] <= timing["total"]
        assert "timing" not in result.to_json()          # the application's, not the model's
        failed = box.dispatch(session, "word_format", {"doc": doc, "targets": ["p:NOPE"], "bold": True})
        assert not failed.ok and failed.timing is not None


def test_a_timed_lock_counts_the_wait_and_the_outermost_hold(monkeypatch):
    now = [0.0]
    monkeypatch.setattr(timing_module.time, "perf_counter", lambda: now[0])
    lock = timing_module.TimedLock()
    waited, held = timing_module.clock()
    with lock:
        now[0] += 2.0
        with lock:                      # taken again: no wait, the hold goes on
            now[0] += 1.0
        assert lock._is_owned()
    after = timing_module.clock()
    assert after[0] - waited == 0.0 and after[1] - held == 3.0
    # Another thread holding it: this one's wait is the time until it lets go -- the holder
    # lets go once this thread has read the clock to start waiting.
    taken, letting_go = threading.Event(), threading.Event()
    main = threading.get_ident()

    def ticking() -> float:
        value = now[0]
        if threading.get_ident() == main and taken.is_set():
            letting_go.set()
        return value

    monkeypatch.setattr(timing_module.time, "perf_counter", ticking)

    def holder():
        with lock:
            taken.set()
            letting_go.wait()
            now[0] += 5.0

    thread = threading.Thread(target=holder)
    thread.start()
    taken.wait()
    with lock:
        pass
    thread.join()
    assert timing_module.clock()[0] - after[0] == 5.0


# -- letting go ---------------------------------------------------------------------------------


def test_closing_a_document_lets_go_of_what_was_kept_for_it(report):
    with _box(workers=0) as box:          # laid out in this process: its layouts held here
        session = box.session()
        doc = session.open(report, "report.docx")
        document = session.entry(doc).document
        described = box.dispatch(session, "describe", {"doc": doc})
        assert set(described.memory) >= {"document"}        # the application's figure
        assert "memory" not in described.to_json()["data"]
        lineage = layout_module.lineage(document)
        assert any(held.lineage == lineage for held in layout_module._HELD.values())
        document.layout()
        assert document._layouts
        session.close(doc)
        assert not document._layouts and document._layout_kept is None
        assert not any(held.lineage == lineage for held in layout_module._HELD.values())


def test_clear_caches_lets_go_of_every_document(report):
    document = Document.open(report)
    with _box(workers=0) as box:
        session = box.session()
        doc = session.open(report, "report.docx")
        box.dispatch(session, "describe", {"doc": doc})
    assert layout_module._HELD
    layout_module.clear_caches()
    assert not layout_module._HELD and not layout_module._RECENT and not blocks_module._BODIES
    assert document.layout().pages_known        # made again when next asked
