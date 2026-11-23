#!/usr/bin/env python3
"""Measure where Word's Insert Rows puts a row beside cells merged down, and whether the new
row joins the merge (round 59).

One probe document (mode 15, every row and paragraph with its paraId) holds a table per
case, each followed by a paragraph ``After <Key>.``.  Every table is three columns by five
rows, its first column merged down over rows 2-4 (``w:vMerge`` restart, continue, continue;
``End``'s over rows 3-5, the table's last); cell text ``<Key> r<i>c<j>``.  Word -- through
``python/docx-agent/tests/oracle.py``'s machine-wide lock -- opens it, makes each case's edits by AppleScript
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

    python crates/ooxml-oracle/word/probes/merged_rows_probe.py   # writes crates/ooxml-oracle/word/observations/merged-rows-word.json

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

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)
from probe_paths import ROOT  # noqa: E402

import oracle  # noqa: E402
from e3_probe import _script  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "merged-rows-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W

from merged_rows_cases import PASTES, TABLES, document  # noqa: E402  (the probe document)


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
        "_about": ("What Word 16.106 for Mac wrote for crates/ooxml-oracle/word/probes/merged_rows_probe.py: Insert Rows Above/Below beside "
                   "a first column merged down over rows 2-4 (End: 3-5, the last), the insertion point in the "
                   "row's second cell or (…Merged) in the merged cell. Per row its properties, per cell its span, "
                   "vMerge, text, inserted mark and cell revision records; rsids, ids, authors and dates dropped "
                   "or replaced. Regenerate with python crates/ooxml-oracle/word/probes/merged_rows_probe.py."),
        **read(saved), **read(pasted_data, pastes)}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
