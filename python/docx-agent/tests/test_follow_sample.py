"""Round 31, R8: ``word_follow_sample`` -- copies of a sample table (or paragraph) made to
follow the revised sample in one call, each keeping its own text and values; what could not
follow reported per copy with why.  The check is the comparison's: after following,
``compare_blocks`` finds nothing left but what the result names (and the copies' own row
counts).  Made-up documents only."""

from __future__ import annotations

import pytest

pytest.importorskip("ooxml_edit.tools.shared", reason="needs ooxml-edit 0.4 (the shared tools)")

from ooxml_edit.tools import Toolbox  # noqa: E402

from layout_reads import block_ids, ids_of, table_element, table_ids, written  # noqa: E402

from docx_agent.tools import FORMAT, GROUPS, TOOLS  # noqa: E402
import word_docs  # noqa: E402

#: Why each difference a copy keeps was not followed (the Rust session's ``word_follow_sample``).
WHY = {
    "kind": "the copy is not the same kind of block as the sample",
    "columns": "the copy's columns hold its values: columns are not added or removed",
    "widths": "the copy has another number of columns than the sample",
    "merges": "cells are not merged or split: it would move the copy's text",
    "missing_paragraph": "a line of the sample's text the copy's cell lacks: text is the copy's own",
    "extra_paragraph": "a line no sample paragraph matches by text or style: it keeps its formatting",
    "paragraph_style": "this document has no such style and Word has no built-in one by its name",
    "table_style": "this document has no such style and Word has no built-in one by its name",
    "list": "list membership is not followed: the sample's list is another list (word_lists)",
}
#: What kind of difference each ``not_followed`` entry is.
KINDS = ("own", "changed_by_copy", "structure")
KIND = {
    "kind": "structure", "columns": "structure", "widths": "structure", "merges": "structure",
    "missing_paragraph": "own", "extra_paragraph": "own", "list": "own",
    "paragraph_style": "changed_by_copy", "table_style": "changed_by_copy",
}
#: Differences reported per copy at most.
NOT_FOLLOWED_CAP = 20
#: The run properties a comparison reads from a paragraph's text.
RUN = {"bold", "italic", "underline", "size", "font", "color", "highlight", "strike", "caps", "small_caps"}
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

AUTHOR, DATE = "A. Reviewer", "2026-11-04T09:00:00Z"
SAMPLE = ("Parts.\n\n"
          "| Part | Maker | Stock |\n| --- | --- | --- |\n| Sample part | Someone | 12 |\n\n"
          "A closing line.")


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, arguments, result.error and result.error.to_json())
    return result


def _texts(box, session, doc: str, table: str) -> list[list[str]]:
    return [[word_docs.cell_text(c) for c in word_docs.cells_of(r)]
            for r in word_docs.rows_of(table_element(box, session, doc, table))]


def _cell(box, session, doc: str, table: str, row: int, column: int) -> str:
    """The id of a cell's first paragraph."""
    found = _ok(box, session, "word_inspect", doc=doc, target=table).data["blocks"][0]
    return found["rows"][row][column]["blocks"][0]["id"]


