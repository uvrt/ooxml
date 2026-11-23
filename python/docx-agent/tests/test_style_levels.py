"""A paragraph style linked to a list level (round 29, W12): ``word_styles``
``modify`` with ``numbering``, as Word's "Link level to style" does it, and ``check``'s
``numbering_skips_level``, through the toolbox.

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
import io
import re
import zipfile

import pytest

from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

# -- the made-up document (as the oracle's facts_documents.make writes it, which this test
# cannot import: it imports docx-agent's former document API) ---------------------------------

NS = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
      'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"')
_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml."


def _numpr(level: int | None) -> str:
    return "<w:numPr>" + (f'<w:ilvl w:val="{level}"/>' if level else "") + '<w:numId w:val="1"/></w:numPr>'


def _style(style_id: str, name: str, ppr: str) -> str:
    return (f'<w:style w:type="paragraph" w:customStyle="1" w:styleId="{style_id}"><w:name w:val="{name}"/>'
            f'<w:basedOn w:val="Normal"/><w:qFormat/><w:pPr>{ppr}</w:pPr></w:style>')


def _lvl(level: int, text: str, indent: int, style: str | None) -> str:
    return (f'<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="decimal"/>'
            + (f'<w:pStyle w:val="{style}"/>' if style else "")
            + f'<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>'
            f'<w:pPr><w:ind w:left="{indent}" w:hanging="{indent}"/></w:pPr></w:lvl>')


def _paragraph(text: str, style: str | None = None) -> str:
    return ("<w:p>" + (f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else "")
            + f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


#: The body: chapters and sections, an aside, body text.
FULL = [("One", "ChapterTitle"), ("One first", "SectionTitle"), ("One second", "SectionTitle"),
        ("Two", "ChapterTitle"), ("Two first", "SectionTitle"), ("Aside", "AsideTitle"), ("Body text", None)]
#: Sections and no chapter.
SECTIONS_ONLY = [("Lone first", "SectionTitle"), ("Lone second", "SectionTitle"), ("Body text", None)]


def make(body=FULL) -> bytes:
    styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {NS}>'
              '<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="20"/></w:rPr></w:rPrDefault></w:docDefaults>'
              '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
              '<w:style w:type="character" w:customStyle="1" w:styleId="Marked"><w:name w:val="Marked"/></w:style>'
              + _style("ChapterTitle", "Chapter title", _numpr(None) + '<w:ind w:left="300" w:hanging="300"/>')
              + _style("SectionTitle", "Section title", _numpr(1) + '<w:ind w:left="900" w:hanging="300"/>')
              + _style("AsideTitle", "Aside title", '<w:ind w:left="1100" w:hanging="200"/>')
              + "</w:styles>")
    numbering = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering {NS}>'
                 '<w:abstractNum w:abstractNumId="0"><w:nsid w:val="5D1E2F30"/>'
                 '<w:multiLevelType w:val="multilevel"/>'
                 + _lvl(0, "%1", 432, "ChapterTitle") + _lvl(1, "%1.%2", 576, "SectionTitle")
                 + _lvl(2, "%1.%2.%3", 720, None)
                 + '</w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>')
    document = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>'
                + "".join(_paragraph(text, style) for text, style in body)
                + '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1418" w:right="1418" '
                'w:bottom="1418" w:left="1418" w:header="709" w:footer="709" w:gutter="0"/></w:sectPr>'
                "</w:body></w:document>")
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            f'<Override PartName="/word/document.xml" ContentType="{_CT}document.main+xml"/>'
            f'<Override PartName="/word/styles.xml" ContentType="{_CT}styles+xml"/>'
            f'<Override PartName="/word/numbering.xml" ContentType="{_CT}numbering+xml"/></Types>'))
        archive.writestr("_rels/.rels", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{_REL}officeDocument" Target="word/document.xml"/></Relationships>'))
        archive.writestr("word/_rels/document.xml.rels", (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f'<Relationship Id="rId1" Type="{_REL}styles" Target="styles.xml"/>'
            f'<Relationship Id="rId2" Type="{_REL}numbering" Target="numbering.xml"/></Relationships>'))
        archive.writestr("word/document.xml", document)
        archive.writestr("word/styles.xml", styles)
        archive.writestr("word/numbering.xml", numbering)
    return out.getvalue()


# -- reading it ---------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def dispatch(box, session, tool, /, **arguments):
    return box.dispatch(session, tool, arguments)


def opened(box, data: bytes | None = None):
    session = box.session(clock=lambda: dt.datetime(2026, 11, 3, 12, 0, tzinfo=dt.timezone.utc))
    session.open(make() if data is None else data, "levels.docx")
    return session


def modify(box, session, name, **arguments):
    """``word_styles`` ``modify``: its result, which must be ok."""
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", name=name, **arguments)
    assert result.ok, result.to_json()
    return result


def style_ppr(session, style_id: str):
    root = word_docs.tree(word_docs.saved(session, "d1"), "word/styles.xml")
    node = next(s for s in root.iter(W + "style") if s.get(W + "styleId") == style_id)
    return node.find(W + "pPr")


def numpr(session, style_id: str) -> tuple[str | None, str | None] | None:
    properties = style_ppr(session, style_id)
    found = properties.find(W + "numPr") if properties is not None else None
    if found is None:
        return None
    ilvl, num = found.find(W + "ilvl"), found.find(W + "numId")
    return (ilvl.get(W + "val") if ilvl is not None else None, num.get(W + "val") if num is not None else None)


def indent(session, style_id: str):
    properties = style_ppr(session, style_id)
    found = properties.find(W + "ind") if properties is not None else None
    return None if found is None else (found.get(W + "left"), found.get(W + "hanging"))


def linked(session) -> dict[int, str | None]:
    """Each level of the list's definition: the style it names."""
    root = word_docs.tree(word_docs.saved(session, "d1"), "word/numbering.xml")
    out = {}
    for lvl in root.iter(W + "lvl"):
        tied = lvl.find(W + "pStyle")
        out[int(lvl.get(W + "ilvl"))] = tied.get(W + "val") if tied is not None else None
    return out


