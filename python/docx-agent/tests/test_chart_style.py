"""A chart's look in a document: axis scales, gridlines, colours and markers, combination
charts, chart text, point labels, filled radar and bubble charts (ooxml-edit's ``Chart``
formatting, tested there) -- through ``edit_chart``.

Here: the document stays valid, the workbook agrees with the cache, every call is one undo
step, and the tool's arguments reach the library with Word's sizes.
"""

from __future__ import annotations

import pytest

from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

C = "{http://schemas.openxmlformats.org/drawingml/2006/chart}"
CATEGORIES = ["Governance", "People", "Data", "Technology", "Procure"]
CURRENT = [2.13, 2.10, 2.17, 2.33, 2.08]
TARGET = [4, 3, 4, 3, 4]


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def report(box):
    """A session holding the report, and the paragraph a chart goes after."""
    session = box.session()
    session.open(word_docs.blank(), "report.docx")
    word_docs.call(box, session, "word_insert_markdown", doc="d1", at="end",
                   markdown="# Maturity\n\nScores per area.\n\nThe end.")
    return session, word_docs.paragraph_ids(box, session, "d1")[1]


def edit_chart(box, session, **arguments):
    result = box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})
    assert result.ok, result.to_json()
    return result


def chart_root(session, drawing: str):
    data = word_docs.saved(session, "d1")
    return word_docs.tree(data, word_docs.chart_part(data, drawing))


def valid(box, session) -> None:
    assert word_docs.validation(box, session, "d1")["new"] == []


def test_the_maturity_gap_chart_in_a_document(box):
    session, anchor = report(box)
    made = edit_chart(box, session, target=anchor, action="add", chart_type="column", categories=CATEGORIES,
                      data=[{"name": "Current", "values": CURRENT},
                            {"name": "Target", "values": TARGET, "type": "line"}], gridlines="none")
    chart = made.data["id"]
    before = word_docs.saved(session, "d1")
    edit_chart(box, session, target=chart, action="set_axis_scale", axis="value",
               scale={"min": 0, "max": 5, "major_unit": 1})
    edit_chart(box, session, target=chart, action="format_series", series="Current", style={"fill": "tx2"})
    edit_chart(box, session, target=chart, action="format_point", series="Current", category="Procure",
               style={"fill": "accent2"})
    edit_chart(box, session, target=chart, action="format_series", series="Target",
               style={"line": "none", "marker": "diamond", "marker_size": 10, "marker_fill": "accent6"})
    edit_chart(box, session, target=chart, action="set_text_style", style={"size": 10})
    read = edit_chart(box, session, target=chart, action="read").data
    assert read["types"] == ["bar", "line"]
    assert read["scales"]["value"]["maximum"] == 5
    assert read["workbook"]["series"][1]["values"]["values"] == TARGET
    valid(box, session)
    for _ in range(5):
        assert box.dispatch(session, "undo", {"doc": "d1"}).ok
    assert word_docs.saved(session, "d1") == before


@pytest.mark.parametrize("kind", ["radar_filled", "bubble"])
def test_the_new_kinds_in_a_document(box, kind):
    session, anchor = report(box)
    if kind == "bubble":
        made = edit_chart(box, session, target=anchor, action="add", chart_type="bubble", values=[1, 2, 4],
                          data=[{"name": "Recs", "values": [4.5, 3, 4], "size": [3, 2, 1]}])
    else:
        made = edit_chart(box, session, target=anchor, action="add", chart_type="radar_filled",
                          categories=["Speed", "Cost", "Risk"], data=[{"name": "Now", "values": [3, 2, 4]}])
    read = edit_chart(box, session, target=made.data["id"], action="read").data
    assert read["type"] == ("bubble" if kind == "bubble" else "radar")
    # Word's sizes (measured): 9 pt labels.
    sizes = {n.get("sz") for n in chart_root(session, made.data["id"]).iter(
        "{http://schemas.openxmlformats.org/drawingml/2006/main}defRPr") if n.get("sz")}
    assert "900" in sizes
    valid(box, session)


