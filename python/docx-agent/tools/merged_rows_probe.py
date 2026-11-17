#!/usr/bin/env python3
"""Measure where Word's Insert Rows puts a row beside cells merged down, and whether the new
row joins the merge (round 59).

One probe document (mode 15, every row and paragraph with its paraId) holds a table per
case, each followed by a paragraph ``After <Key>.``.  Every table is three columns by five
rows, its first column merged down over rows 2-4 (``w:vMerge`` restart, continue, continue;
``End``'s over rows 3-5, the table's last); cell text ``<Key> r<i>c<j>``.  Word -- through
``tests/oracle.py``'s machine-wide lock -- opens it, makes each case's edits by AppleScript
(from the last table up) and saves.

* ``BelowLast``: the insertion point in row 4's second cell (the group's last row), Insert
  Rows Below.  ``BelowLastMerged``: the same in the merged cell itself.
* ``BelowFirst`` / ``Inside`` / ``InsideAbove``: row 2's / row 3's second cell, Insert Rows
  Below / Below / Above -- inside the group.
* ``AboveFirst``: row 2's second cell, Insert Rows Above -- above the group's first row.
  ``AboveFirstMerged``: the same in the merged cell.
* ``End``: the group runs to the table's last row; row 5's second cell, Insert Rows Below.
* ``TrackedInside`` / ``TrackedBelowLast``: as ``Inside`` / ``BelowLast`` with tracking on.
* ``Paste…`` (:data:`PASTES`, a second document): a whole row selected (Select Row), copied,
  and pasted with another row selected whole.

    python tools/merged_rows_probe.py   # writes tests/observations/merged-rows-word.json

Nothing Word writes is committed: per row its properties (rsids dropped, authors and dates
replaced) and per cell its grid span, ``w:vMerge`` value, text and paragraph mark revision.
"""

from __future__ import annotations

import io
import json
import re
import sys
import zipfile
from pathlib import Path

from lxml import etree

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
sys.path.insert(0, str(ROOT / "tools"))

import oracle  # noqa: E402
from e3_probe import _package, _script  # noqa: E402

OBSERVATIONS = ROOT / "tests" / "observations" / "merged-rows-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W

TABLES = ["BelowLast", "BelowLastMerged", "BelowFirst", "Inside", "InsideAbove", "AboveFirst",
          "AboveFirstMerged", "End", "TrackedInside", "TrackedBelowLast"]
#: Copy and paste of a whole row (a second document): ``(key, row copied, row the insertion
#: selected when pasting)``.
PASTES = [("PasteInside", 3, 4), ("PasteFirst", 2, 3), ("PasteAfterGroup", 4, 5), ("PasteAboveFirst", 2, 2),
          ("PasteEnd", 5, 5)]

_paras = iter(range(1, 100000))


def _pid() -> str:
    return f"{0x51000000 + next(_paras):08X}"


def _merged(key: str) -> tuple[int, int]:
    """The rows (1-based, first and last) the first column is merged down over."""
    return (3, 5) if key == "End" else (2, 4)


def _cell(key: str, i: int, j: int) -> str:
    first, last = _merged(key)
    merge = ""
    text = f"{key} r{i}c{j}"
    if j == 1 and first <= i <= last:
        merge = '<w:vMerge w:val="restart"/>' if i == first else "<w:vMerge/>"
        if i != first:
            text = ""
    runs = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/>{merge}</w:tcPr>'
            f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:pPr><w:spacing w:after="0"/></w:pPr>'
            f'{runs}</w:p></w:tc>')


def _table(key: str) -> str:
    rows = "".join(f'<w:tr w14:paraId="{_pid()}" w14:textId="77777777">'
                   + "".join(_cell(key, i, j) for j in (1, 2, 3)) + "</w:tr>" for i in range(1, 6))
    grid = "<w:tblGrid>" + '<w:gridCol w:w="2400"/>' * 3 + "</w:tblGrid>"
    borders = "".join(f'<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
                      for side in ("top", "left", "bottom", "right", "insideH", "insideV"))
    props = f'<w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>{borders}</w:tblBorders></w:tblPr>'
    return f"<w:tbl>{props}{grid}{rows}</w:tbl>"


def _paragraph(text: str) -> str:
    return f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'


def document(keys: list[str] = TABLES) -> bytes:
    out = [_paragraph("Tables follow.")]
    for key in keys:
        out.append(_table(key))
        out.append(_paragraph(f"After {key}."))
    return _package("".join(out))


