"""A table style's formatting by region (round 58, R129): Word's Table Design regions --
header row, total row, first and last column, banded rows and columns, the four corner
cells -- each a ``w:tblStylePr`` of the style, and the whole table, the style's own
properties.

Measured in Word 16.106 for Mac (``tools/table_regions_probe.py``,
``tests/observations/table-regions-word.json``):

* **Which region wins** where they overlap, property by property, lowest first: the
  banded rows, the banded columns, the first and last column, the header and total row,
  the corner cell.  A column band's fill is drawn over a row band's (ECMA-376 orders them
  the other way), the header row's over the first column's, a corner's over both.
* **The whole table** is the style's own properties (``w:pPr``, ``w:rPr``, ``w:tblPr``,
  ``w:tcPr``): what Word writes when its whole-table shading is set (``w:tcPr/w:shd``), and
  all Word draws -- a ``w:tblStylePr w:type="wholeTable"`` is drawn nowhere.
* **Header rows**: every row at the top marked to repeat as a header row
  (``w:tblHeader``) is the header row region, one at least with the header option on; the
  bands count from the row after them.
* **Bands** are drawn only where the style states a band size (``w:tblStyleRowBandSize``,
  ``w:tblStyleColBandSize``; Word's own styles state 1): a size of 2 bands two rows at a
  time.  Columns are counted by cell, a cell spanning two columns one band.
* **Borders** are the region's: a row region's ``left`` at its rows' start, ``right`` at
  their end, ``inside_v`` between its cells, ``top`` and ``bottom`` above and below the
  region (``inside_h`` between its rows: a band of two, header rows); a column region's
  ``top`` on the table's first row, ``bottom`` on its last, ``inside_h`` between the rows.
* **No middle column**: the regions a style has are these; a column that is neither the
  first, the last, nor a band (every other column, or every n-th with a band size) cannot be
  singled out by a style -- a table's own cell formatting can, and a new table can take
  another's frame (``word_insert_markdown`` ``like`` a table, :mod:`.frames`).

A region shows only where the table's look turns it on (Table Design's options,
``word_format_table`` ``look``): :data:`LOOK_OF`.
"""

from __future__ import annotations

import copy
from typing import TYPE_CHECKING, Any

from ..oxml.xml import Element, append_in_order, make, qn, remove
from . import formatting as _formatting
from .errors import EditError
from .table_format import border as _border, read_border as _read_border

if TYPE_CHECKING:  # pragma: no cover
    from .document import Document

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"

#: The regions as the tools name them (``word_format_table`` ``look``'s words where they are
#: the same place) -> the ``w:tblStylePr`` type; ``whole_table`` is the style's own.
REGIONS: dict[str, str | None] = {
    "whole_table": None,
    "header_row": "firstRow",
    "total_row": "lastRow",
    "first_column": "firstCol",
    "last_column": "lastCol",
    "banded_rows": "band1Horz",
    "even_banded_rows": "band2Horz",
    "banded_columns": "band1Vert",
    "even_banded_columns": "band2Vert",
    "top_left_cell": "nwCell",
    "top_right_cell": "neCell",
    "bottom_left_cell": "swCell",
    "bottom_right_cell": "seCell",
}
_NAMES = {kind: name for name, kind in REGIONS.items() if kind is not None}

#: Other names a region is called by, read as it (rule 1): look's internal name, Word's
#: Modify Style list's ("odd banded rows"), the XML's types.
ALIASES = {"last_row": "total_row", "first_row": "header_row", "odd_banded_rows": "banded_rows",
           "odd_banded_columns": "banded_columns", "odd_rows": "banded_rows", "even_rows": "even_banded_rows",
           "odd_columns": "banded_columns", "even_columns": "even_banded_columns",
           "table": "whole_table", **{kind: name for kind, name in _NAMES.items()},
           "wholeTable": "whole_table"}

#: The look options (``word_format_table`` ``look``) a region shows under.
LOOK_OF: dict[str, tuple[str, ...]] = {
    "header_row": ("header_row",), "total_row": ("total_row",), "first_column": ("first_column",),
    "last_column": ("last_column",), "banded_rows": ("banded_rows",), "even_banded_rows": ("banded_rows",),
    "banded_columns": ("banded_columns",), "even_banded_columns": ("banded_columns",),
    "top_left_cell": ("header_row", "first_column"), "top_right_cell": ("header_row", "last_column"),
    "bottom_left_cell": ("total_row", "first_column"), "bottom_right_cell": ("total_row", "last_column"),
}

