"""Lists numbered by their style (round 23, V1): Markdown lists written in a style that numbers
itself take its numbering -- no ``w:numPr``, no indent -- and count on as Word counts them;
``restart`` gives the restarted item alone an instance of its own, Word's form; ``continue``
drops the direct numbering of items whose style numbers them.  What Word does is measured in
``feedback-round23/measurements-F.md`` (made-up documents, the Word oracle).  Through the
toolbox: ``word_insert_markdown``, ``word_lists`` and ``word_inspect``'s list facts."""

from __future__ import annotations

import copy
import re

import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W = "{%s}" % W_NS
_CT = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"
_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering"


def _level(ilvl: int, fmt: str, text: str, left: int, hanging: int, style: str | None) -> str:
    tied = f'<w:pStyle w:val="{style}"/>' if style else ""
    return (f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/>{tied}'
            f'<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>'
            f'<w:pPr><w:ind w:left="{left}" w:hanging="{hanging}"/></w:pPr></w:lvl>')


def _style(style_id: str, name: str, ppr: str, based: str = "Normal") -> str:
    return (f'<w:style w:type="paragraph" w:styleId="{style_id}"><w:name w:val="{name}"/>'
            f'<w:basedOn w:val="{based}"/><w:pPr>{ppr}</w:pPr></w:style>')


def template(*, style_indent: int = 284, extra_styles: str = "") -> bytes:
    """A made-up template: List Number numbers itself (level 0 of a multilevel definition, List
    Number 2 tied to its level 1), List Bullet too; Probe Unnumbered is List Number with numId 0."""
    numbering = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="{W_NS}">'
                 '<w:abstractNum w:abstractNumId="1"><w:nsid w:val="5E1A0001"/><w:multiLevelType w:val="multilevel"/>'
                 + _level(0, "decimal", "%1.", 284, 284, "ListNumber")
                 + _level(1, "lowerLetter", "%2)", 568, 284, "ListNumber2") + "</w:abstractNum>"
                 '<w:abstractNum w:abstractNumId="2"><w:nsid w:val="5E1A0002"/><w:multiLevelType w:val="multilevel"/>'
                 + _level(0, "bullet", "•", 284, 284, "ListBullet") + "</w:abstractNum>"
                 '<w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>'
                 '<w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num></w:numbering>')
    indent = f'<w:ind w:left="{style_indent}" w:hanging="284"/>'
    styles = (_style("ListNumber", "List Number", f'<w:numPr><w:numId w:val="1"/></w:numPr>{indent}')
              + _style("ListNumber2", "List Number 2",
                       '<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr><w:ind w:left="568" w:hanging="284"/>')
              + _style("ListBullet", "List Bullet", f'<w:numPr><w:numId w:val="2"/></w:numPr>{indent}')
              + _style("ProbeUnnumbered", "Probe Unnumbered", '<w:numPr><w:numId w:val="0"/></w:numPr>', "ListNumber")
              + extra_styles)
    return _with_numbering(word_docs.replaced(word_docs.blank(), {
        "word/styles.xml": lambda xml: xml.replace("</w:styles>", styles + "</w:styles>"),
        "[Content_Types].xml": lambda xml: xml.replace(
            "</Types>", f'<Override PartName="/word/numbering.xml" ContentType="{_CT}"/></Types>'),
        "word/_rels/document.xml.rels": lambda xml: xml.replace(
            "</Relationships>", f'<Relationship Id="rIdNumbering9" Type="{_REL}" Target="numbering.xml"/>'
                                "</Relationships>"),
    }), numbering)


def _with_numbering(data: bytes, numbering: str) -> bytes:
    """``data`` (made by :func:`template`) with its numbering part added."""
    import io
    import zipfile

    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            target.writestr(info, source.read(info.filename))
        target.writestr("word/numbering.xml", numbering)
    return out.getvalue()


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as toolbox:
        yield toolbox


def opened(box, data: bytes):
    session = box.session()
    session.open(data, "template.docx")
    return session


def insert(box, session, markdown: str, **arguments):
    return word_docs.call(box, session, "word_insert_markdown", doc="d1", at="end", markdown=markdown, **arguments)


