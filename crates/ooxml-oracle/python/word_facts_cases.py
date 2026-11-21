"""``check``'s facts read from the document alone, docx-agent against Rust (test support;
lane P5a2): ``word_reads_diff.py facts``.

For each document, docx-agent's ``layout_facts`` functions that need no layout --
``empty_paragraphs``, ``numbering_skips_level``, ``accept_loses_text`` -- ``tools.originals``'
``accept_loses_original``, and ``validate``'s ``problems_in`` (each of the body's blocks alone,
and all of them) and ``extension_problems`` (for some file names) -- one case each,
against the ``word-reads`` binary's fact cases (``crates/ooxml-oracle/src/facts.rs``: the
Rust library functions of ``ooxml_word_edit::facts`` as ``check`` words them).  Uncapped and unfiltered:
the cap and the page and range filters are ``check``'s.

Besides the documents given, made-up ones that hold each fact (:func:`made_up`, from
docx-agent's tests' document makers) are compared on every run; for
``accept_loses_original``, made-up documents edited here through docx-agent's API
(:func:`originals`), the edited version against the file as opened (``"opened"``).

Python's behaviour is not changed: this only calls docx-agent's documented functions.
"""

from __future__ import annotations

import io
import json
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Any

TESTS = Path(__file__).resolve().parents[3] / "python" / "docx-agent" / "tests"
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))

from docx_agent import Document
from docx_agent.layout_facts import accept_loses_text, empty_paragraphs, numbering_skips_level, table_grid_mismatch
from docx_agent.tools.originals import accept_loses_original
from docx_agent.validate import check, extension_problems, problems_in

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
#: The body blocks a ``problems_in`` case reads alone at most (``facts.rs``' ``BLOCKS``).
BLOCKS = 200
#: The file names an ``extension`` case writes to (``facts.rs``' ``TARGETS``).
TARGETS = ["out.docx", "out.DOCM", "out.dotx", "out.dotm", "out.txt", ".docx"]
#: When this session's tracked edits are made (:func:`originals`).
DATE = "2026-11-21T10:00:00Z"

def problems_in_blocks(document) -> list:
    """``validate.problems_in`` of each of the body's first :data:`BLOCKS` blocks alone (those
    holding a problem), and of them all: the problems inside and how many are elsewhere."""
    package = document.package
    found = check(package)
    part = package.document_part()
    body = package.tree(part).find(_W + "body")
    blocks = [child for child in body if isinstance(child.tag, str)][:BLOCKS]
    out = []
    for k, block in enumerate(blocks):
        inside = problems_in(found, part, [block])
        if inside:
            out.append({"block": k, "problems": [str(p) for p in inside], "elsewhere": len(found) - len(inside)})
    inside = problems_in(found, part, blocks)
    out.append({"block": "all", "problems": [str(p) for p in inside], "elsewhere": len(found) - len(inside)})
    return out


def extension(document) -> list:
    """``validate.extension_problems`` for each of :data:`TARGETS`."""
    return [{"target": target, "problems": [str(p) for p in extension_problems(document.package, target)]}
            for target in TARGETS]


def _ided(body: str) -> str:
    """``body`` with a paraId on each paragraph (``<w:p>``), so that ids stay as the file has
    them when an edit runs."""
    parts = body.split("<w:p>")
    out = parts[0]
    for n, rest in enumerate(parts[1:]):
        out += f'<w:p xmlns:w14="{W14}" w14:paraId="{0x10000000 + n:08X}">' + rest
    return out


