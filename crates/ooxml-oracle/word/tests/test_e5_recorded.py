"""E5's tables and drawings as the tools write them, held to what Word wrote for the same edits
(``probes/e5_probe.py``, ``observations/e5-word.json``, Word 16.106 for Mac): each probe
document, the probe's edit made through the tools, the edited block compared with Word's."""

from __future__ import annotations

import re

import pytest
from recorded import Opened, observed  # noqa: I001  (puts the probes on the path)

import e5_probe  # noqa: E402

OBSERVED = observed("e5-word.json")
#: The observation's cases these tests compare with (index.py lists why the others are not).
READS = {"tables", "merges", "drawings"}


def plain(xml: str) -> str:
    """A block as compared: ids, rsids, revision authors and dates aside, Word's dropping of
    ``xml:space`` where the text neither starts nor ends with a space, English style ids."""
    xml = re.sub(r"<\?xml[^>]*\?>\s*", "", xml)
    xml = re.sub(r' xmlns(:\w+)?="[^"]*"', "", xml)
    xml = re.sub(r' (w14:paraId|w14:textId|w:rsid\w*|r:id|mc:Ignorable|wp14:editId)="[^"]*"', "", xml)
    xml = re.sub(r' w:(author|date)="[^"]*"| w16du:dateUtc="[^"]*"', "", xml)
    xml = re.sub(r' w:id="\d+"', "", xml)
    xml = re.sub(r'<w:t xml:space="preserve">([^<\s][^<]*[^<\s]|[^<\s])</w:t>', r"<w:t>\1</w:t>", xml)
    return xml.replace('w:val="Tabelraster"', 'w:val="TableGrid"')


def word_blocks(case: str) -> list[str]:
    return [plain(block) for block in OBSERVED[case]["blocks"]]


def tables_by_key(blocks: list[str]) -> dict[str, str]:
    """Each table block by the key its first cell names (``Head r1c1`` -> ``Head``)."""
    out = {}
    for block in blocks:
        if block.startswith("<w:tbl>"):
            found = re.search(r"<w:t>(\w+) r1c1", block)
            if found:
                out[found.group(1)] = block
    return out


def table_ids(document: Opened) -> dict[str, str]:
    """Each table's id by its key (the first cell's text)."""
    out = {}
    for block, text in document.blocks():
        found = re.search(r"(\w+) r1c1", text)
        if block.startswith("t") and found:
            out[found.group(1)] = block
    return out


# -- table properties --------------------------------------------------------------------------

#: Each table case the tools reach: the word_format_table arguments of Word's edit.
TABLE_EDITS = {
    "Head": {"scope": "table", "header_rows": 1},
    "Pct": {"scope": "table", "width_percent": 80},
    "Center": {"scope": "table", "alignment": "center"},
    "Right": {"scope": "table", "alignment": "right"},
    "Exact": {"scope": "row", "row": 0, "height": 30, "height_rule": "exact"},
    "AtLeast": {"scope": "row", "row": 1, "height": 40},
    "CantSplit": {"scope": "row", "row": 0, "cant_split": True},
    "VAlign": [{"scope": "cell", "row": 1, "column": 1, "vertical_alignment": "center"},
               {"scope": "cell", "row": 0, "column": 0, "vertical_alignment": "bottom"}],
    "Widths": {"scope": "column", "column": 1, "width": 150},
}


@pytest.mark.parametrize("key", list(TABLE_EDITS))
def test_table_properties_are_written_as_word_writes_them(box, key):
    document = Opened(box, e5_probe.tables_document())
    table = table_ids(document)[key]
    edits = TABLE_EDITS[key]
    for edit in edits if isinstance(edits, list) else [edits]:
        document.call("word_format_table", table=table, **edit)
    ours = tables_by_key([plain(b) for b in document.body()])[key]
    assert ours == tables_by_key(word_blocks("tables"))[key]


# -- merges and splits -------------------------------------------------------------------------

