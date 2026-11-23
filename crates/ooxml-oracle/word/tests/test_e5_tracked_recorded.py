"""E5's tracked cases as the tools write them, held to what Word wrote with tracking on
(``probes/e5_probe.py``, ``observations/e5-word.json``, Word 16.106 for Mac): table properties,
merges and splits, the candidate tracked forms of a merge after Word's Accept All and Reject All,
floated drawings, and content controls typed into, tracked or not, and only re-saved.

Compared in e3's canonical form (revision ids, authors and dates aside, Dutch style ids in
English). Where the tools part from Word the difference is a strict xfail with its reason, or,
where the tools part on purpose, a rule the test holds instead."""

from __future__ import annotations

import re

import pytest
from lxml import etree
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import e5_probe  # noqa: E402
from test_e3_recorded import canonical  # noqa: E402
from test_e5_recorded import DRAWING_EDITS, MERGE_EDITS, TABLE_EDITS, _AT_LAYOUT, _OFFSET, _anchor, \
    drawing_ids, plain, table_ids  # noqa: E402

OBSERVED = observed("e5-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"tables-tracked", "merges-tracked", "mergeforms-accept", "mergeforms-reject", "drawings-tracked",
         "controls", "controls-tracked", "controls-saved"}
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
#: A property record and the properties it holds the old form of.
_RECORDS = ("tblPrChange", "tcPrChange", "trPrChange", "tblPrExChange", "tblGridChange", "pPrChange")


def without_empty_records(xml: str) -> str:
    """``xml`` (canonical) without the records Word writes of properties that did not change
    (a record whose old properties are the new ones), and without a row's table exceptions
    that only restate the table's own properties: either way they change nothing."""
    root = etree.fromstring(re.sub(r"^<w:(\w+)", rf'<w:\1 xmlns:w="{W}"', xml, count=1))
    for name in _RECORDS:
        for record in list(root.iter(_W + name)):
            holder = record.getparent()
            new = [etree.tostring(c) for c in holder if c is not record]
            old = [etree.tostring(c) for c in (record[0] if len(record) else [])]
            if new == old:
                holder.remove(record)
    table = root.find(_W + "tblPr")
    if table is not None:
        own = {etree.tostring(c) for c in table}
        for exceptions in list(root.iter(_W + "tblPrEx")):
            if all(etree.tostring(c) in own for c in exceptions):
                exceptions.getparent().remove(exceptions)
    return etree.tostring(root, encoding="unicode").replace(f' xmlns:w="{W}"', "")


def tracked(document: Opened) -> None:
    document.track("Writer")


def by_key(blocks: list[str]) -> dict[str, str]:
    """Each table, canonical, by the key its cells name (``Head r1c1`` -> ``Head``; a table
    whose first row went, by its other cells)."""
    out: dict[str, str] = {}
    for block in blocks:
        found = re.search(r"<w:t>(\w+) r\dc\d", block)
        if block.startswith("<w:tbl>") and found:
            out.setdefault(found.group(1), canonical(block))
    return out


# -- table properties, tracked ---------------------------------------------------------------

#: Where the tools' tracked form differs from Word's, records of nothing aside.
TRACKED_TABLE_DIFFERENCES = {
    "Pct": "Word's tracked percentage width divides the grid at the old and the new column edges (five "
           "columns, each cell spanning two, the old spans in w:tcPrChange); the tools write the new grid "
           "with no record of it",
    "Widths": "as Pct: Word's tracked column width divides the grid at the old and new edges; the tools write "
              "the new grid",
    "Center": "the tools record each row's new w:jc in a w:trPrChange; Word records only the table's "
              "(its rows' w:jc applied with no record, so its Reject All leaves the rows centred)",
    "Right": "as Center",
}


@pytest.mark.parametrize("key", [pytest.param(k, marks=pytest.mark.xfail(reason=TRACKED_TABLE_DIFFERENCES[k],
                                                                           strict=True))
                                 if k in TRACKED_TABLE_DIFFERENCES else k for k in TABLE_EDITS])
