"""Round 17, P3: ``word_compare_blocks`` -- tables and paragraphs copied from a sample,
compared with the sample (here or in another open document), every difference with the
reference's value and the calls that give it; the fix calls run through the tools, and a
second comparison finds nothing left to fix."""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit 0.4 (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from layout_reads import block_ids, table_element, table_ids, written  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

#: How far two column widths may differ, in points, and be the same (the Rust comparison's).
WIDTH_TOLERANCE = 0.5
_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

SAMPLE = ("Samples.\n\n"
          "| Item | Owner | Status |\n| --- | --- | --- |\n| Sample item | Someone | Open |\n\n"
          "A closing line.")


def _sample(box, session) -> tuple[str, str]:
    """A made-up sample: a table whose header row repeats and is shaded, its body row's
    middle cell centred vertically, a heading-styled paragraph after it.  The document and
    the table."""
    sample = written(box, session, SAMPLE, "samples.docx")
    table = table_ids(box, session, sample)[0]
    table = _ok(box, session, "word_format_table", doc=sample, table=table, scope="table",
                header_rows=1).renamed.get(table, table)
    for column in range(3):
        _ok(box, session, "word_format_table", doc=sample, table=table, scope="cell", row=0, column=column,
            shading="1F3864")
    _ok(box, session, "word_format_table", doc=sample, table=table, scope="cell", row=1, column=1,
        vertical_alignment="center")
    return sample, table_ids(box, session, sample)[0]


def _cell(box, session, doc: str, table: str, row: int, column: int) -> list[str]:
    """The ids of a cell's paragraphs."""
    found = _ok(box, session, "word_inspect", doc=doc, target=table).data["blocks"][0]
    return [block["id"] for block in found["rows"][row][column]["blocks"]]


def _compare(box, session, doc: str, blocks: list[str], reference: str) -> dict:
    return _ok(box, session, "word_compare_blocks", doc=doc, blocks=blocks, reference={"block": reference}).data


def _columns(box, session, doc: str, table: str) -> list[float]:
    """A table's grid's column widths, in points."""
    grid = table_element(box, session, doc, table).find(_W + "tblGrid")
    return [int(column.get(_W + "w")) / 20 for column in grid]


def _differences(data: dict, block: str | None = None) -> dict[str, dict]:
    return {d["property"]: d for d in data["differences"] if block is None or d["block"] == block}


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, arguments, result.error and result.error.to_json())
    return result


def _apply(box, session, doc: str, data: dict) -> int:
    """Every fix call of a comparison, in the order listed; how many."""
    count = 0
    for difference in data["differences"]:
        if difference["fix"]:
            for call in difference["fix"]["calls"]:
                _ok(box, session, difference["fix"]["tool"], doc=doc, **call)
                count += 1
    return count


def test_a_drifted_copy_from_another_document_is_fixed_toward_its_sample(box):
    session = box.session()
    d2, reference = _sample(box, session)
    d1 = written(box, session, "A report.", "report.docx")
    first = _ok(box, session, "word_copy_from", doc=d1, source_doc=d2, range=reference, at="end").data["blocks"][0]
    second = _ok(box, session, "word_copy_from", doc=d1, source_doc=d2, range=reference, at="end").data["blocks"][0]
    # Filled: another item row in each (more rows than the sample: a difference by design).
    for table in (first, second):
        _ok(box, session, "word_edit_table", doc=d1, table=table, action="insert_row", index=1, side="after")
    # The second drifts: no repeated header, a shaded item row, a column made narrower, a
    # cell's text bold and centred, two cells merged, a vertical alignment lost.
    _ok(box, session, "word_format_table", doc=d1, table=second, scope="table", header_rows=0)
    _ok(box, session, "word_format_table", doc=d1, table=second, scope="row", row=1, shading="F2F2F2")
    _ok(box, session, "word_format_table", doc=d1, table=second, scope="column", column=0, width=100)
    second = table_ids(box, session, d1)[1]
    cell = _cell(box, session, d1, second, 1, 2)[0]
    _ok(box, session, "word_format", doc=d1, targets=[cell], bold=True, alignment="center")
    _ok(box, session, "word_edit_table", doc=d1, table=second, action="merge", first={"row": 2, "col": 1},
        last={"row": 2, "col": 2})
    _ok(box, session, "word_format_table", doc=d1, table=second, scope="cell", row=1, column=1,
        vertical_alignment="bottom")

    result = _ok(box, session, "word_compare_blocks", doc=d1, blocks=[first, second],
                 reference={"doc": d2, "block": reference})
    data = result.data
    assert data["reference"] == reference and data["compared"] == [first, second] and data["same"] == []
    # The untouched copy differs only by its extra row, which has no fix.
    assert {d["property"]: (d["value"], d["reference"], d["fix"]) for d in data["differences"]
            if d["block"] == first} == {"rows": (2, 1, None)}
    found = _differences(data, second)
    assert {"header_rows", "rows", "merges", "widths", "shading", "vertical_alignment", "alignment", "bold"} \
        <= set(found)
    assert found["header_rows"]["value"] == 0 and found["header_rows"]["reference"] == 1
    assert found["shading"]["cells"] == [f"{second}/c1,*"]
    assert (found["shading"]["value"], found["shading"]["reference"]) == ("F2F2F2", None)
    assert found["shading"]["fix"] == {"tool": "word_format_table", "calls": [
        {"table": second, "scope": "row", "row": 1, "shading": "none"}]}
    assert found["widths"]["columns"] == [0] and found["widths"]["value"][0] == 100
    assert found["merges"]["value"] == ["c1..c2"] and found["merges"]["reference"] == []
    assert found["bold"]["value"] is True and found["bold"]["reference"] is None
    assert found["bold"]["fix"]["calls"] == [{"targets": [cell], "clear": ["bold"]}]
    # Every difference has the shape ppt_compare_slides's have, whatever it compares.
    for difference in data["differences"]:
        assert {"block", "property", "value", "reference", "fix"} <= set(difference)
    assert "Compared 2 block(s)" in result.summary

    assert _apply(box, session, d1, data) >= 7
    again = _ok(box, session, "word_compare_blocks", doc=d1, blocks=[first, second],
                reference={"doc": d2, "block": reference}).data
    assert [d["property"] for d in again["differences"]] == ["rows", "rows"], again["differences"]
    assert all(d["fix"] is None for d in again["differences"])
    assert word_docs.validation(box, session, d1)["new"] == []