def _document(box, session) -> tuple[str, str, list[str]]:
    """A sample and two copies filled with their own values (one with an extra row), then the
    sample revised: header row repeated and shaded, centred, a wider first column, a body
    cell centred vertically, the body text bold at 9 pt."""
    doc = written(box, session, SAMPLE, "parts.docx")
    sample = table_ids(box, session, doc)[0]
    copies = [_ok(box, session, "word_copy_from", doc=doc, source_doc=doc, range=sample, at="end").data["blocks"][0]
              for _ in range(2)]
    sample = table_ids(box, session, doc)[0]
    _ok(box, session, "word_set_text", doc=doc, target=_cell(box, session, doc, copies[0], 1, 0), text="Gear wheel")
    _ok(box, session, "word_edit_table", doc=doc, table=copies[0], action="insert_row", index=1)
    _ok(box, session, "word_set_text", doc=doc, target=_cell(box, session, doc, copies[0], 2, 0), text="Axle")
    _ok(box, session, "word_set_text", doc=doc, target=_cell(box, session, doc, copies[1], 1, 2), text="40")
    _ok(box, session, "word_format_table", doc=doc, table=sample, scope="table", header_rows=1, alignment="center")
    for column in range(3):
        _ok(box, session, "word_format_table", doc=doc, table=sample, scope="cell", row=0, column=column,
            shading="1F3864")
    _ok(box, session, "word_format_table", doc=doc, table=sample, scope="cell", row=1, column=1,
        vertical_alignment="center")
    _ok(box, session, "word_format_table", doc=doc, table=sample, scope="column", column=0, width=200)
    _ok(box, session, "word_format", doc=doc, targets=[_cell(box, session, doc, sample, 1, c) for c in range(3)],
        bold=True, size=9)
    return doc, sample, copies


def _left(box, session, doc: str, sample: str, copies: list[str]) -> list[str]:
    """What a comparison still finds, less the copies' own row counts and the run formatting
    of paragraphs with no text (their marks took it, which a comparison does not read)."""
    texts = dict(word_docs.paragraphs(session, doc))
    out = []
    data = _ok(box, session, "word_compare_blocks", doc=doc, blocks=copies, reference={"block": sample}).data
    for d in data["differences"]:
        if d["property"] == "rows":
            continue
        if d["property"] in RUN and d.get("paragraphs") and not any(texts.get(p, "").strip()
                                                                    for p in d["paragraphs"]):
            continue
        out.append(d["property"])
    return out


def _follow(box, session, doc: str, sample: str, copies, **arguments):
    """``word_follow_sample`` of ``copies`` (one or a list) toward ``sample`` in the same document."""
    return box.dispatch(session, "word_follow_sample", {
        "doc": doc, "sample": {"block": sample}, "copies": copies if isinstance(copies, list) else [copies],
        **arguments})


@pytest.fixture
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, arguments, result.error and result.error.to_json())
    return result


