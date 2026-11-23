"""The table style new tables get, and a style's tab stops (round 31, R3 and R4), as Word
writes them (measured in Word 16 for Mac through the oracle: ``feedback-round31/
measurements-C.md``):

* Table Design > Set as Default (this document) writes the settings' ``w:defaultTableStyle``
  (after ``w:hyphenationZone``, before ``w:characterSpacingControl``) and nothing else; Word's
  ``default table style`` reads that setting, not a table style's ``w:default``, and a table
  with no style of its own is drawn as before.
* Word's own tab stop on a style is ``w:tab w:val="right" w:leader="dot" w:pos=...``; its clear
  of an inherited stop ``w:tab w:val="clear" w:pos=...`` (no leader), which Word honours.
  A right stop past the right margin is drawn at the paragraph's right edge.

Made-up documents, a blank document's package."""

from __future__ import annotations


import pytest
from lxml import etree
from ooxml_edit.tools import Toolbox

from docx_agent.tools import FORMAT, GROUPS, TOOLS

import word_docs

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
PLAN = ('<w:style xmlns:w="' + W_NS + '" w:type="table" w:customStyle="1" w:styleId="PlanTable">'
        '<w:name w:val="Plan Table"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders>'
        '<w:top w:val="single" w:sz="4" w:space="0" w:color="2E75B6"/></w:tblBorders></w:tblPr></w:style>')


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def ok(box, session, tool: str, /, **arguments):
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def with_plan_table(box) -> bytes:
    """A blank document with a made-up table style of its own and one Table Grid table."""
    session = box.session()
    doc = session.open(word_docs.blank(), "plan.docx")
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="| a | b |\n|---|---|\n")
    return word_docs.replaced(word_docs.saved(session, doc), {
        "word/styles.xml": lambda xml: xml.replace("</w:styles>", PLAN.replace(f' xmlns:w="{W_NS}"', "")
                                                   + "</w:styles>")})


def opened(box, data: bytes):
    session = box.session()
    return session, session.open(data, "plan.docx")


def settings(session, doc):
    return word_docs.tree(word_docs.saved(session, doc), "word/settings.xml")


def part(session, doc, name: str) -> bytes:
    return etree.tostring(word_docs.tree(word_docs.saved(session, doc), name))


def table_default(box, session, doc) -> str | None:
    """The table style new tables get, as word_styles lists it."""
    listed = ok(box, session, "word_styles", doc=doc, action="list", kind="table").data
    found = [row["name"] for row in listed if row.get("default")]
    return found[0] if found else None


def table_style(session, doc, table: str) -> str | None:
    """A table's style id (``w:tblStyle``)."""
    found = word_docs.element(session, doc, table).find(f"{_W}tblPr/{_W}tblStyle")
    return found.get(_W + "val") if found is not None else None


# -- R3: the default table style -----------------------------------------------------------------


def test_set_default_writes_the_settings_and_nothing_else(box):
    session, doc = opened(box, with_plan_table(box))
    styles_before = part(session, doc, "word/styles.xml")
    body_before = part(session, doc, "word/document.xml")
    assert table_default(box, session, doc) is None
    result = ok(box, session, "word_styles", doc=doc, action="set_default", name="Plan Table")
    assert result.changed
    [node] = settings(session, doc).findall(_W + "defaultTableStyle")
    assert node.get(_W + "val") == "PlanTable"
    names = [child.tag.rpartition("}")[2] for child in settings(session, doc)]
    assert names.index("defaultTableStyle") < names.index("compat")
    assert "defaultTabStop" not in names or names.index("defaultTabStop") < names.index("defaultTableStyle")
    assert part(session, doc, "word/styles.xml") == styles_before
    assert part(session, doc, "word/document.xml") == body_before
    assert table_default(box, session, doc) == "Plan Table"
    again = ok(box, session, "word_styles", doc=doc, action="set_default", name="plan table")
    assert not again.changed