def test_paragraphs_compare_their_style_and_direct_formatting(box):
    session = box.session()
    d = written(box, session, "# A heading\n\nBody text.\n\n# Another heading\n\nMore text.")
    first, _, second, body = block_ids(box, session, d)[:4]
    _ok(box, session, "word_format", doc=d, targets=[second], size=20, space_after=3)
    _ok(box, session, "word_format", doc=d, targets=[body], paragraph_style="Heading 2")
    data = _compare(box, session, d, [second, body], first)
    found = _differences(data, second)
    assert set(found) == {"size", "space_after"}
    assert found["size"]["value"] == 20 and found["size"]["reference"] is None
    assert found["size"]["fix"] == {"tool": "word_format", "calls": [{"targets": [second], "clear": ["size"]}]}
    other = _differences(data, body)
    assert other["paragraph_style"]["value"] == "heading 2" and other["paragraph_style"]["reference"] == "heading 1"
    assert other["paragraph_style"]["fix"]["calls"] == [{"targets": [body], "paragraph_style": "heading 1"}]
    # A table against a paragraph: only the kind.
    _ok(box, session, "word_insert_markdown", doc=d, at="end", markdown="| a | b |\n| --- | --- |\n| 1 | 2 |")
    table = table_ids(box, session, d)[0]
    assert _compare(box, session, d, [table], first)["differences"] == [
        {"block": table, "property": "kind", "value": "table", "reference": "paragraph", "fix": None}]


def test_the_same_sample_in_the_same_document_and_the_width_tolerance(box):
    session = box.session()
    d, reference = _sample(box, session)
    copy = _ok(box, session, "word_copy_from", doc=d, source_doc=d, range=reference, at="end").data["blocks"][0]
    assert _compare(box, session, d, [copy], reference)["same"] == [copy]
    columns = _columns(box, session, d, reference)
    copy = _ok(box, session, "word_format_table", doc=d, table=copy, scope="column", column=1,
               width=columns[1] + WIDTH_TOLERANCE).renamed.get(copy, copy)
    assert _compare(box, session, d, [copy], reference)["same"] == [copy]
    _ok(box, session, "word_format_table", doc=d, table=copy, scope="column", column=1, width=columns[1] + 1)
    data = _compare(box, session, d, [copy], reference)
    found = _differences(data)
    assert set(found) == {"widths"} and found["widths"]["columns"] == [1]
    # Through the tool, the reference in the same document: no doc in it.
    assert _apply(box, session, d, data) == 1
    assert _compare(box, session, d, [copy], reference)["same"] == [copy]
    error = box.dispatch(session, "word_compare_blocks", {"doc": d, "blocks": [copy],
                                                          "reference": {"doc": "d9", "block": reference}})
    assert not error.ok and error.error.code == "not_found"


