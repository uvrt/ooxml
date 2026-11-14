"""Edits of a made-up report (make_report.py) for the differential tests: five kinds, each at
the start, the middle and the end of the report, made with docx-agent's document API.

    edit(document, kind, position)

Each edit is made on the base document, not chained.  Positions are taken over the whole
report (its long paragraphs and its three-column tables in order), so they work for any
number of chapters.
"""
from __future__ import annotations

KINDS = ("text-same", "text-longer", "para-insert", "row-insert", "cell-text")
POSITIONS = ("start", "middle", "end")

LONG = ("Copper lantern meadow river stone harbour quiet orchard signal morning ledger window garden "
        "timber bridge market evening canvas thread pebble summit. ") * 2


def _pick(items: list, position: str, edge: int):
    index = {"start": edge, "middle": len(items) // 2, "end": len(items) - 1 - edge}[position]
    return items[max(0, min(index, len(items) - 1))]


def edit(document, kind: str, position: str) -> None:
    if kind in ("text-same", "text-longer", "para-insert"):
        longs = [p for p in document.paragraphs() if len(p.text) > 200]
        p = _pick(longs, position, 3)
        if kind == "text-same":
            document.paragraph(p.id).set_text(p.text[::-1])
        elif kind == "text-longer":
            document.paragraph(p.id).set_text(p.text + " " + LONG)
        else:
            document.insert_paragraph(LONG, after=p.id)
    elif kind in ("row-insert", "cell-text"):
        tables = [t for t in document.tables() if t.column_count == 3]
        t = _pick(tables, position, 1)
        if kind == "row-insert":
            t.insert_row(4)
        else:
            t.cell(3, 2).paragraphs[0].set_text(LONG)
    else:
        raise ValueError(kind)
