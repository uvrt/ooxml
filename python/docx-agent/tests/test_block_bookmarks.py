"""A bookmark over blocks, tables and content controls included (round 49, R91), written where
Word's Insert > Bookmark writes it for a selection of those blocks (measured in Word for Mac
16.106: the selection's range, Word's saved XML, Word reading our file back to the same
character range, and Word's re-save keeping our markers where they are)."""

from __future__ import annotations

import pytest
from ooxml_edit.tools import Toolbox
from layout_reads import block_ids, table_ids, written

from docx_agent.tools import FORMAT, GROUPS, TOOLS
import word_docs

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

MD = """Opening line.

Second line before the grid.

| Code | Amount |
|---|---|
| K7-1 | 12 |
| K7-2 | 30 |

Third line after the grid.

Fourth line.

| Item | Note |
|---|---|
| one | first |

| Next | Grid |
|---|---|
| two | second |

Closing line."""


@pytest.fixture(scope="module")
def box():
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        yield toolbox


def _made(box, session, name: str = "spans.docx") -> tuple[str, dict[str, str]]:
    """The made-up document the measurements used, and its blocks by a short name."""
    d = written(box, session, MD, name)
    names = ["opening", "second", "grid", "third", "fourth", "items", "gap", "next", "closing"]
    ids = block_ids(box, session, d)
    assert len(ids) == len(names), ids
    return d, dict(zip(names, ids))


def _cc(number: int, text: str, tag: str) -> str:
    return (f'<w:sdt><w:sdtPr><w:tag w:val="{tag}"/><w:id w:val="{number}"/><w:richText/></w:sdtPr><w:sdtContent>'
            f'<w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:sdtContent></w:sdt>')


def _with_controls(box, session) -> tuple[str, dict[str, str]]:
    """A control first in the body before an empty paragraph, one before a table."""
    body = (_cc(11, "Control one.", "ccone") + '<w:p w14:paraId="00000301" w14:textId="77777777"/>'
            + _cc(12, "Control two.", "cctwo")
            + '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4500"/>'
            '<w:gridCol w:w="4500"/></w:tblGrid>'
            + "".join(f'<w:tr w14:paraId="0000031{k}" w14:textId="77777777">'
                      + "".join(f'<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{t}</w:t>'
                                "</w:r></w:p></w:tc>" for t in row) + "</w:tr>"
                      for k, row in enumerate((("A", "B"), ("1", "2"))))
            + '</w:tbl><w:p w14:paraId="00000302" w14:textId="77777777"><w:r><w:t>Last line.</w:t></w:r></w:p>')
    d = session.open(word_docs.with_body(body), "controls.docx")
    return d, {"one": "cc:11", "two": "cc:12", "gap": "p:00000301", "table": "t:00000310", "last": "p:00000302"}


def _bookmark(box, session, d: str, target: str, name: str = "Span1"):
    result = box.dispatch(session, "word_links", {"doc": d, "action": "add_bookmark", "target": target,
                                                  "bookmark": name})
    assert result.ok, result.to_json()
    return result


def _root(session, d: str):
    return word_docs.tree(word_docs.saved(session, d))


def markers(root, name: str = "Span1"):
    start = next(m for m in root.iter(W + "bookmarkStart") if m.get(W + "name") == name)
    end = next(m for m in root.iter(W + "bookmarkEnd") if m.get(W + "id") == start.get(W + "id"))
    return start, end


def element(box, session, d: str, root, identifier: str):
    """A block's element in ``root`` (the document as it is now)."""
    if identifier.startswith("t"):
        index = table_ids(box, session, d).index(identifier)
        return [b for b in word_docs.body_blocks(root) if b.tag == W + "tbl"][index]
    if identifier.startswith("cc:"):
        return next(sdt for sdt in root.iter(W + "sdt") if sdt.find(f"{W}sdtPr/{W}id").get(W + "val")
                    == identifier[3:])
    return word_docs.find(root, identifier)


