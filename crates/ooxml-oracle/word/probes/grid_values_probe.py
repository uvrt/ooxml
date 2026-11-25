#!/usr/bin/env python3
"""Measure how Word reads a table grid's odd values: ``w:gridSpan`` 0, -1, +2, " 2",
"1_0", "x", "", "2.0" and one past 64 bits (lane ph2-types3); ``w:gridBefore``/``w:gridAfter`` of -1; cells inside ``w:sdt``/``w:sdtContent`` and
``w:customXml``; a ``w:vMerge`` continuation with no start (lane ph2-types2, the one table
grid's value and cell-walk rules).

Each case is its own document (``cases/grid_values_cases.py``: three grid columns of 2000
twips, fixed layout; the case's row, then a row ``R1 R2 R3``), so a value Word refuses
blocks only its case.  Word -- through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide
lock -- opens each and (1) saves it again, (2) exports its PDF.

* From the saved document: whether Word opened it, and per row its ``w:gridBefore`` /
  ``w:gridAfter`` and per cell its ``w:gridSpan`` and ``w:vMerge`` as Word wrote them, its
  text, and the wrappers it sits in; the ``w:tblGrid`` columns.
* From the PDF: per marker (``K1``..., ``R1``...) the grid column its text starts in
  (``R1``'s left edge is column 0; 100 pt a column) and whether it shares ``R1``'s line.

    python crates/ooxml-oracle/word/probes/grid_values_probe.py   # writes crates/ooxml-oracle/word/observations/grid-values-word.json
"""

from __future__ import annotations

import io
import json
import sys
import zipfile
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)
from probe_paths import ROOT  # noqa: E402

import oracle  # noqa: E402

from grid_values_cases import CASES, WIDTH, document, table  # noqa: E402  (the probe documents)

OBSERVATIONS = probe_paths.OBSERVATIONS / "grid-values-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
COLUMN_POINTS = WIDTH / 20


def _val(node) -> str | None:
    return None if node is None else node.get(_W + "val")


def _cells(element, wrappers: list[str]) -> list[dict]:
    out = []
    for child in element:
        local = etree.QName(child).localname if isinstance(child.tag, str) else ""
        if local == "tc":
            merge = child.find(f"{_W}tcPr/{_W}vMerge")
            out.append({
                "text": "".join(t.text or "" for t in child.iter(_W + "t")),
                "gridSpan": _val(child.find(f"{_W}tcPr/{_W}gridSpan")),
                "vMerge": None if merge is None else (merge.get(_W + "val") or "continue"),
                "in": wrappers,
            })
        elif local in ("sdt", "customXml", "sdtContent"):
            out.extend(_cells(child, wrappers + [local] if local != "sdtContent" else wrappers))
    return out


def saved(data: bytes) -> dict:
    root = etree.fromstring(zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml"))
    table = next(root.iter(_W + "tbl"))
    rows = []
    for row in table.findall(_W + "tr"):
        rows.append({
            "gridBefore": _val(row.find(f"{_W}trPr/{_W}gridBefore")),
            "gridAfter": _val(row.find(f"{_W}trPr/{_W}gridAfter")),
            "cells": _cells(row, []),
        })
    grid = [int(c.get(_W + "w")) for c in table.findall(f"{_W}tblGrid/{_W}gridCol")]
    return {"tblGrid": grid, "rows": rows}


def placed(pdf: Path, markers: list[str]) -> dict:
    import pypdfium2

    document_ = pypdfium2.PdfDocument(str(pdf))
    try:
        textpage = document_[0].get_textpage()
        text = textpage.get_text_range()
        boxes = {}
        for marker in markers:
            at = text.find(marker)
            if at < 0:
                continue
            left, bottom, _, _ = textpage.get_charbox(at)
            boxes[marker] = (left, bottom)
    finally:
        document_.close()
    if "R1" not in boxes:
        return {"error": "R1 not drawn"}
    x0, y_reference = boxes["R1"]
    out = {}
    for marker in markers:
        if marker not in boxes:
            out[marker] = None
            continue
        x, y = boxes[marker]
        out[marker] = {"column": round((x - x0) / COLUMN_POINTS, 2), "reference_row": abs(y - y_reference) < 1}
    return out


def main() -> None:
    results: dict = {}
    with oracle.session() as session:
        session.recover()
        for key in CASES:
            data = document(key)
            markers = ["K1", "K2", "K3", "R1", "R2", "R3"]
            entry: dict = {"table": table(key)}
            resaved = session.resave(data, name=f"grid-{key}", timeout=120)
            entry["opened"] = resaved.outcome
            if resaved:
                entry["saved"] = saved(resaved.path.read_bytes())
            pdf = session.export_pdf(data, name=f"grid-{key}", timeout=120)
            if pdf:
                entry["drawn"] = placed(pdf.path, markers)
            else:
                entry["drawn"] = pdf.outcome
            results[key] = entry
            print(key, json.dumps(entry)[:300])
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac does with a table grid's odd values, for "
                   "crates/ooxml-oracle/word/probes/grid_values_probe.py (documents in cases/grid_values_cases.py: "
                   "three 2000-twip grid columns, fixed layout; the case's row, then R1 R2 R3). 'table': the case's table as written; 'opened': whether "
                   "Word opened and saved it; 'saved': the rows as Word wrote them again (gridBefore/gridAfter/"
                   "gridSpan/vMerge values, text, the wrappers a cell sits in) and the tblGrid widths; 'drawn': "
                   "per marker in Word's PDF the grid column its text starts in (R1's left is 0, 100 pt a column) "
                   "and whether it is on R1's line. Regenerate with python "
                   "crates/ooxml-oracle/word/probes/grid_values_probe.py."),
        **results}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
