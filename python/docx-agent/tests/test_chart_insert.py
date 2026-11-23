"""edit_chart's add action in a document (LW3).

The chart part and its workbook are the Rust core's; here: the inline drawing Word writes,
the new paragraph after a block, one undo step, the document staying valid, the workbook
agreeing with the cache, tracked insertion, and the tool's refusals.
"""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

VOLUMES = [{"name": "Volume", "values": [120, 135, 128, 141]}]
MONTHS = ["Jan", "Feb", "Mar", "Apr"]
C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def report(box):
    session = box.session()
    session.open(word_docs.blank(), "report.docx")
    word_docs.call(box, session, "word_insert_markdown", doc="d1", at="end",
                   markdown="# Volumes\n\nVolumes rose all year.\n\nThe end.")
    return session


def chart(toolbox, session, **arguments):
    return toolbox.dispatch(session, "edit_chart", {"doc": "d1", **arguments})


def add(box, session, target, chart_type, categories, data, **arguments):
    result = chart(box, session, target=target, action="add", chart_type=chart_type, categories=categories,
                   data=data, **arguments)
    assert result.ok, result.to_json()
    return result


def _frame(session, drawing):
    """The drawing's ``wp:inline`` or ``wp:anchor``."""
    body = word_docs.tree(word_docs.saved(session, "d1"))
    return next(d for d in body.iter(WP + "docPr") if d.get("id") == drawing[2:]).getparent()


def _size(frame) -> list[int]:
    extent = frame.find(WP + "extent")
    return [round(int(extent.get("cx")) / 12700), round(int(extent.get("cy")) / 12700)]


def test_a_chart_after_a_paragraph(box):
    session = report(box)
    body = word_docs.paragraph_ids(box, session, "d1")
    edit = add(box, session, body[1], "line", MONTHS, VOLUMES, text="Monthly volumes")
    for axis, title in (("category", "Month"), ("value", "Units")):
        assert chart(box, session, target=edit.data["id"], action="set_axis_title", axis=axis, text=title).ok
    after = word_docs.paragraph_ids(box, session, "d1")
    assert len(after) == len(body) + 1 and after[2] in edit.created
    read = chart(box, session, target=edit.data["id"], action="read").data
    assert read["type"] == "line" and read["categories"] == MONTHS
    assert read["axes"]["value"] == "Units" and read["title"] == "Monthly volumes"
    book = read["workbook"]
    assert book["categories"]["values"] == MONTHS
    assert book["series"][0]["values"]["values"] == VOLUMES[0]["values"]
    assert word_docs.validation(box, session, "d1")["new"] == []
    frame = _frame(session, edit.data["id"])
    assert frame.tag == WP + "inline" and _size(frame) == [432, 252]


def test_a_chart_after_a_heading_is_not_a_heading(box):
    session = report(box)
    heading = word_docs.paragraph_ids(box, session, "d1")[0]
    edit = add(box, session, heading, "column", MONTHS, VOLUMES)
    new = word_docs.element(session, "d1", edit.data["paragraph"])
    style = new.find(f"{{{word_docs.W}}}pPr/{{{word_docs.W}}}pStyle")
    assert style is None or not style.get(f"{{{word_docs.W}}}val").lower().startswith("heading")


def test_a_radar_chart_has_word_s_legend_at_the_top(box):
    session = report(box)
    edit = add(box, session, word_docs.paragraph_ids(box, session, "d1")[1], "radar", MONTHS,
               VOLUMES + [{"name": "Target", "values": [130, 130, 130, 130]}])
    read = chart(box, session, target=edit.data["id"], action="read").data
    assert read["type"] == "radar"
    assert read["workbook"]["series"][1]["values"]["values"] == [130] * 4
    data = word_docs.saved(session, "d1")
    legend = word_docs.tree(data, word_docs.chart_part(data, edit.data["id"])).find(f".//{C}legendPos")
    assert legend.get("val") == "t"
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_width_keeps_word_s_ratio_and_undo_restores(box):
    session = report(box)
    before = word_docs.read(box, session, "d1")
    edit = add(box, session, word_docs.paragraph_ids(box, session, "d1")[1], "pie", MONTHS, VOLUMES, width=216)
    assert _size(_frame(session, edit.data["id"])) == [216, 126]
    assert box.dispatch(session, "undo", {"doc": "d1"}).ok
    assert word_docs.read(box, session, "d1") == before


def test_bad_data_changes_nothing(box):
    session = report(box)
    before = word_docs.saved(session, "d1")
    bad = chart(box, session, target=word_docs.paragraph_ids(box, session, "d1")[1], action="add",
                chart_type="pie", categories=MONTHS, data=VOLUMES * 2)
    assert not bad.ok and bad.error.code == "invalid_arguments", bad.to_json()
    assert word_docs.saved(session, "d1") == before


def test_tracked_insertion(box):
    session = report(box)
    assert box.dispatch(session, "word_set_tracking", {"doc": "d1", "on": True, "author": "Claude"}).ok
    add(box, session, word_docs.paragraph_ids(box, session, "d1")[1], "column", MONTHS, VOLUMES)
    assert any(r.tag == f"{{{word_docs.W}}}ins" for r in word_docs.revisions(session, "d1"))
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_the_tool_adds_a_chart_and_titles_its_axes_in_one_batch(box):
    session = report(box)
    anchor = word_docs.paragraph_ids(box, session, "d1")[1]
    result = box.dispatch(session, "batch", {"ops": [
        {"tool": "edit_chart", "arguments": {"doc": "d1", "target": anchor, "action": "add",
                                             "chart_type": "line", "categories": MONTHS,
                                             "data": VOLUMES, "text": "Monthly volumes",
                                             "ref": "c"}},
        {"tool": "edit_chart", "arguments": {"doc": "d1", "target": "$c",
                                             "action": "set_axis_title", "axis": "category",
                                             "text": "Month"}},
        {"tool": "edit_chart", "arguments": {"doc": "d1", "target": "$c",
                                             "action": "set_axis_title", "axis": "value",
                                             "text": "Units"}}]})
    assert result.ok, result.to_json()
    refused = chart(box, session, target=anchor, action="add", chart_type="line", categories=MONTHS, data=VOLUMES,
                    box={"x": 0, "y": 0, "w": 1, "h": 1})
    assert not refused.ok and refused.error.field == "box"
    bad = chart(box, session, target=anchor, action="add", chart_type="column", categories=MONTHS,
                data=[{"name": "A", "values": [1]}])
    assert not bad.ok and bad.error.code == "invalid_arguments"