def test_copies_follow_the_revised_sample_and_keep_their_values(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    texts = {c: _texts(box, session, doc, c) for c in copies}
    assert {"widths", "table_alignment", "shading", "vertical_alignment", "bold", "size"} \
        <= set(_left(box, session, doc, sample, copies))
    result = _follow(box, session, doc, sample, copies)
    assert result.ok and len(result.changed) == 2, result.to_json()
    report = result.data
    assert _left(box, session, doc, sample, copies) == []
    assert {c: _texts(box, session, doc, c) for c in copies} == texts       # their own text and rows
    assert [len(word_docs.rows_of(table_element(box, session, doc, c))) for c in copies] == [3, 2]
    first, second = report["copies"]
    assert first["copy"] == copies[0] and first["changed"] and first["table"] == ["alignment"]
    assert first["widths"] and first["rows"] == 3 and first["cells"] and first["runs"]
    assert first["not_followed"] == [] and second["not_followed"] == []
    # The extra row took the sample's last body row's look, and its empty cells its run
    # formatting on their paragraph marks (text typed there is bold at 9 pt).
    row = word_docs.rows_of(table_element(box, session, doc, copies[0]))[2]
    cell = word_docs.cells_of(row)[1]
    assert cell.find(f"{W}tcPr/{W}vAlign").get(W + "val") == "center"
    mark = cell.find(f"{W}p/{W}pPr/{W}rPr")
    assert mark is not None and mark.find(W + "b") is not None
    assert word_docs.validation(box, session, doc)["new"] == []
    # One undo step takes every copy back.
    _ok(box, session, "undo", doc=doc)
    assert {"widths", "bold"} <= set(_left(box, session, doc, sample, copies))


def test_preview_reports_the_same_and_changes_nothing(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    before = word_docs.saved(session, doc)
    preview = _follow(box, session, doc, sample, copies, preview=True)
    assert preview.ok and word_docs.saved(session, doc) == before
    applied = _follow(box, session, doc, sample, copies)
    assert preview.data["preview"] and not applied.data["preview"]
    assert preview.data["copies"] == applied.data["copies"]
    # Following again finds nothing to change: no step.
    again = _follow(box, session, doc, sample, copies)
    assert again.changed == [] and not any(e["changed"] for e in again.data["copies"])


def test_a_copy_s_own_emphasis_stays(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    paragraph = _cell(box, session, doc, copies[1], 1, 0)
    _ok(box, session, "word_set_text", doc=doc, target=paragraph, text="Note: a part")
    _ok(box, session, "word_format", doc=doc, targets=[f"{paragraph}@0:5"], italic=True)
    assert _follow(box, session, doc, sample, copies[1]).ok
    element = word_docs.element(session, doc, _cell(box, session, doc, copies[1], 1, 0))
    italic = [r for r in element.iter(W + "r") if r.find(f"{W}rPr/{W}i") is not None]
    assert italic and "".join(t.text for r in italic for t in r.iter(W + "t")) == "Note:"
    assert all(r.find(f"{W}rPr/{W}b") is not None for r in element.iter(W + "r"))


def _left_over(box, session) -> tuple[str, str, list[str]]:
    """Copies with a column more, two cells merged, a paragraph in a list: differences of
    each kind."""
    doc, sample, copies = _document(box, session)
    _ok(box, session, "word_edit_table", doc=doc, table=copies[0], action="insert_column", index=2)
    _ok(box, session, "word_edit_table", doc=doc, table=copies[1], action="merge", first={"row": 1, "col": 1},
        last={"row": 1, "col": 2})
    listed = _cell(box, session, doc, copies[1], 1, 0)
    _ok(box, session, "word_lists", doc=doc, targets=[listed], action="add", kind="bullet")
    return doc, sample, copies


def test_what_cannot_follow_is_reported_with_why(box):
    session = box.session()
    doc, sample, copies = _left_over(box, session)
    listed = _cell(box, session, doc, copies[1], 1, 0)
    report = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample}, copies=copies).data
    wider, merged = report["copies"]
    found = {d["property"]: d for d in wider["not_followed"]}
    assert set(found) >= {"columns", "widths"}
    assert found["columns"]["why"] == WHY["columns"] and found["columns"]["value"] == 4
    assert wider["changed"]                                  # the table's own properties followed
    found = {d["property"]: d for d in merged["not_followed"]}
    assert found["merges"]["why"] == WHY["merges"] and found["merges"]["fix"]["tool"] == "word_edit_table"
    assert found["list"]["paragraphs"] == [listed] and found["list"]["why"] == WHY["list"]
    # Every entry says why.
    for entry in report["copies"]:
        assert all(d["why"] for d in entry["not_followed"])
        assert len(entry["not_followed"]) <= NOT_FOLLOWED_CAP


def test_the_sample_itself_and_another_kind_of_block(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    # Rule 3: nothing to do is no error.
    result = _follow(box, session, doc, sample, [sample])
    assert result.ok and result.changed == [] and result.data["copies"] == []
    assert any("the sample itself" in w for w in result.warnings)
    paragraph = ids_of(session, doc)[0]
    report = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample}, copies=[paragraph]).data
    assert report["copies"][0]["changed"] is False
    assert report["copies"][0]["not_followed"][0]["property"] == "kind"


def test_a_paragraph_follows_a_sample_paragraph(box):
    session = box.session()
    doc = written(box, session, "Lead one.\n\nLead two, a copy.\n\nLead three, a copy.")
    sample, *copies = block_ids(box, session, doc)[:3]
    _ok(box, session, "word_format", doc=doc, targets=[sample], alignment="center", space_after=18, italic=True,
        color="2F5496")
    result = _follow(box, session, doc, sample, copies)
    assert result.ok and len(result.changed) == 2
    assert _ok(box, session, "word_compare_blocks", doc=doc, blocks=copies,
               reference={"block": sample}).data["differences"] == []
    texts = dict(word_docs.paragraphs(session, doc))
    assert [texts[c] for c in copies] == ["Lead two, a copy.", "Lead three, a copy."]


