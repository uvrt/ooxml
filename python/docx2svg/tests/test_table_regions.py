"""A table style's regions as Word draws them (round 58): measured in Word 16.106 for Mac by
``crates/ooxml-oracle/word/probes/table_regions_probe.py``
(``crates/ooxml-oracle/word/observations/table-regions-word.json``, where docx-agent's tests
compare docx2svg's drawing of the whole probe document with Word's, cell by cell and line by
line).  Here the rules themselves, on made-up tables."""

from __future__ import annotations

from docx2svg.layout import automatic_text_on_cell
from docx2svg.model import Style
from docx2svg.parse.document import cell_conditions, leading_header_rows
from docx2svg.resolve.cascade import banded
from docx2svg.table import _region_borders, _regions
from ooxml_common.xmlutil import parse_xml

ALL_ON = {"firstRow": True, "lastRow": True, "firstColumn": True, "lastColumn": True, "noHBand": False,
          "noVBand": False}
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def style(rows: int | None = 1, columns: int | None = 1) -> Style:
    table = {}
    if rows is not None:
        table["tblStyleRowBandSize"] = rows
    if columns is not None:
        table["tblStyleColBandSize"] = columns
    return Style(style_id="Regions", kind="table", table=table)


def named(conditions, chain) -> tuple:
    return banded(chain, conditions)


def test_the_regions_are_in_words_order_row_band_column_band_columns_rows_corner():
    """Case S/C: a column band's fill over a row band's; the header row over the columns;
    the corner over all. No wholeTable: Word draws that part nowhere (cases W, V)."""
    assert cell_conditions(ALL_ON, 1, 1, 5, 5) == ("hBand:0", "vBand:0")
    assert named(cell_conditions(ALL_ON, 2, 2, 5, 5), [style()]) == ("band2Horz", "band2Vert")
    assert cell_conditions(ALL_ON, 0, 0, 5, 5) == ("firstCol", "firstRow", "nwCell")
    assert cell_conditions(ALL_ON, 4, 4, 5, 5) == ("lastCol", "lastRow", "seCell")
    assert cell_conditions(ALL_ON, 0, 2, 5, 5) == ("vBand:1", "firstRow")


def test_rows_marked_to_repeat_are_all_the_header_row_and_bands_count_after_them():
    """Case H: two rows marked as header rows, both the header row; case G: the header
    option off, they are even bands and the bands count from the row after them."""
    look = dict(ALL_ON, lastRow=False, firstColumn=False, lastColumn=False, noVBand=True)
    assert [cell_conditions(look, r, 0, 6, 3, 2) for r in range(4)] == [
        ("firstRow",), ("firstRow",), ("hBand:0",), ("hBand:1",)]
    off = dict(look, firstRow=False)
    assert [named(cell_conditions(off, r, 0, 5, 2, 2), [style()]) for r in range(4)] == [
        ("band2Horz",), ("band2Horz",), ("band1Horz",), ("band2Horz",)]
    rows = [parse_xml(f'<w:tr xmlns:w="{W}"><w:trPr><w:tblHeader/></w:trPr></w:tr>'),
            parse_xml(f'<w:tr xmlns:w="{W}"><w:trPr><w:tblHeader w:val="1"/></w:trPr></w:tr>'),
            parse_xml(f'<w:tr xmlns:w="{W}"><w:trPr><w:tblHeader w:val="0"/></w:trPr></w:tr>')]
    assert leading_header_rows(rows) == 2


def test_band_sizes_band_rows_and_columns_several_at_a_time_and_none_without_one():
    """Case N: sizes of 2; a style stating no size draws no band (make_table_border_probe)."""
    look = dict(ALL_ON, lastRow=False, lastColumn=False)
    rows = [named(cell_conditions(look, r, 0, 7, 7), [style(2, 2)]) for r in range(1, 7)]
    assert [c[0] for c in rows] == ["band1Horz", "band1Horz", "band2Horz", "band2Horz", "band1Horz", "band1Horz"]
    columns = [named(cell_conditions(look, 1, c, 7, 7), [style(2, 2)])[-1] for c in range(1, 7)]
    assert columns == ["band1Vert", "band1Vert", "band2Vert", "band2Vert", "band1Vert", "band1Vert"]
    assert named(cell_conditions(look, 1, 1, 7, 7), [style(None, 0)]) == ()
    # The nearest style in the chain stating a size decides.
    assert named(("hBand:1",), [style(1), style(2)]) == ("band1Horz",)


def test_a_regions_borders_are_on_its_edges_and_inside_between_its_cells():
    """Cases E, F, Q: a row region's left at its start, right at its end, insideV between its
    cells; a column region's top and bottom at the table's, insideH between its rows."""
    grid = [[("firstRow",)] * 3, [("band1Horz",)] * 3, [("band1Horz",)] * 3, [("band2Horz",)] * 3]
    at = _regions(grid)
    part = {"tcBorders.left": "L", "tcBorders.right": "R", "tcBorders.insideV": "V", "tcBorders.top": "T",
            "tcBorders.bottom": "B", "tcBorders.insideH": "H", "shd": "fill"}
    middle = _region_borders(part, at(1, 1)("band1Horz"))
    assert middle == {"shd": "fill", "tcBorders.left": "V", "tcBorders.right": "V", "tcBorders.top": "T",
                      "tcBorders.bottom": "H"}
    assert _region_borders(part, at(2, 0)("band1Horz"))["tcBorders.left"] == "L"
    assert _region_borders(part, at(2, 2)("band1Horz"))["tcBorders.bottom"] == "B"
    columns = [[("firstCol",), ("band1Vert",)] for _ in range(3)]
    at = _regions(columns)
    assert _region_borders(part, at(1, 0)("firstCol")) == {"shd": "fill", "tcBorders.left": "L",
                                                           "tcBorders.right": "R", "tcBorders.top": "H",
                                                           "tcBorders.bottom": "H"}
    assert _region_borders(part, at(0, 0)("firstCol"))["tcBorders.top"] == "T"
    assert _region_borders({"shd": "x"}, at(0, 0)("firstCol")) == {"shd": "x"}


def test_automatic_text_is_white_on_a_cell_fill_darker_than_word_s_threshold():
    """Case A: Rec. 601 luma below 60 (grey 3B white, 3C black; C00000 white, FF0000 black)."""
    shd = lambda fill: ("clear", "auto", fill)  # noqa: E731
    assert automatic_text_on_cell(shd("3B3B3B")) == "FFFFFF"
    assert automatic_text_on_cell(shd("3C3C3C")) is None
    assert automatic_text_on_cell(shd("C00000")) == "FFFFFF"
    assert automatic_text_on_cell(shd("FF0000")) is None
    assert automatic_text_on_cell(shd("203864")) == "FFFFFF" and automatic_text_on_cell(shd("2F5496")) is None
    assert automatic_text_on_cell(None) is None and automatic_text_on_cell(("nil", "auto", "000000")) is None