def lists(box, session, action: str, target: str):
    return word_docs.call(box, session, "word_lists", doc="d1", targets=[target], action=action)


def inspected(box, session, identifier: str) -> dict:
    [block] = word_docs.call(box, session, "word_inspect", doc="d1", target=identifier).data["blocks"]
    return block


def list_of(box, session, identifier: str) -> dict | None:
    return inspected(box, session, identifier).get("list")


def labels(box, session, ids: list[str]) -> list[str | None]:
    return [(list_of(box, session, i) or {}).get("label") for i in ids]


def own_numbering(session, identifier: str):
    return word_docs.element(session, "d1", identifier).find(f"{W}pPr/{W}numPr")


def items(box, session, result) -> list[str]:
    return [i for i in result.data["blocks"] if list_of(box, session, i) is not None]


def _numbering(session):
    return word_docs.tree(word_docs.saved(session, "d1"), "word/numbering.xml")


def nums(session) -> dict[int, object]:
    return {int(n.get(W + "numId")): n for n in _numbering(session).iter(W + "num")}


def abstract_of(session, num_id: int) -> str:
    return nums(session)[num_id].find(W + "abstractNumId").get(W + "val")


def style_num(session, style_id: str) -> int:
    styles = word_docs.tree(word_docs.saved(session, "d1"), "word/styles.xml")
    style = next(s for s in styles.iter(W + "style") if s.get(W + "styleId") == style_id)
    return int(style.find(f"{W}pPr/{W}numPr/{W}numId").get(W + "val"))


def valid(box, session) -> None:
    found = word_docs.validation(box, session, "d1")
    assert found["new"] == [], found


def markdown_of(box, session, first: str, last: str) -> str:
    """``word_read``'s Markdown of a span, its id comments left out."""
    text = word_docs.read(box, session, "d1", range=f"{first}..{last}")
    text = re.sub(r"^<!-- [pt][:@][^>]*-->\n", "", text, flags=re.M)
    return re.sub(r" ?<!-- [pt][:@][^>]*-->", "", text)


# -- writing -------------------------------------------------------------------------------------


def test_a_markdown_list_in_a_style_that_numbers_itself_takes_its_numbering_across_calls(box):
    session = opened(box, template())
    nums_before = set(nums(session))
    first = insert(box, session, "1. one\n2. two\n")
    insert(box, session, "Between the lists.\n")
    second = insert(box, session, "1. three\n")
    ids = first.data["blocks"] + second.data["blocks"]
    for identifier in ids:
        paragraph = word_docs.element(session, "d1", identifier)
        assert paragraph.find(f"{W}pPr/{W}pStyle").get(W + "val") == "ListNumber"
        assert paragraph.find(f"{W}pPr/{W}numPr") is None and paragraph.find(f"{W}pPr/{W}ind") is None
        assert list_of(box, session, identifier)["from_style"]
    assert labels(box, session, ids) == ["1.", "2.", "3."]       # Word: 3 after the interlude (measured)
    assert set(nums(session)) == nums_before     # no instance, no definition added
    valid(box, session)


def test_nesting_through_a_style_tied_to_the_level_and_bullets_by_style(box):
    session = opened(box, template())
    blocks = insert(box, session, "1. a\n   1. inner\n   2. inner two\n2. b\n\n- dot\n- dot two\n").data["blocks"]
    assert all(own_numbering(session, i) is None for i in blocks)
    assert [list_of(box, session, i)["level"] for i in blocks] == [0, 1, 1, 0, 0, 0]
    assert labels(box, session, blocks[:4]) == ["1.", "a)", "b)", "2."]
    assert markdown_of(box, session, blocks[0], blocks[-1]) == \
        "1. a\n   1. inner\n   2. inner two\n2. b\n\n- dot\n- dot two\n"