def bookmark_text(box, session, d: str, name: str) -> str:
    """The whole text a bookmark marks, read from the package: each paragraph's part of it a
    line (``word_links`` list says it shortened, on one line)."""
    root = word_docs.tree(word_docs.saved(session, d))
    start = next(m for m in root.iter(W + "bookmarkStart") if m.get(W + "name") == name)
    mark = start.get(W + "id")
    lines: list[str] | None = None
    in_paragraph = False                   # the start is inside a paragraph, or between blocks
    for node in root.iter():
        if node is start:
            lines = [""]
            in_paragraph = start.getparent().tag == W + "p"
        elif lines is None:
            continue
        elif node.tag == W + "bookmarkEnd" and node.get(W + "id") == mark:
            break
        elif node.tag == W + "p":
            if in_paragraph:
                lines.append("")
            in_paragraph = True
        elif node.tag == W + "t" and not any(a.tag == W + "del" for a in node.iterancestors()):
            lines[-1] += node.text or ""
        elif node.tag == W + "tab" and node.getparent().tag == W + "r":
            lines[-1] += "\t"
    return "\n".join(lines or [])


def first_cell_paragraph(table):
    return next(table.iter(W + "p"))


# -- where the markers go (Word's own placement) ---------------------------------------------


def test_a_span_from_a_paragraph_through_a_table_ends_after_the_table(box):
    session = box.session()
    d, ids = _made(box, session)
    result = _bookmark(box, session, d, f"{ids['second']}..{ids['grid']}")
    assert result.created == ["bm:Span1"]
    root = _root(session, d)
    start, end = markers(root)
    paragraph, table = element(box, session, d, root, ids["second"]), element(box, session, d, root, ids["grid"])
    assert start.getparent() is paragraph and start.getprevious() is None      # before its text
    assert end.getparent() is table.getparent() and end.getprevious() is table  # beside the table
    assert end.getnext() is element(box, session, d, root, ids["third"])


def test_a_span_starting_on_a_table_starts_in_its_first_cell(box):
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, f"{ids['grid']}..{ids['third']}")
    root = _root(session, d)
    start, end = markers(root)
    cell_paragraph = first_cell_paragraph(element(box, session, d, root, ids["grid"]))
    assert start.getparent() is cell_paragraph and start.getnext().tag == W + "r"
    third = element(box, session, d, root, ids["third"])
    assert end.getparent() is third and end.getprevious() is third.findall(W + "r")[-1]  # after its text


def test_a_table_followed_by_no_text_holds_the_end_after_its_last_row(box):
    """Word: an empty paragraph (or no paragraph) after the table puts the end inside it."""
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, f"{ids['fourth']}..{ids['items']}")
    root = _root(session, d)
    _, end = markers(root)
    table = element(box, session, d, root, ids["items"])
    assert end.getparent() is table and end.getprevious() is table.findall(W + "tr")[-1]


def test_a_table_alone_and_two_tables_with_their_separator(box):
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, ids["grid"])
    root = _root(session, d)
    start, end = markers(root)
    table = element(box, session, d, root, ids["grid"])
    assert start.getparent() is first_cell_paragraph(table) and end.getprevious() is table
    _bookmark(box, session, d, f"{ids['items']}..{ids['next']}", "Pair")
    root = _root(session, d)
    start, end = markers(root, "Pair")
    assert start.getparent() is first_cell_paragraph(element(box, session, d, root, ids["items"]))
    assert end.getprevious() is element(box, session, d, root, ids["next"])
    assert bookmark_text(box, session, d, "Pair") == "Item\nNote\none\nfirst\n\nNext\nGrid\ntwo\nsecond"


def test_a_span_of_paragraphs_marks_their_text_as_a_range_of_it_would(box):
    """Ending on a paragraph: the end after its text (Word's form for a selection to the end of
    its text), so the span writes what p:A@0..p:B@<length> writes."""
    session = box.session()
    d, ids = _made(box, session)
    other = session.open(word_docs.saved(session, d), "other.docx")
    third = dict(word_docs.paragraphs(session, d))[ids["third"]]
    _bookmark(box, session, d, f"{ids['second']}..{ids['third']}")
    _bookmark(box, session, other, f"{ids['second']}@0..{ids['third']}@{len(third)}")
    assert word_docs.part(word_docs.saved(session, d)) == word_docs.part(word_docs.saved(session, other))


