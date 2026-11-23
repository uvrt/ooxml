"""Calls a production run made often and had refused (round 31, R5): ``word_delete`` with
``find`` and ``target`` (find searched within the target), a ``caption`` in
``word_insert_markdown``'s ``style_map`` (a caption paragraph in Markdown), ``word_inspect``
of a cell the table lacks (the table's rows, columns and merged cells said) and
``word_format_table`` ``keep_together`` on a one-row table (``test_table_keeps.py``).
Made-up documents, a new blank document's package."""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def opened(box, data: bytes):
    session = box.session()
    return session, session.open(data, "made-up.docx")


def _para(text: str, pid: str) -> str:
    run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return f'<w:p w14:paraId="{pid}" w14:textId="77777777">{run}</w:p>'


def two_paragraphs() -> bytes:
    """An empty paragraph, then two of text."""
    return word_docs.with_body(_para("", "1A000001") + _para("Shelf one holds the spare parts", "1A000002")
                               + _para("Shelf two holds the spare tools", "1A000003"))


def texts(session, d) -> list[str]:
    return [text for _, text in word_docs.paragraphs(session, d)]


def styled(session, d) -> list[tuple[str, str, str | None]]:
    """Each body paragraph's id, text and style id."""
    out = []
    for p in word_docs.tree(word_docs.saved(session, d)).find(_W + "body").iter(_W + "p"):
        style = p.find(f"{_W}pPr/{_W}pStyle")
        out.append((f"p:{p.get(word_docs.W14_PARA)}", word_docs.text_of(p),
                    style.get(_W + "val") if style is not None else None))
    return out


# -- R5a: word_delete with find and target --------------------------------------------------------


def test_find_with_a_target_is_find_within_it(box):
    session, d = opened(box, two_paragraphs())
    second = word_docs.paragraphs(session, d)[2][0]
    result = ok(box, session, "word_delete", doc=d, find="spare ", target=second)
    assert texts(session, d)[1:] == ["Shelf one holds the spare parts", "Shelf two holds the tools"]
    assert result.warnings == [f"find was searched within target {second}: within names where to search"]


def test_find_with_target_and_another_within_is_refused_and_neither_is_too(box):
    session, d = opened(box, two_paragraphs())
    first, second = (block for block, _ in word_docs.paragraphs(session, d)[1:])
    refused = box.dispatch(session, "word_delete", {"doc": d, "find": "spare", "target": second, "within": first})
    assert not refused.ok and refused.error.code == "invalid_arguments" and refused.error.field == "target"
    # The same block twice is one place to search.
    ok(box, session, "word_delete", doc=d, find="spare ", target=second, within=second)
    refused = box.dispatch(session, "word_delete", {"doc": d})
    assert not refused.ok and refused.error.valid_options == ["find", "target"]


def test_a_target_alone_still_deletes_the_block(box):
    session, d = opened(box, two_paragraphs())
    second = word_docs.paragraphs(session, d)[2][0]
    result = ok(box, session, "word_delete", doc=d, target=second)
    assert result.warnings == [] and len(word_docs.paragraphs(session, d)) == 2


# -- R5b: a caption in Markdown -------------------------------------------------------------------


CAPTIONED = """Before the table.

| Item | Count |
|------|-------|
| Bolts | 12 |

Table: Parts on the shelf

: Shelf layout, seen from the door

Table: a plain paragraph, no table beside it

:no space after the colon
"""


def test_a_caption_paragraph_is_written_in_the_caption_style(box):
    session, d = opened(box, word_docs.blank())
    result = ok(box, session, "word_insert_markdown", doc=d, at="end", markdown=CAPTIONED,
                style_map=[{"element": "caption", "style": "Caption"}])
    assert "style added: Caption (Caption), as Word writes it" in result.warnings
    body = [(text, style) for block, text, style in styled(session, d) if block in result.data["blocks"]]
    assert body == [("Before the table.", None), ("Parts on the shelf", "Caption"),
                    ("Shelf layout, seen from the door", "Caption"),
                    ("Table: a plain paragraph, no table beside it", None),
                    (":no space after the colon", None)]


def test_the_caption_style_by_default_and_another_by_the_map(box):
    session, d = opened(box, word_docs.blank())
    ok(box, session, "word_styles", doc=d, action="add", name="Figure note", based_on="Normal")
    ok(box, session, "word_insert_markdown", doc=d, at="end", markdown=": First\n")
    ok(box, session, "word_insert_markdown", doc=d, at="end", markdown=": Second\n",
       style_map=[{"element": "caption", "style": "Figure note"}])
    styles = {text: style for _, text, style in styled(session, d)}
    assert styles["First"] == "Caption"
    sheet = word_docs.tree(word_docs.saved(session, d), "word/styles.xml")
    named = next(s for s in sheet.iter(_W + "style") if s.get(_W + "styleId") == styles["Second"])
    assert named.find(_W + "name").get(_W + "val") == "Figure note"


def test_a_caption_in_a_quote_or_list_is_text_and_reading_is_unchanged(box):
    session, d = opened(box, word_docs.blank())
    ok(box, session, "word_insert_markdown", doc=d, at="end", markdown="> : quoted\n\n- : listed\n\n: Shown\n")
    found = texts(session, d)
    assert ": quoted" in found and ": listed" in found and "Shown" in found
    # The reader shows a caption paragraph's style on its block, as before round 31.
    assert "style: caption -->\nShown" in word_docs.read(box, session, d)


# -- R5c: a cell the table lacks ------------------------------------------------------------------


def merged_table(box):
    """A 2 x 3 table, its first column merged down and its first row's last two cells across."""
    session, d = opened(box, word_docs.blank())
    made = ok(box, session, "word_insert_markdown", doc=d, at="end",
              markdown="| a | b | c |\n|---|---|---|\n| d | e | f |")
    table = next(block for block in made.created if block.startswith("t:"))
    ok(box, session, "word_edit_table", doc=d, table=table, action="merge",
       first={"row": 0, "col": 0}, last={"row": 1, "col": 0})
    ok(box, session, "word_edit_table", doc=d, table=table, action="merge",
       first={"row": 0, "col": 1}, last={"row": 0, "col": 2})
    return session, d, table


def test_a_missing_cell_says_the_rows_columns_and_merged_cells(box):
    session, d, table = merged_table(box)
    refused = box.dispatch(session, "word_inspect", {"doc": d, "target": f"{table}/c2,0"})
    assert refused.error.code == "not_found" and "it has 2 row(s) (0 to 1) and 3 column(s)" in refused.error.message
    assert refused.error.valid_options == [f"{table}/c0,0", f"{table}/c0,1", f"{table}/c1,1", f"{table}/c1,2"]
    refused = box.dispatch(session, "word_inspect", {"doc": d, "target": f"{table}/c1,3"})
    assert refused.error.message.startswith(
        f"{table} has no cell at (1, 3): it has 2 row(s) (0 to 1) and 3 column(s) (0 to 2), counted from 0; "
        "merged: (0, 0)-(1, 0), (0, 1)-(0, 2) (a merged cell's id is its first position's)"), refused.error.message
    # The table's own edits say the same.
    refused = box.dispatch(session, "word_edit_table", {"doc": d, "table": table, "action": "split",
                                                        "first": {"row": 4, "col": 0}})
    assert not refused.ok and "it has 2 row(s)" in refused.error.message