#: The order a style's regions are written in (Word's own table styles' order).
_ORDER = ("firstRow", "lastRow", "firstCol", "lastCol", "band1Vert", "band2Vert", "band1Horz", "band2Horz",
          "neCell", "nwCell", "seCell", "swCell")

#: A region's fields: what each writes.
FIELDS = ("shading", "color", "bold", "italic", "size", "alignment", "borders", "band_size", "clear")
SIDES = {"top": "w:top", "bottom": "w:bottom", "left": "w:left", "right": "w:right", "inside_h": "w:insideH",
         "inside_v": "w:insideV"}
BORDER_STYLES = ("none", "single", "double", "thick", "dotted", "dashed")


class RegionError(EditError):
    """A region given wrong: what, and the options."""

    def __init__(self, message: str, options: list[str] | None = None, field: str | None = None) -> None:
        super().__init__(message)
        self.options = options or []
        self.field = field


def region_name(given: str) -> tuple[str, str | None]:
    """``(region, note)``: the region ``given`` names, and a note where it was another name."""
    key = (given or "").strip()
    if key in REGIONS:
        return key, None
    spaced = key.lower().replace(" ", "_").replace("-", "_")
    found = spaced if spaced in REGIONS else ALIASES.get(key) or ALIASES.get(spaced)
    if found is None:
        raise RegionError(f"no table style region {given!r}: the regions are {', '.join(REGIONS)} (a middle "
                          "column is none of them: give the column cell formatting, or copy a table's frame "
                          "with word_insert_markdown like)", list(REGIONS), "regions[].region")
    return found, f"region {given!r} read as {found}"


def _style_part(node: Element, kind: str | None, create: bool) -> Element | None:
    """The style's ``w:tblStylePr`` of ``kind`` (``None``: the style itself)."""
    if kind is None:
        return node
    for part in node.findall(_W + "tblStylePr"):
        if part.get(_W + "type") == kind:
            return part
    if not create:
        return None
    part = make("w:tblStylePr", **{"w:type": kind})
    rank = _ORDER.index(kind) if kind in _ORDER else len(_ORDER)
    later = next((p for p in node.findall(_W + "tblStylePr")
                  if (_ORDER.index(p.get(_W + "type")) if p.get(_W + "type") in _ORDER else len(_ORDER)) > rank),
                 None)
    if later is not None:
        later.addprevious(part)
    else:
        append_in_order(node, part)
    return part


def _child(owner: Element, tag: str, create: bool) -> Element | None:
    node = owner.find(qn(tag))
    if node is None and create:
        node = append_in_order(owner, make(tag))
    return node


def _tidy(owner: Element) -> None:
    for tag in ("w:pPr", "w:rPr", "w:tcPr"):
        node = owner.find(qn(tag))
        if node is not None and not len(node) and not node.attrib:
            remove(node)
    borders = owner.find(f"{qn('w:tcPr')}/{qn('w:tcBorders')}")
    if borders is not None and not len(borders):
        parent = borders.getparent()
        remove(borders)
        if not len(parent):
            remove(parent)


def check_region(region: str, values: dict) -> None:
    """Refuse values no region takes (rule 2: which, and the forms)."""
    unknown = set(values) - set(FIELDS) - {"region"}
    if unknown:
        raise RegionError(f"a region takes {', '.join(FIELDS)}; not {', '.join(sorted(unknown))}", list(FIELDS))
    borders = values.get("borders")
    if borders is not None:
        values["borders"] = borders_by_side(borders)


def borders_by_side(borders) -> dict:
    """A region's ``borders`` as ``{side: style, "color": .., "width": ..}``: given so, or as
    the tools give them -- ``{style, sides (default all), color, width}``."""
    if not isinstance(borders, dict):
        raise RegionError("borders is {style, sides, color, width}: sides of top, bottom, left, right, inside_h, "
                          "inside_v (default all)", field="regions[].borders")
    borders = {k: v for k, v in borders.items() if v not in (None, [], "")}    # a strict decoder's nulls (rule 1)
    if not borders:
        return {}
    out = {k: v for k, v in borders.items() if k in ("color", "width")}
    if "style" in borders or "sides" in borders:
        style = borders.get("style")
        if style is None:
            raise RegionError("borders: give the style for its sides", list(BORDER_STYLES), "regions[].borders.style")
        sides = borders.get("sides") or list(SIDES)
        unknown = [k for k in borders if k not in ("style", "sides", "color", "width")]
        if unknown:
            raise RegionError(f"borders takes style, sides, color and width; not {', '.join(unknown)}",
                              ["style", "sides", "color", "width"], "regions[].borders")
        given = {side: style for side in sides}
    else:
        given = {k: v for k, v in borders.items() if k not in ("color", "width")}
    for side, style in given.items():
        if side not in SIDES:
            raise RegionError(f"no border side {side!r}", list(SIDES), "regions[].borders.sides")
        if style not in BORDER_STYLES:
            raise RegionError(f"border {side}: {style!r} is no style", list(BORDER_STYLES), "regions[].borders.style")
    color = out.get("color")
    if color is not None and not (isinstance(color, str) and (color.lower() == "auto" or _formatting._HEX.fullmatch(
            color.strip()))):
        raise RegionError("border color is RRGGBB or auto", field="regions[].borders.color")
    width = out.get("width")
    if width is not None and not (isinstance(width, (int, float)) and 0.25 <= width <= 6):
        raise RegionError("border width is in points, 0.25 to 6", field="regions[].borders.width")
    return {**given, **out}