def test_tracked_the_copies_changes_are_revisions_and_rows_untracked(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    _ok(box, session, "word_set_tracking", doc=doc, on=True, author=AUTHOR)
    result = _follow(box, session, doc, sample, copies)
    assert result.ok and any("does not track a row" in w for w in result.warnings)
    xml = word_docs.tree(word_docs.saved(session, doc))
    for tag in ("tblPrChange", "tblGridChange", "tcPrChange", "rPrChange"):
        assert next(xml.iter(W + tag), None) is not None, tag
    assert word_docs.validation(box, session, doc)["new"] == []
    # Rejecting them takes the copies back (but the rows, untracked as Word does).
    _ok(box, session, "word_changes", doc=doc, action="reject", all=True)
    assert {"widths", "bold", "shading", "table_alignment"} <= set(_left(box, session, doc, sample, copies))


#: A paragraph style of 9 pt, which the report lacks.
PART_TEXT = ('<w:style w:type="paragraph" w:customStyle="1" w:styleId="Parttext"><w:name w:val="Part text"/>'
             '<w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr></w:style>')


def _styles_named(session, doc: str) -> set[str]:
    root = word_docs.tree(word_docs.saved(session, doc), "word/styles.xml")
    return {name.get(W + "val") for name in root.iter(W + "name")}


def test_from_another_document_its_styles_by_name(box):
    session = box.session()
    made = written(box, session, SAMPLE, "made.docx")
    data = word_docs.replaced(word_docs.saved(session, made), {
        "word/styles.xml": lambda xml: xml.replace("</w:styles>", PART_TEXT + "</w:styles>")})
    d1 = session.open(data, "samples.docx")
    d2 = written(box, session, "A report.", "report.docx")
    sample = table_ids(box, session, d1)[0]
    copies = [_ok(box, session, "word_copy_from", doc=d2, source_doc=d1, range=sample, at="end").data["blocks"][0]
              for _ in range(2)]
    # The sample is revised: Table Grid's built-in style, a narrower column, and its body text
    # in a style the report lacks (a custom one) and one Word has (built-in).
    sample = _ok(box, session, "word_format_table", doc=d1, table=sample, scope="column", column=2,
                 width=60).renamed.get(sample, sample)
    _ok(box, session, "word_format", doc=d1, targets=[_cell(box, session, d1, sample, 1, 0)],
        paragraph_style="Part text")
    _ok(box, session, "word_format", doc=d1, targets=[_cell(box, session, d1, sample, 1, 1)],
        paragraph_style="Caption")
    preview = _ok(box, session, "word_follow_sample", doc=d2, sample={"doc": d1, "block": sample}, copies=copies,
                  preview=True)
    assert preview.changed == [] and "would follow" in preview.summary
    result = _ok(box, session, "word_follow_sample", doc=d2, sample={"doc": d1, "block": sample}, copies=copies)
    assert result.changed == copies and result.data["copies"] == preview.data["copies"]
    for entry in result.data["copies"]:
        assert entry["widths"]
        styles = [d for d in entry["not_followed"] if d["property"] == "paragraph_style"]
        assert [(d["reference"], d["why"]) for d in styles] == [("Part text", WHY["paragraph_style"])]
    named = _styles_named(session, d2)
    assert "caption" in named and "Part text" not in named                   # added, a built-in
    assert word_docs.validation(box, session, d2)["new"] == []
    _ok(box, session, "undo", doc=d2)
    assert "caption" not in _styles_named(session, d2)


def test_in_a_batch_and_by_reference(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    result = _ok(box, session, "batch", ops=[
        {"tool": "word_format_table", "arguments": {"doc": doc, "table": sample, "scope": "table", "alignment": "right"}},
        {"tool": "word_follow_sample", "arguments": {"doc": doc, "sample": {"block": sample}, "copies": copies}}])
    assert result.ok
    assert _left(box, session, doc, sample, copies) == []


# -- the tool design rules (tool-design-rules.md) ------------------------------------------------


def test_rule_1_addresses_other_tools_take_and_placeholders(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    cell = f"{copies[0]}/c1,1"
    paragraph = _cell(box, session, doc, copies[1], 1, 0)
    sample_cell = _cell(box, session, doc, sample, 0, 0)
    result = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample_cell},
                 copies=[cell, "", paragraph, copies[0]])
    report = result.data
    assert [e["copy"] for e in report["copies"]] == copies and report["sample"] == sample
    notes = " ".join(result.warnings)
    assert f"{cell} is in table {copies[0]}" in notes and "empty copy address was ignored" in notes
    assert f"is a paragraph in table {copies[1]}" in notes and "named twice" in notes
    assert _left(box, session, doc, sample, copies) == []


def test_rule_2_a_refusal_names_the_tables_and_their_shapes(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    refused = _follow(box, session, doc, sample, ["t:0BADF00D"])
    assert not refused.ok
    message = refused.error.message + " " + " ".join(refused.error.valid_options or [])
    assert "t:0BADF00D" in message and f"{copies[0]} (3 rows x 3 columns)" in message
    empty = _follow(box, session, doc, sample, [])
    assert not empty.ok


def test_rule_3_a_copy_already_following_does_not_fail_a_batch(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    follow = {"tool": "word_follow_sample", "arguments": {"doc": doc, "sample": {"doc": "", "block": sample},
                                                          "copies": copies, "preview": False}}
    result = _ok(box, session, "batch", ops=[follow, follow])
    assert result.ok
    again = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample}, copies=copies)
    assert again.changed == [] and "nothing left to follow" in again.summary
    assert all(e["changed"] is False for e in again.data["copies"])


# -- round 32, R13: each difference left says its kind ------------------------------------------


def test_every_not_followed_entry_has_a_kind(box):
    assert set(KIND.values()) <= set(KINDS) and set(WHY) <= set(KIND)
    session = box.session()
    doc, sample, copies = _left_over(box, session)
    report = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample}, copies=copies).data
    found = {d["property"]: d["kind"] for e in report["copies"] for d in e["not_followed"]}
    assert all(e.get("kind") in KINDS for c in report["copies"] for e in c["not_followed"])
    assert found["columns"] == found["merges"] == "structure" and found["list"] == "own"
    report = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample},
                 copies=[ids_of(session, doc)[0]]).data
    assert report["copies"][0]["not_followed"][0]["kind"] == "structure"


