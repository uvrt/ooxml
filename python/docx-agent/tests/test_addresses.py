"""One reading of every address (round 28, W9) and word_set_text's paragraph split
(W10, ``\\n`` in the text).  W9: a cell id, a row, a section, a bookmark and a span -- closed, open (``p:A..``
to the end of its container, ``..p:B`` from the start) or with refs at its ends -- are read
the same by every tool that takes a place; a form a parameter cannot take is refused with
the ids that would do, a malformed span with the syntax and never as an empty id.  W10:
``\\n`` writes consecutive paragraphs in the target's paragraph properties.  Made-up
documents on a blank package, through the toolbox."""

from __future__ import annotations

import json

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

MD = """# Opening

Alpha words here.

Bravo words here.

| Name | Value |
|---|---|
| left | right |

Charlie words here.

Delta words here.
"""


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


@pytest.fixture
def session(box):
    return box.session()


def made(box, *, section_after: str | None = None, bookmark: str | None = None) -> bytes:
    session = box.session()
    doc = session.open(word_docs.blank(), "addresses.docx")
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown=MD)
    if section_after is not None:
        ok(box, session, "word_sections", doc=doc, action="insert_break", after=_ids(session, doc)[section_after])
    if bookmark is not None:
        ok(box, session, "word_links", doc=doc, action="add_bookmark", target=_ids(session, doc)[bookmark],
           bookmark="Spot")
    return word_docs.saved(session, doc)


def _ids(session, doc: str) -> dict[str, str]:
    """Each paragraph's id (cells' too) by its first word."""
    return {text.split()[0]: block for block, text in word_docs.paragraphs(session, doc) if text}


def texts(session, doc: str) -> list[str]:
    return [text for _, text in word_docs.paragraphs(session, doc)]


def _table(session, doc: str) -> str:
    return "t:" + word_docs.rows_of(word_docs.tables(session, doc)[0])[0].get(word_docs.W14_PARA)


def _cell(session, doc: str, row: int, column: int):
    return word_docs.cells_of(word_docs.rows_of(word_docs.tables(session, doc)[0])[row])[column]


def cell_paragraphs(session, doc: str, row: int, column: int) -> list[tuple[str, str]]:
    return [("p:" + p.get(word_docs.W14_PARA), word_docs.text_of(p))
            for p in _cell(session, doc, row, column).findall(W + "p")]


def opened(box, session, data: bytes) -> tuple[str, dict[str, str]]:
    doc = session.open(data, "addresses.docx")
    return doc, _ids(session, doc)


def _paragraph(session, doc: str, text: str):
    return next(p for p in word_docs.tree(word_docs.saved(session, doc)).iter(W + "p") if word_docs.text_of(p) == text)


def first_run_has(session, doc: str, text: str, tag: str) -> bool:
    run = _paragraph(session, doc, text).find(f".//{W}r")
    return run is not None and run.find(f"{W}rPr/{W}{tag}") is not None


def inspected(box, session, doc: str, identifier: str) -> dict:
    [block] = ok(box, session, "word_inspect", doc=doc, target=identifier).data["blocks"]
    return block


def call(box, session, tool: str, /, **arguments):
    return box.dispatch(session, tool, arguments)