def write_region(document: "Document", node: Element, region: str, values: dict) -> list[str]:
    """Write ``values`` (a region's fields) for ``region`` on the table style ``node``;
    returns notes.  ``None`` or a missing field leaves that property as it is; ``shading``
    ``"none"`` removes the region's fill, a border side ``"none"`` states no border there
    (over a lower region's); ``clear`` first removes what the region states."""
    notes: list[str] = []
    kind = REGIONS[region]
    if values.get("clear"):
        if kind is None:
            for tag in ("w:pPr", "w:rPr"):
                found = node.find(qn(tag))
                if found is not None:
                    remove(found)
            cell = node.find(qn("w:tcPr"))
            if cell is not None:
                remove(cell)
            table = node.find(qn("w:tblPr"))
            if table is not None:
                for tag in ("w:tblBorders", "w:shd"):
                    found = table.find(qn(tag))
                    if found is not None:
                        remove(found)
        else:
            found = _style_part(node, kind, create=False)
            if found is not None:
                remove(found)
    given = {k: v for k, v in values.items() if k not in ("region", "clear") and v is not None}
    if not given:
        return notes
    part = _style_part(node, kind, create=True)
    run = {k: given[k] for k in ("color", "bold", "italic", "size") if k in given}
    try:
        if run:
            _formatting.write_run(part, run, document._theme_color)
        if "alignment" in given:
            _formatting.write_paragraph(part, {"alignment": given["alignment"]})
        if "shading" in given:
            fill = given["shading"]
            attributes = None if isinstance(fill, str) and fill.strip().lower() == "none" else \
                _formatting.shading_attributes(fill, document._theme_color)
            cell = _child(part, "w:tcPr", attributes is not None)
            if cell is not None:
                _formatting._set_child(cell, "w:shd", attributes)
    except _formatting.FormattingError as error:
        raise RegionError(f"{region}: {error}") from None
    if given.get("borders"):
        _write_borders(part, kind, given["borders"])
    if "band_size" in given:
        size = given["band_size"]
        band = {"banded_rows": "w:tblStyleRowBandSize", "even_banded_rows": "w:tblStyleRowBandSize",
                "banded_columns": "w:tblStyleColBandSize", "even_banded_columns": "w:tblStyleColBandSize"}.get(region)
        if band is None:
            notes.append(f"band_size is the banded regions': left out of {region}")
        else:
            table = _child(node, "w:tblPr", True)
            _formatting._set_child(table, band, {"w:val": str(int(size))})
    _tidy(part)
    if kind is not None and part.getparent() is not None and not len(part):
        remove(part)
    return notes


def _write_borders(part: Element, kind: str | None, borders: dict) -> None:
    """A region's borders: its ``w:tcBorders`` (the whole table's ``w:tblBorders``), each
    side given replaced -- ``"none"`` stated as no border (``nil``)."""
    color = borders.get("color")
    width = borders.get("width")
    if kind is None:
        holder = _child(_child(part, "w:tblPr", True), "w:tblBorders", True)
    else:
        holder = _child(_child(part, "w:tcPr", True), "w:tcBorders", True)
    for side, tag in SIDES.items():
        style = borders.get(side)
        if style is None:
            continue
        # As word_format_table writes its borders (thick: Word's "thick" line), 0.5 pt
        # unless a width is given.
        spec: dict[str, Any] = {"style": "nil" if style == "none" else style}
        if style != "none":
            spec["size"] = width if width is not None else 0.5
            spec["color"] = color.upper() if isinstance(color, str) and color.lower() != "auto" else "auto"
        old = holder.find(qn(tag))
        new = _border(tag, spec)
        if old is not None:
            old.addprevious(new)
            remove(old)
        else:
            append_in_order(holder, new)