def test_a_content_control_is_marked_from_before_it_to_after_it(box):
    session = box.session()
    d, ids = _with_controls(box, session)
    _bookmark(box, session, d, ids["one"])
    root = _root(session, d)
    start, end = markers(root)
    control = element(box, session, d, root, ids["one"])
    assert start.getnext() is control and start.get(W + "displacedByCustomXml") == "next"
    assert end.getprevious() is control and end.get(W + "displacedByCustomXml") is None
    assert bookmark_text(box, session, d, "Span1") == "Control one."


def test_a_content_control_before_a_table_ends_in_the_tables_first_cell(box):
    """Word: the control's range ends where the table starts -- in its first cell, before
    its first paragraph."""
    session = box.session()
    d, ids = _with_controls(box, session)
    _bookmark(box, session, d, ids["two"])
    root = _root(session, d)
    _, end = markers(root)
    cell = next(element(box, session, d, root, ids["table"]).iter(W + "tc"))
    assert end.getparent() is cell and end.getprevious().tag == W + "tcPr"
    assert bookmark_text(box, session, d, "Span1") == "Control two."
    _bookmark(box, session, d, f"{ids['two']}..{ids['table']}", "Both")
    assert bookmark_text(box, session, d, "Both") == "Control two.\nA\nB\n1\n2"


def test_a_bookmark_over_blocks_is_valid_and_reads_back_as_written(box):
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, f"{ids['second']}..{ids['grid']}")
    assert word_docs.validation(box, session, d)["new"] == []
    reopened = session.open(word_docs.saved(session, d), "reopened.docx")
    assert bookmark_text(box, session, reopened, "Span1") == \
        "Second line before the grid.\nCode\nAmount\nK7-1\n12\nK7-2\n30"


# -- the bookmark as an address --------------------------------------------------------------


def test_the_bookmark_addresses_its_blocks(box):
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, f"{ids['second']}..{ids['grid']}")
    markdown = box.dispatch(session, "word_read", {"doc": d, "range": "bm:Span1"}).data
    assert "Second line before the grid." in markdown and "| K7-2 | 30 |" in markdown
    assert "Third line" not in markdown
    found = box.dispatch(session, "find_text", {"doc": d, "text": "K7", "range": "bm:Span1"})
    assert found.ok and len(found.data) == 2, found.to_json()


def test_a_heading_range_is_bookmarked_as_its_blocks(box):
    session = box.session()
    d = written(box, session, "# Part one\n\nBody text.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n# Part two\n\nMore.")
    head = block_ids(box, session, d)[0]
    _bookmark(box, session, d, f"h:{head}", "Part1")
    _, end = markers(_root(session, d), "Part1")
    assert end.getprevious().tag == W + "tbl"           # the range ends on its table
    assert bookmark_text(box, session, d, "Part1") == "Part one\nBody text.\nA\nB\n1\n2"


# -- no-ops, refusals, tracking, undo -----------------------------------------------------------


def test_the_same_span_again_changes_nothing_and_elsewhere_is_refused(box):
    session = box.session()
    d, ids = _made(box, session)
    _bookmark(box, session, d, f"{ids['second']}..{ids['grid']}")
    data = word_docs.saved(session, d)
    again = _bookmark(box, session, d, f"{ids['second']}..{ids['grid']}", "span1")
    assert not again.changed and again.warnings == ["bm:Span1 marks this text already"]
    assert word_docs.saved(session, d) == data
    refused = box.dispatch(session, "word_links", {"doc": d, "action": "add_bookmark", "target": ids["items"],
                                                   "bookmark": "Span1"})
    assert not refused.ok and "already has a bookmark" in refused.error.message


def test_a_span_across_containers_or_reversed_is_refused_with_the_syntax(box):
    session = box.session()
    d, ids = _made(box, session)
    inside = _ok_inspect(box, session, d, ids["grid"])
    for target, said in ((f"{ids['second']}..{inside}", "a span is two block ids"),
                         (f"{ids['third']}..{ids['second']}", "ends before it starts")):
        refused = box.dispatch(session, "word_links", {"doc": d, "action": "add_bookmark", "target": target,
                                                       "bookmark": "Span1"})
        assert not refused.ok and said in refused.error.message, refused.to_json()