def originals() -> dict[str, tuple[bytes, Any]]:
    """Documents as opened and the tracked (or untracked) edits this session makes to them,
    by name, for ``accept_loses_original``: a column, a row, paragraphs (more than an entry
    lists), an untracked deletion, and a row deletion pending when the file was opened."""
    from facts_documents import _document, _p, _tbl

    rows = [["Site", "Crew", "Note"], ["Harbour", "Ines", "Pump check"], ["Orchard", "Tomas", ""],
            ["Quarry", "Lena", "Fence repair"]]
    table = _tbl([[_p(text) if text else "<w:p></w:p>" for text in row] for row in rows])
    lines = "".join(_p(f"Line {n}.") for n in range(25))
    opened = _document(_ided(_p("Crews") + _p("Intro line.") + table + _p("Closing line.") + lines))
    prior = Document.open(opened)
    with prior.tracking(author="Editor B", date="2026-11-13T09:00:00Z"):
        prior.delete_row(prior.tables()[0].id, 1)
    earlier = prior.to_bytes()

    def tracked(edit):
        def run(document):
            with document.tracking(author="Editor A", date=DATE):
                edit(document)
        return run

    def first_table(document):
        return document.tables()[0].id

    def paragraph(document, text):
        return next(p.id for p in document.paragraphs() if p.text == text)

    def lines_gone(document):
        for identifier in [paragraph(document, f"Line {n}.") for n in range(25)]:
            document.delete_block(identifier)

    return {
        "originals-column": (opened, tracked(lambda d: d.delete_column(first_table(d), 2))),
        "originals-row": (opened, tracked(lambda d: d.delete_row(first_table(d), 1))),
        "originals-paragraphs": (opened, tracked(lambda d: [d.delete_block(i) for i in (
            paragraph(d, "Intro line."), paragraph(d, "Closing line."))])),
        "originals-many": (opened, tracked(lines_gone)),
        "originals-untracked": (opened, lambda d: d.delete_column(first_table(d), 2)),
        "originals-earlier": (earlier, tracked(lambda d: d.delete_block(paragraph(d, "Intro line.")))),
    }


FACTS = {
    "empty_paragraphs": empty_paragraphs,
    "numbering_skips_level": numbering_skips_level,
    "table_grid_mismatch": table_grid_mismatch,
    "accept_loses_text": accept_loses_text,
    "accept_loses_original": accept_loses_original,
    "problems_in": problems_in_blocks,
    "extension": extension,
}