def ok(box, session, tool: str, /, **arguments):
    result = call(box, session, tool, **arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def refused(box, session, tool: str, code: str, /, **arguments):
    result = call(box, session, tool, **arguments)
    assert not result.ok and result.error.code == code, (tool, result.to_json())
    return result.error


# -- W9: reading -----------------------------------------------------------------------------


def test_word_inspect_on_a_cell_inspects_the_cell(box, session):
    doc, _ = opened(box, session, made(box))
    table = _table(session, doc)
    cell = f"{table}/c1,1"
    data = ok(box, session, "word_inspect", doc=doc, target=cell).data
    assert {k: v for k, v in data["cell"].items() if k != "properties"} == {
        "id": cell, "table": table, "row": 1, "column": 1}
    assert "properties" in data["cell"]
    assert [b["t"] for b in data["blocks"]] == ["right"]
    assert [b["id"] for b in data["blocks"]] == [i for i, _ in cell_paragraphs(session, doc, 1, 1)]


def test_word_read_takes_a_cell_a_row_and_open_spans(box, session):
    doc, ids = opened(box, session, made(box))
    table = _table(session, doc)
    text = ok(box, session, "word_read", doc=doc, range=f"{table}/c1,0").data
    assert "left" in text and "right" not in text and "Name" not in text
    # a row by its tr: id (by its place, t:X/r1, it is refused since stamp on open)
    rows = [tr.get(word_docs.W14_PARA) for tr in word_docs.tree(word_docs.saved(session, doc)).iter(W + "tr")]
    row = ok(box, session, "word_read", doc=doc, range=f"tr:{rows[1]}").data
    assert "left" in row and "right" in row and "Name" not in row
    to_end = ok(box, session, "word_read", doc=doc, range=f"{ids['Charlie']}..").data
    assert "Charlie" in to_end and "Delta" in to_end and "Bravo" not in to_end
    from_start = ok(box, session, "word_read", doc=doc, range=f"..{ids['Alpha']}").data
    assert "Opening" in from_start and "Alpha" in from_start and "Bravo" not in from_start


def test_a_span_takes_refs_at_both_ends(box, session):
    doc = session.open(word_docs.blank(), "refs.docx")
    for text, ref in (("Alpha", None), ("Bravo", "first"), ("Charlie", "last"), ("Delta", None)):
        ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown=text, **({"ref": ref} if ref else {}))
    text = ok(box, session, "word_read", doc=doc, range="$first..$last").data
    assert "Bravo" in text and "Charlie" in text and "Alpha" not in text and "Delta" not in text
    tail = ok(box, session, "word_read", doc=doc, range="$last..").data
    assert "Charlie" in tail and "Delta" in tail and "Bravo" not in tail


@pytest.mark.parametrize("bad", ["p:A...p:B", "..", "p:A..p:B..p:C"])
def test_a_malformed_span_is_refused_with_the_syntax_never_as_an_empty_id(box, session, bad):
    doc, ids = opened(box, session, made(box))
    bad = bad.replace("p:A", ids["Alpha"]).replace("p:B", ids["Bravo"]).replace("p:C", ids["Charlie"])
    for tool, arguments in (("word_read", {"range": bad}), ("word_inspect", {"target": bad}),
                            ("find_text", {"text": "words", "range": bad}),
                            ("word_delete", {"target": bad}), ("word_move", {"blocks": bad, "after": ids["Delta"]})):
        error = refused(box, session, tool, "invalid_arguments", doc=doc, **arguments)
        assert "<first>..<last>" in error.message and "''" not in error.message, (tool, error.message)


def test_find_text_and_replace_text_take_a_cell_a_section_a_bookmark_and_an_open_span(box, session):
    doc, ids = opened(box, session, made(box, section_after="Bravo", bookmark="Charlie"))
    table = _table(session, doc)
    found = ok(box, session, "find_text", doc=doc, text="words", range=f"{ids['Charlie']}..").data
    assert [m["address"].partition("@")[0] for m in found] == [ids["Charlie"], ids["Delta"]]
    assert ok(box, session, "find_text", doc=doc, text="right", range=f"{table}/c1,1").data
    assert not ok(box, session, "find_text", doc=doc, text="left", range=f"{table}/c1,1").data
    first_section = ok(box, session, "find_text", doc=doc, text="words", range="s:1").data
    assert [m["address"].partition("@")[0] for m in first_section] == [ids["Alpha"], ids["Bravo"]]
    marked = ok(box, session, "find_text", doc=doc, text="words", range="bm:Spot").data
    assert [m["address"].partition("@")[0] for m in marked] == [ids["Charlie"]]
    ok(box, session, "replace_text", doc=doc, find="words", replace="lines", expect="all", range="s:body")
    assert [t for t in texts(session, doc) if t.endswith("here.")] == [
        "Alpha words here.", "Bravo words here.", "Charlie lines here.", "Delta lines here."]


