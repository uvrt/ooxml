"""Round 36: the outline as heading-owned ranges (R32) and the guards that keep a heading's
content from changing owner unseen (R33), through the toolbox.  Made-up documents only."""

from __future__ import annotations

import re

import pytest
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
MD = """# Opening

Opening words.

## First part

First words.

| k | v |
| --- | --- |
| 1 | 2 |

### Detail

Detail words.

## Second part

Second words.

# Closing

Closing words.
"""


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def _ids(box, session, doc: str = "d1") -> dict[str, str]:
    """Each paragraph's id by its text (a heading's without its Markdown marks)."""
    out: dict[str, str] = {}
    for block, text in word_docs.blocks(box, session, doc):
        if block.startswith("p:") and text:
            out.setdefault(re.sub(r"^#+ ", "", text), block)
    return out


def _blocks(box, session, doc: str = "d1") -> list[str]:
    return [block for block, _ in word_docs.blocks(box, session, doc)]


def made(box, *, section_after: str | None = None, in_table: bool = False) -> bytes:
    session = box.session()
    doc = session.open(word_docs.blank(), "outline.docx")
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown=MD)
    ids = _ids(box, session, doc)
    if section_after is not None:
        ok(box, session, "word_sections", doc=doc, action="insert_break", after=ids[section_after])
    if in_table:
        ok(box, session, "word_format", doc=doc, targets=[_cell_paragraph(session, doc)], paragraph_style="Heading 2")
    return word_docs.saved(session, doc)


def _cell_paragraph(session, doc: str = "d1") -> str:
    """The id of the first table's second row's first cell's paragraph."""
    table = word_docs.tables(session, doc)[0]
    cell = word_docs.cells_of(word_docs.rows_of(table)[1])[0]
    return "p:" + cell.find(W + "p").get(word_docs.W14_PARA)


def _table(session, doc: str = "d1") -> str:
    table = word_docs.tables(session, doc)[0]
    return "t:" + word_docs.rows_of(table)[0].get(word_docs.W14_PARA)