MERGE_EDITS = {
    "MergeAcross": {"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 0, "col": 1}},
    "MergeDown": {"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 1, "col": 0}},
    "MergeRect": {"action": "merge", "first": {"row": 0, "col": 0}, "last": {"row": 1, "col": 1}},
    "SplitCols": {"action": "split", "first": {"row": 1, "col": 1}, "rows": 1, "columns": 2},
    "ColRightOfSpan": {"action": "insert_column", "index": 0, "side": "after"},
    "RowInMerge": {"action": "insert_row", "index": 1, "side": "after"},
    "RowDelMerge": {"action": "delete_row", "index": 1},
    "RowDelFirst": {"action": "delete_row", "index": 0},
    "RowInSpan": {"action": "insert_row", "index": 0, "side": "after"},
    "ColDelPlain": {"action": "delete_column", "index": 1},
}


#: Where the tools part from Word on purpose (each a rule, with the reason and Word's form).
#: ``RowInSpan``: Word's Insert Rows Below a row whose first cell spans two grid columns
#: copies the span (the new row has two cells, the first over two columns). The tools give
#: the new row the cells of the nearest row without a span, one per column, and say so in a
#: warning: round 57, R125 -- the team's rows that copied a span left ragged tables when
#: values were written into them; a new row is a row for values.
DELIBERATE = {"RowInSpan"}


def _row_for_values(word_table: str) -> str:
    """Word's RowInSpan table with its inserted row (the second) as the tools write it: the
    third row's cells (the nearest without a span), each with an empty paragraph."""
    rows = re.findall(r"<w:tr>.*?</w:tr>", word_table)
    cells = re.findall(r"<w:tc><w:tcPr>.*?</w:tcPr>", rows[2])
    return word_table.replace(rows[1], "<w:tr>" + "".join(c + "<w:p/></w:tc>" for c in cells) + "</w:tr>", 1)


@pytest.mark.parametrize("key", list(MERGE_EDITS))
def test_merges_and_splits_are_written_as_word_writes_them(box, key):
    document = Opened(box, e5_probe.merges_document())
    result = document.call("word_edit_table", table=table_ids(document)[key], **MERGE_EDITS[key])
    ours = [plain(b) for b in document.body()]
    if key in DELIBERATE:
        assert any("the new row has row 1's cells, one per column" in w for w in result.warnings), result.warnings
        assert tables_by_key(ours)[key] == _row_for_values(tables_by_key(word_blocks("merges"))[key])
        return
    if key == "RowDelFirst":  # the first row's text went with it: the table by its second
        assert next(b for b in ours if "RowDelFirst r2c2" in b) == \
            next(b for b in word_blocks("merges") if "RowDelFirst r2c2" in b)
        return
    assert tables_by_key(ours)[key] == tables_by_key(word_blocks("merges"))[key]


def test_split_rows_is_words_but_for_the_heights_word_takes_from_its_layout(box):
    """Splitting a cell down, Word also gives both rows an at-least height of half the row's
    (130 twips here) from its layout, and splitting both ways widens the grid by 33 twips."""
    document = Opened(box, e5_probe.merges_document())
    tables = table_ids(document)
    for key, rows, columns in (("SplitRows", 2, 1), ("SplitBoth", 2, 2)):
        document.call("word_edit_table", table=tables[key], action="split", first={"row": 1, "col": 1},
                      rows=rows, columns=columns)
        ours = tables_by_key([plain(b) for b in document.body()])[key]
        word = tables_by_key(word_blocks("merges"))[key].replace('<w:trPr><w:trHeight w:val="130"/></w:trPr>', "")
        if key == "SplitBoth":
            word = word.replace('<w:gridCol w:w="1033"/>', '<w:gridCol w:w="1000"/>')
        assert ours == word, key


# -- drawings ----------------------------------------------------------------------------------

#: Each drawing case the tools reach: word_drawings' float (or set) arguments of Word's edit.
DRAWING_EDITS = {
    "Float": {},
    "Square": {"wrap": "square"},
    "Tight": {"wrap": "tight"},
    "TopBottom": {"wrap": "top_and_bottom"},
    "Front": {"wrap": "front"},
    "Behind": {"wrap": "behind"},
}
#: The cases whose offsets are where Word's layout put the picture (the tools float it at 0, 0
#: from the column and the paragraph).
_AT_LAYOUT = {"Float", "Square", "Tight", "TopBottom", "Front", "Behind"}
_OFFSET = re.compile(r"<wp:posOffset>-?\d+</wp:posOffset>")


def _anchor(block: str) -> str:
    found = re.search(r"<wp:(anchor|inline) .*?</wp:(anchor|inline)>", block)
    text = re.sub(r"<a:graphic>.*</a:graphic>", "<a:graphic/>", found.group(0))  # the picture: the probe's
    return re.sub(r' relativeHeight="\d+"', "", re.sub(r' wp14:anchorId="[^"]*"', "", text))


def drawing_ids(document: Opened) -> dict[str, str]:
    """Each drawing's id by its case (its alternative text, ``Case <key>``)."""
    listed = document.call("word_drawings", action="list").data
    return {item["alt_text"].split()[-1]: item["id"] for item in listed if item.get("alt_text")}


@pytest.mark.parametrize("key", [*DRAWING_EDITS, "Inline"])
def test_drawings_are_floated_as_word_floats_them(box, key):
    document = Opened(box, e5_probe.drawings_document())
    target = drawing_ids(document)[key]
    document.call("word_drawings", action="float", target=target, **DRAWING_EDITS.get(key, {}))
    if key == "Inline":
        document.call("word_drawings", action="inline", target=target)
    ours = _anchor(next(b for b in document.body() if f'descr="Case {key}"' in b))
    word = _anchor(next(b for b in word_blocks("drawings") if f'descr="Case {key}"' in b))
    if key in _AT_LAYOUT:
        ours, word = _OFFSET.sub("<wp:posOffset/>", ours), _OFFSET.sub("<wp:posOffset/>", word)
    if key == "Tight":  # Word's quirk: its through case, the same steps, writes 0
        word = word.replace('behindDoc="1"', 'behindDoc="0"')
    assert plain(ours) == plain(word)