def test_table_style_and_borders_are_fixed_where_the_tool_writes_them(box):
    session = box.session()
    d, reference = _sample(box, session)
    _ok(box, session, "word_format_table", doc=d, table=reference, scope="table", borders="single")
    # Borders alone leave the table's style as it was (they set it to "single" before).
    style = table_element(box, session, d, reference).find(f"{_W}tblPr/{_W}tblStyle")
    assert style is not None and style.get(_W + "val") == "TableGrid"
    _ok(box, session, "word_format_table", doc=d, table=reference, scope="cell", row=1, column=0, borders="dashed")
    copy = _ok(box, session, "word_copy_from", doc=d, source_doc=d, range=reference, at="end").data["blocks"][0]
    assert _ok(box, session, "word_compare_blocks", doc=d, blocks=[copy],
               reference={"block": reference}).data["same"] == [copy]
    _ok(box, session, "word_format_table", doc=d, table=copy, scope="table", style="Normal Table", borders="dotted")
    _ok(box, session, "word_format_table", doc=d, table=copy, scope="cell", row=1, column=0, borders="double")
    data = _ok(box, session, "word_compare_blocks", doc=d, blocks=[copy], reference={"block": reference}).data
    found = _differences(data)
    assert set(found) == {"table_style", "table_borders", "borders"}, data["differences"]
    assert (found["table_style"]["value"], found["table_style"]["reference"]) == ("Normal Table", "Table Grid")
    assert found["table_borders"]["reference"]["top"] == "single 0.5pt auto"
    assert found["table_borders"]["fix"]["calls"] == [{"table": copy, "scope": "table", "borders": "single"}]
    # The cell's four sides, and the sides its neighbours share with it (setting a cell's
    # border sets the neighbour's side it meets too): only the cell's own has a fix, which
    # gives the neighbours theirs back.
    borders = {tuple(e["cells"]): e for e in data["differences"] if e["property"] == "borders"}
    assert set(borders) == {(f"{copy}/c1,0",), (f"{copy}/c0,0",), (f"{copy}/c1,1",)}
    assert borders[(f"{copy}/c1,0",)]["fix"]["calls"] == [{"table": copy, "scope": "cell", "row": 1, "column": 0,
                                                          "borders": "dashed"}]
    assert borders[(f"{copy}/c1,1",)]["value"] == {"left": "double 0.5pt auto"}
    assert borders[(f"{copy}/c1,1",)]["fix"] is None
    assert _apply(box, session, d, data) == 3
    assert _ok(box, session, "word_compare_blocks", doc=d, blocks=[copy],
               reference={"block": reference}).data["same"] == [copy]


# -- round 26, W6: a cell's paragraphs paired by an alignment -------------------------------------


def _two_paragraph_cell(box, session) -> tuple[str, str, str]:
    """A made-up sample whose notes cell holds a heading-styled label line and a body line,
    and a copy of it in the same document: ``(document, sample, copy)``."""
    d = written(box, session, "| Item | Notes |\n| --- | --- |\n| Item one | Body text here |", "cells.docx")
    sample = table_ids(box, session, d)[0]
    [body] = _cell(box, session, d, sample, 1, 1)
    label = _ok(box, session, "word_insert_markdown", doc=d, at=f"before:{body}",
                markdown="Section label").data["blocks"][0]
    _ok(box, session, "word_format", doc=d, targets=[label], paragraph_style="Heading 2")
    sample = table_ids(box, session, d)[0]
    copy = _ok(box, session, "word_copy_from", doc=d, source_doc=d, range=sample, at="end").data["blocks"][0]
    assert _compare(box, session, d, [copy], sample)["same"] == [copy]
    return d, sample, copy


def _notes(box, session, d: str, table: str) -> list[str]:
    return _cell(box, session, d, table, 1, 1)


def test_a_cell_missing_its_label_line_is_never_restyled_as_the_label(box):
    session = box.session()
    d, sample, copy = _two_paragraph_cell(box, session)
    label, body = _notes(box, session, d, copy)
    _ok(box, session, "word_delete", doc=d, target=label)
    _ok(box, session, "word_set_text", doc=d, target=body, text="Filled in by hand")
    _ok(box, session, "word_format", doc=d, targets=[body], bold=True)
    data = _compare(box, session, d, [copy], sample)
    found = _differences(data)
    # By position the body line would pair with the label and be given its style.
    assert "paragraph_style" not in found, data["differences"]
    assert set(found) == {"paragraphs", "missing_paragraph", "bold"}
    assert (found["paragraphs"]["value"], found["paragraphs"]["reference"], found["paragraphs"]["fix"]) == (1, 2, None)
    assert found["paragraphs"]["cells"] == [f"{copy}/c1,1"]
    assert found["missing_paragraph"]["cells"] == [f"{copy}/c1,1"] and found["missing_paragraph"]["fix"] is None
    assert found["missing_paragraph"]["reference"] == {"paragraph_style": "heading 2", "text": "Section label",
                                                       "index": 0}
    # The body line pairs with the sample's body line (by style): its direct formatting is compared.
    assert found["bold"]["paragraphs"] == [body]
    assert found["bold"]["fix"] == {"tool": "word_format", "calls": [{"targets": [body], "clear": ["bold"]}]}
    # The fixes leave only what no tool fixes.
    assert _apply(box, session, d, data) == 1
    again = _compare(box, session, d, [copy], sample)
    assert [e["property"] for e in again["differences"]] == ["paragraphs", "missing_paragraph"]


