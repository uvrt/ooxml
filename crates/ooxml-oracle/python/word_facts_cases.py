"""``check``'s facts read from the document alone, docx-agent against Rust (test support;
lane P5a2): ``word_reads_diff.py facts``.

For each document, docx-agent's ``layout_facts`` functions that need no layout --
``empty_paragraphs``, ``numbering_skips_level``, ``table_grid_mismatch``, ``accept_loses_text``,
``punctuation_spacing`` and ``text_language`` (whole, with the body's top-level paragraphs as
ranges, and as of some pages' words) -- one case each,
against the ``word-reads`` binary's fact cases (``crates/ooxml-oracle/src/facts.rs``: the
Rust library functions of ``ooxml_word_edit::facts`` as ``check`` words them).  Uncapped and unfiltered (``text_language`` capped as ``check`` caps it):
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
from docx_agent.layout_facts import (CAP, _Facts, accept_loses_text, empty_paragraphs, numbering_skips_level,
                                     punctuation_spacing, table_grid_mismatch, text_language_facts)


def _top_level(document: Document):
    """The body's top-level blocks as ``check``'s layout facts read them (``_Facts._top_level``,
    which reads only the document and its main part)."""
    from types import SimpleNamespace

    return _Facts._top_level(SimpleNamespace(document=document, main=document.package.document_part()))


FACTS = {
    "empty_paragraphs": empty_paragraphs,
    "numbering_skips_level": numbering_skips_level,
    "table_grid_mismatch": table_grid_mismatch,
    "accept_loses_text": accept_loses_text,
    "punctuation_spacing": punctuation_spacing,
    "text_language": lambda document: text_language_facts(document, cap=CAP),
    "text_language_ranges": lambda document: text_language_facts(document, top_level=_top_level(document), cap=CAP),
    "text_language_checked": lambda document: text_language_facts(document, whole=False, cap=CAP),
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
    out.update(_languages(_document))
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


def _languages(document) -> dict[str, bytes]:
    """Text in each spacing rule's language and in another language than its tag: French
    spacing missing, breakable and in guillemets (Canadian French, a code style, a field's
    result, marks between words, more marks than an entry shows, text a fix's find must
    widen), French spacing tagged English by the default, a style and a run, paragraphs in
    French, German and Dutch under English (one listed, consecutive ones a range, a short one
    between, one in a table, one tagged by its runs), text too short or unclear, text not
    proofed, a text box's paragraph; and more mismatched paragraphs than an entry lists."""

    def run(text: str, lang: str | None = None, extra: str = "") -> str:
        props = (f"<w:lang w:val='{lang}'/>" if lang else "") + extra
        return f"<w:r>{f'<w:rPr>{props}</w:rPr>' if props else ''}<w:t xml:space='preserve'>{text}</w:t></w:r>"

    def para(*runs: str, style: str | None = None) -> str:
        return f"<w:p>{f'<w:pPr><w:pStyle w:val={chr(39)}{style}{chr(39)}/></w:pPr>' if style else ''}{''.join(runs)}</w:p>"

    def table(*cells: str) -> str:
        return ("<w:tbl><w:tblGrid><w:gridCol w:w='3000'/></w:tblGrid>"
                + "".join(f"<w:tr><w:tc>{cell}</w:tc></w:tr>" for cell in cells) + "</w:tbl>")

    styles = ("<w:style w:type='character' w:styleId='Allemand'><w:name w:val='Allemand'/>"
              "<w:rPr><w:lang w:val='de-DE'/></w:rPr></w:style>"
              "<w:style w:type='character' w:styleId='HTMLCode'><w:name w:val='HTML Code'/></w:style>"
              "<w:style w:type='paragraph' w:styleId='Francais'><w:name w:val='Francais'/>"
              "<w:rPr><w:lang w:val='fr-BE'/></w:rPr></w:style>")
    field = ("<w:r><w:fldChar w:fldCharType='begin'/></w:r><w:r><w:instrText> TITLE </w:instrText></w:r>"
             "<w:r><w:fldChar w:fldCharType='separate'/></w:r>" + run("Titre:", "fr-FR")
             + "<w:r><w:fldChar w:fldCharType='end'/></w:r>")
    box = ("<w:r><w:pict><v:shape xmlns:v='urn:schemas-microsoft-com:vml'><v:textbox><w:txbxContent>"
           + para(run("Der Hund ist nicht in dem Haus und er wird auch nicht kommen.", "de-DE"))
           + "</w:txbxContent></v:textbox></v:shape></w:pict></w:r>")
    french = "Le chat est sur la table et il dort dans le soleil du matin avec les autres."
    german = "Der Hund ist nicht in dem Haus und er wird auch nicht mit uns kommen."
    dutch = "De hond is niet in het huis en hij zal ook niet met ons komen vandaag."
    english = "The cat is on the table and it sleeps in the sun of the morning with the others."
    spacing = document(
        para(run("Bonjour: le monde ! Fin?", "fr-FR"))
        + para(run("«oui» et « non » puis «peut-être »", "fr-FR"))
        + para(run("«oui» : non!", "fr-CA"))
        + para(run("Il est 12:30 et x?y reste; voir http://exemple", "fr-FR"))
        + para(run("code:", "fr-FR", "<w:rStyle w:val='HTMLCode'/>"), run(" et sans:", "fr-FR", "<w:noProof/>"))
        + para(field, run(" puis fin:", "fr-FR"))
        + para(run("a: b: c: d: e: f: g: h: i: j: k: l: fin:", "fr-FR"))
        + para(run("oui: non oui: non oui:", "fr-FR"))
        + para(run("Mixed: ", "fr-FR"), run("then English: done", "en-US"))
        + para(run("Quoi ?! Vraiment!! « dit-il»", "fr-FR"), style="Francais")
        + table(para(run("Dans la table:", "fr-FR")))
        + para(run("Bonjour : tagged English by the default ; really ?"))
        + para(run("Guten Tag : im Stil ; ja ?", extra="<w:rStyle w:val='Allemand'/>"))
        + para(run("Hello : tagged by the run !", "en-GB"))
        + para(run("Hola : « sí » ?", "es-ES"))
        )
    language = document(
        para(run(french)) + para(run(french)) + para(run(german, "en-US")) + para(run("Kurz."))
        + para(run(german)) + para(run(dutch, "nl-NL")) + para(run(dutch)) + table(para(run(german)))
        + para(run(german, "en-GB")) + para(run(german, "en-GB")) + para(run(english))
        + para(run("Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor"))
        + para(run(french, extra="<w:noProof/>")) + para(run("Too short.")) + para(box)
        + para(run(french[:40]), run(french[40:], "fr-FR")))
    many = document("".join(para(run(german if k % 2 else english)) for k in range(50)))
    french_doc = document("".join(para(run(french)) for _ in range(6)) + para(run(english)) + para(run(german)))
    return {
        "spacing-french": _replaced(spacing, "word/styles.xml", "</w:styles>", styles + "</w:styles>"),
        "language-mixed": language,
        "language-many": many,
        "language-mostly-french": french_doc,
    }


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