def test_report_actionable_leaves_the_copies_own_out_counted(box):
    session = box.session()
    doc, sample, copies = _left_over(box, session)
    every = _follow(box, session, doc, sample, copies, preview=True).data
    actionable = _follow(box, session, doc, sample, copies, preview=True, report="actionable").data
    for full, short in zip(every["copies"], actionable["copies"]):
        own = [d for d in full["not_followed"] if d["kind"] == "own"]
        assert short["not_followed"] == [d for d in full["not_followed"] if d["kind"] != "own"]
        assert short.get("own", 0) == len(own)
    assert actionable["copies"][1]["own"] == 1                     # the list membership
    blank = _follow(box, session, doc, sample, copies, preview=True)   # report left out: every form
    assert blank.ok, blank.to_json()
    assert blank.data["copies"] == every["copies"]


def test_report_names_its_forms_when_refused(box):
    session = box.session()
    doc, sample, copies = _document(box, session)
    refused = _follow(box, session, doc, sample, copies, report="some")
    assert not refused.ok and "actionable" in (refused.error.message + str(refused.error.valid_options))


def test_the_tool_takes_report(box):
    session = box.session()
    doc, sample, copies = _left_over(box, session)
    result = _ok(box, session, "word_follow_sample", doc=doc, sample={"block": sample}, copies=copies,
                 preview=True, report="actionable")
    assert result.data["copies"][1]["own"] == 1
    assert all(d["kind"] != "own" for e in result.data["copies"] for d in e["not_followed"])


