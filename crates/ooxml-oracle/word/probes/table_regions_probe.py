#!/usr/bin/env python3
"""Measure how Word draws a table style's conditional formats (``w:tblStylePr``): which
region wins where they overlap, which rows count as the header, how bands are counted
(band sizes, look options off, merged cells), and the borders a region gives (round 58,
R129).

One document (mode 15) holds a table per case, each in its own table style, every cell's
text a label ``<case><row><column>``.  Word -- through ``crates/ooxml-oracle/word/oracle.py``'s machine-wide
lock -- exports it as PDF; each label's fill (the smallest filled rectangle under it), text
colour, size, weight, font and horizontal place in its cell are read from the PDF, and
each case's border lines (filled rectangles under 2.5 pt across).

* ``S``: every region (and the style's own cell shading) a fill of its own, look all on.
* ``C``: every region (and the style's own run properties) a text colour and size of its own.
* ``B``: bold in the header row, first column and odd bands; ``K``: bold in header row and
  first column, the top-left cell not bold.
* ``H``: two rows marked to repeat as header rows; header row, odd and even row bands
  each a fill.
* ``N``: band sizes 2 (rows and columns).  ``O``: header row and first column off.
* ``M``: merged cells (``w:gridSpan``) against column bands, first and last column.
* ``P``: paragraph alignment by region.
* ``D``: borders by region over the style's own.

    python crates/ooxml-oracle/word/probes/table_regions_probe.py     # writes crates/ooxml-oracle/word/observations/table-regions-word.json

Nothing Word writes is committed: only the facts read from its PDF.
"""

from __future__ import annotations

import ctypes
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402  (the driver, the fixture makers and the cases on the path)
from probe_paths import ROOT  # noqa: E402

import oracle  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "table-regions-word.json"

from table_regions_cases import CASES, REGIONS, document  # noqa: E402,F401  (the probe document)


def _hex(r: int, g: int, b: int) -> str:
    return f"{r:02X}{g:02X}{b:02X}"


def read_pdf(path: Path) -> list[dict]:
    """Per page: its characters (text, box, colour, size, weight, font) and its filled
    rectangles (box, fill)."""
    import pypdfium2
    import pypdfium2.raw as raw

    pdf = pypdfium2.PdfDocument(str(path))
    pages = []
    try:
        for page in pdf:
            textpage = page.get_textpage()
            chars = []
            for index in range(textpage.count_chars()):
                code = raw.FPDFText_GetUnicode(textpage.raw, index)
                left, right, bottom, top = (ctypes.c_double() for _ in range(4))
                raw.FPDFText_GetCharBox(textpage.raw, index, left, right, bottom, top)
                rgba = [ctypes.c_uint() for _ in range(4)]
                raw.FPDFText_GetFillColor(textpage.raw, index, *rgba)
                buffer = ctypes.create_string_buffer(256)
                flags = ctypes.c_int()
                raw.FPDFText_GetFontInfo(textpage.raw, index, buffer, 256, flags)
                matrix = raw.FS_MATRIX()
                raw.FPDFText_GetMatrix(textpage.raw, index, matrix)
                scale = (matrix.a ** 2 + matrix.b ** 2) ** 0.5 or 1.0
                chars.append({"c": chr(code) if code else "", "box": (left.value, bottom.value, right.value, top.value),
                              "color": _hex(*(v.value for v in rgba[:3])),
                              "size": round(raw.FPDFText_GetFontSize(textpage.raw, index) * scale, 2),
                              "weight": raw.FPDFText_GetFontWeight(textpage.raw, index),
                              "font": buffer.value.decode("latin-1")})
            rects = []
            for obj in page.get_objects():
                if raw.FPDFPageObj_GetType(obj.raw) != raw.FPDF_PAGEOBJ_PATH:
                    continue
                rgba = [ctypes.c_uint() for _ in range(4)]
                if not raw.FPDFPageObj_GetFillColor(obj.raw, *rgba):
                    continue
                mode, stroke = ctypes.c_int(), ctypes.c_int()
                raw.FPDFPath_GetDrawMode(obj.raw, mode, stroke)
                if mode.value == 0:
                    continue
                bounds = [ctypes.c_float() for _ in range(4)]
                raw.FPDFPageObj_GetBounds(obj.raw, *bounds)
                left, bottom, right, top = (v.value for v in bounds)
                rects.append({"box": (left, bottom, right, top), "fill": _hex(*(v.value for v in rgba[:3]))})
            pages.append({"chars": chars, "rects": rects})
    finally:
        pdf.close()
    return pages