def test_the_tool_formats_a_chart_and_labels_a_matrix(box):
    session, anchor = report(box)

    def run(**arguments):
        result = box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})
        assert result.ok, result.to_json()
        return result

    run(target=anchor, action="add", chart_type="bar", categories=["Worst", "Mid", "Best"],
        data=[{"name": "Score", "values": [1.8, 2.4, 3.1]}], axis="category",
        scale={"reverse": True}, gridlines="none", ref="r")
    read = run(target="$r", action="read")
    assert read.data["scales"]["category"]["reverse"] is True
    assert read.data["scales"]["value"]["major_gridlines"] is False
    run(target=anchor, action="add", chart_type="bubble", values=[1, 2, 4],
        data=[{"name": "Recs", "values": [4.5, 3, 4], "size": [3, 2, 1]}], ref="b")
    labelled = run(target="$b", action="set_point_labels", series="Recs",
                   labels=["R1", "R2", "R3"], position="right", cells=True)
    assert labelled.data["point_labels"] == {"Recs": ["R1", "R2", "R3"]}
    run(target="$b", action="set_text_style", part="data_labels", style={"size": 8})
    refused = box.dispatch(session, "edit_chart", {"doc": "d1", "target": "$r",
                                                    "action": "set_series_type",
                                                    "series": "Score", "chart_type": "line"})
    assert not refused.ok and "horizontal bars" in refused.error.message
    valid(box, session)


# -- add with its look (round 5, T2) -------------------------------------------------------------


@pytest.mark.parametrize("kind, data, extra", [
    ("column", [{"name": "Now", "values": [3, 2, 4]},
                {"name": "Goal", "values": [4, 4, 4], "type": "line"}], {}),
    ("line", [{"name": "Now", "values": [3, 2, 4]}, {"name": "Goal", "values": [4, 4, 4]}], {}),
    ("pie", [{"name": "Now", "values": [3, 2, 4]}], {}),
    ("scatter", [{"name": "Now", "values": [3, 2, 4]}], {"values": [1, 2, 3]}),
    ("dot", [{"name": "Now", "values": [3, 2, 4]}, {"name": "Goal", "values": [4, 4, 4]}], {}),
])
def test_add_with_its_look_writes_the_chart_the_separate_calls_write(box, kind, data, extra):
    from ooxml_edit.charts.actions import add_style_actions

    # A colour per series, a pie's per slice; the scatter chart's one series takes one.
    look = {"data_labels": True, "colors": ["accent2", "1F4E79"][:1 if kind == "scatter" else 2],
            "style": {"size": 8, "bold": True}}
    arguments = {"doc": "d1", "action": "add", "chart_type": kind,
                 "data": data, "width": 360, "gridlines": "none", **extra}
    if not extra:
        arguments["categories"] = ["Speed", "Cost", "Risk"]
    written = []
    for together in (True, False):
        session, anchor = report(box)
        arguments["target"] = anchor
        added = box.dispatch(session, "edit_chart", {**arguments, **(look if together
                                                                      else {})})
        assert added.ok, added.to_json()
        chart_id = next(c for c in added.created if c.startswith("d:"))
        if not together:
            for action, given in add_style_actions(kind, data, **look):
                done = box.dispatch(session, "edit_chart", {"doc": "d1", "target": chart_id,
                                                            "action": action, **given})
                assert done.ok, done.to_json()
        written.append(etree.tostring(chart_root(session, chart_id)))
        valid(box, session)
    assert written[0] == written[1]
    assert b'val="accent2"' in written[0] and b'sz="800"' in written[0]
    assert b"c:dLbls" in written[0]


# -- the dot chart (round 6, R10) ---------------------------------------------------------------


