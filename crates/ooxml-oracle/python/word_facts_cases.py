"""``check``'s facts read from the document alone, docx-agent against Rust (test support;
lane P5a2): ``word_reads_diff.py facts``.

For each document, docx-agent's ``layout_facts`` functions that need no layout --
``empty_paragraphs``, ``numbering_skips_level``, ``table_grid_mismatch``, ``accept_loses_text``
-- two read by ``layout_facts``' own class from the document alone, ``direct_formatting``
and the style list (``styles``: ``styles_not_allowed`` and ``styles_unknown`` for
:data:`ALLOWED`), their ``pages`` left out -- ``tools.originals``' ``accept_loses_original``,
and ``validate``'s ``problems_in`` (each of the body's blocks alone, and all of them) and
``extension_problems`` (for some file names), ``row_text_format`` (with made-up rows
written, :func:`_written`), ``punctuation_spacing`` and ``text_language`` (whole, with the
body's top-level paragraphs as ranges, and as of some pages' words) -- one case each,
against the ``word-reads`` binary's fact cases (``crates/ooxml-oracle/src/facts.rs``: the
Rust library functions of ``ooxml_word_edit::facts`` as ``check`` words them).  Uncapped and unfiltered (``text_language`` capped as ``check`` caps it):
the cap and the page and range filters are ``check``'s.

Besides the documents given, made-up ones that hold each fact (:func:`made_up`, from
the oracle's document makers, facts_documents.py and shelf_tables.py) are compared on every run; for
``accept_loses_original``, made-up documents edited here through docx-agent's API
(:func:`originals`), the edited version against the file as opened (``"opened"``).

Python's behaviour is not changed: this only calls docx-agent's documented functions.
"""

from __future__ import annotations

import io
import json
import tempfile
import zipfile
from pathlib import Path
from typing import Any

from docx_agent import Document
from docx_agent.layout_facts import (
    CAP, _PARA_ID, _Facts, accept_loses_text, empty_paragraphs, numbering_skips_level, punctuation_spacing,
    row_text_format, table_grid_mismatch, text_language_facts)
from docx_agent.tools.originals import accept_loses_original
from docx_agent.validate import check, extension_problems, problems_in

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


def _written(document) -> frozenset:
    """Made-up rows written for ``row_text_format`` (the session's record in check): each body
    table's every third row from row 2, by ``w14:paraId`` (``crates/ooxml-oracle/src/facts.rs``
    reads the same)."""
    index = document._index(document.package.document_part())
    return frozenset(row.get(_PARA_ID) for table in index.tables for row, _ in table.rows[2::3]
                     if row.get(_PARA_ID))


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
    "direct_formatting": direct_formatting,
    "styles": styles,
    "accept_loses_original": accept_loses_original,
    "problems_in": problems_in_blocks,
    "extension": extension,
    "row_text_format": lambda document: row_text_format(document, _written(document)),
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
    out.update(_formatting(_document, _p, _tbl))
    out.update(_validity(_document, _p, _tbl))
    out.update(_languages(_document))
    # A mark deleted without an id (no change to name: the reject within the cell), and a
    # cell lost in a nested table.
    out["accept-unnamed"] = _document(_edge((0, 1)).replace('w:del w:id="95" ', "w:del "))
    out["accept-nested"] = _document(_tbl([[_p("Outer") + _edge((1, 1)) + _p("Below"), _p("Side")]]))
    # Rows written unlike the rows around them (row 2 red among blue ones), and alike.
    from shelf_tables import ROWS, _shelf

    out["rows-unlike"] = _row_ids(_shelf(ROWS, colors=["1F4E79", "C00000", "1F4E79", "1F4E79"]))
    out["rows-banded"] = _row_ids(_shelf(ROWS, colors=["1F4E79", "7F7F7F", "1F4E79", "7F7F7F"]))
    return out


def _row_ids(data: bytes) -> bytes:
    """``data`` with each table row given a ``w14:paraId`` (``0A000001`` on)."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for item in source.infolist():
            content = source.read(item.filename)
            if item.filename == "word/document.xml":
                parts = content.split(b"<w:tr>")
                content = parts[0] + b"".join(b'<w:tr w14:paraId="0A%06X" w14:textId="77777777">' % k + part
                                              for k, part in enumerate(parts[1:], 1))
            target.writestr(item, content)
    return out.getvalue()


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