#: The cells of the budget's sample table: rows by columns.
BUDGET_ROWS, BUDGET_COLUMNS = 6, 5
#: Copies of it, each leaving one entry per cell not followed: more than one reply holds.
BUDGET_COPIES = 7


def _budget_documents(box, session) -> tuple[str, str, str, list[str]]:
    """A samples document whose table's every cell paragraph is in a paragraph style of its
    own, and a report holding copies made before the styles were given (the report has none
    of them): ``(samples, sample table, report, copies)``."""
    cells = BUDGET_ROWS * BUDGET_COLUMNS
    styles = "".join(f'<w:style w:type="paragraph" w:customStyle="1" w:styleId="Part{k}"><w:name w:val="Part {k}"/>'
                     '<w:basedOn w:val="Normal"/></w:style>' for k in range(cells))
    header = "| " + " | ".join(f"Head {c}" for c in range(BUDGET_COLUMNS)) + " |"
    rows = "\n".join("| " + " | ".join(f"r{r}c{c}" for c in range(BUDGET_COLUMNS)) + " |"
                     for r in range(1, BUDGET_ROWS))
    markdown = f"Parts.\n\n{header}\n|{'---|' * BUDGET_COLUMNS}\n{rows}\n\nA closing line."
    made = written(box, session, markdown, "made.docx")
    samples = session.open(word_docs.replaced(word_docs.saved(session, made), {
        "word/styles.xml": lambda xml: xml.replace("</w:styles>", styles + "</w:styles>")}), "samples.docx")
    sample = table_ids(box, session, samples)[0]
    report = written(box, session, "A report.", "report.docx")
    copies = [_ok(box, session, "word_copy_from", doc=report, source_doc=samples, range=sample,
                  at="end").data["blocks"][0] for _ in range(BUDGET_COPIES)]
    found = _ok(box, session, "word_inspect", doc=samples, target=sample).data["blocks"][0]
    paragraphs = [cell["blocks"][0]["id"] for row in found["rows"] for cell in row]
    for k, paragraph in enumerate(paragraphs):
        _ok(box, session, "word_format", doc=samples, targets=[paragraph], paragraph_style=f"Part {k}")
    return samples, table_ids(box, session, samples)[0], report, copies


def test_rule_4_the_reply_has_a_budget_and_names_what_it_left_out(box):
    """Every copy leaves a style per cell not followed (the report has none of the sample's):
    more entries than one reply holds.  Each copy lists at most its cap and counts the rest in
    ``more``; the reply lists fewer than there are, and ``left_out`` names how many and where
    to read them (``word_compare_blocks``)."""
    session = box.session()
    samples, sample, report, copies = _budget_documents(box, session)
    result = _ok(box, session, "word_follow_sample", doc=report, sample={"doc": samples, "block": sample},
                 copies=copies, preview=True)
    entries = result.data["copies"]
    assert len(entries) == BUDGET_COPIES
    cells = BUDGET_ROWS * BUDGET_COLUMNS
    for entry in entries:
        styles = [d for d in entry["not_followed"] if d["property"] == "paragraph_style"]
        assert len(entry["not_followed"]) <= NOT_FOLLOWED_CAP
        assert len(styles) + entry.get("more", 0) >= cells        # every one listed or counted
    shown = sum(len(e["not_followed"]) for e in entries)
    more = sum(e.get("more", 0) for e in entries)
    assert more > 0 and shown < BUDGET_COPIES * cells
    assert f"{more} more difference(s) not followed" in result.data["left_out"]
    assert "word_compare_blocks" in result.data["left_out"]