def test_a_tracked_table_property_is_words_but_for_records_of_nothing(box, key):
    """``tables-tracked``: Word also records, around a row's or cell's setting, the table's and
    the first row's cells' properties unchanged; those records change nothing and are left
    out on both sides."""
    document = Opened(box, e5_probe.tables_document())
    tracked(document)
    edits = TABLE_EDITS[key]
    for edit in edits if isinstance(edits, list) else [edits]:
        document.call("word_format_table", table=table_ids(document)[key], **edit)
    ours = by_key(document.body())[key]
    assert without_empty_records(ours) == without_empty_records(by_key(OBSERVED["tables-tracked"]["blocks"])[key])


# -- merges, splits, rows and columns, tracked --------------------------------------------------

#: The edits Word makes untracked even with tracking on (merges, a split, a column deleted, a
#: merge's first row deleted).
#: The tools record them, by decision (docx-agent's ROADMAP, E3 and E5: "merges, splits and a
#: column's deletion: untracked"), in forms whose Accept All is Word's result and whose Reject
#: All is the table as it was.
UNTRACKED_IN_WORD = ("MergeAcross", "MergeDown", "MergeRect", "SplitCols", "ColDelPlain", "RowDelFirst")
#: The edits Word tracks, whose tracked forms the tools write as Word does.
TRACKED_AS_WORD = ("RowInMerge",)


@pytest.mark.parametrize("key", UNTRACKED_IN_WORD)
def test_a_tracked_merge_is_accepted_as_word_applies_it_and_rejected_to_the_table(box, key):
    original = by_key(Opened(box, e5_probe.merges_document()).body())[key]
    for review, expected in (("accept", by_key(OBSERVED["merges-tracked"]["blocks"])[key]), ("reject", original)):
        document = Opened(box, e5_probe.merges_document())
        tracked(document)
        document.call("word_edit_table", table=table_ids(document)[key], **MERGE_EDITS[key])
        document.call("word_changes", action=review, all=True)
        assert by_key(document.body())[key] == expected, review


@pytest.mark.parametrize("key", TRACKED_AS_WORD)
def test_a_tracked_row_is_written_as_word_writes_it(box, key):
    document = Opened(box, e5_probe.merges_document())
    tracked(document)
    document.call("word_edit_table", table=table_ids(document)[key], **MERGE_EDITS[key])
    assert by_key(document.body())[key] == by_key(OBSERVED["merges-tracked"]["blocks"])[key]


def test_a_tracked_column_beside_a_span_is_words_with_the_old_grid_recorded(box):
    """``ColRightOfSpan``: Word's tracked column insert is its new cells with their paragraph
    marks inserted; the tools write the same cells and also record the old grid
    (``w:tblGridChange``, each row's ``w:trPrChange``) and every cell's old properties, so that
    Reject All gives the table back (followups-word.json, ``colins-tools-reject``: Word's own
    Reject All of that form). With those records left out, Word's form."""
    document = Opened(box, e5_probe.merges_document())
    tracked(document)
    document.call("word_edit_table", table=table_ids(document)[key := "ColRightOfSpan"], **MERGE_EDITS[key])
    ours = by_key(document.body())[key]
    assert "<w:tblGridChange>" in ours and "<w:trPrChange>" in ours
    ours = re.sub(r"<w:tblGridChange>.*?</w:tblGridChange>", "", without_empty_records(ours))
    ours = re.sub(r"<w:trPr><w:trPrChange><w:trPr><w:gridAfter w:val=\"0\"/></w:trPr></w:trPrChange></w:trPr>", "",
                  ours)
    ours = re.sub(r"<w:tcPrChange><w:tcPr>.*?</w:tcPr></w:tcPrChange>", "", ours)
    assert ours == by_key(OBSERVED["merges-tracked"]["blocks"])[key]


# -- the candidate tracked forms of a merge across ---------------------------------------------

