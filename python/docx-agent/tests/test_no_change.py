"""Asked for what is so already (round 27): a list removal of a paragraph in no list, a
header removed or linked where the section has none of its own, unlinked where it has one,
and a drawing floated or put in the line where it is so -- each succeeds with no change, a
warning saying why.  (A table's keeps off: ``test_table_keeps.py``; the drawings' library
half was the library's.)  Made-up documents in a new blank package."""

from __future__ import annotations


import pytest
from ooxml_edit.tools import Toolbox
from test_tools_layout_facts import docx, ok, p
from layout_reads import ids_of

import word_docs
from docx_agent.tools import FORMAT, GROUPS, TOOLS


from probe_package import picture_document  # noqa: E402


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


@pytest.fixture
def session(box):
    return box.session()


def test_word_lists_remove_over_a_span_takes_out_the_items_and_names_the_rest(box, session):
    """A span of items and plain paragraphs was refused at the first plain one; now the
    items leave their list and one warning names the paragraphs that were in none."""
    d = session.open(docx(p("Lead") + p("One") + p("Two") + p("Tail")), "lists.docx")
    lead, one, two, tail = ids_of(session, d)
    ok(box, session, "word_lists", doc=d, targets=[f"{one}..{two}"], action="add", kind="number")
    one, two = ids_of(session, d)[1:3]
    result = ok(box, session, "word_lists", doc=d, targets=[f"{lead}..{tail}"], action="remove")
    assert word_docs.tree(word_docs.saved(session, d)).find(".//{*}numPr") is None
    assert result.warnings == [f"2 paragraph(s) were not in a list, left as they were: {lead}, {tail}"]
    before = word_docs.saved(session, d)
    again = ok(box, session, "word_lists", doc=d, targets=[lead], action="remove")
    assert again.changed == [] and again.touched == [] and word_docs.saved(session, d) == before
    assert again.warnings == [f"1 paragraph(s) were not in a list, left as they were: {lead}"]


@pytest.mark.parametrize("action", ["remove", "link"])
def test_no_header_of_its_own_to_remove_or_link(box, session, action):
    d = session.open(docx(p("Body")), "plain.docx")
    result = ok(box, session, "word_headers_footers", doc=d, section="s:body", which="header", action=action)
    note = "s:body had no default header of its own: no change"
    assert result.summary == note and result.warnings == [note] and result.touched == []


def test_removing_a_header_of_its_own_still_says_removed(box, session):
    d = session.open(docx(p("Body")), "own.docx")
    ok(box, session, "word_headers_footers", doc=d, section="s:body", which="footer", action="set", text="Foot")
    result = ok(box, session, "word_headers_footers", doc=d, section="s:body", which="footer", action="remove")
    assert result.summary == "Removed the default footer of s:body" and not result.warnings


def test_unlink_a_header_of_its_own_already(box, session):
    d = session.open(docx(p("Body")), "unlink.docx")
    story = ok(box, session, "word_headers_footers", doc=d, section="s:body", which="header", action="set",
               text="Head").data["story"]
    result = ok(box, session, "word_headers_footers", doc=d, section="s:body", which="header", action="unlink")
    note = f"s:body had its own default header already ({story}): no change"
    assert result.summary == note and result.warnings == [note] and result.data == {"story": story}
    assert result.touched == []


def test_word_drawings_inline_and_float_what_is_so_already(box, session):
    d = session.open(picture_document(), "drawings.docx")
    picture = "d:1"
    result = ok(box, session, "word_drawings", doc=d, action="inline", target=picture)
    assert result.summary == f"No change to {picture}" and result.changed == [] and result.touched == []
    assert result.warnings == [f"{picture} is inline already"]
    ok(box, session, "word_drawings", doc=d, action="float", target=picture)
    result = ok(box, session, "word_drawings", doc=d, action="float", target=picture)
    assert result.summary == f"No change to {picture}" and result.warnings == [f"{picture} floats already"]
    result = ok(box, session, "word_drawings", doc=d, action="float", target=picture, wrap="square")
    assert result.changed == [picture]
    assert word_docs.tree(word_docs.saved(session, d)).find(".//{*}anchor/{*}wrapSquare") is not None


def test_a_bookmark_already_there_changes_nothing(box, session):
    """Round 27: the same name over exactly the same text is as asked; elsewhere refused."""
    d = session.open(docx(p("Alpha beta gamma") + p("Delta")), "marks.docx")
    first, second = ids_of(session, d)
    ok(box, session, "word_links", doc=d, action="add_bookmark", target=f"{first}@6:10", bookmark="Mark")
    first = ids_of(session, d)[0]
    before = word_docs.saved(session, d)
    result = ok(box, session, "word_links", doc=d, action="add_bookmark", target=f"{first}@6:10", bookmark="mark")
    assert result.warnings == ["bm:Mark marks this text already"] and result.touched == []
    assert result.summary == "No change: bm:Mark marks this text already" and word_docs.saved(session, d) == before
    for target in (f"{first}@0:5", f"{second}@0:5"):
        refused = box.dispatch(session, "word_links", {"doc": d, "action": "add_bookmark", "target": target,
                                                       "bookmark": "Mark"})
        assert not refused.ok and "already has a bookmark" in refused.error.message