def _ok_inspect(box, session, d: str, table: str) -> str:
    """The first paragraph of the table's cell (1, 0)."""
    found = box.dispatch(session, "word_inspect", {"doc": d, "target": table}).data["blocks"][0]
    return found["rows"][1][0]["blocks"][0]["id"]


def test_tracked_or_not_the_same_markers_and_undo_restores_the_bytes(box):
    """Word tracks no bookmark: with tracking on, the markers are written alike, no revision."""
    session = box.session()
    plain, ids = _made(box, session)
    data = word_docs.saved(session, plain)
    tracked = session.open(data, "tracked.docx")
    _bookmark(box, session, plain, f"{ids['grid']}..{ids['third']}")
    assert box.dispatch(session, "word_set_tracking", {"doc": tracked, "on": True, "author": "A"}).ok
    _bookmark(box, session, tracked, f"{ids['grid']}..{ids['third']}")
    assert word_docs.part(word_docs.saved(session, tracked)) == word_docs.part(word_docs.saved(session, plain))
    assert box.dispatch(session, "undo", {"doc": tracked}).ok
    assert word_docs.part(word_docs.saved(session, tracked)) == word_docs.part(data)


# -- the tool ------------------------------------------------------------------------------------


def test_word_links_bookmarks_a_span_a_table_and_a_ref(box):
    session = box.session()
    doc, ids = _made(box, session)
    result = box.dispatch(session, "word_links", {"doc": doc, "action": "add_bookmark",
                                                  "target": f"{ids['second']}..{ids['grid']}", "bookmark": "Span1"})
    assert result.ok and result.summary == "Bookmarked as Span1", result.to_json()
    result = box.dispatch(session, "word_links", {"doc": doc, "action": "add_bookmark", "target": ids["items"],
                                                  "bookmark": "Items"})
    assert result.ok, result.to_json()
    again = box.dispatch(session, "word_links", {"doc": doc, "action": "add_bookmark", "target": ids["items"],
                                                 "bookmark": "items"})
    assert again.ok and again.summary == "No change: bm:Items marks this text already"
    listed = box.dispatch(session, "word_links", {"doc": doc, "action": "list"}).data["bookmarks"]
    assert [b["name"] for b in listed] == ["Span1", "Items"]
    read = box.dispatch(session, "word_read", {"doc": doc, "range": "bm:Span1"})
    assert read.ok and "| K7-2 | 30 |" in read.data and "Second line before the grid." in read.data, read.to_json()


def test_a_hyperlink_over_blocks_is_still_refused_with_the_way_out(box):
    session = box.session()
    doc, ids = _made(box, session)
    result = box.dispatch(session, "word_links", {"doc": doc, "action": "add_hyperlink",
                                                  "target": f"{ids['second']}..{ids['grid']}",
                                                  "url": "https://example.com"})
    assert not result.ok and "is a span of blocks, not text" in result.error.message
    assert "p:A@3..p:B@8" in result.error.message


# -- Word (opt in: pytest -m oracle) ---------------------------------------------------------------


@pytest.mark.oracle
def test_word_resaves_our_markers_where_they_are():
    import re
    import zipfile
    import io

    import oracle

    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as box:
        session = box.session()
        d, ids = _made(box, session)
        for target, name in ((f"{ids['second']}..{ids['grid']}", "Span1"), (f"{ids['fourth']}..{ids['items']}", "Inside"),
                             (f"{ids['grid']}..{ids['third']}", "Other")):
            _bookmark(box, session, d, target, name)
        data = word_docs.saved(session, d)

    def placement(blob: bytes) -> list[str]:
        xml = zipfile.ZipFile(io.BytesIO(blob)).read("word/document.xml").decode()
        found = re.findall(r"<w:bookmark(Start|End)([^>]*)/>(</?w:\w+)?", xml)   # the marker, what follows it
        return [(kind, re.sub(r' w:id="\d+"', "", attributes), after) for kind, attributes, after in found]

    try:
        with oracle.session() as word:
            saved = word.resave(data, name="r49-block-bookmarks")
    except oracle.WordBusy as busy:
        pytest.skip(str(busy))
    assert saved.ok, saved.detail
    assert placement(saved.path.read_bytes()) == placement(data)
