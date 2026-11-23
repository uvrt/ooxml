"""A scale read by ``check(scale=...)`` in a Word document (round 36, R31; ooxml-edit 0.85.0):
each target's value -- a paragraph, a cell, a table's column -- and the step it is styled
with, or none.  (``style_by_value``, which styles them, is out of the tools until it is
ported: P4e.)  Every document and figure is made up.
"""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.scales", reason="needs ooxml-edit 0.85 (scales)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402

import word_docs  # noqa: E402
from test_tables_from_data import COLUMNS, GRID, ROWS, CLOCK, Grid, call, ok  # noqa: E402

STEPS = [{"min": 1, "fill": "C62828", "label": "Low"},
         {"min": 2.5, "fill": "F9A825"},
         {"min": 4, "fill": "2E7D32", "label": "High"}]

MARKDOWN = ("Alpha scored {{t:checks/Alpha/rating}} and Gamma {{t:checks/Gamma/rating}}.\n\n"
            "| Topic | Score |\n|---|---|\n| Door | 4.5 |\n| Roof | 2 |\n| Wall | n/a |\n\n"
            "3\n\nThe mean is 4,2 here.\n")


@pytest.fixture(scope="module")
def toolbox():
    with Toolbox(TOOLS, formats=[FORMAT, GRID], groups=GROUPS, workers=0) as box:
        yield box


def _session(toolbox):
    session = toolbox.session(clock=CLOCK)
    session.open(word_docs.blank(), "report.docx")                       # d1
    session.adopt(Grid({"checks": (COLUMNS, ROWS)}), "grid", "book.grid")        # d2
    ok(call(toolbox, session, "word_insert_markdown", doc="d1", at="end", markdown=MARKDOWN))
    return session


def _ids(session):
    table = next(block for block in word_docs.body_blocks(word_docs.tree(word_docs.saved(session, "d1")))
                 if block.tag == f"{{{word_docs.W}}}tbl")
    table = f"t:{word_docs.rows_of(table)[0].get(word_docs.W14_PARA)}"
    found = word_docs.paragraphs(session, "d1")
    three = next(block for block, text in found if text == "3")
    mean = next(block for block, text in found if text.startswith("The mean"))
    return table, three, mean


def scale(*targets, **more):
    return {"name": "Score", "steps": STEPS, "targets": list(targets), **more}


def test_check_with_scale_alone_is_only_its_fact_and_range_scopes_it(toolbox):
    session = _session(toolbox)
    table, three, _ = _ids(session)
    alone = ok(call(toolbox, session, "check", doc="d1", scale=scale(f"{table}/col1", three)))
    assert set(alone.data) == {"scale", "validation"}  # and what saving would refuse (round 53)
    scoped = ok(call(toolbox, session, "check", doc="d1", range=[three],
                     scale=scale(f"{table}/col1", three))).data["scale"]
    assert [e["target"] for e in scoped["none"]] == [three] and scoped["checked"] == 1
    both = ok(call(toolbox, session, "check", doc="d1", include=["validate"], scale=scale(three)))
    assert "validate" in both.data and both.data["scale"]["none"]


def test_check_with_scale_and_pages_scopes_by_page(toolbox):
    """Round 59: ``pages`` with a scale read the values' pages in one layout request (the
    values' paragraphs asked for at once); it raised a TypeError before."""
    session = _session(toolbox)
    table, three, _ = _ids(session)
    given = scale(f"{table}/col1", three)
    found = ok(call(toolbox, session, "check", doc="d1", pages=[1], scale=given)).data["scale"]
    assert found["checked"] == 3 and [e["target"] for e in found["none"]] == [f"{table}/c1,1", f"{table}/c2,1",
                                                                               three]
    assert {e["page"] for e in found["none"]} == {1}
    beyond = ok(call(toolbox, session, "check", doc="d1", pages=[2], scale=given)).data["scale"]
    assert beyond["checked"] == 0


def test_a_target_of_another_kind_is_told_the_kinds_as_targets_are_written(toolbox):
    """A section given as a target is skipped with the kinds a target takes, values by
    reference written as a target writes them (``{{d:...}}``, as a target naming nothing is
    told too)."""
    import json

    session = _session(toolbox)
    _, three, _ = _ids(session)
    found = ok(call(toolbox, session, "check", doc="d1", scale=scale("s:1", three))).data["scale"]
    said = json.dumps(found)
    assert ("s:1 is a section: a target is a paragraph, a range, a cell, a table's column or {{d:...}} values"
            in said), said