# -- W9: writing -----------------------------------------------------------------------------


def test_word_format_takes_a_cell_a_row_a_section_and_a_bookmark(box, session):
    doc, ids = opened(box, session, made(box, section_after="Bravo", bookmark="Delta"))
    table = _table(session, doc)
    ok(box, session, "word_format", doc=doc, targets=[f"{table}/c1,1"], bold=True)
    ok(box, session, "word_format", doc=doc, targets=["s:1", "bm:Spot"], italic=True)
    bold = {t for t in texts(session, doc) if t and first_run_has(session, doc, t, "b")}
    italic = {t for t in texts(session, doc) if t and first_run_has(session, doc, t, "i")}
    assert bold == {"right"}
    assert italic == {"Opening", "Alpha words here.", "Bravo words here.", "Delta words here."}


def test_word_delete_and_word_move_take_open_spans(box, session):
    doc, ids = opened(box, session, made(box))
    ok(box, session, "word_move", doc=doc, blocks=f"..{ids['Alpha']}", after=ids["Delta"])
    assert [t.split()[0] for t in texts(session, doc) if t][-2:] == ["Opening", "Alpha"]
    ok(box, session, "word_delete", doc=doc, target=f"{ids['Delta']}..")
    assert [t.split()[0] for t in texts(session, doc) if t] == [
        "Bravo", "Name", "Value", "left", "right", "Charlie"]


def test_text_targets_take_a_cell_a_bookmark_and_a_paragraph(box, session):
    doc, ids = opened(box, session, made(box, bookmark="Bravo"))
    cell = f"{_table(session, doc)}/c1,1"
    ok(box, session, "word_insert_text", doc=doc, target=cell, text=" side")
    assert word_docs.cell_text(_cell(session, doc, 1, 1)) == "right side"
    ok(box, session, "word_comments", doc=doc, action="add", items=[{"target": cell, "text": "Check."}])
    listed = ok(box, session, "word_comments", doc=doc, action="list").data
    assert json.dumps(listed).count("right side") >= 1, listed
    ok(box, session, "word_notes", doc=doc, action="insert", at=ids["Alpha"], text="A note.")
    ok(box, session, "word_notes", doc=doc, action="insert", at="bm:Spot", text="Another.")
    marks = [word_docs.text_of(p) for p in word_docs.tree(word_docs.saved(session, doc)).iter(W + "p")
             if p.find(f".//{W}footnoteReference") is not None]
    assert marks == ["Alpha words here.", "Bravo words here."]


def test_a_section_parameter_takes_any_block_and_names_its_section(box, session):
    doc, ids = opened(box, session, made(box, section_after="Bravo"))
    ok(box, session, "word_sections", doc=doc, action="set", section=ids["Alpha"], orientation="landscape")
    ok(box, session, "word_headers_footers", doc=doc, section=f"{_table(session, doc)}/c0,0", which="header",
       action="set", text="Later pages")
    data = word_docs.saved(session, doc)
    first, last = list(word_docs.tree(data).iter(W + "sectPr"))
    orient = lambda section: section.find(W + "pgSz").get(W + "orient", "portrait")  # noqa: E731
    assert orient(first) == "landscape" and orient(last) == "portrait"
    header = last.find(W + "headerReference")
    assert header is not None and first.find(W + "headerReference") is None
    rels = word_docs.tree(data, "word/_rels/document.xml.rels")
    target = next(r.get("Target") for r in rels if r.get("Id") == header.get(
        "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"))
    assert "Later pages" in word_docs.part(data, f"word/{target}")
    error = refused(box, session, "word_sections", "invalid_arguments", doc=doc, action="set",
                    section=f"{ids['Alpha']}..{ids['Bravo']}", orientation="portrait")
    assert "span" in error.message


