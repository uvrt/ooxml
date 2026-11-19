"""The object tools' cases for ``word_edits_diff.py --objects`` (lane P4c): the tools
called through docx-agent's toolbox on a document as opened, compared with the Rust core's
(``word-edits``, ``op: "objects"``) as the structure tools' cases are
(:func:`word_structure_cases.python_case`: the reply, every changed part, the part order,
``touched``, the validation delta, the problems and the next ids).

Python's behaviour is not changed: this calls docx-agent's public and documented internal
functions only.
"""

from __future__ import annotations

import os

from word_structure_cases import ToolCases, tracked

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
            fmt(width_of="body"), fmt(width_of="window", width=100), fmt(keep_together=True),
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
