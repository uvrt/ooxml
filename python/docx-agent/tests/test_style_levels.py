"""A paragraph style linked to a list level (round 29, W12): ``Styles.modify(numbering=...)``
and ``word_styles`` ``numbering``, as Word's "Link level to style" does it, and ``check``'s
``numbering_skips_level``.

Word's behaviour, measured on made-up documents (``feedback-round29/measurements-W.md``):
the level names the style (``w:pStyle``), the style gets ``w:numPr`` and loses its own
``w:ind``; the style that held the level loses its ``w:numPr`` and ``w:ind``, so its
paragraphs are no longer numbered; unlinking takes the level's ``w:pStyle`` and the style's
``w:numPr`` and ``w:ind`` away.  Linked chapter, section and subsection styles number 1, 1.1,
1.2, 2, 2.1, 2.1.1; a section style used with no chapter before it numbers 1.1, 1.2.

The document: one multilevel list (numId 1) whose level 0 (``%1``) is linked to "Chapter
title", level 1 (``%1.%2``) to "Section title", level 2 (``%1.%2.%3``) to no style; "Aside
title" is in no list.  Each style states an indent of its own.
"""

from __future__ import annotations

import datetime as dt
import re

import pytest

from docx_agent import Document, EditError
from docx_agent.edit.numbering import problems
from docx_agent.layout_facts import numbering_skips_level
from docx_agent.validate import check
from facts_documents import SECTIONS_ONLY, make

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
def style_ppr(document: Document, style_id: str):
    root = document.package.tree(document.package.styles_part())
    node = next(s for s in root.iter(W + "style") if s.get(W + "styleId") == style_id)
    return node.find(W + "pPr")


def numpr(document: Document, style_id: str) -> tuple[str | None, str | None] | None:
    properties = style_ppr(document, style_id)
    found = properties.find(W + "numPr") if properties is not None else None
    if found is None:
        return None
    ilvl, num = found.find(W + "ilvl"), found.find(W + "numId")
    return (ilvl.get(W + "val") if ilvl is not None else None, num.get(W + "val") if num is not None else None)


def indent(document: Document, style_id: str):
    properties = style_ppr(document, style_id)
    found = properties.find(W + "ind") if properties is not None else None
    return None if found is None else (found.get(W + "left"), found.get(W + "hanging"))


def linked(document: Document) -> dict[int, str | None]:
    """Each level of the list's definition: the style it names."""
    root = document.package.tree(document.package.numbering_part())
    out = {}
    for lvl in root.iter(W + "lvl"):
        tied = lvl.find(W + "pStyle")
        out[int(lvl.get(W + "ilvl"))] = tied.get(W + "val") if tied is not None else None
    return out


def labels(document: Document) -> list[str]:
    """docx2svg's list labels, line by line ('' where a line has none)."""
    import docx2svg

    out = []
    for svg in docx2svg.convert_docx_to_svg(document.to_bytes()):
        for line in re.findall(r'<g data-docx-line="\d+"[^>]*>(.*?)</g>', svg, re.S):
            found = re.search(r'<text [^>]*data-docx-kind="label"[^>]*>([^<]*)</text>', line)
            out.append(found.group(1) if found else "")
    return out


def valid(document: Document) -> None:
    assert problems(document) == []
    reopened = Document.open(document.to_bytes())
    assert [str(p) for p in check(reopened.package) if p.code.startswith(("numbering", "style"))] == []


def test_the_document_numbers_chapters_and_sections():
    assert labels(Document.open(make())) == ["1", "1.1", "1.2", "2", "2.1", "", ""]