def _labels(pages: list[dict]) -> dict[str, dict]:
    """Each label's first character's facts, its box and its page."""
    out = {}
    for number, page in enumerate(pages):
        chars = page["chars"]
        text = "".join(c["c"] for c in chars)
        for case, rows, columns, *_ in CASES:
            for r in range(rows):
                for c in range(columns):
                    label = f"{case}{r}{c}"
                    at = text.find(label)
                    if at < 0 or label in out:
                        continue
                    first, last = chars[at], chars[at + len(label) - 1]
                    box = (first["box"][0], min(first["box"][1], last["box"][1]), last["box"][2],
                           max(first["box"][3], last["box"][3]))
                    out[label] = {**first, "box": box, "page": number}
    return out


def _under(page: dict, box: tuple) -> dict | None:
    """The smallest filled rectangle wider than 3 pt under the box's centre."""
    x, y = (box[0] + box[2]) / 2, (box[1] + box[3]) / 2
    found = [r for r in page["rects"] if r["box"][0] <= x <= r["box"][2] and r["box"][1] <= y <= r["box"][3]
             and r["box"][2] - r["box"][0] > 3 and r["box"][3] - r["box"][1] > 3]
    return min(found, key=lambda r: (r["box"][2] - r["box"][0]) * (r["box"][3] - r["box"][1]), default=None)


def facts(pages: list[dict]) -> dict:
    labels = _labels(pages)
    out: dict = {}
    for case, rows, columns, _look, _header, merged in CASES:
        cells = {}
        lefts = {}
        for label, found in labels.items():
            if label[0] == case:
                lefts.setdefault(label[2:], []).append(found["box"][0])
        for r in range(rows):
            for c in range(columns):
                label = f"{case}{r}{c}"
                found = labels.get(label)
                if found is None:
                    continue
                page = pages[found["page"]]
                under = _under(page, found["box"])
                cell = {"fill": under["fill"] if under else None, "color": found["color"],
                        "size": found["size"], "bold": found["weight"] >= 600 or "Bold" in found["font"],
                        "italic": "Italic" in found["font"]}
                if case == "P":
                    cell["x"] = round(found["box"][0], 1)
                cells[label[1:]] = cell
        out[case] = cells
    for case in ("D", "E", "F", "Q"):
        out[f"{case}_lines"] = _lines(pages, labels, case)
    return out


def _lines(pages: list[dict], labels: dict, case: str) -> list[dict]:
    """The thin filled rectangles (lines) around a case's table, relative to its first label."""
    origin = labels.get(f"{case}00")
    if origin is None:
        return []
    page = pages[origin["page"]]
    xs = [v["box"][0] for k, v in labels.items() if k[0] == case]
    ys = [v["box"][1] for k, v in labels.items() if k[0] == case]
    x0, x1, y0, y1 = min(xs) - 20, max(xs) + 90, min(ys) - 20, max(ys) + 30
    out = []
    for r in page["rects"]:
        left, bottom, right, top = r["box"]
        if right < x0 or left > x1 or top < y0 or bottom > y1:
            continue
        if min(right - left, top - bottom) > 3.5:
            continue
        out.append({"fill": r["fill"], "x": round(left - origin["box"][0], 1), "y": round(origin["box"][1] - top, 1),
                    "w": round(right - left, 2), "h": round(top - bottom, 2)})
    return sorted(out, key=lambda d: (d["y"], d["x"]))


def main() -> None:
    data = document()
    if "--keep" in sys.argv:
        Path("table-regions-in.docx").write_bytes(data)
    with oracle.session() as session:
        outcome = session.export_pdf(data, name="table-regions")
        if not outcome:
            raise SystemExit(f"Word did not export: {outcome.outcome} {outcome.detail}")
        pages = read_pdf(outcome.path)
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac drew for crates/ooxml-oracle/word/probes/table_regions_probe.py: per case (a table in a table "
                   "style of its own) each cell's fill, text colour, size, bold, italic (P: the text's left edge), "
                   "and D's border lines relative to its first cell's text. Regenerate with python "
                   "crates/ooxml-oracle/word/probes/table_regions_probe.py."),
        **facts(pages)}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
