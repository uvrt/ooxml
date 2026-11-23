"""``check``'s ``column_fit`` (round 31, R6): a table column whose widest line uses well
under its width beside a column that wraps heavily, with the widths that let both fit.
Made-up label and value tables in a new blank package; the thresholds are the Rust layout
facts' (``crates/ooxml-word-pages`` ``layout_facts/columns.rs``: ``FIT_SLACK`` and the rest)."""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox
from layout_reads import table_ids, text_width

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import word_docs

VALUE = ("The courier collects the parcels from the side door each weekday morning, checks the labels "
         "against the list and signs the sheet that stays in the tray by the door. ") * 2
#: Points a fitted column keeps past its widest line (the Rust layout facts' ``FIT_SLACK``).
FIT_SLACK = 3.0


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, result.to_json()
    return result


def labelled(box, rows: list[list[str]], widths: list[float] | None = None) -> tuple[object, str, str]:
    """A table of ``rows`` after the blank document's paragraph, as wide as the text column
    (its columns equal, or ``widths`` in points): the session, the document and the table."""
    total = text_width()
    columns = len(rows[0])
    twips = [round(w * 20) for w in widths] if widths else [total // columns] * columns
    grid = "".join(f'<w:gridCol w:w="{w}"/>' for w in twips)
    body = (f'<w:p/><w:tbl><w:tblPr><w:tblW w:w="{sum(twips)}" w:type="dxa"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>'
            + "".join("<w:tr>" + "".join(f'<w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/></w:tcPr><w:p><w:r>'
                                         f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p></w:tc>'
                                         for w, text in zip(twips, row)) + "</w:tr>" for row in rows)
            + "</w:tbl><w:p/>")
    session = box.session()
    d = session.open(word_docs.with_body(body), "fit.docx")
    return session, d, table_ids(box, session, d)[0]


def column_fit(box, session, d: str) -> list:
    return _ok(box, session, "check", doc=d, include=["layout"]).data["layout"]["column_fit"]


def lines_of(box, session, d: str, table: str, column: int) -> list[int]:
    """How many lines each cell of ``column`` is drawn on."""
    found = _ok(box, session, "word_inspect", doc=d, target=table, layout=True).data["blocks"][0]
    return [sum(placed["lines"] for block in row[column]["blocks"] for placed in block["placements"])
            for row in found["rows"]]


def test_a_label_column_beside_wrapping_values_is_a_fact_with_widths_that_fit(box):
    session, d, table = labelled(box, [["Pickup", VALUE], ["Day", "Weekdays"], ["Contact", "The front desk"]])
    [entry] = column_fit(box, session, d)
    roomy, wraps = entry["roomy"], entry["wraps"]
    assert entry["table"] == table and entry["pages"] == [1]
    assert roomy["column"] == 0 and wraps["column"] == 1 and roomy["width"] == wraps["width"]
    assert roomy["widest_line"] < 0.6 * roomy["width"] and roomy["longest_word"] <= roomy["widest_line"]
    assert wraps["lines"] >= 6 and wraps["longest_word"] < wraps["widest_line"]
    widths = entry["widths"]
    assert sum(widths) == pytest.approx(roomy["width"] + wraps["width"], abs=0.2)
    assert widths[0] > roomy["widest_line"] + FIT_SLACK            # its line, the slack and its margins
    assert entry["fix"] == {"tool": "word_format_table", "calls": [
        {"table": table, "scope": "column", "column": 0, "width": widths[0]},
        {"table": table, "scope": "column", "column": 1, "width": widths[1]}]}
    before = lines_of(box, session, d, table, 1)[0]
    for call in entry["fix"]["calls"]:
        result = box.dispatch(session, entry["fix"]["tool"], {"doc": d, **call})
        assert result.ok, result.error
    table = table_ids(box, session, d)[0]
    assert lines_of(box, session, d, table, 0) == [1, 1, 1]                  # the labels still on one line
    assert lines_of(box, session, d, table, 1)[0] < before                   # the value on fewer
    assert column_fit(box, session, d) == []


def test_columns_that_both_wrap_or_fit_are_no_fact(box):
    for rows in ([[VALUE, VALUE]], [["Pickup", "Weekdays"], ["Day", "Mornings"]], [[VALUE]]):
        session, d, _ = labelled(box, rows)
        assert column_fit(box, session, d) == [], rows


def test_a_roomy_column_that_frees_little_is_no_fact(box):
    # The label column already narrow: under MIN_FREED to give.
    session, d, _ = labelled(box, [["Pickup", VALUE]], widths=[60, 393.3])
    assert column_fit(box, session, d) == []


def test_the_neighbour_that_wraps_most_gets_the_room(box):
    session, d, table = labelled(box, [["A long first value that wraps " * 4, "Day", VALUE]])
    [entry] = column_fit(box, session, d)
    assert entry["roomy"]["column"] == 1 and entry["wraps"]["column"] == 2
    grid = word_docs.tables(session, d)[0].find("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}tblGrid")
    first = int(grid[0].get("{http://schemas.openxmlformats.org/wordprocessingml/2006/main}w")) / 20
    assert entry["widths"][0] == pytest.approx(first, abs=0.1)