def read_regions(node: Element) -> dict[str, dict]:
    """A table style's regions as it states them: ``{region: {field: value}}`` -- the whole
    table from its own properties -- with ``band_size`` on the banded ones where stated."""
    out: dict[str, dict] = {}
    whole = _read_part(node, None)
    if whole:
        out["whole_table"] = whole
    table = node.find(qn("w:tblPr"))
    sizes = {}
    for tag, regions in (("w:tblStyleRowBandSize", ("banded_rows", "even_banded_rows")),
                         ("w:tblStyleColBandSize", ("banded_columns", "even_banded_columns"))):
        found = table.find(qn(tag)) if table is not None else None
        if found is not None and found.get(qn("w:val")) is not None:
            for region in regions:
                sizes[region] = int(found.get(qn("w:val")))
    for part in node.findall(_W + "tblStylePr"):
        kind = part.get(_W + "type")
        name = _NAMES.get(kind)
        values = _read_part(part, kind)
        if name is None:
            if kind == "wholeTable" and values:
                out["whole_table_part"] = {**values, "note": "w:tblStylePr wholeTable: Word draws it nowhere"}
            continue
        if values:
            out[name] = values
    for region, size in sizes.items():
        if region in out:
            out[region]["band_size"] = size
    return out


def _read_part(part: Element, kind: str | None) -> dict:
    values: dict[str, Any] = {}
    run = part.find(qn("w:rPr"))
    for name in ("color", "bold", "italic", "size"):
        value = _formatting.read_run(run, name)
        if value is not None:
            values[name] = value
    alignment = _formatting.read_paragraph(part.find(qn("w:pPr")), "alignment")
    if alignment is not None:
        values["alignment"] = alignment
    cell = part.find(qn("w:tcPr"))
    shading = _formatting.read_shading(cell)
    if shading is not None:
        values["shading"] = shading
    holder = (part.find(f"{qn('w:tblPr')}/{qn('w:tblBorders')}") if kind is None
              else part.find(f"{qn('w:tcPr')}/{qn('w:tcBorders')}"))
    if holder is not None:
        borders = {}
        for side, tag in SIDES.items():
            found = _read_border(holder.find(qn(tag)))
            if found is not None:
                borders[side] = _border_words(found)
        if borders:
            values["borders"] = borders
    return values


def _border_words(found: dict) -> str:
    """A border as a region reports it: ``none``, or ``style width pt colour``."""
    style = found.get("style")
    if style in (None, "nil", "none"):
        return "none"
    out = f"{style} {found.get('size', 0.5):g} pt"
    color = found.get("theme_color") or found.get("color")
    return out + (f" {color}" if color and color != "auto" else "")


def ensure_band_sizes(document: "Document", node: Element, regions: set[str]) -> list[str]:
    """A style with banded regions draws its bands only where its chain states a band
    size (measured): write 1 on the style where none is stated.  Returns notes."""
    notes = []
    wanted = {"w:tblStyleRowBandSize": {"banded_rows", "even_banded_rows"},
              "w:tblStyleColBandSize": {"banded_columns", "even_banded_columns"}}
    for tag, names in wanted.items():
        if not (regions & names):
            continue
        current: Element | None = node
        seen = set()
        stated = False
        while current is not None and id(current) not in seen:
            seen.add(id(current))
            table = current.find(qn("w:tblPr"))
            if table is not None and table.find(qn(tag)) is not None:
                stated = True
                break
            based = current.find(qn("w:basedOn"))
            current = document.styles._node(based.get(qn("w:val"))) if based is not None else None
        if not stated:
            _formatting._set_child(_child(node, "w:tblPr", True), tag, {"w:val": "1"})
            notes.append(f"{'rows' if 'Row' in tag else 'columns'} banded one at a time (band_size 1): a style "
                         "states a band size for Word to draw its bands")
    return notes


def needed_looks(node_regions: dict) -> list[str]:
    """The look options (``word_format_table`` ``look``) the regions given need, in order."""
    out: list[str] = []
    for region in node_regions:
        for option in LOOK_OF.get(region, ()):
            if option not in out:
                out.append(option)
    return out


def snapshot(node: Element) -> bytes:
    """The style's bytes, to tell whether a change changed anything (rule 3)."""
    from lxml import etree

    return etree.tostring(copy.deepcopy(node))