def test_linking_a_style_to_a_level_unlinks_the_style_that_held_it():
    data = make()
    document = Document.open(data)
    result = document.styles.modify("Aside title", numbering={"list": 1, "level": 1})
    # The level names the new style; it numbers at that level with the level's indent.
    assert linked(document) == {0: "ChapterTitle", 1: "AsideTitle", 2: None}
    assert numpr(document, "AsideTitle") == ("1", "1") and indent(document, "AsideTitle") is None
    # The style that held the level loses its numbering and its indent (Word, measured).
    assert numpr(document, "SectionTitle") is None and indent(document, "SectionTitle") is None
    assert result.warnings == ["'Section title' held level 1 of list 1 before: unlinked, its 3 "
                               "paragraph(s) are no longer numbered"]
    assert numpr(document, "ChapterTitle") == (None, "1") and indent(document, "ChapterTitle") == ("300", "300")
    assert labels(document) == ["1", "", "", "2", "", "2.1", ""]      # as Word numbers it
    valid(document)
    assert document.undo()
    assert document.to_bytes() == Document.open(data).to_bytes()


def test_a_level_no_style_held_and_the_list_named_by_a_style_in_it():
    document = Document.open(make())
    result = document.styles.modify("Aside title", numbering={"list": "Chapter title", "level": 2})
    assert not result.warnings
    assert linked(document) == {0: "ChapterTitle", 1: "SectionTitle", 2: "AsideTitle"}
    assert numpr(document, "AsideTitle") == ("2", "1") and indent(document, "SectionTitle") == ("900", "300")
    assert labels(document) == ["1", "1.1", "1.2", "2", "2.1", "2.1.1", ""]
    valid(document)


def test_level_zero_is_written_without_an_ilvl():
    document = Document.open(make())
    document.styles.modify("Aside title", numbering={"list": 1, "level": 0})
    assert numpr(document, "AsideTitle") == (None, "1") and numpr(document, "ChapterTitle") is None
    assert linked(document)[0] == "AsideTitle"
    valid(document)


def test_a_style_linked_to_another_level_moves():
    document = Document.open(make())
    document.styles.modify("Section title", numbering={"list": 1, "level": 2})
    assert linked(document) == {0: "ChapterTitle", 1: None, 2: "SectionTitle"}
    assert numpr(document, "SectionTitle") == ("2", "1")
    valid(document)


def test_none_unlinks_the_style():
    document = Document.open(make())
    result = document.styles.modify("Section title", numbering=None)
    assert result.changed and not result.warnings
    assert linked(document) == {0: "ChapterTitle", 1: None, 2: None}
    assert numpr(document, "SectionTitle") is None and indent(document, "SectionTitle") is None
    assert labels(document) == ["1", "", "", "2", "", "", ""]
    valid(document)
    # A style in no list: unlinking it leaves it as it is (its indent too).
    document.styles.modify("Aside title", numbering=None)
    assert indent(document, "AsideTitle") == ("1100", "200")


def test_formatting_and_numbering_in_one_call_are_one_undo_step():
    data = make()
    document = Document.open(data)
    document.styles.modify("Aside title", bold=True, numbering={"list": 1, "level": 2})
    assert numpr(document, "AsideTitle") == ("2", "1")
    assert document.undo() and document.to_bytes() == Document.open(data).to_bytes()


@pytest.mark.parametrize("numbering, says", [
    ({"list": 7, "level": 0}, "no list 7: its lists are 1"),
    ({"list": 1, "level": 3}, "list 1 has no level 3: its levels are 0, 1, 2"),
    ({"list": 1, "level": 9}, "level is 0 to 8"),
    ({"list": "Aside title", "level": 1}, "'Aside title' is in no list"),
    ({"list": "Nothing here", "level": 1}, "no paragraph style 'Nothing here'"),
    ({"level": 1}, 'numbering is {"list"'),
    ({"list": 1, "level": 1, "start": 2}, 'numbering is {"list"'),
])
def test_a_list_or_level_the_document_lacks_is_refused(numbering, says):
    data = make()
    document = Document.open(data)
    with pytest.raises(EditError, match=re.escape(says)):
        document.styles.modify("Chapter title", numbering=numbering)
    assert document.to_bytes() == Document.open(data).to_bytes()