def test_new_tables_and_markdown_tables_take_the_default_first(box):
    session, doc = opened(box, with_plan_table(box))

    def table(**more) -> str:
        made = ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="| a | b |\n|---|---|\n| 1 | 2 |\n",
                  **more)
        return next(b for b in made.data["blocks"] if b.startswith("t:"))

    assert table_style(session, doc, table()) == "TableGrid"
    ok(box, session, "word_styles", doc=doc, action="set_default", name="Plan Table")
    assert table_style(session, doc, table()) == "PlanTable"
    # A style map's own table style still wins.
    assert table_style(session, doc, table(style_map=[{"element": "table", "style": "Normal Table"}])) in ("TableNormal", None)


def test_a_built_in_default_is_written_and_a_missing_one_is_none(box):
    session, doc = opened(box, word_docs.blank())
    ok(box, session, "word_styles", doc=doc, action="set_default", name="Table Grid")
    assert table_default(box, session, doc) == "Table Grid"
    for name in ("Normal", "Plan Table"):                       # a paragraph style; none by that name
        refused = box.dispatch(session, "word_styles", {"doc": doc, "action": "set_default", "name": name})
        assert not refused.ok, name
    gone = word_docs.replaced(word_docs.saved(session, doc), {"word/settings.xml": lambda xml: xml.replace(
        'w:defaultTableStyle w:val="TableGrid"', 'w:defaultTableStyle w:val="Gone"')})
    session, doc = opened(box, gone)
    assert table_default(box, session, doc) is None


def test_word_styles_sets_lists_and_describes_the_default(box):
    session = box.session()
    d = session.open(with_plan_table(box), "plan.docx")
    result = ok(box, session, "word_styles", doc=d, action="set_default", name="Plan Table")
    assert result.summary == "New tables get Plan Table"
    listed = ok(box, session, "word_styles", doc=d, action="list", kind="table")
    assert listed.summary.endswith("; new tables get Plan Table")
    assert [row["name"] for row in listed.data if row.get("default")] == ["Plan Table"]
    assert ok(box, session, "word_styles", doc=d, action="describe", name="Plan Table").data["default"] is True
    refused = box.dispatch(session, "word_styles", {"doc": d, "action": "set_default", "name": "Normal"})
    assert refused.error.code == "not_found" and "Plan Table" in refused.error.valid_options
    refused = box.dispatch(session, "word_styles", {"doc": d, "action": "set_default", "name": "Plan Table",
                                                    "kind": "paragraph"})
    assert refused.error.field == "kind"
    again = ok(box, session, "word_styles", doc=d, action="set_default", name="Plan Table")
    assert again.summary == "No change: new tables get Plan Table"


# -- R4: tab stops ---------------------------------------------------------------------------------


def contents_styles(box):
    """A contents style with a right dot stop at 346.95 pt, a style based on it, and a
    paragraph in each: the session and the document."""
    session, doc = opened(box, word_docs.blank())
    ok(box, session, "word_styles", doc=doc, action="add", name="Contents level", based_on="Normal",
       tabs=[{"position": 346.95, "alignment": "right", "leader": "dot"}])
    ok(box, session, "word_styles", doc=doc, action="add", name="Contents wide", based_on="Contents level")
    ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown="Entry\n\nEntry")
    for target, name in zip(word_docs.paragraph_ids(box, session, doc)[-2:], ("Contents level", "Contents wide")):
        ok(box, session, "word_set_text", doc=doc, target=target, text="Entry\t12", paragraph_style=name)
    return session, doc


def own_tabs(session, doc, name: str) -> list[dict]:
    root = word_docs.tree(word_docs.saved(session, doc), "word/styles.xml")
    node = next(style for style in root.iter(_W + "style") if style.find(_W + "name").get(_W + "val") == name)
    return [dict(tab.attrib) for tab in node.findall(f"{_W}pPr/{_W}tabs/{_W}tab")]


def text_width(session, doc, section: int = -1) -> int:
    """A section's text column width in twips (the page less its margins)."""
    root = word_docs.tree(word_docs.saved(session, doc))
    found = list(root.iter(_W + "sectPr"))[section]
    page, margins = found.find(_W + "pgSz"), found.find(_W + "pgMar")
    return int(page.get(_W + "w")) - int(margins.get(_W + "left")) - int(margins.get(_W + "right"))


def modify(box, session, doc, name, tabs):
    return box.dispatch(session, "word_styles", {"doc": doc, "action": "modify", "name": name, "tabs": tabs})