def labels(data: bytes) -> list[str]:
    """docx2svg's list labels, line by line ('' where a line has none)."""
    import docx2svg

    out = []
    for svg in docx2svg.convert_docx_to_svg(data):
        for line in re.findall(r'<g data-docx-line="\d+"[^>]*>(.*?)</g>', svg, re.S):
            found = re.search(r'<text [^>]*data-docx-kind="label"[^>]*>([^<]*)</text>', line)
            out.append(found.group(1) if found else "")
    return out


def paragraph_ids(box, session) -> list[str]:
    """The paragraphs' ids as ``word_read`` names them, in order (a document without paraIds
    has positional ones, ``p@body/0``; a list item's comment ends its line)."""
    return re.findall(r"<!-- (p(?::[0-9A-Fa-f]+|@[\w/]+))", word_docs.read(box, session, "d1"))


def labels_of(session) -> list[str]:
    return labels(word_docs.saved(session, "d1"))


def valid(box, session) -> None:
    """No numbering or style problem the document did not open with."""
    found = word_docs.validation(box, session, "d1")
    assert [p for p in found["new"] if p.startswith(("numbering", "style"))] == [], found


def undone_is_opened(box, session, opened_as: bytes) -> None:
    assert dispatch(box, session, "undo", doc="d1").ok
    assert word_docs.saved(session, "d1") == opened_as


# -- Word's numbering of the made-up document ----------------------------------------------------


def test_the_document_numbers_chapters_and_sections():
    assert labels(make()) == ["1", "1.1", "1.2", "2", "2.1", "", ""]


# -- linking: word_styles modify numbering -------------------------------------------------------


def test_linking_a_style_to_a_level_unlinks_the_style_that_held_it(box):
    session = opened(box)
    before = word_docs.saved(session, "d1")
    result = modify(box, session, "Aside title", numbering={"list": "1", "level": 1})
    # The level names the new style; it numbers at that level with the level's indent.
    assert linked(session) == {0: "ChapterTitle", 1: "AsideTitle", 2: None}
    assert numpr(session, "AsideTitle") == ("1", "1") and indent(session, "AsideTitle") is None
    # The style that held the level loses its numbering and its indent (Word, measured).
    assert numpr(session, "SectionTitle") is None and indent(session, "SectionTitle") is None
    assert result.warnings == ["'Section title' held level 1 of list 1 before: unlinked, its 3 "
                               "paragraph(s) are no longer numbered"]
    assert numpr(session, "ChapterTitle") == (None, "1") and indent(session, "ChapterTitle") == ("300", "300")
    assert labels_of(session) == ["1", "", "", "2", "", "2.1", ""]      # as Word numbers it
    valid(box, session)
    undone_is_opened(box, session, before)


def test_a_level_no_style_held_and_the_list_named_by_a_style_in_it(box):
    session = opened(box)
    result = modify(box, session, "Aside title", numbering={"list": "Chapter title", "level": 2})
    assert not result.warnings
    assert linked(session) == {0: "ChapterTitle", 1: "SectionTitle", 2: "AsideTitle"}
    assert numpr(session, "AsideTitle") == ("2", "1") and indent(session, "SectionTitle") == ("900", "300")
    assert labels_of(session) == ["1", "1.1", "1.2", "2", "2.1", "2.1.1", ""]
    valid(box, session)


def test_level_zero_is_written_without_an_ilvl(box):
    session = opened(box)
    modify(box, session, "Aside title", numbering={"list": "1", "level": 0})
    assert numpr(session, "AsideTitle") == (None, "1") and numpr(session, "ChapterTitle") is None
    assert linked(session)[0] == "AsideTitle"
    valid(box, session)


