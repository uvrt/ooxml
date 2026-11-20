"""The object tools' cases for ``word_edits_diff.py --objects`` (lane P4c): the tools
called through docx-agent's toolbox on a document as opened, compared with the Rust core's
(``word-edits``, ``op: "objects"``) as the structure tools' cases are
(:func:`word_tool_reference.python_case`: the reply, every changed part, the part order,
``touched``, the validation delta, the problems and the next ids).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import os
import struct
import zlib

from word_structure_cases import ToolCases, fonts_of, tracked

CASES = ToolCases()


def call(tool: str, **arguments) -> dict:
    return {"op": "objects", "tool": tool, "arguments": {"doc": "d1", **arguments}, "tracked": None}


def _tables(document) -> list[str]:
    return [b["id"] for b in document.blocks() if b["id"] and b["kind"] == "table"]


def _table_scope(table: str) -> list[dict]:
    def fmt(**arguments) -> dict:
        return call("word_format_table", table=table, scope="table", **arguments)

    return [fmt(style="Table Grid"), fmt(style="No Such Table Style"), fmt(style="Normal"),
            fmt(width=300), fmt(width_percent=80), fmt(width_percent=33.33333333), fmt(width=400, width_percent=50),
            fmt(alignment="center"), fmt(alignment="right"), fmt(alignment="left"),
            fmt(shading="D9E2F3"), fmt(shading="none"), fmt(borders="single"), fmt(borders="none"),
            fmt(borders="double", shading="FFF2CC"), fmt(header_rows=1), fmt(header_rows=0), fmt(header_rows=99),
            fmt(look=["header_row", "banded_rows"]), fmt(look=[]), fmt(look=["total_row", "last_column"]),
            fmt(style="Table Grid", look=["first_column"]), fmt(cant_split=True), fmt(cant_split=False),
            fmt(width_of="body"), fmt(width_of="body", width_percent=80), fmt(width_of="body", alignment="center"),
            fmt(width_of="body", width_percent=50, alignment="right"), fmt(width_of="body", width_percent=0.01),
            fmt(width_of="body", style="Table Grid", borders="single", cant_split=True),
            fmt(width_of="window"), fmt(width_of="window", width=100), fmt(keep_together=True),
            fmt(keep_together=False, keep_with_next=True), fmt(), fmt(style="Table Grid", width=250,
                                                                     alignment="center", header_rows=1)]


def _part_scopes(table: str) -> list[dict]:
    def fmt(scope: str, **arguments) -> dict:
        return call("word_format_table", table=table, scope=scope, **arguments)

    return [fmt("row", row=0, height=20, height_rule="exact"), fmt("row", row=1, height=15),
            fmt("row", row=0, cant_split=True), fmt("row", row=0, shading="E2EFDA"),
            fmt("row", row=1, borders="single", vertical_alignment="center"), fmt("row", row=99, height=12),
            fmt("row", height=12), fmt("row", row=0, header_rows=1), fmt("row", row=0, width=50),
            fmt("column", column=0, width=100), fmt("column", column=1, shading="FCE4D6"),
            fmt("column", column=0, borders="dashed"), fmt("column", column=99, width=40),
            fmt("column", column=0, vertical_alignment="bottom", width=80),
            fmt("cell", row=0, column=0, shading="FFFF00"), fmt("cell", row=0, column=1, borders="thick"),
            fmt("cell", row=1, column=1, width=50, vertical_alignment="bottom"),
            fmt("cell", row=0, column=0, borders="none", shading="none"),
            fmt("cell", row=9, column=9, shading="FFFF00"), fmt("cell", row=0, shading="FFFF00"),
            fmt("row", row=0, look=["header_row"]), fmt("cell", row=0, column=0, keep_together=True),
            fmt("column", column=0, cant_split=True), fmt("cell", row=0, column=0)]


@CASES
def format_table_cases(document) -> list[dict]:
    """``word_format_table``: a table's style, width, alignment, shading, borders, header
    rows, look and breaking; a row's height and breaking; a column's width; cells of a row,
    column or one cell formatted; what is not ported (layout); refusals -- untracked and
    tracked."""
    out = []
    for table in _tables(document)[:3]:
        out += _table_scope(table) + _part_scopes(table)
    paragraphs = [b["id"] for b in document.blocks() if b["id"] and b["kind"] == "paragraph"]
    out += [call("word_format_table", table="t:00000000", scope="table", shading="FFFFFF")]
    if paragraphs:
        out.append(call("word_format_table", table=paragraphs[0], scope="table", shading="FFFFFF"))
    out = _laid(document, out)
    return out + [tracked(case) for case in out]


def _laid(document, out: list[dict]) -> list[dict]:
    """``out`` with the fonts the layout reads on each case keeping a table together (its
    taller-than-a-page warning reads the layout)."""
    if not any(case["arguments"].get("keep_together") for case in out):
        return out
    fonts = fonts_of(document)
    return [{**case, "fonts": fonts} if case["arguments"].get("keep_together") else case for case in out]


@CASES
def table_keep_cases(document) -> list[dict]:
    """``word_format_table``'s keeps: ``keep_together`` and ``keep_with_next`` on and off,
    alone and together, with a row's breaking or a table's shading beside them, on each of
    the first eight tables -- untracked and tracked; ``keep_together`` on with the fonts the
    layout reads (a table taller than a page is said so)."""
    def fmt(table: str, **arguments) -> dict:
        return call("word_format_table", table=table, scope="table", **arguments)

    out = []
    for table in _tables(document)[:8]:
        out += [fmt(table, keep_together=False), fmt(table, keep_with_next=True), fmt(table, keep_with_next=False),
                fmt(table, keep_together=False, keep_with_next=False), fmt(table, keep_together=True),
                fmt(table, keep_together=True, keep_with_next=True),
                fmt(table, keep_together=True, keep_with_next=False),
                fmt(table, keep_with_next=False, cant_split=True), fmt(table, keep_together=False, shading="FFF2CC")]
    out = _laid(document, out)
    return out + [tracked(case) for case in out]


def _rows(table: str) -> list[dict]:
    def edit(action: str, **arguments) -> dict:
        return call("word_edit_table", table=table, action=action, **arguments)

    return [edit("insert_row"), edit("insert_row", index=0), edit("insert_row", index=0, side="before"),
            edit("insert_row", index=1, side="before"), edit("insert_row", index=99), edit("delete_row", index=0),
            edit("delete_row", index=1), edit("delete_row"), edit("delete_row", index=99),
            edit("insert_row", key="Region"), edit("insert_row", **{"from": "d:nothing"}),
            edit("delete_row", index=0, header=["A"]), edit("insert_row", grow=True),
            edit("insert_row", key="", remove=False, match=[]), edit("copy_row"), edit("copy_row", index=0),
            edit("copy_row", index=0, side="before"), edit("copy_row", index=1), edit("copy_row", index=99)]


def _columns(table: str) -> list[dict]:
    def edit(action: str, **arguments) -> dict:
        return call("word_edit_table", table=table, action=action, **arguments)

    return [edit("delete_column", index=0), edit("delete_column", index=1), edit("delete_column"),
            edit("delete_column", index=99), edit("insert_column"), edit("insert_column", index=0),
            edit("insert_column", index=0, side="before"), edit("insert_column", index=1, side="before"),
            edit("insert_column", index=99)]


def _cells(table: str) -> list[dict]:
    def edit(action: str, **arguments) -> dict:
        return call("word_edit_table", table=table, action=action, **arguments)

    def at(row: int, col: int) -> dict:
        return {"row": row, "col": col}

    return [edit("merge", first=at(0, 0), last=at(0, 1)), edit("merge", first=at(0, 0), last=at(1, 0)),
            edit("merge", first=at(0, 0), last=at(1, 1)), edit("merge", first=at(1, 0), last=at(2, 2)),
            edit("merge", first=at(0, 1), last=at(0, 99)), edit("merge", first=at(0, 0), last=at(0, 0)),
            edit("merge", first=at(1, 1), last=at(0, 0)), edit("merge", first=at(0, 0)),
            edit("merge", first=at(0, 0), last=at(99, 0)), edit("split", first=at(0, 0)),
            edit("split", first=at(0, 0), columns=3), edit("split", first=at(0, 1), rows=2),
            edit("split", first=at(1, 0), rows=3, columns=2), edit("split", first=at(0, 0), rows=1, columns=1),
            edit("split", first=at(99, 0)), edit("split", first=at(0, 99), rows=2), edit("split"),
            edit("split", first=at(0, 0), columns=0)]


def _set_cells(document, table: str) -> list[dict]:
    def edit(cells, **arguments) -> dict:
        return call("word_edit_table", table=table, action="set_cells", cells=cells, **arguments)

    def text(row: int, col: int) -> str:
        try:
            return document.table(table).cell(row, col).text
        except (IndexError, KeyError):
            return "none"

    def cell(row: int, col: int, value: str = "Alpha") -> dict:
        return {"row": row, "col": col, "text": value}

    return [edit([cell(0, 0)]), edit([cell(1, 1, "One\nTwo")]), edit([cell(0, 0, text(0, 0))]),
            edit([cell(0, 1, "B"), cell(1, 0, "C"), cell(0, 1, "D")]), edit([cell(99, 0)]), edit([cell(0, 99)]),
            edit([{"row_label": text(1, 0), "col": 1, "text": "By row"}]),
            edit([{"row": 1, "col_label": text(0, 1), "text": "By column"}]),
            edit([{"row_label": text(1, 0), "col_label": text(0, 0), "text": "Both"}]),
            edit([cell(1, 0, "Lbl"), {"row_label": "Lbl", "col": 1, "text": "Found"}]),
            edit([{"row_label": "No such row", "col": 0, "text": "x"}]), edit([{"col": 0, "text": "x"}]),
            edit([{"row": 0, "text": "x"}]), edit([{"row": 0, "col": 0}]), edit([cell(0, 0), cell(0, 1, "Two")]),
            edit([{"col": 1, "rows": "*", "text": "Yes"}]), edit([{"col": 0, "rows": "0..1", "text": "Top"}]),
            edit([{"col": 1, "rows": "1", "text": "Yes"}]), edit([{"col": 1, "rows": "..1", "text": "No"}]),
            edit([{"col": 1, "rows": "5..2", "text": "x"}]), edit([{"col": 1, "rows": "0..99", "text": "x"}]),
            edit([{"col": 1, "rows": "x", "text": "x"}]), edit([{"row": 1, "col": 1, "rows": "*", "text": "x"}]),
            edit([{"rows": "*", "text": "x"}]), edit([{"col": 99, "rows": "*", "text": "x"}]),
            edit([{"col_label": text(0, 1), "rows": "*", "text": "Label fill"}]),
            edit([{"col": 0, "rows": "1..", "text": "x"}, cell(0, 0, "Head")]),
            edit([{**cell(0, 0, "Empty fields"), "from": "", "rows": "", "row_label": ""}]), edit(None), edit([]),
            edit([cell(0, 0, "{{d:nothing/#1/a}}")]), edit([{"row": 0, "col": 0, "from": "d:nothing/#1/a"}])]


_HELD = ("{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}docPr",
         "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}sdtPr")


def _held_rows(document, most: int = 4) -> list[dict]:
    """Rows holding a drawing or a content control, copied below and above (their drawing
    numbers and control ids given as docx-agent's importer gives them): at most ``most``."""
    out: list[dict] = []
    for table in _tables(document):
        try:
            _part, entry = document._resolve(table)
        except KeyError:
            continue
        for index, (row, *_rest) in enumerate(getattr(entry, "rows", ())):
            if len(out) >= 2 * most:
                return out
            if any(True for _ in row.iter(*_HELD)):
                for side in ("after", "before"):
                    out.append(call("word_edit_table", table=table, action="copy_row", index=index, side=side))
    return out


@CASES
def edit_table_cases(document) -> list[dict]:
    """``word_edit_table``: rows inserted (after, before, at the top, past the end) and
    deleted (the first, a middle one, none named, past the end); columns deleted (the first,
    the second, none named, past the end), each with the file's text it removes; columns
    inserted (after the last, after and before the first, before the second, past the end);
    rows copied (the last, the first below and above it, the second, past the end, rows
    holding a drawing or a content control); cells
    merged (across, down, both, a rectangle grown, past the grid, already one, reversed, no
    last) and split (by default, across, down, both, into one, past the table, no first);
    cells set (by place, in lines, as they are, by labels, a label an earlier cell wrote,
    past the table, missing a place or text, one merged cell given two values, a column's
    rows filled and the ranges refused, references left to the host);
    arguments
    of another action refused; strict mode's empty values taken as none -- untracked and
    tracked."""
    out = []
    for table in _tables(document)[:3]:
        out += _rows(table) + _columns(table) + _cells(table) + _set_cells(document, table)
    out += _held_rows(document)
    out += [call("word_edit_table", table="t:00000000", action="insert_row")]
    return out + [tracked(case) for case in out]


def _control_values(kind: str) -> list[str]:
    """Values a control of ``kind`` is filled with: each kind's own, and ones it refuses."""
    if kind == "checkbox":
        return ["true", "no", "Checked"]
    if kind == "date":
        return ["2026-03-05", "2024-02-29T10:00", "2026-02-30", "not a date"]
    if kind in ("drop-down", "combo-box"):
        return ["Two", "b", "Something else"]
    return ["Filled", "", "Line one\nLine two", "Tab\there"]


@CASES
def controls_cases(document) -> list[dict]:
    """``word_controls``: the controls listed; one of each kind inserted at a position, a
    range, inside a control and after text found once (with a title and a tag; a list
    without items, a position past the text, none or both of at and find refused); each
    control filled with its kind's values (several lines, a choice by text or value, a
    date in its picture, a check box's state; refusals) and removed; a control that is not
    there -- untracked and tracked."""
    def edit(action: str, **arguments) -> dict:
        return call("word_controls", action=action, **arguments)

    out = [edit("list")]
    paragraphs = [b["id"] for b in document.blocks() if b["id"] and b["kind"] == "paragraph"]
    if paragraphs:
        first = paragraphs[0]
        out += [edit("insert", at=f"{first}@0"), edit("insert", at=f"{first}@0:2", type="rich-text"),
                edit("insert", at=first, type="drop-down", items=["One", "Two"], title="Pick", tag="pick"),
                edit("insert", at=f"{first}@0", type="combo-box", items=["a", "b"]),
                edit("insert", at=f"{first}@0", type="date"), edit("insert", at=f"{first}@0", type="checkbox"),
                edit("insert", at=f"{first}@0", type="drop-down"),
                edit("insert", at=f"{first}@9999", tag="past the text"),
                edit("insert"), edit("insert", at=first, find="x")]
        words = document.paragraph(first).text.split()
        if words:
            out.append(edit("insert", find=words[0], type="text", title="After"))
    for control in document.content_controls()[:16]:
        out += [edit("fill", control=control.id, value=value) for value in _control_values(control.kind)]
        out += [edit("remove", control=control.id), edit("insert", at=f"{control.id}")]
    out += [edit("fill", control="cc:999999", value="x"), edit("remove", control="cc:999999"), edit("fill"),
            edit("remove"), edit("fill", control="cc:999999")]
    return out + [tracked(case) for case in out]


def cases(document) -> list[dict]:
    """Every object case for ``document``."""
    found = CASES.of(document)
    wanted = os.environ.get("OBJECT_TOOLS")       # a comma list, to run some tools only
    found = [c for c in found if not wanted or c["tool"] in wanted.split(",")]
    actions = os.environ.get("OBJECT_ACTIONS")    # a comma list of actions (word_edit_table's)
    found = [c for c in found if not actions or c["arguments"].get("action") in actions.split(",")]
    every = int(os.environ.get("OBJECTS_EVERY") or 0)      # every n-th case, across the scopes
    if every:
        found = found[::every]
    limit = int(os.environ.get("OBJECTS_PER_TOOL") or 0)   # the first n of each tool's
    if limit:
        taken: dict[str, int] = {}
        kept = []
        for case in found:
            key = case["tool"] + ("+tracked" if case.get("tracked") else "")
            if taken.get(key, 0) < limit:
                taken[key] = taken.get(key, 0) + 1
                kept.append(case)
        found = kept
    return found


def _drawing_edits(target: str, inline: bool) -> list[dict]:
    """A drawing moved, resized, floated, put in the line and set (each setting, and none)."""
    def edit(action: str, **arguments) -> dict:
        return call("word_drawings", action=action, target=target, **arguments)

    out = [edit("resize", width=120), edit("resize", height=40), edit("resize", width=90, height=30),
           edit("set", alt_text="A drawing for the oracle"), edit("set", alt_text=""), edit("set")]
    if inline:
        out += [edit("move", x=10, y=20), edit("set", wrap="square"), edit("inline"),
                edit("float"), edit("float", wrap="tight", x=36, y=-12), edit("float", wrap="behind", align="center"),
                edit("float", wrap="top_and_bottom", against="page", align="right", y=50),
                edit("float", wrap="front", against="margin", x=-20)]
    else:
        out += [edit("move", x=10, y=20), edit("move", x=-30.5, y=0), edit("move", x=10),
                edit("set", wrap="square"), edit("set", wrap="tight", z_order="front"),
                edit("set", wrap="behind", z_order="back"), edit("set", wrap="top_and_bottom", x=5, y=6),
                edit("set", z_order="forward"), edit("set", z_order="backward"), edit("set", wrap="front"),
                edit("float"), edit("float", wrap="square", align="center"), edit("inline")]
    return out


@CASES
def drawings_cases(document) -> list[dict]:
    """``word_drawings``: the drawings listed; each drawing (a few) resized, moved, floated,
    put in the line and set (wrapping, order, place, alt text; nothing to set refused); a
    text box and shapes inserted at a position, a range and after text found once (sizes,
    places, wrapping, text in lines; an unknown preset, a place in a text box, none or both
    of at and find refused); a drawing that is not there -- untracked and tracked."""
    def edit(action: str, **arguments) -> dict:
        return call("word_drawings", action=action, **arguments)

    out = [edit("list")]
    found = document._all_drawings()
    for _part, identifier, frame in found[:5]:
        out += _drawing_edits(identifier, frame.tag.endswith("}inline"))
    paragraphs = [b["id"] for b in document.blocks() if b["id"] and b["kind"] == "paragraph"]
    if paragraphs:
        first = paragraphs[0]
        out += [edit("insert_text_box", at=f"{first}@0", text="Inside the box\nA second line"),
                edit("insert_text_box", at=first, width=200, height=50, x=12, y=-6, wrap="tight"),
                edit("insert_shape", at=f"{first}@0", preset="ellipse", text="Round"),
                edit("insert_shape", at=f"{first}@0", preset="roundRect", wrap="behind", width=30),
                edit("insert_shape", at=f"{first}@0", preset="triangle", wrap="top_and_bottom", text=""),
                edit("insert_shape", at=f"{first}@0", wrap="square", ref="box"),
                edit("insert_shape", at=f"{first}@0", preset="no-such-shape"),
                edit("insert_text_box", at=f"{first}@9999"), edit("insert_shape"),
                edit("insert_shape", at=first, find="x")]
        words = document.paragraph(first).text.split()
        if words:
            out.append(edit("insert_text_box", find=words[0], text="After the word"))
    boxes = [identifier for _part, identifier, frame in found
             if frame.find(".//{http://schemas.openxmlformats.org/wordprocessingml/2006/main}txbxContent")
             is not None]
    if boxes:
        inside = document.text_box_paragraphs(boxes[0])
        if inside:
            out.append(edit("insert_shape", at=f"{inside[0].id}@0"))
    out += [edit("move", target="d:999999", x=1, y=1), edit("resize", target="d:999999", width=10),
            edit("inline", target="d:999999"), edit("move"), edit("float", target="d:999999")]
    return out + [tracked(case) for case in out]


def _chunk(kind: bytes, body: bytes) -> bytes:
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


def _png(width: int, height: int, per_metre: int | None = None) -> bytes:
    """A black PNG, stating a density in pixels per metre when given."""
    rows = b"".join(b"\0" + b"\0" * (width * 3) for _ in range(height))
    out = b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    if per_metre:
        out += _chunk(b"pHYs", struct.pack(">IIB", per_metre, per_metre, 1))
    return out + _chunk(b"IDAT", zlib.compress(rows)) + _chunk(b"IEND", b"")


def _jpeg(width: int, height: int, dpi: int) -> bytes:
    """A JPEG's headers: JFIF stating ``dpi``, a baseline frame (nothing is decoded)."""
    app0 = b"JFIF\0" + bytes([1, 1, 1]) + struct.pack(">HH", dpi, dpi) + b"\0\0"
    sof = bytes([8]) + struct.pack(">HH", height, width) + bytes([1, 1, 0x11, 0])
    return (b"\xff\xd8\xff\xe0" + struct.pack(">H", len(app0) + 2) + app0
            + b"\xff\xc0" + struct.pack(">H", len(sof) + 2) + sof + b"\xff\xd9")


def _gif(width: int, height: int) -> bytes:
    return b"GIF89a" + struct.pack("<HH", width, height) + b"\0\0\0;"


def _bmp(width: int, height: int, per_metre: int) -> bytes:
    pixels = b"\0" * (((width * 3 + 3) // 4) * 4 * height)
    header = struct.pack("<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(pixels), per_metre, per_metre, 0, 0)
    return b"BM" + struct.pack("<IHHI", 14 + len(header) + len(pixels), 0, 0, 54) + header + pixels


_SVG = b'<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10"/></svg>'


def _blob(data: bytes, name: str, mime: str) -> dict:
    return {"name": name, "mime": mime, "hex": data.hex()}


#: The pictures the cases insert, each the session's b1 in its case.
_PICTURES = {"png": _blob(_png(40, 20), "a.png", "image/png"),
             "png300": _blob(_png(30, 30, 11811), "b.png", "image/png"),
             "jpeg": _blob(_jpeg(64, 48, 72), "c.jpg", "image/jpeg"),
             "gif": _blob(_gif(10, 10), "d.gif", "image/gif"),
             "bmp": _blob(_bmp(20, 10, 3780), "e.bmp", "image/bmp"),
             "svg": _blob(_SVG, "f.svg", "image/svg+xml"),
             "text": _blob(b"plain words", "g.txt", "text/plain"),
             "headless": _blob(b"\x89PNG\r\n\x1a\n" + _chunk(b"IEND", b""), "h.png", "image/png")}


def _media_bytes(document) -> bytes | None:
    """The bytes of the document's first picture part, when small: inserted again, the part
    is reused."""
    for name in document.package.part_names:
        if name.startswith("word/media/") and name.rsplit(".", 1)[-1] in ("png", "jpeg", "jpg", "gif"):
            data = document.package.read(name)
            return data if len(data) <= 64 * 1024 else None
    return None


#: The alt text of a picture the cases float (wrap given).
FLOATED = "A floated picture for the oracle"


@CASES
def picture_cases(document) -> list[dict]:
    """``word_drawings insert_picture``: a picture of each format (natural size, its stated
    density or 96 dpi; a width keeping the aspect ratio) at a position, a range, after text
    found once, in a cell, in a paragraph of its own after or before a block (aligned in the
    line, or floating), floating with each wrap and place; the bytes of a picture already in
    the document (its part reused); refusals (no image, a blob that is not there or not an
    image, no alt text, a height, a place without wrap, a tint on a bitmap, a header that
    gives no size, both or neither of at and find); an SVG left to the host -- untracked and
    tracked."""
    paragraphs = [b["id"] for b in document.blocks() if b["id"] and b["kind"] == "paragraph"]
    if not paragraphs:
        return []
    first = paragraphs[0]

    def put(picture: str = "png", **arguments) -> dict:
        arguments = {"image": "b1", "alt_text": "A picture for the oracle", **arguments}
        case = call("word_drawings", action="insert_picture",
                    **{k: v for k, v in arguments.items() if v is not None})
        return {**case, "blobs": [_PICTURES[picture]]}

    def floated(**arguments) -> dict:
        # its own alt text: known-differences.toml names the floated pictures by it
        return put(alt_text=FLOATED, **arguments)

    out = [put(at=f"{first}@0"), put(at=first, width=50), put("png300", at=f"{first}@0"),
           put("jpeg", at=f"{first}@0", width=120.5), put("gif", at=f"{first}@0"), put("bmp", at=f"{first}@0"),
           put(at=f"after:{first}"), put(at=f"before:{first}", width=30),
           put(at=f"after:{first}", align="center"), floated(at=f"after:{first}", wrap="square", align="right"),
           floated(at=f"{first}@0", wrap="square", x=10, y=5),
           floated(at=f"{first}@0", wrap="behind", against="page", align="center"),
           floated(at=f"{first}@0", wrap="tight"), floated(at=f"{first}@0", wrap="front", against="margin", y=-8),
           put(at=f"{first}@0", alt_text="Line one\nline \"two\" & <three>"),
           put(at=f"{first}@0", image=None), put(at=f"{first}@0", image="b9"), put("text", at=f"{first}@0"),
           put(at=f"{first}@0", alt_text=None), put(at=f"{first}@0", height=40), put(at=f"{first}@0", x=10),
           put(at=f"{first}@0", color="accent1"), put("svg", at=f"{first}@0"), put("headless", at=f"{first}@0"),
           put(at=f"{first}@0", find="x"), put(), put(at=f"{first}@9999"), put(at="after:p:00000000"),
           put(at=f"{first}@0", ref="pic")]
    words = document.paragraph(first).text.split()
    if words:
        out.append(put(find=words[0], width=20))
    tables = _tables(document)
    if tables:
        out += [put(at=f"{tables[0]}/c0,0"), put(at=f"after:{tables[0]}"), put(at=f"{tables[0]}/c99,0")]
    existing = _media_bytes(document)
    if existing is not None:
        case = call("word_drawings", action="insert_picture", image="b1", at=f"{first}@0", alt_text="Again")
        out.append({**case, "blobs": [_blob(existing, "again.png", _sniffed(existing))]})
    return out + [tracked(case) for case in out]


def _sniffed(data: bytes) -> str:
    return "image/png" if data.startswith(b"\x89PNG") else "image/gif" if data.startswith(b"GIF") else "image/jpeg"
