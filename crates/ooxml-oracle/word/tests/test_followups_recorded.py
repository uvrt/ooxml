"""The e1 and e3 cases Phase 1 left open, measured again (``probes/followups_probe.py``,
``observations/followups-word.json``, Word 16.106 for Mac): a picture's size by its density,
tracked bold through Word's own Bold command, Reject All of an inserted column (Word's form and
the tools'), an inserted last cell paragraph rejected, and an untracked join made by setting a
range's content."""

from __future__ import annotations

import re

import pytest
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import e1_probe  # noqa: E402
import e3_probe  # noqa: E402
import followups_probe  # noqa: E402
from test_e3_recorded import TABLE_EDITS, TEXT_EDITS, _fill, by_key, located  # noqa: E402

OBSERVED = observed("followups-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"pictures", "bold-command", "tables-reject", "colins-tools-reject", "cells-reject",
         "insertlast-reject-one", "join-content"}


# -- a picture's size --------------------------------------------------------------------------

def _extents(blocks: list[str]) -> list[dict[str, int]]:
    found = re.findall(r'<wp:extent cx="(\d+)" cy="(\d+)"/>', "".join(blocks))
    return [{"cx": int(cx), "cy": int(cy)} for cx, cy in found]


def _inserted(box, picture: bytes) -> dict[str, int]:
    document = Opened(box, e1_probe.probe_document())
    blob = document.session.add_blob(picture, "picture.png")
    target = document.paragraph_ids()[-1]
    document.call("word_drawings", action="insert_picture", image=blob, at=f"{target}@0", alt_text="Probe")
    extent = re.search(r'<wp:extent cx="(\d+)" cy="(\d+)"/>', document.part())
    return {"cx": int(extent.group(1)), "cy": int(extent.group(2))}


@pytest.mark.parametrize("k", [k for k, (_, dpi) in enumerate(followups_probe.PICTURES) if dpi])
def test_a_picture_stating_its_density_takes_words_size(box, k):
    """``pictures``: 96 x 48 px stated at 72, 96 and 144 dpi, each its own size."""
    _, dpi = followups_probe.PICTURES[k]
    assert _inserted(box, followups_probe.png(dpi)) == _extents(OBSERVED["pictures"]["blocks"])[k]


def test_a_picture_stating_no_density_is_96_dpi_as_windows_word_places_it(box):
    """A deliberate rule: Word 16.106 for Mac places a picture that states no density at 144 dpi
    (609600 EMU for 96 px, measured twice: e1 and ``pictures``), seemingly its screen's Retina
    backing scale; the tools place it at 96 dpi, Windows Word's convention, which the clients'
    Word follows (as "EMF: follow Windows"). Unmeasured on Windows."""
    k = next(k for k, (_, dpi) in enumerate(followups_probe.PICTURES) if dpi is None)
    assert _extents(OBSERVED["pictures"]["blocks"])[k] == {"cx": 609600, "cy": 304800}
    assert _inserted(box, followups_probe.png(None)) == {"cx": 914400, "cy": 457200}


# -- tracked bold, through Word's own command --------------------------------------------------


def test_tracked_bold_is_what_words_bold_command_writes(box):
    """``bold-command``: Word's Bold command (Cmd+B, ``execute key binding``) on text of both
    scripts writes ``w:b`` and ``w:bCs``, as the tools do (e3's AppleScript ``bold`` property
    writes ``w:b`` alone)."""
    assert "[command: Bold]" in "".join(OBSERVED["bold-command"]["blocks"])
    document = Opened(box, e3_probe.text_document())
    paragraph = located(document)["Bold"]
    document.track("Writer")
    for tool, arguments in TEXT_EDITS["Bold"]:
        document.call(tool, **{k: _fill(v, paragraph) for k, v in arguments.items()})
    assert by_key(document.body())["Bold"] == by_key(OBSERVED["bold-command"]["blocks"])["Bold"]