def opened(box, data: bytes):
    session = box.session()
    doc = session.open(data, "outline.docx")
    return session, doc, _ids(box, session, doc)


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def refused(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert not result.ok, (tool, result.to_json())
    return result.error.to_json()


def outline(box, session, doc: str = "d1") -> list[dict]:
    return ok(box, session, "describe", doc=doc).data["headings"]


def texts(session, doc: str = "d1") -> list[str]:
    return [text for _, text in word_docs.paragraphs(session, doc)]


def inspected(box, session, identifier: str, doc: str = "d1") -> dict:
    [block] = ok(box, session, "word_inspect", doc=doc, target=identifier).data["blocks"]
    return block


def read(box, session, range_: str, doc: str = "d1") -> str:
    return word_docs.read(box, session, doc, range=range_)


# -- R32: the outline -------------------------------------------------------------------------


def test_the_outline_gives_each_heading_its_level_blocks_and_section(box):
    session, doc, ids = opened(box, made(box, section_after="First words."))
    headings = outline(box, session, doc)
    assert [h["text"] for h in headings] == ["Opening", "First part", "Detail", "Second part", "Closing"]
    by_text = {h["text"]: h for h in headings}
    first = by_text["First part"]
    assert first["id"] == ids["First part"] and first["level"] == 2
    # The heading, its words, the break's paragraph, the table, Detail and its words.
    assert first["blocks"] == 6 and first["section"] == "s:1"
    assert by_text["Second part"]["section"] == "s:2"
    assert by_text["Opening"]["blocks"] == 10 and by_text["Closing"]["blocks"] == 2
    assert by_text["Detail"]["blocks"] == 2 and by_text["Detail"]["level"] == 3


def test_a_heading_range_reads_as_its_span(box):
    session, doc, ids = opened(box, made(box))
    span = f"{ids['First part']}..{ids['Detail words.']}"
    assert read(box, session, "h:" + ids["First part"]) == read(box, session, span)
    second = f"{ids['Second part']}..{ids['Second words.']}"
    assert inspected_blocks(box, session, "h:" + ids["Second part"]) == inspected_blocks(box, session, second)
    # A span's ends may be headings: from the first through the end of the second's range.
    both = read(box, session, f"h:{ids['First part']}..h:{ids['Second part']}")
    assert "First part" in both and "Second words." in both and "Closing" not in both
    assert read(box, session, "h:" + ids["Closing"]) == read(box, session, f"{ids['Closing']}..{_blocks(box, session)[-1]}")
    # A heading with nothing under it is just itself.
    ok(box, session, "word_delete", doc=doc, target=ids["Closing words."])
    assert read(box, session, "h:" + ids["Closing"]) == read(box, session, ids["Closing"])
    # A span through a heading range's end.
    ok(box, session, "word_move", doc=doc, blocks="h:" + ids["Detail"], after=ids["Closing"])
    assert _blocks(box, session)[-2:] == [ids["Detail"], ids["Detail words."]]


def inspected_blocks(box, session, target: str) -> list[str]:
    return [b["id"] for b in ok(box, session, "word_inspect", doc="d1", target=target).data["blocks"]]


def test_only_body_paragraphs_with_an_outline_level_are_headings(box):
    session, doc, ids = opened(box, made(box, in_table=True))
    assert "1" not in [h["text"] for h in outline(box, session, doc)]   # a heading style in a cell owns nothing
    error = refused(box, session, "word_read", doc=doc, range="h:" + _cell_paragraph(session, doc))
    assert "not a block of the body itself" in error["message"] and error["valid_options"][0].startswith("h:")
    error = refused(box, session, "word_read", doc=doc, range="h:" + _table(session, doc))
    assert "is not a paragraph" in error["message"]
    # Body text: refused with the headings to use.
    error = refused(box, session, "word_read", doc=doc, range="h:" + ids["First words."])
    assert "no outline level" in error["message"]
    assert any("First part" in option for option in error["valid_options"]), error


def _state_level(data: bytes, paragraph: str, value: int) -> bytes:
    """``data`` with the paragraph stating an outline level of its own."""
    from lxml import etree

    body = word_docs.tree(data)
    properties = word_docs.find(body, paragraph).find(W + "pPr")
    etree.SubElement(properties, W + "outlineLvl").set(W + "val", str(value))
    return word_docs.replaced(data, {"word/document.xml": etree.tostring(
        body, xml_declaration=True, encoding="UTF-8", standalone=True)})


def test_a_title_is_a_heading_only_with_an_outline_level(box):
    session, doc, ids = opened(box, made(box))
    words = ids["Opening words."]
    ok(box, session, "word_format", doc=doc, targets=[words], paragraph_style="Title")
    assert words not in [h["id"] for h in outline(box, session, doc)]
    # A paragraph stating an outline level is a heading of that level.
    session, doc, _ = opened(box, _state_level(word_docs.saved(session, doc), words, 1))
    assert {h["id"]: h["level"] for h in outline(box, session, doc)}.get(words) == 2


# -- R32: every tool's range takes h: ---------------------------------------------------------


def test_the_tools_take_a_heading_range(box):
    session, doc, ids = opened(box, made(box))
    first = "h:" + ids["First part"]
    data = ok(box, session, "word_read", doc=doc, range=first).data
    assert "First part" in data and "Detail words." in data and "Second part" not in data
    assert len(ok(box, session, "word_inspect", doc=doc, target=first).data["blocks"]) == 5
    found = ok(box, session, "find_text", doc=doc, text="words", range=first).data
    assert [f["context"] for f in found] == ["First [words].", "Detail [words]."]
    assert ok(box, session, "check", doc=doc, include=["validate"], range=[first]).ok
    ok(box, session, "word_format", doc=doc, targets=[first], keep_with_next=True)
    kept = lambda p: inspected(box, session, p)["effective"]["keep_with_next"]  # noqa: E731
    assert all(kept(p) for p in (ids["First part"], ids["Detail"]))
    assert not kept(ids["Second part"])
    ok(box, session, "word_move", doc=doc, blocks="h:" + ids["Second part"], before=ids["First part"])
    assert [h["text"] for h in outline(box, session, doc)][1:3] == ["Second part", "First part"]
    ok(box, session, "word_delete", doc=doc, target=first)
    assert [h["text"] for h in outline(box, session, doc)] == ["Opening", "Second part", "Closing"]
    assert "Detail words." not in texts(session, doc)


def test_a_heading_range_naming_no_heading_is_refused_with_the_headings(box):
    session, doc, ids = opened(box, made(box))
    words = ids["First words."]
    error = refused(box, session, "word_read", doc=doc, range="h:" + words)
    assert error["code"] == "invalid_arguments" and "no outline level" in error["message"]
    assert error["valid_options"][0].startswith("h:" + ids["Opening"])
    error = refused(box, session, "check", doc=doc, range=["h:" + words])
    assert error["valid_options"][0].startswith("h:")


def test_describe_names_each_headings_section_only_with_several(box):
    session, doc, _ = opened(box, made(box))
    headings_ = outline(box, session, doc)
    assert all("section" not in h for h in headings_) and headings_[1]["blocks"] == 5
    session, doc, _ = opened(box, made(box, section_after="First words."))
    headings_ = outline(box, session, doc)
    assert [h["section"] for h in headings_] == ["s:1", "s:1", "s:2", "s:2", "s:2"]


# -- R33: deleting a heading whose content stays ----------------------------------------------


def test_deleting_a_heading_alone_is_refused_with_its_range_and_force(box):
    session, doc, ids = opened(box, made(box))
    error = refused(box, session, "word_delete", doc=doc, target=ids["First part"])
    assert error["field"] == "target" and "would go under" in error["message"]
    assert ids["Opening"] in error["message"]
    assert error["valid_options"] == ["h:" + ids["First part"], "force"]
    assert "First part" in texts(session, doc)           # nothing changed
    # A span covering the heading but not all it owns: refused alike.
    error = refused(box, session, "word_delete", doc=doc, target=f"{ids['First part']}..{ids['First part']}")
    assert "h:" + ids["First part"] in error["valid_options"]
    # force: deleted, the warning says where its content went.
    result = ok(box, session, "word_delete", doc=doc, target=ids["First part"], force=True)
    assert any("now go under" in w and ids["Opening"] in w for w in result.warnings)
    assert "First words." in texts(session, doc)


def test_deleting_a_heading_with_nothing_under_it_or_its_whole_range_is_not_refused(box):
    session, doc, ids = opened(box, made(box))
    ok(box, session, "word_delete", doc=doc, target=f"{ids['Detail']}..{ids['Detail words.']}")
    ok(box, session, "word_delete", doc=doc, target=_blocks(box, session, doc)[-1])   # body text: no heading
    ok(box, session, "word_delete", doc=doc, target=ids["Closing"])     # owns nothing now


# -- R33: unnumbering a paragraph its style numbers ----------------------------------------------


def _numbered_style(box, session, doc: str, words: str) -> None:
    """A style that numbers its paragraphs ("Step text", "Step plain" based on it), the
    paragraph ``words`` in it."""
    ok(box, session, "word_lists", doc=doc, targets=[words], action="add", kind="number")
    num_id = inspected(box, session, words, doc)["list"]["num_id"]
    ok(box, session, "word_styles", doc=doc, action="add", name="Step text", kind="paragraph", based_on="Normal")
    ok(box, session, "word_styles", doc=doc, action="modify", name="Step text",
       numbering={"list": str(num_id), "level": 0})
    ok(box, session, "word_styles", doc=doc, action="add", name="Step plain", kind="paragraph", based_on="Step text")
    ok(box, session, "word_lists", doc=doc, targets=[words], action="remove")
    ok(box, session, "word_format", doc=doc, targets=[words], paragraph_style="Step text")


def test_unnumbering_a_style_numbered_paragraph_is_refused_with_styles_that_fit(box):
    session, doc, ids = opened(box, made(box))
    words = ids["Second words."]
    _numbered_style(box, session, doc, words)
    session, doc, _ = opened(box, word_docs.saved(session, doc))
    assert inspected(box, session, words, doc).get("list") is not None
    error = refused(box, session, "word_lists", doc=doc, targets=[words], action="remove")
    assert error["field"] == "targets" and "'Step text' numbers them" in error["message"]
    assert "Normal" in error["valid_options"] and error["valid_options"][-1] == "force"
    assert inspected(box, session, words, doc).get("list") is not None                        # nothing changed
    ok(box, session, "word_lists", doc=doc, targets=[words], action="remove", force=True)
    assert inspected(box, session, words, doc).get("list") is None


def test_unnumbering_an_own_numbered_paragraph_is_not_refused(box):
    session, doc, ids = opened(box, made(box))
    words = ids["Second words."]
    ok(box, session, "word_lists", doc=doc, targets=[words], action="add", kind="number")
    ok(box, session, "word_lists", doc=doc, targets=[words], action="remove")
    assert inspected(box, session, words, doc).get("list") is None


# -- R33: a heading's level changed: a warning, where its content goes ------------------------


def test_demoting_a_heading_warns_where_its_content_goes(box):
    session, doc, ids = opened(box, made(box))
    result = ok(box, session, "word_format", doc=doc, targets=[ids["Second part"]], paragraph_style="Heading 3")
    assert any("demoted 2 -> 3" in w and ids["First part"] in w for w in result.warnings), result.warnings


def test_promoting_a_heading_warns_what_it_takes(box):
    session, doc, ids = opened(box, made(box))
    result = ok(box, session, "word_format", doc=doc, targets=[ids["First part"]], paragraph_style="Heading 1")
    assert any("promoted 2 -> 1" in w and "Second part" in w and "from under" in w for w in result.warnings), \
        result.warnings


def test_a_heading_made_body_text_warns_where_its_blocks_go(box):
    session, doc, ids = opened(box, made(box))
    result = ok(box, session, "word_format", doc=doc, targets=[ids["Detail"]], paragraph_style="Normal")
    assert any("no longer a heading" in w and ids["First part"] in w for w in result.warnings), result.warnings
    result = ok(box, session, "word_format", doc=doc, targets=[ids["Detail"]], paragraph_style="Heading 3")
    assert any("is now a level 3 heading" in w for w in result.warnings), result.warnings


def test_a_style_change_that_leaves_levels_alone_warns_nothing(box):
    session, doc, ids = opened(box, made(box))
    result = ok(box, session, "word_format", doc=doc, targets=[ids["First words."]], paragraph_style="Normal")
    assert not any("heading" in w for w in result.warnings)


def test_a_heading_s_level_changed_back_and_forth_names_the_one_move(box):
    """The outline compared before and after (``moves``): one heading promoted, one line."""
    session, doc, ids = opened(box, made(box))
    result = ok(box, session, "word_format", doc=doc, targets=[ids["Detail"]], paragraph_style="Heading 2")
    assert [w for w in result.warnings if "promoted 3 -> 2" in w] and \
        len([w for w in result.warnings if "promoted" in w or "demoted" in w]) == 1, result.warnings
