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

from word_structure_cases import tracked


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


def edit_table_cases(document) -> list[dict]:
    """``word_edit_table``: rows inserted (after, before, at the top, past the end) and
    deleted (the first, a middle one, none named, past the end); columns deleted (the first,
    the second, none named, past the end), each with the file's text it removes; columns
    inserted (after the last, after and before the first, before the second, past the end);
    rows copied (the last, the first below and above it, the second, past the end); cells
    merged (across, down, both, a rectangle grown, past the grid, already one, reversed, no
    last) and split (by default, across, down, both, into one, past the table, no first);
    arguments
    of another action refused; strict mode's empty values taken as none -- untracked and
    tracked."""
    out = []
    for table in _tables(document)[:3]:
        out += _rows(table) + _columns(table) + _cells(table)
    out += [call("word_edit_table", table="t:00000000", action="insert_row")]
    return out + [tracked(case) for case in out]


def cases(document) -> list[dict]:
    """Every object case for ``document``."""
    found = format_table_cases(document) + edit_table_cases(document)
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