#: The tools' own form is ``Row`` (every cell's old properties and the old grid recorded): Word's
#: Accept All and Reject All of it are the tools'. The other forms are ones the tools do not
#: write; where the tools' review of them differs from Word's, why.
MERGEFORM_DIFFERENCES = {
    ("E3", "reject"): "without the old grid Word's Reject All rebuilds the grid from what is left (a third "
                      "column split, a row skipping two columns, w:tcW auto); the tools give back the cells "
                      "as recorded",
    ("Grid", "reject"): "without every cell's old properties Word's Reject All leaves the cells' w:tcW auto; "
                        "the tools keep 2000 twips",
    ("GridDel", "reject"): "as Grid: the cell that was not recorded gets w:tcW auto from Word",
    ("Legacy", "accept"): "Word's Accept All of a w:hMerge form keeps it as a span; the tools read w:hMerge as "
                          "a horizontal merge and write a w:gridSpan",
    ("Legacy", "reject"): "Word's Reject All of a w:hMerge form rebuilds the grid (five columns); the tools give "
                          "back the three",
}
MERGEFORMS = ("E3", "Grid", "GridDel", "Row", "Legacy")


def _forms(blocks: list[str]) -> dict[str, str]:
    out = {}
    for block in blocks:
        found = re.search(r"<w:t>(\w+) r1c1", block)
        if block.startswith("<w:tbl>") and found:
            out[found.group(1)] = canonical(block)
    return out


@pytest.mark.parametrize("form, review", [
    pytest.param(f, r, marks=[pytest.mark.xfail(reason=MERGEFORM_DIFFERENCES[(f, r)], strict=True)]
                 if (f, r) in MERGEFORM_DIFFERENCES else [], id=f"{f}-{r}")
    for f in MERGEFORMS for r in ("accept", "reject")])
def test_a_tracked_merge_form_is_reviewed_as_word_reviews_it(box, form, review):
    document = Opened(box, e5_probe.mergeforms_document())
    document.call("word_changes", action=review, all=True)
    assert _forms(document.body())[form] == _forms(OBSERVED[f"mergeforms-{review}"]["blocks"])[form]


# -- drawings, tracked -------------------------------------------------------------------------


@pytest.mark.parametrize("key", [*DRAWING_EDITS, "Inline"])
def test_a_drawing_floated_with_tracking_on_is_words_and_untracked(box, key):
    """``drawings-tracked``: Word floats, wraps and inlines a picture with no record, and so do
    the tools."""
    document = Opened(box, e5_probe.drawings_document())
    tracked(document)
    target = drawing_ids(document)[key]
    document.call("word_drawings", action="float", target=target, **DRAWING_EDITS.get(key, {}))
    if key == "Inline":
        document.call("word_drawings", action="inline", target=target)
    block = next(b for b in document.body() if f'descr="Case {key}"' in b)
    word = next(b for b in OBSERVED["drawings-tracked"]["blocks"] if f'descr="Case {key}"' in b)
    assert not re.search(r"<w:(ins|del)\b", block) and not re.search(r"<w:(ins|del)\b", word)
    ours, theirs = _anchor(block), _anchor(word)
    if key in _AT_LAYOUT:
        ours, theirs = _OFFSET.sub("<wp:posOffset/>", ours), _OFFSET.sub("<wp:posOffset/>", theirs)
    if key == "Tight":  # Word's quirk: its through case, the same steps, writes 0
        theirs = theirs.replace('behindDoc="1"', 'behindDoc="0"')
    assert plain(ours) == plain(theirs)


# -- content controls --------------------------------------------------------------------------

#: The text Word typed into each control (``e5_probe.CONTROL_CASES``), as a fill's value: a date
#: control's as its ISO date.
FILLS = {"Plain": "Plain typed", "Rich": "Rich typed", "Combo": "Purple", "Date": "2026-10-05",
         "BoundTyped": "Typed into bound"}