def test_a_cell_with_a_line_the_sample_has_not_reports_it_as_extra(box):
    session = box.session()
    d, copy_of_sample, sample = _two_paragraph_cell(box, session)   # the two-line cell is the copy now
    label, body = _notes(box, session, d, sample)
    _ok(box, session, "word_delete", doc=d, target=label)
    data = _compare(box, session, d, [copy_of_sample], sample)
    found = _differences(data)
    assert set(found) == {"paragraphs", "extra_paragraph"}, data["differences"]
    assert (found["paragraphs"]["value"], found["paragraphs"]["reference"]) == (2, 1)
    extra = found["extra_paragraph"]
    assert extra["paragraphs"] == [_notes(box, session, d, copy_of_sample)[0]]
    assert extra["value"] == {"paragraph_style": "heading 2", "text": "Section label"}
    assert extra["reference"] is None and extra["fix"] is None


def test_reordered_lines_are_missing_and_extra_not_restyled(box):
    session = box.session()
    d, sample, copy = _two_paragraph_cell(box, session)
    label, body = _notes(box, session, d, copy)
    _ok(box, session, "word_move", doc=d, blocks=label, after=body)
    data = _compare(box, session, d, [copy], sample)
    found = _differences(data)
    # Equal counts, but the texts say the pairs are crossed: no style fixes, no count.
    assert set(found) == {"missing_paragraph", "extra_paragraph"}, data["differences"]
    missing = found["missing_paragraph"]["reference"]
    assert found["extra_paragraph"]["value"] == {k: v for k, v in missing.items() if k != "index"}
    assert all(d_["fix"] is None for d_ in data["differences"])


def test_equal_counts_keep_pairing_by_position(box):
    session = box.session()
    d, sample, copy = _two_paragraph_cell(box, session)
    label, body = _notes(box, session, d, copy)
    # Filled with other text and the label restyled: still compared line by line.
    _ok(box, session, "word_set_text", doc=d, target=label, text="Another label")
    _ok(box, session, "word_set_text", doc=d, target=body, text="Other text entirely")
    _ok(box, session, "word_format", doc=d, targets=[label], paragraph_style="Heading 3")
    found = _differences(_compare(box, session, d, [copy], sample))
    assert set(found) == {"paragraph_style"}
    assert found["paragraph_style"]["paragraphs"] == [label]
    assert found["paragraph_style"]["fix"]["calls"] == [{"targets": [label], "paragraph_style": "heading 2"}]


def test_a_label_matching_by_text_is_restyled_and_like_lines_repeat_their_pattern(box):
    session = box.session()
    d, sample, copy = _two_paragraph_cell(box, session)
    label, body = _notes(box, session, d, copy)
    # The label's text a little edited and restyled, the body line gone: the label still
    # pairs by its text, so its style is fixed; the body line is missing.
    _ok(box, session, "word_delete", doc=d, target=body)
    _ok(box, session, "word_set_text", doc=d, target=label, text="Section labels")
    _ok(box, session, "word_format", doc=d, targets=[label], paragraph_style="Heading 3")
    found = _differences(_compare(box, session, d, [copy], sample))
    assert set(found) == {"paragraphs", "missing_paragraph", "paragraph_style"}
    assert found["missing_paragraph"]["reference"]["text"] == "Body text here"
    assert found["paragraph_style"]["fix"]["calls"] == [{"targets": [label], "paragraph_style": "heading 2"}]
    # A sample line is the pattern for more lines like it after it: a cell filled with
    # three body lines under its label is as the sample.
    d, sample, copy = _two_paragraph_cell(box, session)
    _, body = _notes(box, session, d, copy)
    _ok(box, session, "word_insert_markdown", doc=d, at=f"after:{body}", markdown="A second body line")
    third = _notes(box, session, d, copy)[2]
    _ok(box, session, "word_insert_markdown", doc=d, at=f"after:{third}", markdown="A third body line")
    assert len(_notes(box, session, d, copy)) == 4
    data = _compare(box, session, d, [copy], sample)
    assert data["same"] == [copy], data["differences"]