def test_a_list_starting_elsewhere_and_a_later_list_restart_on_their_first_item(box):
    session = opened(box, template())
    insert(box, session, "1. earlier\n")
    result = insert(box, session, "4. four\n5. five\n\nBetween.\n\n1. again\n2. again two\n")
    listed = items(box, session, result)
    first, restarted = own_numbering(session, listed[0]), own_numbering(session, listed[2])
    assert first is not None and restarted is not None
    assert own_numbering(session, listed[1]) is None and own_numbering(session, listed[3]) is None
    by_style = style_num(session, "ListNumber")
    for node, start in ((first, "4"), (restarted, "1")):
        num_id = int(node.find(W + "numId").get(W + "val"))
        assert abstract_of(session, num_id) == abstract_of(session, by_style)
        override = nums(session)[num_id].find(f"{W}lvlOverride/{W}startOverride")
        assert override.get(W + "val") == start
    assert labels(box, session, listed) == ["4.", "5.", "1.", "2."]
    # Word's restart form reads back as one list each.
    blocks = result.data["blocks"]
    assert markdown_of(box, session, blocks[0], blocks[-1]) == "4. four\n5. five\n\nBetween.\n\n1. again\n2. again two\n"
    valid(box, session)


def test_a_style_that_does_not_number_itself_keeps_direct_numbering(box):
    tied = _style("ProbeTied", "Probe Tied", "<w:numPr><w:numId w:val=\"1\"/></w:numPr>")
    session = opened(box, template(extra_styles=tied))
    for style in ("Probe Unnumbered", "Probe Tied"):    # numId 0; a level tied to List Number
        blocks = insert(box, session, "1. x\n2. y\n", style_map=[{"element": "number", "style": style}]).data["blocks"]
        assert all(own_numbering(session, i) is not None for i in blocks), style
        assert labels(box, session, blocks) == ["1.", "2."]


def test_word_s_own_list_number_2_is_its_own_list_so_nesting_stays_direct(box):
    session = opened(box, word_docs.blank())
    a, b, c = insert(box, session, "1. a\n   1. b\n2. c\n").data["blocks"]
    assert own_numbering(session, a) is None and own_numbering(session, c) is None
    assert own_numbering(session, b) is not None and list_of(box, session, b)["level"] == 1
    assert labels(box, session, [a, b, c]) == ["1.", "a.", "2."]


# -- restart and continue --------------------------------------------------------------------------


def test_restart_on_a_style_numbered_item_overrides_that_item_alone(box):
    session = opened(box, template())
    listed = items(box, session, insert(box, session, "1. one\n2. two\n\nBetween.\n\n3. three\n4. four\n5. five\n"))
    edit = lists(box, session, "restart", listed[2])
    assert own_numbering(session, listed[2]) is not None
    assert all(own_numbering(session, i) is None for i in listed[3:])
    assert labels(box, session, listed) == ["1.", "2.", "1.", "2.", "3."]     # Word: measured
    assert edit.warnings == []          # the level's indent is the style's
    valid(box, session)


def _override_level(session, identifier: str):
    num_id = int(own_numbering(session, identifier).find(W + "numId").get(W + "val"))
    return nums(session)[num_id].find(f"{W}lvlOverride")


def test_restart_keeps_the_style_s_indent_where_the_level_s_differs(box):
    # Round 24, W7: direct numbering takes the level's indent, so the restart's instance
    # carries the level with the style's indent -- Word draws the item at the style's indent
    # (measured; a startOverride alone put it at the level's).
    session = opened(box, template(style_indent=720))
    blocks = insert(box, session, "1. one\n2. two\n3. three\n").data["blocks"]
    edit = lists(box, session, "restart", blocks[1])
    assert edit.warnings == []
    override = _override_level(session, blocks[1])
    assert override.find(f"{W}startOverride").get(W + "val") == "1"
    level = override.find(W + "lvl")
    assert level.find(f"{W}start").get(W + "val") == "1"
    assert level.find(f"{W}pPr/{W}ind").attrib == {W + "left": "720", W + "hanging": "284"}
    assert level.find(f"{W}lvlText").get(W + "val") == "%1." and level.find(f"{W}pStyle") is not None
    assert labels(box, session, blocks) == ["1.", "1.", "2."]
    indent = inspected(box, session, blocks[1])["effective"]["indent"]
    assert (indent["left"], round(indent["hanging"], 1)) == (36, 14.2)
    valid(box, session)