def controls_by_key(blocks: list[str]) -> dict[str, str]:
    """Each control case's paragraph by its key (``Plain: ...``), canonical, with what Word's
    save alone changes left out: the order of ``w:sdtPr``'s children and an empty
    ``w:sdtEndPr``, neither of which Word reads differently."""
    out = {}
    for block in blocks:
        xml = canonical(block)
        found = re.match(r"<w:p>(?:<w:pPr>.*?</w:pPr>)?<w:r><w:t[^>]*>(\w+): ", xml)
        if not found:
            continue
        root = etree.fromstring(re.sub(r"^<w:p", f'<w:p xmlns:w="{W}" xmlns:w14="urn:w14"', xml, count=1))
        for properties in root.iter(_W + "sdtPr"):
            children = sorted(properties, key=lambda c: etree.tostring(c))
            for child in children:
                properties.append(child)
        for end in list(root.iter(_W + "sdtEndPr")):
            if not len(end):
                end.getparent().remove(end)
        out[found.group(1)] = re.sub(r' xmlns(:\w+)?="[^"]*"', "", etree.tostring(root, encoding="unicode"))
    return out


def filled(box, case: str) -> Opened:
    document = Opened(box, e5_probe.controls_document())
    if case.endswith("tracked"):
        tracked(document)
    ids = {c["tag"]: c["id"] for c in document.call("word_controls", action="list").data}
    for key, value in FILLS.items():
        document.call("word_controls", action="fill", control=ids[key], value=value)
    return document


@pytest.mark.parametrize("case", ["controls", "controls-tracked"])
@pytest.mark.parametrize("key", list(FILLS))
def test_a_control_filled_is_what_word_writes_typing_into_it(box, case, key):
    """``controls``, ``controls-tracked``: typing replaces the content's text in its first run
    (tracked: deleted and inserted inside ``w:sdtContent``); a date control's ``w:fullDate``
    follows the date."""
    ours = controls_by_key(filled(box, case).body())[key]
    assert ours == controls_by_key(OBSERVED[case]["blocks"])[key]


@pytest.mark.xfail(reason="a date typed in the control's own picture (d-M-yyyy, as Word takes it) is refused: "
                          "'5-10-2026' is not a date; the ISO date is filled", strict=True)
def test_a_date_control_takes_a_date_in_its_own_picture(box):
    document = Opened(box, e5_probe.controls_document())
    ids = {c["tag"]: c["id"] for c in document.call("word_controls", action="list").data}
    document.call("word_controls", action="fill", control=ids["Date"], value="5-10-2026")


def test_a_drop_down_is_filled_by_its_item_where_word_refuses_typing(box):
    """A deliberate rule: Word refuses typing into a drop-down list (``Drop`` keeps ``Alpha``);
    the tools fill it by choosing the item, as Word's list does."""
    document = Opened(box, e5_probe.controls_document())
    ids = {c["tag"]: c["id"] for c in document.call("word_controls", action="list").data}
    document.call("word_controls", action="fill", control=ids["Drop"], value="Beta")
    assert ">Beta<" in controls_by_key(document.body())["Drop"]
    assert ">Alpha<" in controls_by_key(OBSERVED["controls"]["blocks"])["Drop"]


def test_a_bound_control_filled_writes_its_node_where_words_typing_does_not(box):
    """A deliberate rule (docx-agent's ROADMAP, E5): the tools keep a bound control's custom XML
    node in step with what it shows; Word's own typing left the node as it was."""
    for case in ("controls", "controls-tracked"):
        assert "<when>2026-10-04</when>" in OBSERVED[case]["customXml/item1.xml"]
        assert "<when>Typed into bound</when>" in filled(box, case).part("customXml/item1.xml")


#: A bound control's cache that disagrees with its node, the document only opened and saved.
STALE_CACHE = ("Word shows a bound control's node on opening and saves it in the content ('Bound from XML'); "
               "the tools leave an untouched control's stale cache as it is")


@pytest.mark.parametrize("key", [pytest.param("Bound", marks=pytest.mark.xfail(reason=STALE_CACHE, strict=True)),
                                 *(k for k in e5_probe.CONTROL_CASES if k != "Bound")])
def test_a_control_only_saved_is_words(box, key):
    """``controls-saved``: Word's open and save of the controls probe, no edit."""
    ours = controls_by_key(Opened(box, e5_probe.controls_document()).body())[key]
    assert ours == controls_by_key(OBSERVED["controls-saved"]["blocks"])[key]
