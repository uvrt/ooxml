"""E3's tracked edits as the tools write them, held to what Word wrote tracking the same edits,
and to its Accept All and Reject All of them (``probes/e3_probe.py``, ``observations/e3-word.json``,
Word 16.106 for Mac, Dutch interface).  Compared in a canonical form: revision ids, authors and
dates aside, and the runs Word splits (where it typed or measured) joined again."""

from __future__ import annotations

import re

import pytest
from lxml import etree
from recorded import Opened, observed, plain  # noqa: I001  (puts the probes on the path)

import e3_probe  # noqa: E402

OBSERVED = observed("e3-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"text", "text-accept", "text-reject", "tables", "tables-accept", "tables-reject", "cells-accept",
         "cells-reject", "join-untracked"}
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
_NS = " ".join(f'xmlns:{prefix}="{uri}"' for prefix, uri in (
    ("w", W), ("w14", "http://schemas.microsoft.com/office/word/2010/wordml"),
    ("w16du", "http://schemas.microsoft.com/office/word/2023/wordml/word16du"),
    ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"),
    ("wp14", "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"),
    ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
    ("pic", "http://schemas.openxmlformats.org/drawingml/2006/picture"),
    ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships")))


def canonical(xml: str) -> str:
    """A block with revision ids, authors and dates aside, Dutch style ids in English, the
    table look Word adds on saving a table without one left out, and adjacent runs of the same
    properties (and adjacent revisions of the same kind) joined."""
    xml = plain(xml)
    xml = re.sub(r' w:(id|author|date)="[^"]*"| w16du:dateUtc="[^"]*"', "", xml)
    for dutch, english in (("Kop2", "Heading2"), ("Tabelraster", "TableGrid"), ("Verwijzingopmerking", "CommentReference"),
                           ("Tekstopmerking", "CommentText")):
        xml = xml.replace(f'w:val="{dutch}"', f'w:val="{english}"')
    xml = re.sub(r"<w:tblLook [^>]*/>", "", xml)
    known = set(re.findall(r'xmlns:(\w+)=', _NS))
    used = set(re.findall(r"</?(\w+):", xml)) | set(re.findall(r" (\w+):\w+=", xml))
    others = " ".join(f'xmlns:{p}="urn:x-{p}"' for p in sorted(used - known - {"xml"}))
    root = etree.fromstring(re.sub(r"^<(w:\w+)", rf"<\1 {_NS} {others}", xml, count=1))
    for parent in root.iter():
        _join(parent)
    text = etree.tostring(root, encoding="unicode")
    text = re.sub(r' xmlns(:\w+)?="[^"]*"', "", text)
    return re.sub(r'<w:(t|delText) xml:space="preserve">([^<\s][^<]*[^<\s]|[^<\s])</w:\1>', r"<w:\1>\2</w:\1>", text)


def _key(node) -> str | None:
    """What two siblings must share to be joined: a run's properties (text runs only), a
    revision's kind."""
    if node.tag == _W + "r":
        kids = [c for c in node if c.tag != _W + "rPr"]
        if kids and all(c.tag in (_W + "t", _W + "delText") for c in kids) and len({c.tag for c in kids}) == 1:
            props = node.find(_W + "rPr")
            return "r" + kids[0].tag + (etree.tostring(props).decode() if props is not None else "")
    if node.tag in (_W + "ins", _W + "del"):
        return node.tag
    return None


def _join(parent) -> None:
    children = list(parent)
    k = 0
    while k + 1 < len(children):
        a, b = children[k], children[k + 1]
        key = _key(a)
        if key is not None and key == _key(b):
            if a.tag == _W + "r":
                text_a = [c for c in a if c.tag != _W + "rPr"]
                text_b = [c for c in b if c.tag != _W + "rPr"]
                text_a[-1].text = "".join(c.text or "" for c in text_a + text_b)
                for extra in text_a[:-1]:
                    a.remove(extra)
                if text_a[-1].text.strip() != text_a[-1].text:
                    text_a[-1].set("{http://www.w3.org/XML/1998/namespace}space", "preserve")
            else:
                for child in list(b):
                    a.append(child)
                _join(a)
            parent.remove(b)
            children.pop(k + 1)
            continue
        k += 1


def by_key(blocks: list[str]) -> dict[str, str]:
    """Each block by its case key: the first word of its text (a table's: of its first cell)."""
    out: dict[str, str] = {}
    for block in blocks:
        text = "".join(re.findall(r"<w:(?:t|delText)(?: [^>]*)?>([^<]*)<", block))
        found = re.match(r"(\w+)", text)
        if found:
            out.setdefault(found.group(1), canonical(block))
    return out


def word(case: str) -> dict[str, str]:
    return by_key(OBSERVED[case]["blocks"])


def located(document: Opened) -> dict[str, str]:
    """Each paragraph's id by its case key."""
    return {re.match(r"\W*(\w+)", text).group(1): block for block, text in document.blocks()
            if re.match(r"\W*\w+", text)}


# -- text --------------------------------------------------------------------------------------

#: Each text case the tools reach: its calls, ``{p}`` the case's paragraph.
TEXT_EDITS = {
    "Insert": [("word_insert_text", {"at": "{p}@7", "text": "INSERTED "})],
    "Delete": [("word_delete", {"target": "{p}@7:13"})],
    "Own": [("word_insert_text", {"at": "{p}@4", "text": "NEWTEXT "}), ("word_delete", {"target": "{p}@6:9"})],
    "Bold": [("word_format", {"targets": ["{p}@5:8"], "bold": True})],
    "Style": [("word_format", {"targets": ["{p}"], "paragraph_style": "heading 2"})],
    "Align": [("word_format", {"targets": ["{p}"], "alignment": "center"})],
    "Gone": [("word_delete", {"target": "{p}"})],
}


def _fill(value, paragraph: str):
    if isinstance(value, str):
        return value.replace("{p}", paragraph)
    if isinstance(value, list):
        return [_fill(v, paragraph) for v in value]
    return value


#: Where the tools' form differs from Word's.
TEXT_DIFFERENCES = {
    "Bold": "Word's AppleScript bold property (the probe's step) writes w:b alone; the tools write w:b and "
            "w:bCs, as Word's own Bold command does (measured: followups-word.json, bold-command)",
}


@pytest.mark.parametrize("key", [pytest.param(k, marks=pytest.mark.xfail(reason=TEXT_DIFFERENCES[k], strict=True))
                                 if k in TEXT_DIFFERENCES else k for k in TEXT_EDITS])
def test_a_tracked_text_edit_is_words_and_so_are_its_accept_and_reject(box, key):
    for review in (None, "accept", "reject"):
        document = Opened(box, e3_probe.text_document())
        paragraph = located(document)[key]
        document.track("Writer")
        for tool, arguments in TEXT_EDITS[key]:
            document.call(tool, **{k: _fill(v, paragraph) for k, v in arguments.items()})
        if review:
            document.call("word_changes", action=review, all=True)
        case = f"text-{review}" if review else "text"
        assert by_key(document.body()).get(key) == word(case).get(key), case


def test_a_comment_is_written_as_word_writes_one(box):
    """``text``: a comment on a word -- its anchor in the paragraph and its part."""
    document = Opened(box, e3_probe.text_document())
    paragraph = located(document)["Comment"]
    document.call("word_comments", action="add", items=[{"target": f"{paragraph}@8:11", "text": "Probe comment"}],
                  author="Writer")
    ours = by_key(document.body())["Comment"]
    assert re.sub(r"<w:comment\w+/?>", "", ours) == re.sub(r"<w:comment\w+/?>", "", word("text")["Comment"])
    anchors = re.compile(r"<w:comment(?:RangeStart|RangeEnd|Reference)\b")
    assert anchors.findall(ours) == anchors.findall(word("text")["Comment"])
    comment = re.compile(r"<w:comment(?: [^>]*)?>.*?</w:comment>")
    ours_comment = comment.search(canonical(document.part("word/comments.xml")))
    words_comment = comment.search(canonical(OBSERVED["text"]["word/comments.xml"]))
    assert ours_comment and words_comment
    assert re.sub(r' w:initials="[^"]*"', "", ours_comment.group(0)) == \
        re.sub(r' w:initials="[^"]*"', "", words_comment.group(0))


# -- tables ------------------------------------------------------------------------------------

#: Each table case the tools reach: word_edit_table's arguments.
TABLE_EDITS = {
    "RowIns": {"action": "insert_row", "index": 0, "side": "after"},
    "RowDel": {"action": "delete_row", "index": 1},
    "ColIns": {"action": "insert_column", "index": 1, "side": "after"},
    "ColDel": {"action": "delete_column", "index": 1},
    "MergeH": {"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 0, "col": 1}},
    "MergeV": {"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 1, "col": 0}},
}


#: The table edits Word applies untracked even with tracking on (a column deleted, cells
#: merged): the tools record them, by decision (docx-agent's ROADMAP, E3, "Delete a column,
#: merge cells"), in the nearest forms Word accepts, so only their Accept All is Word's result.
UNTRACKED_IN_WORD = {"ColDel", "MergeH", "MergeV"}


#: Where the observation and the tools part.
TABLE_DIFFERENCES = {
    ("ColIns", "reject"): "Word tracks its own column insert as the new cells' marks only, so its Reject All "
                          "keeps the empty cells; the tools also record the old grid, and Word's Reject All of "
                          "that form removes the column as the tools' does (measured: followups-word.json, "
                          "colins-tools-reject)",
}


def _table_cases():
    for key in TABLE_EDITS:
        for review in ("accept",) if key in UNTRACKED_IN_WORD else (None, "accept", "reject"):
            reason = TABLE_DIFFERENCES.get((key, review))
            marks = [pytest.mark.xfail(reason=reason, strict=True)] if reason else []
            yield pytest.param(key, review, marks=marks, id=f"{key}-{review or 'tracked'}")


@pytest.mark.parametrize("key, review", list(_table_cases()))
def test_a_tracked_table_edit_is_words_and_so_are_its_accept_and_reject(box, key, review):
    document = Opened(box, e3_probe.table_document())
    document.track("Writer")
    document.call("word_edit_table", table=located(document)[key], **TABLE_EDITS[key])
    if review:
        document.call("word_changes", action=review, all=True)
    case = f"tables-{review}" if review else "tables"
    assert by_key(document.body())[key] == word(case)[key]


# -- revisions already there -------------------------------------------------------------------


@pytest.mark.parametrize("review", ["accept", "reject"])
def test_accepting_or_rejecting_recorded_cell_revisions_is_words(box, review):
    """``cells-accept``, ``cells-reject``: the cells probe holds another writer's cell
    insertions, deletions and merges; Word's Accept All and Reject All of them."""
    document = Opened(box, e3_probe.cells_document())
    document.call("word_changes", action=review, all=True)
    ours, words = by_key(document.body()), word(f"cells-{review}")
    if review == "reject":
        # Word's Reject All of a cell's last paragraph, inserted, takes the cell's first
        # paragraph out of the table and the cell with it (InsertLast); its reject of that
        # revision alone keeps the table, as the tools' Reject All does -- a deliberate rule
        # (test_followups_recorded.py, insertlast-reject-one).
        ours.pop("InsertLast"), words.pop("InsertLast")
    assert ours == words


# -- an untracked join -------------------------------------------------------------------------

#: ``join-untracked``: each deletion, tracking off, as a ``word_delete`` target (``{A}``/``{B}``
#: the case's two paragraphs).
JOINS = {"Mark": "{Mark}@11..{Marked}@0", "JoinA": "{JoinA}@11..{JoinB}@6"}
#: Word's untracked join depends on how it is made (followups-word.json, join-content): typing a
#: backspace over JoinA's ``kappa.``, mark and ``JoinB `` keeps the mark; recorded, not judged.
JOIN_DIFFERENCES = {
    "JoinA": "Word's backspace over a selection across JoinA's mark keeps the mark (no join); its content-set "
             "deletion joins with JoinB's properties, its accepted tracked deletion with JoinA's -- the tools "
             "join with JoinA's",
}


@pytest.mark.parametrize("key", [pytest.param(k, marks=pytest.mark.xfail(reason=JOIN_DIFFERENCES[k], strict=True))
                                 if k in JOIN_DIFFERENCES else k for k in JOINS])
def test_an_untracked_join_is_words(box, key):
    """``join-untracked``: a paragraph's mark deleted with tracking off joins it to the next
    with the first one's properties (``Mark``)."""
    document = Opened(box, e3_probe.join_document())
    ids = located(document)
    document.call("word_delete", target=JOINS[key].format(**ids))
    assert by_key(document.body())[key] == word("join-untracked")[key]