def script() -> str:
    t = {key: k + 1 for k, key in enumerate(TABLES)}

    def insert(key: str, r: int, c: int, where: str) -> list[str]:
        return [f"select text object of (get cell from table (table {t[key]} of d) row {r} column {c})",
                f"insert rows selection position {where}"]

    lines = ["set track revisions of d to true"]
    lines += insert("TrackedBelowLast", 4, 2, "below")
    lines += insert("TrackedInside", 3, 2, "below")
    lines += ["set track revisions of d to false"]
    lines += insert("End", 5, 2, "below")
    lines += insert("AboveFirstMerged", 2, 1, "above")
    lines += insert("AboveFirst", 2, 2, "above")
    lines += insert("InsideAbove", 3, 2, "above")
    lines += insert("Inside", 3, 2, "below")
    lines += insert("BelowFirst", 2, 2, "below")
    lines += insert("BelowLastMerged", 2, 1, "below")
    lines += insert("BelowLast", 4, 2, "below")
    return _script(lines)


def paste_script() -> str:
    """Each paste case: its row ``copied`` selected whole (Select Row) and copied, row ``at``
    selected whole, and pasted."""
    lines: list[str] = []
    for k, (key, copied, at) in reversed(list(enumerate(PASTES))):
        cell = f"(get cell from table (table {k + 1} of d) row {{}} column 2)"
        lines += [f"select text object of {cell.format(copied)}", "select row selection", "copy object selection",
                  f"select text object of {cell.format(at)}", "select row selection", "paste object selection"]
    return _script(lines)


_DROP = re.compile(r' w:rsid\w*="[^"]*"')


def _xml(node) -> str:
    if node is None:
        return ""
    xml = etree.tostring(node, encoding="unicode")
    xml = re.sub(r' xmlns:\w+="[^"]*"', "", xml)
    xml = _DROP.sub("", xml)
    xml = re.sub(r'w:author="[^"]*"', 'w:author="{author}"', xml)
    xml = re.sub(r'w:date="[^"]*"', 'w:date="{date}"', xml)
    xml = re.sub(r' w16du:dateUtc="[^"]*"', "", xml)
    return re.sub(r' w:id="\d+"', "", xml)


def read(data: bytes, keys: list[str] = TABLES) -> dict:
    """Each table's rows: the row's properties; per cell its span, merge, text and whether
    its paragraph mark is a tracked insertion."""
    root = etree.fromstring(zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml"))
    out: dict = {}
    for key, table in zip(keys, root.iter(_W + "tbl")):
        rows = []
        for row in table.findall(_W + "tr"):
            cells = []
            for cell in row.findall(_W + "tc"):
                span = cell.find(f"{_W}tcPr/{_W}gridSpan")
                merge = cell.find(f"{_W}tcPr/{_W}vMerge")
                cells.append({
                    "span": int(span.get(_W + "val")) if span is not None else 1,
                    "vMerge": None if merge is None else (merge.get(_W + "val") or "continue"),
                    "text": "".join(t.text or "" for t in cell.iter(_W + "t")),
                    "mark_inserted": cell.find(f"{_W}p/{_W}pPr/{_W}rPr/{_W}ins") is not None,
                    "records": sorted({node.tag.split("}")[1] for node in cell.iter(
                        _W + "cellIns", _W + "cellDel", _W + "cellMerge", _W + "tcPrChange")}),
                })
            rows.append({"trPr": _xml(row.find(_W + "trPr")), "cells": cells})
        out[key] = rows
    return out


def main() -> None:
    with oracle.session() as session:
        session.recover()
        data = document()
        if "--keep" in sys.argv:
            Path("merged-rows-in.docx").write_bytes(data)
        outcome = session.run_script(script(), data, name="merged-rows", tag="word", timeout=200)
        if not outcome:
            raise SystemExit(f"Word did not save: {outcome.outcome} {outcome.detail}")
        saved = outcome.path.read_bytes()
        outcome.path.unlink()
        pastes = [key for key, _, _ in PASTES]
        pasted = session.run_script(paste_script(), document(pastes), name="merged-rows-paste", tag="word",
                                    timeout=200)
        if not pasted:
            raise SystemExit(f"Word did not save the pastes: {pasted.outcome} {pasted.detail}")
        pasted_data = pasted.path.read_bytes()
        pasted.path.unlink()
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for tools/merged_rows_probe.py: Insert Rows Above/Below beside "
                   "a first column merged down over rows 2-4 (End: 3-5, the last), the insertion point in the "
                   "row's second cell or (…Merged) in the merged cell. Per row its properties, per cell its span, "
                   "vMerge, text, inserted mark and cell revision records; rsids, ids, authors and dates dropped "
                   "or replaced. Regenerate with python tools/merged_rows_probe.py."),
        **read(saved), **read(pasted_data, pastes)}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
