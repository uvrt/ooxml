"""``check``'s facts read from the document alone, docx-agent against Rust (test support;
lane P5a2): ``word_reads_diff.py facts``.

For each document, docx-agent's ``layout_facts`` functions that need no layout --
``empty_paragraphs``, ``numbering_skips_level``, ``table_grid_mismatch``, ``accept_loses_text``
-- and two read by ``layout_facts``' own class from the document alone, ``direct_formatting``
and the style list (``styles``: ``styles_not_allowed`` and ``styles_unknown`` for
:data:`ALLOWED`), their ``pages`` left out -- one case each,
against the ``word-reads`` binary's fact cases (``crates/ooxml-oracle/src/facts.rs``: the
Rust library functions of ``ooxml_word_edit::facts`` as ``check`` words them).  Uncapped and unfiltered:
the cap and the page and range filters are ``check``'s.

Besides the documents given, made-up ones that hold each fact (:func:`made_up`, from
docx-agent's tests' document makers) are compared on every run.

Python's behaviour is not changed: this only calls docx-agent's documented functions.
"""

from __future__ import annotations

import io
import json
import sys
import tempfile
import zipfile
from pathlib import Path

TESTS = Path(__file__).resolve().parents[3] / "python" / "docx-agent" / "tests"
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))

from docx_agent import Document
from docx_agent.layout_facts import (
    _Facts, accept_loses_text, empty_paragraphs, numbering_skips_level, table_grid_mismatch)

#: The styles a ``styles`` case allows (``crates/ooxml-oracle/src/facts.rs``' ``ALLOWED``): a
#: name in another case, an id, a heading's name and one no style has.
ALLOWED = ["normal", "TableGrid", "heading 1", "No Such Style"]


def _uncapped(document, styles=None) -> _Facts:
    """``layout_facts``' class for ``document``'s layout, its lists not capped (the cap is
    ``check``'s)."""
    facts = _Facts(document.layout(), None, None, styles)
    facts.capped = lambda entries, name, out: out.__setitem__(name, entries)
    return facts


def _unplaced(entry: dict) -> dict:
    return {key: value for key, value in entry.items() if key != "pages"}


def direct_formatting(document) -> list:
    return [_unplaced(entry) for entry in _uncapped(document).direct_formatting()]


def styles(document) -> list:
    out: dict = {}
    _uncapped(document, list(ALLOWED)).style_list(out)
    return [{"styles_not_allowed": [_unplaced(e) for e in out["styles_not_allowed"]],
             "styles_unknown": out["styles_unknown"]}]


FACTS = {
    "empty_paragraphs": empty_paragraphs,
    "numbering_skips_level": numbering_skips_level,
    "table_grid_mismatch": table_grid_mismatch,
    "accept_loses_text": accept_loses_text,
    "direct_formatting": direct_formatting,
    "styles": styles,
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
    out.update(_formatting(_document, _p, _tbl))
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


def _formatting(document, p, tbl) -> dict[str, bytes]:
    """Paragraphs set apart from their styles by direct formatting (of the paragraph, of all
    its text, of some of it; in a cell; a table's rows kept together, whose keeps are the
    table's), and styles in use that :data:`ALLOWED` does not name."""
    def para(ppr: str, *runs: str) -> str:
        return f"<w:p><w:pPr>{ppr}</w:pPr>{''.join(runs)}</w:p>"

    def run(rpr: str, text: str) -> str:
        return f"<w:r><w:rPr>{rpr}</w:rPr><w:t xml:space='preserve'>{text}</w:t></w:r>"

    kept = "<w:keepNext/>"
    lead = para(kept, run("", "Lead"))
    body = (
        para("<w:jc w:val='center'/><w:spacing w:before='240' w:after='0' w:line='360' w:lineRule='auto'/>",
             run("", "Centred"))
        + para("<w:ind w:left='720' w:hanging='360'/>", run("", "Hanging"))
        + para("<w:spacing w:line='300' w:lineRule='exact'/><w:keepLines/><w:pageBreakBefore/>", run("", "Exact"))
        + para("", run("<w:b/><w:sz w:val='28'/>", "Bold big"))
        + para("", run("", "Plain "), run("<w:i/>", "some"))
        + para("", run("<w:color w:val='FF0000'/>", "aaaa"), run("<w:color w:val='0000FF'/>", "bb"))
        + para("", run("<w:rFonts w:ascii='Courier New' w:hAnsi='Courier New'/><w:highlight w:val='yellow'/>"
                       "<w:u w:val='single'/><w:strike/>", "Marked"))
        + para("", "<w:r><w:rPr><w:b/></w:rPr><w:tab/><w:t>tabbed</w:t></w:r>")
        + para("<w:pStyle w:val='Heading1'/><w:keepNext/>", run("", "Heading kept"))
        + para("<w:pStyle w:val='Heading1'/><w:jc w:val='right'/>", run("<w:b w:val='0'/>", "Heading right"))
        + tbl([[lead, p("x")], [lead, p("y")], [p("last"), p("z")]])
        + tbl([[lead, p("x")], [p("loose"), p("y")]])
        + tbl([[para("<w:jc w:val='both'/>", run("<w:b/>", "In a cell")), p("w")]])
    )
    styled = (
        para("<w:pStyle w:val='Heading1'/>", run("", "One"))
        + "".join(para("<w:pStyle w:val='Heading2'/>", run("", f"Two {n}")) for n in range(3))
        + para("<w:pStyle w:val='Title'/>", run("", "Title"))
        + para("<w:pStyle w:val='Ghost'/>", run("", "Ghost"))
        + para("<w:pStyle/>", run("", "No value"))
        + tbl([[p("a"), p("b")]])
        + tbl([[p("c"), p("d")]]).replace("<w:tblPr>", "<w:tblPr><w:tblStyle w:val='TableGrid'/>", 1)
    )
    return {"direct-shapes": document(body), "styles-mixed": document(styled)}


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
    rust = {key: line["reply"] for key, line in run_rust(args.rust, manifest).items()}
    return report(expected, rust)