def test_only_a_paragraph_style_is_linked():
    with pytest.raises(EditError, match="only a paragraph style"):
        Document.open(make()).styles.modify("Marked", numbering={"list": 1, "level": 0})


# -- check: numbering_skips_level ---------------------------------------------------------------


def test_a_level_used_with_the_level_above_never_used_is_a_fact():
    document = Document.open(make(SECTIONS_ONLY))
    assert labels(document)[:2] == ["1.1", "1.2"]                 # Word: 1.1, 1.2 (measured)
    [entry] = numbering_skips_level(document)
    assert entry == {"para": document.paragraphs()[0].id, "style": "Section title", "level": 1, "list": 1,
                     "unused_levels": [0], "label": "%1.%2", "shows": "1.1", "paragraphs": 2}
    assert numbering_skips_level(Document.open(make())) == []


def test_a_skipped_middle_level_names_it_and_shows_no_number():
    document = Document.open(make())
    document.styles.modify("Section title", numbering={"list": 1, "level": 2})   # level 1 now unused
    [entry] = numbering_skips_level(document)
    assert (entry["style"], entry["level"], entry["unused_levels"], entry["paragraphs"]) == \
        ("Section title", 2, [1], 3)
    assert "shows" not in entry                  # level 0 is used: its count is the chapter's


# -- the tools ----------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def box():
    from ooxml_edit.tools import Toolbox

    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def dispatch(box, session, tool, /, **arguments):
    return box.dispatch(session, tool, arguments)


@pytest.fixture
def session(box):
    session = box.session(clock=lambda: dt.datetime(2026, 11, 3, 12, 0, tzinfo=dt.timezone.utc))
    session.open(make(), "levels.docx")
    return session


def test_word_styles_links_and_unlinks(box, session):
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", name="Aside title",
                      numbering={"list": "1", "level": 1})
    assert result.ok, result.to_json()
    assert result.summary == "Modified style Aside title: linked to level 1 of list 1"
    assert result.warnings and "'Section title' held level 1" in result.warnings[0]
    document = session.entry("d1").document
    assert linked(document)[1] == "AsideTitle"
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", name="Aside title",
                      numbering={"list": "none"})
    assert result.ok and result.summary.endswith("unlinked from its list level")
    assert linked(document)[1] is None
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", name="Section title",
                      numbering={"list": "Chapter title", "level": 1})
    assert result.ok and numpr(document, "SectionTitle") == ("1", "1")


@pytest.mark.parametrize("arguments, code, field", [
    ({"name": "Chapter title", "numbering": {"list": "9", "level": 0}}, "invalid_arguments", "numbering.list"),
    ({"name": "Chapter title", "numbering": {"list": "1", "level": 5}}, "invalid_arguments", "numbering.level"),
    ({"name": "Chapter title", "numbering": {"list": "Missing style"}}, "not_found", "numbering.list"),
    ({"name": "Chapter title", "numbering": {"list": "none", "level": 1}}, "invalid_arguments",
     "numbering.level"),
    ({"name": "Marked", "numbering": {"list": "1"}}, "invalid_arguments", "name"),
    ({"name": "Missing style", "numbering": {"list": "1"}}, "not_found", "name"),
])
def test_word_styles_numbering_refusals(box, session, arguments, code, field):
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", **arguments)
    assert not result.ok and result.error.code == code and result.error.field == field, result.to_json()


def test_numbering_is_modifys_only(box, session):
    result = dispatch(box, session, "word_styles", doc="d1", action="add", name="New title",
                      numbering={"list": "1"})
    assert not result.ok and result.error.field == "numbering"


def test_check_reports_the_skipped_level(box):
    session = box.session()
    session.open(make(SECTIONS_ONLY), "sections.docx")
    result = dispatch(box, session, "check", doc="d1", include=["layout"])
    assert result.ok, result.to_json()
    [entry] = result.data["layout"]["numbering_skips_level"]
    assert entry["style"] == "Section title" and entry["unused_levels"] == [0] and entry["shows"] == "1.1"