# -- Reject All of an inserted column ----------------------------------------------------------


def _col_ins(box) -> Opened:
    document = Opened(box, e3_probe.table_document())
    document.track("Writer")
    document.call("word_edit_table", table=located(document)["ColIns"], **TABLE_EDITS["ColIns"])
    return document


def test_reject_all_of_the_tools_column_insert_is_words_reject_all_of_it(box):
    """``colins-tools-reject``: the tools record a column insert with the old grid, and Word's
    Reject All of that form removes the column -- the tools' Reject All gives the same table."""
    document = _col_ins(box)
    document.call("word_changes", action="reject", all=True)
    assert by_key(document.body())["ColIns"] == by_key(OBSERVED["colins-tools-reject"]["blocks"])["ColIns"]


def test_words_reject_all_of_its_own_column_insert_keeps_the_cells(box):
    """``tables-reject`` (measured again; e3 saw the same): Word tracks its own column insert as
    the new cells' paragraph marks only, so its Reject All leaves the empty cells. The tools'
    form records the old grid too, so their Reject All (and Word's of their form) does not."""
    word = by_key(OBSERVED["tables-reject"]["blocks"])["ColIns"]
    assert word.count("<w:gridCol ") == 4 and word.count("<w:p/>") == 3
    document = _col_ins(box)
    document.call("word_changes", action="reject", all=True)
    assert by_key(document.body())["ColIns"].count("<w:gridCol ") == 3


# -- an inserted last cell paragraph, rejected -------------------------------------------------


def _insert_last(blocks: list[str]) -> str:
    return next(by_key([b])["InsertLast"] for b in blocks if b.startswith("<w:tbl") and "InsertLast other" in b)


def test_rejecting_an_inserted_last_cell_paragraph_keeps_the_table_as_word_rejecting_it_alone(box):
    """A deliberate rule. ``cells-reject`` (measured again): Word's Reject All of a cell's last
    paragraph, inserted, takes the cell's first paragraph out of the table and the cell with it
    (the grid one column narrower). ``insertlast-reject-one``: Word's reject of that revision
    alone keeps the table, the cell's first paragraph and an empty one. The tools' Reject All
    keeps the table as Word's single reject does, but for the empty paragraph Word leaves where
    the rejected one was (the cell's end mark): the tools take the paragraph out whole."""
    word_all = OBSERVED["cells-reject"]["blocks"]
    assert any(re.match(r"<w:p[ >]", b) and "InsertLast cell." in b for b in word_all)
    document = Opened(box, e3_probe.cells_document())
    document.call("word_changes", action="reject", all=True)
    word = _insert_last(OBSERVED["insertlast-reject-one"]["blocks"])
    assert "<w:p/></w:tc>" in word
    assert _insert_last(document.body()) == word.replace("<w:p/></w:tc>", "</w:tc>", 1)


# -- an untracked join, by content set ---------------------------------------------------------

#: Word's untracked join of ``JoinA``'s ``kappa.``, its mark and ``JoinB `` depends on how it is
#: made: typing a backspace over the selection keeps the mark (e3, ``join-untracked``); setting
#: the range's content to nothing joins and keeps the second paragraph's properties (here); its
#: tracked deletion accepted keeps the first's (e3, ``text-accept``). The tools join and keep
#: the first's. Recorded, not judged: Word disagrees with itself.
JOIN_CONTENT = ("Word's content-set join keeps JoinB's properties; the tools keep JoinA's, as Word's accepted "
                "tracked join")


@pytest.mark.xfail(reason=JOIN_CONTENT, strict=True)
def test_an_untracked_join_by_content_is_words(box):
    document = Opened(box, e3_probe.join_document())
    ids = located(document)
    document.call("word_delete", target=f"{ids['JoinA']}@11..{ids['JoinB']}@6")
    assert by_key(document.body())["JoinA"] == by_key(OBSERVED["join-content"]["blocks"])["JoinA"]