def test_a_cell_where_a_block_is_wanted_names_its_table_and_paragraphs(box, session):
    doc, ids = opened(box, session, made(box))
    table = _table(session, doc)
    cell = f"{table}/c1,1"
    inside = cell_paragraphs(session, doc, 1, 1)[0][0]
    for tool, arguments in (("word_insert_markdown", {"markdown": "New.", "at": f"after:{cell}"}),
                            ("word_move", {"blocks": ids["Delta"], "after": cell}),
                            ("word_delete", {"target": cell}),
                            ("word_compare_blocks", {"blocks": [cell], "reference": {"block": ids["Alpha"]}})):
        error = refused(box, session, tool, "invalid_arguments", doc=doc, **arguments)
        assert "is a cell" in error.message and table in error.valid_options, (tool, error.message)
        assert inside in error.valid_options
    error = refused(box, session, "word_format_table", "invalid_arguments", doc=doc, table=inside,
                    scope="table", alignment="center")
    assert error.valid_options == [table]


def test_a_span_where_one_block_is_wanted_names_its_ends(box, session):
    doc, ids = opened(box, session, made(box))
    span = f"{ids['Alpha']}..{ids['Bravo']}"
    error = refused(box, session, "word_move", "invalid_arguments", doc=doc, blocks=ids["Delta"], after=span)
    assert "is a span" in error.message and error.valid_options == [ids["Alpha"], ids["Bravo"]]


# -- W10: a newline splits -----------------------------------------------------------------


def test_a_newline_split_writes_each_line_as_a_paragraph_in_the_targets_properties(box, session):
    doc, ids = opened(box, session, made(box))
    ok(box, session, "word_format", doc=doc, targets=[ids["Bravo"]], alignment="center", space_after=3, bold=True)
    style = inspected(box, session, doc, ids["Bravo"]).get("style")
    result = ok(box, session, "word_set_text", doc=doc, target=ids["Bravo"],
                text="First line.\nSecond\vwith a break.\n\nFourth.")
    now = texts(session, doc)
    at = now.index("First line.")
    assert now[at:at + 4] == ["First line.", "Second\vwith a break.", "", "Fourth."]
    assert len(result.created) == 3 and result.changed == [ids["Bravo"]]
    body = word_docs.tree(word_docs.saved(session, doc))
    assert [word_docs.text_of(word_docs.find(body, i)) for i in result.created] == ["Second\vwith a break.", "",
                                                                                    "Fourth."]
    for made_ in result.created:
        block = inspected(box, session, doc, made_)
        assert block.get("style") == style
        assert block["effective"]["alignment"] == "center" and block["effective"]["spacing"]["after"] == 3
    # The last run's formatting, as Enter continues it.
    assert word_docs.find(body, result.created[0]).find(f".//{W}r/{W}rPr/{W}b") is not None
    again = ok(box, session, "word_set_text", doc=doc, target=ids["Bravo"],
               text="First line.\nSecond\vwith a break.\n\nFourth.")
    assert again.created == [] and again.changed == []
    assert texts(session, doc) == now


def test_without_a_newline_a_cell_target_rewrites_its_first_paragraph_as_before(box, session):
    doc, _ = opened(box, session, made(box))
    table = _table(session, doc)
    ok(box, session, "word_set_text", doc=doc, target=f"{table}/c1,1", text="R1\nR2")
    ok(box, session, "word_set_text", doc=doc, target=f"{table}/c1,1", text="Only\vbroken")
    assert [t for _, t in cell_paragraphs(session, doc, 1, 1)] == ["Only\vbroken", "R2"]


def test_a_newline_split_applies_per_target_and_writes_a_cells_paragraphs(box, session):
    doc, ids = opened(box, session, made(box))
    table = _table(session, doc)
    result = ok(box, session, "word_set_text", doc=doc, items=[{"target": ids["Alpha"], "text": "A1\nA2"},
                                                                {"target": f"{table}/c1,1", "text": "R1\nR2"},
                                                                {"target": ids["Delta"], "text": "One line only"}])
    now = texts(session, doc)
    assert now[now.index("A1") + 1] == "A2"
    assert [t for _, t in cell_paragraphs(session, doc, 1, 1)] == ["R1", "R2"]
    assert "One line only" in now and len(result.created) == 2
    again = ok(box, session, "word_set_text", doc=doc, items=[{"target": f"{table}/c1,1", "text": "R1\nR2"}])
    assert again.created == [] and again.changed == []


