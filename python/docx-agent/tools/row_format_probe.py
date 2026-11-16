#!/usr/bin/env python3
"""Measure the text formatting Word gives a row it adds with Insert Rows Above/Below, and
text typed into it or into an empty cell (round 58).

One probe document (mode 15, every row and paragraph with its paraId) holds a table per
case, each followed by a paragraph ``After <Key>.``.  Every table is three columns by two
rows: shaded cells centred vertically, paragraphs with no space after, the third column's
centred; the runs Arial 8 pt, coloured, the third column's bold -- their paragraph marks
with no run properties of their own unless the case says so.  Word -- through
``tests/oracle.py``'s machine-wide lock -- opens it, makes each case's edits by
AppleScript (from the last table up) and saves.

* ``Below`` / ``Above``: Insert Rows Below row 2 / Above row 1, then a word typed into each
  new cell.
* ``Blank``: Insert Rows Below row 2, nothing typed (what the new row's marks hold).
* ``Marked``: as ``Below``, the source paragraphs' marks with run properties of their own
  (Courier New 14 pt, red) unlike their runs'.
* ``Mixed``: as ``Below``, the first column's cells two runs (the first bold and red, the
  second plain).
* ``Empty``: row 2's middle cell an empty paragraph with no mark properties; a word typed
  into it.  ``EmptyMarked``: the same, its mark Arial 8 pt coloured.
* ``Tracked``: as ``Below`` with tracking on.

    python tools/row_format_probe.py     # writes tests/observations/row-format-word.json

Nothing Word writes is committed: only the cells' paragraph and run properties and text,
rsids dropped, authors and dates replaced.
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
from e3_probe import _package, _script, _type  # noqa: E402

OBSERVATIONS = ROOT / "tests" / "observations" / "row-format-word.json"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W

TABLES = ["Below", "Above", "Blank", "Marked", "Mixed", "Empty", "EmptyMarked", "Tracked"]

RUN = ('<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>{bold}<w:color w:val="1F4E79"/>'
       '<w:sz w:val="16"/><w:szCs w:val="16"/>')
MARK_OTHER = ('<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:cs="Courier New"/>'
              '<w:color w:val="C00000"/><w:sz w:val="28"/><w:szCs w:val="28"/>')

_paras = iter(range(1, 100000))


def _pid() -> str:
    return f"{0x50000000 + next(_paras):08X}"


def _cell(key: str, i: int, j: int) -> str:
    bold = "<w:b/><w:bCs/>" if j == 2 else ""
    run_props = RUN.format(bold=bold)
    mark = ""
    if key == "Marked":
        mark = MARK_OTHER
    text = [f"{key}", f"text r{i}", f"{10 + i}"][j]
    runs = f'<w:r><w:rPr>{run_props}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>'
    if key == "Mixed" and j == 0:
        runs = ('<w:r><w:rPr><w:b/><w:color w:val="C00000"/></w:rPr><w:t xml:space="preserve">Lead</w:t></w:r>'
                f'<w:r><w:t xml:space="preserve"> {text}</w:t></w:r>')
    if key in ("Empty", "EmptyMarked") and i == 2 and j == 1:
        runs = ""
        mark = RUN.format(bold="") if key == "EmptyMarked" else ""
    jc = '<w:jc w:val="center"/>' if j == 2 else ""
    mark = f"<w:rPr>{mark}</w:rPr>" if mark else ""
    return (f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/><w:shd w:val="clear" w:color="auto" '
            f'w:fill="DEEAF6"/><w:vAlign w:val="center"/></w:tcPr>'
            f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:pPr><w:spacing w:after="0"/>{jc}{mark}</w:pPr>'
            f'{runs}</w:p></w:tc>')


def _table(key: str) -> str:
    rows = "".join(f'<w:tr w14:paraId="{_pid()}" w14:textId="77777777">'
                   + "".join(_cell(key, i, j) for j in range(3)) + "</w:tr>" for i in (1, 2))
    grid = "<w:tblGrid>" + '<w:gridCol w:w="2400"/>' * 3 + "</w:tblGrid>"
    props = '<w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    return f"<w:tbl>{props}{grid}{rows}</w:tbl>"


def _paragraph(text: str) -> str:
    return f'<w:p w14:paraId="{_pid()}" w14:textId="77777777"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'


def document() -> bytes:
    out = [_paragraph("Tables follow.")]
    for key in TABLES:
        out.append(_table(key))
        out.append(_paragraph(f"After {key}."))
    return _package("".join(out))


def script() -> str:
    t = {key: k + 1 for k, key in enumerate(TABLES)}

    def cell(key: str, r: int, c: int) -> str:
        return f"(get cell from table (table {t[key]} of d) row {r} column {c})"

    def type_into(key: str, r: int, c: int, text: str) -> list[str]:
        return [f"set e to start of content of text object of {cell(key, r, c)}",
                "select (create range d start e end e)", _type(text)]

    def insert(key: str, r: int, where: str) -> list[str]:
        return [f"select text object of {cell(key, r, 1)}", f"insert rows selection position {where}"]

    def type_row(key: str, r: int) -> list[str]:
        lines: list[str] = []
        for c, word in ((1, "new"), (2, "typed"), (3, "99")):
            lines += type_into(key, r, c, word)
        return lines

    lines = ["set track revisions of d to true"]
    lines += insert("Tracked", 2, "below") + type_row("Tracked", 3)
    lines += ["set track revisions of d to false"]
    lines += type_into("EmptyMarked", 2, 2, "typed")
    lines += type_into("Empty", 2, 2, "typed")
    for key in ("Mixed", "Marked"):
        lines += insert(key, 2, "below") + type_row(key, 3)
    lines += insert("Blank", 2, "below")
    lines += insert("Above", 1, "above") + type_row("Above", 1)
    lines += insert("Below", 2, "below") + type_row("Below", 3)
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


def read(data: bytes) -> dict:
    """Each table's rows: each cell's paragraph properties and its runs (properties, text)."""
    root = etree.fromstring(zipfile.ZipFile(io.BytesIO(data)).read("word/document.xml"))
    out: dict = {}
    for key, table in zip(TABLES, root.iter(_W + "tbl")):
        rows = []
        for row in table.findall(_W + "tr"):
            cells = []
            for cell in row.findall(_W + "tc"):
                paragraph = cell.find(_W + "p")
                runs = [[_xml(run.find(_W + "rPr")), "".join(t.text or "" for t in run.iter(_W + "t"))]
                        for run in paragraph.iter(_W + "r")]
                cells.append({"pPr": _xml(paragraph.find(_W + "pPr")), "runs": runs})
            rows.append({"trPr": _xml(row.find(_W + "trPr")), "cells": cells})
        out[key] = rows
    return out


def main() -> None:
    with oracle.session() as session:
        session.recover()
        data = document()
        if "--keep" in sys.argv:
            Path("row-format-in.docx").write_bytes(data)
        outcome = session.run_script(script(), data, name="row-format", tag="word", timeout=200)
        if not outcome:
            raise SystemExit(f"Word did not save: {outcome.outcome} {outcome.detail}")
        saved = outcome.path.read_bytes()
        outcome.path.unlink()
    OBSERVATIONS.write_text(json.dumps({
        "_about": ("What Word 16.106 for Mac wrote for tools/row_format_probe.py: rows added by Insert Rows "
                   "Above/Below beside rows whose runs carry the formatting (their marks none, or other), words "
                   "typed into them and into empty cells. Each cell's paragraph properties and runs; rsids, ids, "
                   "authors and dates dropped or replaced. Regenerate with python tools/row_format_probe.py."),
        **read(saved)}, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {OBSERVATIONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