def test_tabs_are_written_as_word_writes_them(box):
    session, doc = contents_styles(box)
    assert own_tabs(session, doc, "Contents level") == [
        {f"{_W}val": "right", f"{_W}leader": "dot", f"{_W}pos": "6939"}]
    result = modify(box, session, doc, "Contents wide", [{"position": 400, "alignment": "right", "leader": "dot"},
                                                         {"position": 346.95, "alignment": "clear"}])
    assert result.ok and result.warnings == []
    assert own_tabs(session, doc, "Contents wide") == [{f"{_W}val": "clear", f"{_W}pos": "6939"},
                                                       {f"{_W}val": "right", f"{_W}leader": "dot", f"{_W}pos": "8000"}]


def test_a_stop_at_the_text_columns_end_and_what_tabs_refuse(box):
    session, doc = contents_styles(box)
    width = text_width(session, doc)
    assert modify(box, session, doc, "Contents wide", [{"alignment": "right", "leader": "dot"}]).ok
    assert own_tabs(session, doc, "Contents wide") == [
        {f"{_W}val": "right", f"{_W}leader": "dot", f"{_W}pos": str(width)}]
    warned = modify(box, session, doc, "Contents wide", [{"position": 100, "alignment": "clear"}])
    assert warned.warnings == ["the style inherits no tab stop at 100 pt to clear (it inherits: 346.95)"]
    assert modify(box, session, doc, "Contents wide", []).ok
    assert own_tabs(session, doc, "Contents wide") == []
    for tabs in ([{"position": 10, "alignment": "middle"}], [{"position": 10, "leader": "stars"}],
                 [{"position": 2000}], [{"alignment": "clear"}], [{"position": 10}, {"position": 10}],
                 [{"position": 10, "alignment": "clear", "leader": "dot"}], [{"at": 10}]):
        assert not modify(box, session, doc, "Contents wide", tabs).ok, tabs
    ok(box, session, "word_styles", doc=doc, action="add", name="Marked", kind="character")
    assert not modify(box, session, doc, "Marked", [{"position": 10}]).ok


def test_sections_of_different_widths_put_the_stop_at_the_widest_and_say_so(box):
    session, doc = contents_styles(box)
    after = word_docs.paragraph_ids(box, session, doc)[1]
    ok(box, session, "word_sections", doc=doc, action="insert_break", after=after)
    listed = ok(box, session, "word_sections", doc=doc, action="list").data
    first = (listed["sections"] if isinstance(listed, dict) else listed)[0]["id"]
    ok(box, session, "word_sections", doc=doc, action="set", section=first, margins={"left": 36, "right": 36})
    widths = sorted({text_width(session, doc, k) for k in (0, -1)})
    assert len(widths) == 2
    result = modify(box, session, doc, "Contents wide", [{"alignment": "right"}])
    assert result.ok and own_tabs(session, doc, "Contents wide")[0][f"{_W}pos"] == str(widths[-1])
    assert result.warnings and result.warnings[0].startswith("the sections' text columns differ")


def test_word_styles_tabs(box):
    session, d = contents_styles(box)
    ok(box, session, "word_styles", doc=d, action="modify", name="Contents wide",
       tabs=[{"position": 346.95, "alignment": "clear"}, {"alignment": "right", "leader": "dot"}])
    described = ok(box, session, "word_styles", doc=d, action="describe", name="Contents wide").data
    width = text_width(session, d) / 20
    assert described["declared"]["tabs"] == [{"position": 346.95, "alignment": "clear"},
                                             {"position": width, "alignment": "right", "leader": "dot"}]
    ok(box, session, "word_styles", doc=d, action="add", name="Contents third", based_on="Contents level",
       tabs=[{"position": 200, "alignment": "left"}])
    assert ok(box, session, "word_styles", doc=d, action="describe",
              name="Contents third").data["declared"]["tabs"] == [{"position": 200.0, "alignment": "left"}]
    refused = box.dispatch(session, "word_styles", {"doc": d, "action": "describe", "name": "Contents wide",
                                                    "tabs": [{"position": 10}]})
    assert refused.error.field == "tabs"
    ok(box, session, "word_styles", doc=d, action="modify", name="Contents wide", tabs=[])
    assert "tabs" not in ok(box, session, "word_styles", doc=d, action="describe",
                            name="Contents wide").data["declared"]