def test_a_newline_split_is_tracked_and_undone_as_one_call(box, session):
    doc, ids = opened(box, session, made(box))
    before = texts(session, doc)
    ok(box, session, "word_set_tracking", doc=doc, on=True, author="Reviewer")
    ok(box, session, "word_set_text", doc=doc, target=ids["Charlie"], text="Charlie words here.\nAdded.")
    assert any(r.tag == W + "ins" for r in word_docs.revisions(session, doc))
    assert "Added." in texts(session, doc)
    ok(box, session, "undo", doc=doc)
    assert texts(session, doc) == before and not word_docs.revisions(session, doc)


def test_a_newline_split_refuses_a_paragraph_that_ends_a_section(box, session):
    doc, ids = opened(box, session, made(box, section_after="Bravo"))
    ending = next(s["id"] for s in ok(box, session, "word_sections", doc=doc, action="list").data
                  if s["id"] != "s:body")
    error = refused(box, session, "word_set_text", "refused", doc=doc, target="p" + ending[1:], text="One\nTwo")
    assert "ends a section" in error.message


def test_a_newline_costs_no_parameter_and_the_strict_set_stays_within_24(box):
    # 23 then; round 40's ref on word_insert_markdown made it 24, Claude's limit.  Round 42's
    # paragraph_style (the single form's and each item's) takes word_set_text to 5 optional
    # parameters: the strict plan sends it non-strict and keeps the set within the limit;
    # word_insert_markdown's like (R47) fills the 24th place.
    definitions = box.definitions("anthropic", defer=False)
    set_text = next(d for d in definitions if d["name"] == "word_set_text")
    assert list(set_text["input_schema"]["properties"]) == ["doc", "target", "text", "paragraph_style", "items"]
    tools = {t.name: t for t in TOOLS}
    assert len(tools["word_set_text"].optional_parameters()) == 5
    # edit_smartart, strict too, is out of the Word tools until its port (P4e): 22 of 24.
    assert sum(len(tools[d["name"]].optional_parameters()) for d in definitions if d.get("strict")) == 22
    assert {d["name"] for d in definitions if d.get("strict")} == {
        "replace_text", "word_changes", "word_set_tracking", "word_insert_markdown", "word_template"}


def test_a_reversed_span_is_refused_alike_everywhere(box, session):
    doc, ids = opened(box, session, made(box))
    reversed_ = f"{ids['Charlie']}..{ids['Alpha']}"
    for tool, arguments in (("word_read", {"range": reversed_}), ("word_delete", {"target": reversed_}),
                            ("word_insert_markdown", {"markdown": "New.", "at": f"replace:{reversed_}"}),
                            ("word_move", {"blocks": reversed_, "after": ids["Delta"]})):
        error = refused(box, session, tool, "refused", doc=doc, **arguments)
        assert error.message == f"{reversed_!r} ends before it starts", (tool, error.message)


def test_a_bookmark_or_story_where_a_block_is_wanted_gets_the_hint(box, session):
    doc, ids = opened(box, session, made(box, bookmark="Charlie"))
    for anchor in ("Spot", "bm:Spot"):
        error = refused(box, session, "word_move", "invalid_arguments", doc=doc, blocks=ids["Delta"], after=anchor)
        assert "is a bookmark" in error.message and error.valid_options == [ids["Charlie"]]
    error = refused(box, session, "word_edit_table", "invalid_arguments", doc=doc, table="Spot",
                    action="insert_row")
    assert "is a bookmark" in error.message
    error = refused(box, session, "word_insert_markdown", "invalid_arguments", doc=doc, markdown="New.",
                    at="after:body")
    assert "is a story" in error.message