def test_a_markdown_restart_keeps_the_style_s_indent_and_its_start(box):
    session = opened(box, template(style_indent=720))
    insert(box, session, "1. earlier\n")
    result = insert(box, session, "4. four\n5. five\n\nBetween.\n\n1. again\n")
    listed = items(box, session, result)
    for identifier, start in ((listed[0], "4"), (listed[2], "1")):
        override = _override_level(session, identifier)
        assert override.find(f"{W}startOverride").get(W + "val") == start
        # With a whole level in the override Word counts from the level's start (measured).
        assert override.find(f"{W}lvl/{W}start").get(W + "val") == start
        assert inspected(box, session, identifier)["effective"]["indent"]["left"] == 36
    assert labels(box, session, listed) == ["4.", "5.", "1."]
    blocks = result.data["blocks"]
    assert markdown_of(box, session, blocks[0], blocks[-1]) == "4. four\n5. five\n\nBetween.\n\n1. again\n"


def _direct_items(box, session, ids: list[str]):
    """0.1.20's form: each item its own numPr over a copy of the definition -- written into the
    saved package, which a new session opens (the ids are the paragraphs' paraIds)."""
    data = word_docs.saved(session, "d1")
    numbering = word_docs.tree(data, "word/numbering.xml")
    source = abstract_of(session, style_num(session, "ListNumber"))
    abstract = next(a for a in numbering.iter(W + "abstractNum") if a.get(W + "abstractNumId") == source)
    made = copy.deepcopy(abstract)
    new_abstract = str(max(int(a.get(W + "abstractNumId")) for a in numbering.iter(W + "abstractNum")) + 1)
    made.set(W + "abstractNumId", new_abstract)
    for nsid in made.iter(W + "nsid"):
        nsid.set(W + "val", "5E1A0F0F")
    for tied in made.iter(W + "pStyle"):
        tied.getparent().remove(tied)
    last_abstract = list(numbering.iter(W + "abstractNum"))[-1]
    last_abstract.addnext(made)
    num_id = str(max(int(n.get(W + "numId")) for n in numbering.iter(W + "num")) + 1)
    num = etree.SubElement(numbering, W + "num")
    num.set(W + "numId", num_id)
    etree.SubElement(num, W + "abstractNumId").set(W + "val", new_abstract)
    body = word_docs.tree(data)
    for identifier in ids:
        ppr = word_docs.find(body, identifier).find(W + "pPr")
        numpr = etree.Element(W + "numPr")
        etree.SubElement(numpr, W + "ilvl").set(W + "val", "0")
        etree.SubElement(numpr, W + "numId").set(W + "val", num_id)
        style = ppr.find(W + "pStyle")
        (style.addnext(numpr) if style is not None else ppr.insert(0, numpr))
    xml = lambda root: etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)  # noqa: E731
    again = box.session()
    again.open(word_docs.replaced(data, {"word/numbering.xml": xml(numbering), "word/document.xml": xml(body)}),
               "direct.docx")
    return again


def test_continue_drops_direct_numbering_where_the_style_numbers_the_items(box):
    session = opened(box, template())
    listed = items(box, session, insert(box, session, "1. one\n2. two\n\nBetween.\n\n1. three\n2. four\n"))
    session = _direct_items(box, session, listed[2:])
    assert labels(box, session, listed) == ["1.", "2.", "1.", "2."]
    lists(box, session, "continue", listed[2])
    assert all(own_numbering(session, i) is None for i in listed)
    assert labels(box, session, listed) == ["1.", "2.", "3.", "4."]
    valid(box, session)


def test_continue_with_nothing_to_continue_says_why(box):
    session = opened(box, template())
    blocks = insert(box, session, "1. one\n2. two\n").data["blocks"]
    session = _direct_items(box, session, blocks)
    result = box.dispatch(session, "word_lists", {"doc": "d1", "targets": [blocks[0]], "action": "continue"})
    assert not result.ok, result.to_json()
    assert re.search("has direct numbering .* while its style 'List Number' numbers itself", result.error.message)


def test_the_lists_tool_restarts_and_continues_style_numbered_items(box):
    session = opened(box, template())
    listed = items(box, session, insert(box, session, "1. one\n2. two\n3. three\n"))
    lists(box, session, "restart", listed[1])
    assert labels(box, session, listed) == ["1.", "1.", "2."]
    lists(box, session, "continue", listed[1])
    assert labels(box, session, listed) == ["1.", "2.", "3."] and own_numbering(session, listed[1]) is None