def _replaced(data: bytes, part: str, old: str, new: str) -> bytes:
    """``data`` (a package) with ``old`` replaced by ``new`` in ``part``."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for item in source.infolist():
            content = source.read(item.filename)
            if item.filename == part:
                assert old.encode() in content, (part, old)
                content = content.replace(old.encode(), new.encode())
            target.writestr(item, content)
    return out.getvalue()


def made_up() -> dict[str, bytes]:
    """Documents that hold each fact, by name: blank paragraphs in the body, in cells (two
    runs in one cell, a vertically merged cell, a content control's paragraphs, a run ended
    by a page break before, a section break or a deleted mark); list levels skipped (no
    chapter, a start override, a level not decimal, direct numbering two levels down);
    cells Accept All removes (each of docx-agent's row-edge shapes)."""
    from facts_documents import SECTIONS_ONLY, SHAPES, _document, _edge, _p, _tbl, empty_document, make

    out: dict[str, bytes] = {"empty-repro": empty_document()[0].to_bytes()}
    blank = "<w:p/>"
    spaced = "<w:p><w:r><w:t xml:space='preserve'>  </w:t></w:r></w:p>"
    kept = ("<w:p><w:pPr><w:pageBreakBefore/></w:pPr></w:p>", "<w:p><w:pPr><w:sectPr/></w:pPr></w:p>",
            "<w:p><w:pPr><w:framePr w:w='2000'/></w:pPr></w:p>")
    going = "<w:p><w:pPr><w:rPr><w:del w:id='77' w:author='A'/></w:rPr></w:pPr></w:p>"
    merged = ("<w:tc><w:tcPr><w:vMerge w:val='restart'/></w:tcPr>" + _p("Top") + blank + _p("Under") + "</w:tc>",
              "<w:tc><w:tcPr><w:vMerge/></w:tcPr>" + _p("Merged") + blank + _p("Away") + "</w:tc>")
    out["empty-shapes"] = _document(
        _p("One") + blank + spaced + _p("Two") + blank + kept[0] + blank + _p("Three") + kept[1] + _p("Four")
        + blank + kept[2] + _p("Five") + going + blank + _p("Six")
        + "<w:sdt><w:sdtContent>" + _p("Inside") + blank + _p("Still") + "</w:sdtContent></w:sdt>" + blank
        + _p("Seven") + blank + _tbl([[_p("A") + blank + _p("B") + blank + blank + _p("C"), _p("D")]]) + blank
        + _p("Eight") + "<w:tbl><w:tblGrid><w:gridCol w:w='3000'/><w:gridCol w:w='3000'/></w:tblGrid>"
        "<w:tr><w:trPr><w:gridBefore w:val='1'/></w:trPr>" + merged[0] + "</w:tr><w:tr>"
        + "<w:tc>" + _p("Left") + "</w:tc>" + merged[1] + "</w:tr></w:tbl>" + _p("Nine")
        + "<w:customXml w:element='x'>" + _p("Custom") + "</w:customXml>" + blank + _p("Ten"))
    data = make(SECTIONS_ONLY)
    numbering = "word/numbering.xml"
    out["levels-full"] = make()
    out["levels-sections-only"] = data
    out["levels-start-override"] = _replaced(
        data, numbering, '<w:abstractNumId w:val="0"/></w:num>',
        '<w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="4"/></w:lvlOverride>'
        '</w:num>')
    out["levels-roman"] = _replaced(data, numbering, '<w:numFmt w:val="decimal"/>',
                                    '<w:numFmt w:val="upperRoman"/>')
    out["levels-direct"] = _replaced(
        data, "word/document.xml", "<w:sectPr>",
        '<w:p><w:pPr><w:numPr><w:ilvl w:val="2"/><w:numId w:val="1"/></w:numPr></w:pPr>'
        '<w:r><w:t>Deep</w:t></w:r></w:p><w:sectPr>')
    for name, body in SHAPES.items():
        out["accept-" + name.replace(" ", "-").replace("'", "")] = _document(body)
    out.update(_grids(_document, _p))
    out.update(_validity(_document, _p, _tbl))
    # A mark deleted without an id (no change to name: the reject within the cell), and a
    # cell lost in a nested table.
    out["accept-unnamed"] = _document(_edge((0, 1)).replace('w:del w:id="95" ', "w:del "))
    out["accept-nested"] = _document(_tbl([[_p("Outer") + _edge((1, 1)) + _p("Below"), _p("Side")]]))
    return out


def _grids(document, p) -> dict[str, bytes]:
    """Tables off their grid (widened, short, by tracked cells or a tracked grid) and ragged
    (now, or as Accept All leaves them), each with ``document``'s body (docx-agent's test
    document maker) around it."""
    record = 'w:id="{}" w:author="Editor B" w:date="2026-11-13T09:00:00Z"'

    def tc(text: str, props: str = "") -> str:
        return f"<w:tc><w:tcPr><w:tcW w:w='2000' w:type='dxa'/>{props}</w:tcPr>{p(text)}</w:tc>"

    def tbl(columns: int, rows: list[str], grid_change: str = "") -> str:
        grid = "".join("<w:gridCol w:w='2000'/>" for _ in range(columns))
        return (f"<w:tbl><w:tblPr><w:tblW w:w='0' w:type='auto'/></w:tblPr><w:tblGrid>{grid}{grid_change}"
                "</w:tblGrid>" + "".join(f"<w:tr>{row}</w:tr>" for row in rows) + "</w:tbl>")

    def row(*cells: str, props: str = "") -> str:
        return (f"<w:trPr>{props}</w:trPr>" if props else "") + "".join(cells)

    plain = row(tc("a"), tc("b"))
    return {
        "grid-widened": document(tbl(2, [plain, row(tc("a"), tc("b"), tc("c"))])),
        "grid-short": document(tbl(3, [row(tc("a"), tc("b"), tc("c")), row(tc("a")),
                                       row(tc("a"), props="<w:gridAfter w:val='2'/>")])),
        "grid-cell-inserted": document(tbl(2, [plain, row(tc("a"), tc("b"), tc("new", f"<w:cellIns {record.format(41)}/>"))])),
        "grid-cell-deleted": document(tbl(2, [plain, row(tc("a"), tc("", f"<w:cellDel {record.format(42)}/>"), tc("b"))])),
        "grid-changed": document(tbl(3, [plain, plain],
                                     f"<w:tblGridChange w:id='43'><w:tblGrid><w:gridCol w:w='3000'/>"
                                     "<w:gridCol w:w='3000'/></w:tblGrid></w:tblGridChange>")),
        "grid-many-rows": document(tbl(1, [plain] * 23)),
        "ragged-now": document(tbl(2, [plain, row(tc("b"), props="<w:gridBefore w:val='1'/>"),
                                       row(tc("a"), props="<w:gridAfter w:val='1'/>")])),
        "ragged-after-accept": document(tbl(3, [row(tc("a"), tc("gone", f"<w:cellDel {record.format(44)}/>"), tc("c")),
                                                row(tc("a"), tc("b"), tc("c"))])),
        "ragged-nested": document(tbl(1, [row(tc("x")[:-len("</w:tc>")] + tbl(2, [plain, row(tc("only"))])
                                              + p("after") + "</w:tc>")])),
    }


def _validity(document, p, tbl) -> dict[str, bytes]:
    """Problems in some blocks (a repeated paraId, an unpaired bookmark and move range, a
    missing style, list and relationship, text in a deletion, a record without an author, a
    cell ending in a table), and packages whose kind does not fit an extension (a template's
    content type, a VBA project)."""
    w14 = f'xmlns:w14="{W14}"'
    body = (f'<w:p {w14} w14:paraId="1A2B3C4D"><w:r><w:t>One</w:t></w:r></w:p>'
            f'<w:p {w14} w14:paraId="1A2B3C4D"><w:r><w:t>Two</w:t></w:r></w:p>'
            '<w:p><w:bookmarkStart w:id="5" w:name="Lonely"/><w:r><w:t>Mark</w:t></w:r></w:p>'
            '<w:p><w:pPr><w:pStyle w:val="Missing"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="7"/></w:numPr>'
            '</w:pPr><w:r><w:t>Styled</w:t></w:r></w:p>'
            '<w:p xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
            '<w:hyperlink r:id="rId99"><w:r><w:t>Link</w:t></w:r></w:hyperlink></w:p>'
            '<w:p><w:del w:id="61" w:author="A" w:date="2026-11-13T09:00:00Z"><w:r><w:t>Wrong</w:t></w:r></w:del>'
            '<w:ins w:id="62"><w:r><w:t>Nobody</w:t></w:r></w:ins></w:p>'
            '<w:p><w:moveFromRangeStart w:id="63" w:name="away" w:author="A"/><w:r><w:t>Moved</w:t></w:r></w:p>'
            + tbl([[p("Outer") + tbl([[p("Inner")]]), p("Side")]]))
    problems = document(body)
    data = document(p("Plain"))
    types = "[Content_Types].xml"
    document_type = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
    template_type = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml"
    rels = "word/_rels/document.xml.rels"
    vba = ('<Relationship Id="rIdVba" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" '
           'Target="vbaProject.bin"/></Relationships>')
    return {"problems-mixed": problems, "kind-template": _replaced(data, types, document_type, template_type),
            "kind-macros": _replaced(data, rels, "</Relationships>", vba)}


def check_facts(args, run_rust, report, files_of) -> int:
    expected, manifest = {}, []
    scratch = Path(tempfile.mkdtemp(prefix="word-facts-"))
    made = []
    for name, data in made_up().items():
        made.append(scratch / f"{name}.docx")
        made[-1].write_bytes(data)
    for path in [*made, *files_of(args)]:
        try:
            document = Document.open(path.read_bytes())
        except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
            print(f"{path}: python failed to open: {error}")
            continue
        for case, fact in FACTS.items():
            try:
                found = json.loads(json.dumps(fact(document)))
            except Exception as error:  # noqa: BLE001
                print(f"{path} [{case}]: python failed: {error}")
                continue
            expected[(str(path), case)] = {"facts": found}
            manifest.append({"file": str(path), "case": case, "fact": case})
    for name, (opened, edit) in originals().items():
        before, after = scratch / f"{name}-opened.docx", scratch / f"{name}.docx"
        before.write_bytes(opened)
        try:
            document = Document.open(opened)
            edit(document)
            found = json.loads(json.dumps(accept_loses_original(document)))
            after.write_bytes(document.to_bytes())
        except Exception as error:  # noqa: BLE001
            print(f"{name} [accept_loses_original]: python failed: {error}")
            continue
        expected[(str(after), "accept_loses_original")] = {"facts": found}
        manifest.append({"file": str(after), "opened": str(before), "case": "accept_loses_original",
                         "fact": "accept_loses_original"})
    rust = {key: line["reply"] for key, line in run_rust(args.rust, manifest).items()}
    return report(expected, rust)