def test_a_style_linked_to_another_level_moves(box):
    session = opened(box)
    modify(box, session, "Section title", numbering={"list": "1", "level": 2})
    assert linked(session) == {0: "ChapterTitle", 1: None, 2: "SectionTitle"}
    assert numpr(session, "SectionTitle") == ("2", "1")
    valid(box, session)


def test_none_unlinks_the_style(box):
    session = opened(box)
    result = modify(box, session, "Section title", numbering={"list": "none"})
    assert result.summary.endswith("unlinked from its list level") and not result.warnings
    assert linked(session) == {0: "ChapterTitle", 1: None, 2: None}
    assert numpr(session, "SectionTitle") is None and indent(session, "SectionTitle") is None
    assert labels_of(session) == ["1", "", "", "2", "", "", ""]
    valid(box, session)
    # A style in no list: unlinking it leaves it as it is (its indent too).
    modify(box, session, "Aside title", numbering={"list": "none"})
    assert indent(session, "AsideTitle") == ("1100", "200")


def test_formatting_and_numbering_in_one_call_are_one_undo_step(box):
    session = opened(box)
    before = word_docs.saved(session, "d1")
    modify(box, session, "Aside title", bold=True, numbering={"list": "1", "level": 2})
    assert numpr(session, "AsideTitle") == ("2", "1")
    undone_is_opened(box, session, before)


def test_word_styles_links_and_unlinks(box):
    session = opened(box)
    result = modify(box, session, "Aside title", numbering={"list": "1", "level": 1})
    assert result.summary == "Modified style Aside title: linked to level 1 of list 1"
    assert result.warnings and "'Section title' held level 1" in result.warnings[0]
    assert linked(session)[1] == "AsideTitle"
    result = modify(box, session, "Aside title", numbering={"list": "none"})
    assert result.summary.endswith("unlinked from its list level")
    assert linked(session)[1] is None
    modify(box, session, "Section title", numbering={"list": "Chapter title", "level": 1})
    assert numpr(session, "SectionTitle") == ("1", "1")


@pytest.mark.parametrize("arguments, code, field", [
    ({"name": "Chapter title", "numbering": {"list": "9", "level": 0}}, "invalid_arguments", "numbering.list"),
    ({"name": "Chapter title", "numbering": {"list": "1", "level": 5}}, "invalid_arguments", "numbering.level"),
    ({"name": "Chapter title", "numbering": {"list": "Missing style"}}, "not_found", "numbering.list"),
    ({"name": "Chapter title", "numbering": {"list": "none", "level": 1}}, "invalid_arguments",
     "numbering.level"),
    ({"name": "Aside title", "numbering": {"list": "Aside title", "level": 1}}, "invalid_arguments",
     "numbering.list"),
    ({"name": "Marked", "numbering": {"list": "1"}}, "invalid_arguments", "name"),
    ({"name": "Missing style", "numbering": {"list": "1"}}, "not_found", "name"),
])
def test_word_styles_numbering_refusals(box, arguments, code, field):
    session = opened(box)
    before = word_docs.saved(session, "d1")
    result = dispatch(box, session, "word_styles", doc="d1", action="modify", **arguments)
    assert not result.ok and result.error.code == code and result.error.field == field, result.to_json()
    assert word_docs.saved(session, "d1") == before


def test_numbering_is_modifys_only(box):
    session = opened(box)
    result = dispatch(box, session, "word_styles", doc="d1", action="add", name="New title",
                      numbering={"list": "1"})
    assert not result.ok and result.error.field == "numbering"


# -- check: numbering_skips_level ---------------------------------------------------------------


def skips(box, session) -> list[dict]:
    result = dispatch(box, session, "check", doc="d1", include=["layout"])
    assert result.ok, result.to_json()
    return result.data["layout"].get("numbering_skips_level", [])


def test_a_level_used_with_the_level_above_never_used_is_a_fact(box):
    assert labels(make(SECTIONS_ONLY))[:2] == ["1.1", "1.2"]                 # Word: 1.1, 1.2 (measured)
    session = opened(box, make(SECTIONS_ONLY))
    first = paragraph_ids(box, session)[0]
    [entry] = skips(box, session)
    assert entry == {"para": first, "style": "Section title", "level": 1, "list": 1,
                     "unused_levels": [0], "label": "%1.%2", "shows": "1.1", "paragraphs": 2}
    assert skips(box, opened(box)) == []


def test_a_skipped_middle_level_names_it_and_shows_no_number(box):
    session = opened(box)
    modify(box, session, "Section title", numbering={"list": "1", "level": 2})   # level 1 now unused
    [entry] = skips(box, session)
    assert (entry["style"], entry["level"], entry["unused_levels"], entry["paragraphs"]) == \
        ("Section title", 2, [1], 3)
    assert "shows" not in entry                  # level 0 is used: its count is the chapter's
