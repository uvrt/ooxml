#!/usr/bin/env python3
"""Measure Word's Accept All of the table shapes where the readers' final views differed
before one View module (lane ph3-rd1c): a row whose cells are all pending deletions, a cell
whose last block is a content control holding a paragraph whose mark is deleted, and a
deleted cell whose content stays after a cell whose end accepting removes.

One document per case (``CASES``); Word -- through ``crates/ooxml-oracle/word/oracle.py``'s
machine-wide lock -- accepts all revisions and saves.  What it saved is read as each table's
rows, their cells' text and spans, and the grid.

    python crates/ooxml-oracle/word/probes/view_probe.py    # writes crates/ooxml-oracle/word/observations/view-word.json

Nothing Word writes is committed: only the facts read from it.
"""

from __future__ import annotations

import io
import json
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402,F401  (the driver on the path)

import oracle  # noqa: E402

OBSERVATIONS = probe_paths.OBSERVATIONS / "view-word.json"

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
DATE = 'w:author="Other" w:date="2026-11-01T00:00:00Z"'
_ids = iter(range(100, 10000))


def p(text: str, deleted_mark: bool = False) -> str:
    mark = f'<w:pPr><w:rPr><w:del w:id="{next(_ids)}" {DATE}/></w:rPr></w:pPr>' if deleted_mark else ""
    run = f"<w:r><w:t>{text}</w:t></w:r>" if text else ""
    return f"<w:p>{mark}{run}</w:p>"


def tc(content: str, deleted: bool = False) -> str:
    pr = f'<w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:cellDel w:id="{next(_ids)}" {DATE}/></w:tcPr>' if deleted \
        else '<w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>'
    return f"<w:tc>{pr}{content}</w:tc>"


def table(rows: list[list[str]], columns: int) -> str:
    grid = "".join('<w:gridCol w:w="2000"/>' for _ in range(columns))
    body = "".join(f"<w:tr>{''.join(r)}</w:tr>" for r in rows)
    return (f'<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>'
            f'<w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
            f'</w:tblBorders></w:tblPr><w:tblGrid>{grid}</w:tblGrid>{body}</w:tbl>')


def sdt(content: str) -> str:
    return f'<w:sdt><w:sdtPr><w:id w:val="{next(_ids)}"/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>'


CASES = {
    # a row whose every cell is a pending deletion (its row itself not deleted)
    "AllCellsDeleted": table([[tc(p("A1")), tc(p("B1"))], [tc(p("A2"), True), tc(p("B2"), True)],
                              [tc(p("A3")), tc(p("B3"))]], 2),
    # the same with the deleted cells empty (nothing of them kept)
    "AllCellsDeletedEmpty": table([[tc(p("A1")), tc(p("B1"))], [tc(p(""), True), tc(p(""), True)],
                                   [tc(p("A3")), tc(p("B3"))]], 2),
    # a cell whose last block is a content control holding a paragraph whose mark is deleted
    "ControlEndsCell": table([[tc(sdt(p("P1", True))), tc(p("Q1"))], [tc(p("P2")), tc(p("Q2"))]], 2),
    # the same with the paragraph directly in the cell (the reference)
    "ParagraphEndsCell": table([[tc(p("P1", True)), tc(p("Q1"))], [tc(p("P2")), tc(p("Q2"))]], 2),
    # a deleted cell whose content stays, after a cell whose end accepting removes
    "KeptAfterGone": table([[tc(p("Y1", True)), tc(p("Z1"), True)], [tc(p("Y2")), tc(p("Z2"))]], 2),
    # a deleted cell whose content stays, after a plain cell (the reference, round 54 R110)
    "KeptAfterPlain": table([[tc(p("Y1")), tc(p("Z1"), True)], [tc(p("Y2")), tc(p("Z2"))]], 2),
}


def document(case: str) -> bytes:
    body = f'{p("Before " + case)}{CASES[case]}{p("After " + case)}<w:sectPr/>'
    xml = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>'
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml",
                   '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                   '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                   '<Default Extension="xml" ContentType="application/xml"/>'
                   '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>')
        z.writestr("_rels/.rels",
                   '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                   '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
        z.writestr("word/document.xml", xml)
    return buffer.getvalue()


def shape(data: bytes) -> dict:
    """The accepted document's tables: per row, its cells' text and gridSpan; any revision left."""
    from xml.etree import ElementTree as ET
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        root = ET.fromstring(z.read("word/document.xml"))
    q = lambda n: f"{{{W}}}{n}"  # noqa: E731
    body = root.find(q("body"))
    out: dict = {"blocks": [], "revisions_left": 0}
    for e in root.iter():
        if e.tag in (q("ins"), q("del"), q("cellDel"), q("cellIns"), q("moveFrom"), q("moveTo")):
            out["revisions_left"] += 1
    for block in body:
        if block.tag == q("p"):
            out["blocks"].append({"p": "".join(t.text or "" for t in block.iter(q("t")))})
        elif block.tag == q("tbl"):
            rows = []
            for tr in block.iter(q("tr")):
                cells = []
                for cell in tr.findall(q("tc")):
                    span = cell.find(f"{q('tcPr')}/{q('gridSpan')}")
                    paragraphs = ["".join(t.text or "" for t in p.iter(q("t"))) for p in cell.iter(q("p"))]
                    cells.append({"text": paragraphs, "span": int(span.get(q("val"))) if span is not None else 1})
                rows.append(cells)
            grid = [int(g.get(q("w"))) for g in block.iter(q("gridCol"))]
            out["blocks"].append({"table": rows, "grid": grid})
    return out


def main() -> None:
    result: dict = {"_about": ("What Word 16.106 for Mac's Accept All leaves of the table shapes in "
                               "crates/ooxml-oracle/word/probes/view_probe.py (lane ph3-rd1c)")}
    with oracle.session() as session:
        for case in CASES:
            data = document(case)
            done = session.run_script(oracle.review_script("accept"), data, name=f"rd1c-{case}", tag="accept",
                                      timeout=200)
            if not done:
                result[case] = {"error": f"{done.outcome} {done.detail}"}
                continue
            result[case] = shape(done.path.read_bytes())
            done.path.unlink()
    OBSERVATIONS.write_text(json.dumps(result, indent=1) + "\n")


if __name__ == "__main__":
    main()