def test_a_dot_chart_on_a_fixed_scale_reads_back_as_dot(box):
    session, anchor = report(box)

    def run(**arguments):
        result = box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})
        assert result.ok, result.to_json()
        return result

    added = run(target=anchor, action="add", chart_type="dot",
                categories=["North", "South", "East"],
                data=[{"name": "Mean", "values": [3.2, 4.1, 2.7]}], axis="value",
                scale={"min": 1, "max": 5}, data_labels=True, ref="dots")
    assert added.data["type"] == "dot"
    read = run(target="$dots", action="read")
    assert read.data["type"] == "dot" and read.data["types"] == ["line"]
    assert (read.data["scales"]["value"]["minimum"],
            read.data["scales"]["value"]["maximum"]) == (1, 5)
    run(target="$dots", action="set_values", series="Mean", values=[1, 2, 3])
    assert run(target="$dots", action="add_series", text="Goal",
               values=[4, 4, 4]).data["type"] == "dot"
    again = run(target="$dots", action="read").data
    assert again["type"] == "dot" and again["types"] == ["line"]
    valid(box, session)


# -- round 9, N2: a scale's default axis; refusals that name what to give -------------------------


def test_a_scale_without_an_axis_and_what_a_refusal_names(box):
    session, anchor = report(box)

    def call(**arguments):
        return box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})

    made = call(target=anchor, action="add", chart_type="column", categories=["A", "B", "C"],
                data=[{"name": "Score", "values": [1, 2, 3]}], scale={"max": 4}, ref="c")
    assert made.ok, made.to_json()
    assert call(target="$c", action="read").data["scales"]["value"]["maximum"] == 4
    assert call(target="$c", action="set_axis_scale", scale={"min": 1}).ok
    before = word_docs.saved(session, "d1")
    scatter = call(target=anchor, action="add", chart_type="scatter", values=[1, 2],
                   data=[{"name": "Y", "values": [3, 4]}], scale={"max": 5})
    assert scatter.error.message.startswith("add with scale needs axis: x or y")
    assert word_docs.saved(session, "d1") == before      # nothing stays
    for arguments, words in [
            ({"action": "set_value", "series": "Score", "value": 2},
             "set_value needs category: 'A', 'B' or 'C' (or a number from 0 to 2)"),
            ({"action": "rename_series", "text": "Now"},
             "rename_series needs series: 'Score' (or 0)"),
            ({"action": "set_values", "series": "Score"},
             "set_values needs values: one number per category (3)"),
            ({"action": "add_category", "category": "D"},  # the label is text (S10)
             "add_category needs text: the new category's label"),
            ({"action": "set_axis_scale", "scale": {"reverse": True}},
             "set_axis_scale needs axis: category or value"),
            ({"action": "format_point", "series": "Score", "style": {"fill": "tx2"}},
             "format_point needs category: 'A', 'B' or 'C'")]:
        error = call(target="$c", **arguments).error
        assert error.code == "invalid_arguments" and words in error.message, error.message
    missing = call(target=anchor, action="add", categories=["A"],
                   data=[{"name": "S", "values": [1]}]).error
    assert "add needs chart_type: one of column, stacked_column" in missing.message


# N4 (review): one lookup in both formats, ooxml-edit's; a name wins over a position.
# pptx-agent's tests/test_chart_style.py makes the same calls on a slide.


@pytest.mark.parametrize("names, which, changed", [
    (["1", "2"], "1", 0),          # the series called "1"
    (["2", "1"], "1", 1),
    (["North", "South"], "1", 1),  # no series called "1": position 1
])
def test_a_series_name_wins_over_a_position(box, names, which, changed):
    session, anchor = report(box)

    def call(**arguments):
        return box.dispatch(session, "edit_chart", {"doc": "d1", **arguments})

    assert call(target=anchor, action="add", chart_type="column", categories=["1", "Q2"],
                data=[{"name": name, "values": [1, 2]} for name in names], ref="c").ok
    done = call(target="$c", action="set_value", series=which, category="1", value=9)
    assert done.ok, done.to_json()
    assert [series["values"] for series in done.data["series"]] == [
        [1 if k != changed else 9, 2] for k in range(2)]   # category "1" by name, too
    error = call(target="$c", action="set_value", series="9", value=1, category="Q2").error
    assert error.code == "not_found" and error.field == "series"
    assert error.valid_options == names
    error = call(target="$c", action="rename_series", series="East", text="x").error
    assert error.code == "label_not_found" and error.valid_options == names
