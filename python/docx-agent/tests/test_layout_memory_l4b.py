"""Lane L4b: the Rust layout's memory as docx-agent holds it -- one engine per document in
the process that owns the document and none in the workers; a check's facts made here from
it; the layouts kept for readers bounded by the process's memory budget, by weight."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent import Document  # noqa: E402
from docx_agent import layout as layout_module  # noqa: E402
from docx_agent import pages  # noqa: E402
from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))

import worker_probe  # noqa: E402
from tracked_report import build  # noqa: E402

pytestmark = [pytest.mark.any_faces,
              pytest.mark.skipif(not pages.available(), reason="needs ooxml_core's Rust layout")]


def test_checks_are_answered_here_and_no_worker_holds_a_rust_engine():
    data = build(chapters=1, seed=3)[0]
    expected = Document.open(data).layout()
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=1) as box:
        session = box.session()
        doc = session.open(data, "report.docx")
        document = session.entry(doc).document
        described = box.dispatch(session, "describe", {"doc": doc})
        assert described.ok, described.to_json()
        assert described.data["pages"] == expected.pages_known
        checked = box.dispatch(session, "check", {"doc": doc, "include": ["reflow", "layout"]})
        assert checked.ok, checked.to_json()
        assert checked.data["reflow"]["pages"] == expected.page_count
        table = document.tables()[0].id
        rendered = box.dispatch(session, "render", {"doc": doc, "range": [table]})
        assert rendered.ok, rendered.to_json()
        assert box.pool.run(worker_probe.rust_engines, timeout=120) == 0
        assert box.pool.run(worker_probe.held_lineages, timeout=120) == {}
        assert all(pages.is_rust(kept.raw) for kept in document._layouts.values())


def test_kept_layouts_are_bounded_by_the_budget_by_weight(monkeypatch):
    import ooxml_core

    document = Document.open(build(chapters=1, seed=4)[0])
    document.layout()
    first = next(iter(document._layouts.values())).raw
    cost = layout_module.layout_cost(first)
    assert first.weight < cost
    budget = ooxml_core.MemoryBudget(3 * cost)
    monkeypatch.setattr(document, "_layout_account", budget.account())
    for k in range(8):
        document.paragraphs()[3].set_text(f"Changed {k}.")
        document.layout()
    kept = list(document._layouts.values())
    assert 1 <= len(kept) < 8, "the oldest let go for room, by what each weighs"
    assert kept[-1].data == document.to_bytes(), "the newest always kept"
    assert 0 < budget.used <= budget.limit


def test_the_process_budget_counts_the_engines_memos_and_paint():
    # The process's budget is every engine's: other tests' engines, let go at any drain
    # (docx_agent.finalizers), move it either way.  So this document's own engine is what
    # is measured, and the budget is asked only whether it counts that engine's part.
    import docx2svg

    document = Document.open(build(chapters=1, seed=6)[0])
    layout = document.layout()
    layout.page_count
    options = docx2svg.ConvertOptions(**layout_module.layout_options(document, {}))
    engine = pages._engine(layout_module.lineage(document), options, make=False)
    assert engine is not None, "the document's own engine laid it out"
    mine = engine.stats()
    now = pages.memory()
    assert mine["memo_bytes"] > 0, "the engine's memos are counted as held"
    assert now["held"] >= mine["memo_bytes"], "the process budget holds what the engine's memos hold"
    assert now["used"] >= mine["paint_bytes"], "and its pages' paint within the limit"
