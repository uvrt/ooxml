#!/usr/bin/env python3
"""Write ``tests/fixtures/generated/table-keeps.docx`` and ``tables-placed.docx``.

``table-keeps.docx``: tables kept on one page by their rows'
keeps in every way ``word_format_table``'s ``keep_together`` / ``keep_with_next`` read and
write them (round 25, W2/W5; round 27, W7), written here by hand with
:mod:`make_fixtures`' parts (compatibility mode 15, paraIds on every paragraph):

- a table kept together by direct keeps (every row's first paragraph but the last's), then a
  paragraph that does not keep with the next and another table (the "loose paragraph"
  between two tables);
- a table kept together and with the next block, by direct keeps on every row;
- a paragraph keeping with the next, then a table kept together and with the next by its
  rows' paragraph style (``Label``, which keeps with the next);
- a table whose last row alone keeps with the next;
- a one-row table;
- a table kept by its rows' style whose last row releases it directly (``w:val="0"``);
- a table whose first row's first cell starts with a nested table (the row's keep is that
  table's first paragraph's);
- a table of sixty rows kept together, taller than a page.

``tables-placed.docx``: two tables placed from a source, each with its record of where its
values came from (docx-agent's provenance item, written here as docx-agent writes one), for
``word_edit_table``'s changes that keep the record true (``_Kept``) and ``sync``:

- a table whose header row and four rows are the record's block (``d:long/#1:#4``), its
  second row the differential harness's author's own pending tracked insertion (a tracked
  deletion of it removes it outright), a row with no key, and a total row (a formula);
- a table with a title row above the block, the block (its header row and two rows, two of
  its three columns recorded: ``d:sales/#1:#2``) and a row below it.

Deterministic: running it again writes the same bytes.

    python tools/make_table_fixtures.py
"""

from __future__ import annotations

from make_fixtures import DECL, NS, REL, WML, content_types, p, relationships, settings, styles, write

_COUNTER = iter(range(1, 10_000))
_KEEP = "<w:keepNext/>"
_RELEASE = '<w:keepNext w:val="0"/>'


def _para(text: str, *, style: str | None = None, keep: str = "") -> str:
    """A paragraph with its own paraId; ``keep``, its direct ``w:keepNext`` (or none)."""
    para = f"5A{next(_COUNTER):06X}"
    props = (f'<w:pStyle w:val="{style}"/>' if style else "") + keep
    body = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return (f'<w:p w14:paraId="{para}" w14:textId="77777777">'
            f'{f"<w:pPr>{props}</w:pPr>" if props else ""}{body}</w:p>')


def _cell(content: str, width: int = 4513) -> str:
    return f'<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/></w:tcPr>{content}</w:tc>'


def _table(rows: list[str], width: int = 4513) -> str:
    return ('<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/>'
            '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" '
            'w:noHBand="0" w:noVBand="1"/></w:tblPr>'
            f'<w:tblGrid><w:gridCol w:w="{width}"/><w:gridCol w:w="{width}"/></w:tblGrid>'
            + "".join(rows) + "</w:tbl>")


def _row(name: str, k: int, *, style: str | None = None, keep: str = "", first: str | None = None) -> str:
    """A row of two cells; its first cell's first paragraph styled and kept as given (or
    ``first`` in its place)."""
    lead = first if first is not None else _para(f"{name} row {k + 1}", style=style, keep=keep)
    return f"<w:tr>{_cell(lead)}{_cell(_para(f'{name} value {k + 1}'))}</w:tr>"


def _kept(name: str, count: int, *, last: str = "", style: str | None = None, keep: str = _KEEP) -> str:
    """A table of ``count`` rows, every row but the last keeping (``keep``), the last ``last``."""
    return _table([_row(name, k, style=style, keep=keep if k < count - 1 else last) for k in range(count)])


def document() -> str:
    nested = _table([_row("Inner", k, keep=_KEEP if k == 0 else "") for k in range(2)], width=2000)
    body = "".join([
        p("Table keeps", "5A100001", style="Heading1"),
        _para("Tables kept on one page by their rows' keeps."),
        _kept("Direct", 3),
        _para("A paragraph that does not keep with the next."),
        _kept("With next", 3, last=_KEEP),
        _para("A caption kept with the table.", keep=_KEEP),
        _kept("Styled", 3, style="Label", keep="", last=""),
        _para("Between."),
        _table([_row("Alone", k, keep=_KEEP if k == 2 else "") for k in range(3)]),
        _para("Between."),
        _table([_row("Single", 0)]),
        _para("Between."),
        _kept("Released", 2, style="Label", keep="", last=_RELEASE),
        _para("Between."),
        _table([_row("Outer", 0, first=nested + _para("After the inner table.")), _row("Outer", 1)]),
        _para("Between."),
        _kept("Tall", 60),
        _para("The end."),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
    ])
    return f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>"


_WRITER = 'w:author="Writer A" w:date="2026-11-17T10:00:00Z"'
_REVISIONS = iter(range(900, 10_000))


def _row_of(texts: list[str], *, inserted: bool = False, formula: bool = False) -> str:
    """A row of ``texts`` with its own paraId; ``inserted``: wholly Writer A's pending tracked
    insertion (the row's, each paragraph mark's and run's); ``formula``: its second cell a
    ``=SUM(ABOVE)`` field."""
    cells = []
    for k, text in enumerate(texts):
        para = f"5B{next(_COUNTER):06X}"
        run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
        if formula and k == 1:
            run = f'<w:fldSimple w:instr=" =SUM(ABOVE) "><w:r><w:t>{text or "0"}</w:t></w:r></w:fldSimple>'
        props = ""
        if inserted:
            props = f'<w:pPr><w:rPr><w:ins w:id="{next(_REVISIONS)}" {_WRITER}/></w:rPr></w:pPr>'
            run = f'<w:ins w:id="{next(_REVISIONS)}" {_WRITER}>{run}</w:ins>' if run else ""
        cells.append(_cell(f'<w:p w14:paraId="{para}" w14:textId="77777777">{props}{run}</w:p>', 3000))
    row_id = f"5B{next(_COUNTER):06X}"
    record = f'<w:trPr><w:ins w:id="{next(_REVISIONS)}" {_WRITER}/></w:trPr>' if inserted else ""
    return f'<w:tr w14:paraId="{row_id}" w14:textId="77777777">{record}{"".join(cells)}</w:tr>'


def _placed_table(rows: list[str], columns: int) -> str:
    grid = "".join('<w:gridCol w:w="3000"/>' for _ in range(columns))
    return ('<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/>'
            '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" '
            f'w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>' + "".join(rows) + "</w:tbl>")


def placed() -> tuple[str, str]:
    """``tables-placed.docx``'s document and its provenance item."""
    first = [_row_of(["Code", "Name"]), _row_of(["K-1", "Item 1"]), _row_of(["K-2", "Item 2"], inserted=True),
             _row_of(["K-3", "Item 3"]), _row_of(["", ""]), _row_of(["K-4", "Item 4"]),
             _row_of(["Total", "4"], formula=True)]
    second = [_row_of(["Placed", "", ""]), _row_of(["Region", "Note", "Units"]), _row_of(["North", "a", "12"]),
              _row_of(["South", "b", "7.5"]), _row_of(["Below", "", ""])]
    keys = [row.split('w14:paraId="', 1)[1][:8] for row in (first[0], second[0])]
    body = "".join([
        p("Tables placed from a source", "5B100001", style="Heading1"),
        _para("Placed from d:long."),
        _placed_table(first, 2),
        _para("Placed from d:sales."),
        _placed_table(second, 3),
        _para("The end."),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
    ])
    records = [
        (keys[0], '{"v":1,"source":"d:long/#1:#4","header_row":0,"first_row":1,"columns":['
                  '{"col":0,"column":"Code","rows":"1-4"},{"col":1,"column":"Name","rows":"1-4"}]}'),
        (keys[1], '{"v":1,"source":"d:sales/#1:#2","header_row":1,"first_row":2,"columns":['
                  '{"col":0,"column":"Region","rows":"1-2"},{"col":2,"column":"Units","rows":"1-2"}]}'),
    ]
    item = (DECL + '<prov:sources xmlns:prov="urn:uvrt:docx-agent:provenance:v1">'
            + "".join(f'<prov:source kind="table" key="t:{key}">{text}</prov:source>' for key, text in records)
            + "</prov:sources>")
    return f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>", item


def main_placed() -> None:
    document_xml, item = placed()
    provenance = "urn:uvrt:docx-agent:provenance:v1"
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "customXml/itemProps1.xml": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rId1", REL + "styles", "styles.xml", False),
            ("rId2", REL + "settings", "settings.xml", False),
            ("rId3", REL + "customXml", "../customXml/item1.xml", False),
        ]),
        "word/document.xml": document_xml,
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15),
        "customXml/item1.xml": item,
        "customXml/itemProps1.xml": (
            DECL + '<ds:datastoreItem xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml" '
            'ds:itemID="{5B000000-0000-4000-8000-000000000001}"><ds:schemaRefs>'
            f'<ds:schemaRef ds:uri="{provenance}"/></ds:schemaRefs></ds:datastoreItem>'),
        "customXml/_rels/item1.xml.rels": relationships([("rId1", REL + "customXmlProps", "itemProps1.xml", False)]),
    }
    print(write("tables-placed.docx", parts))


def main() -> None:
    label = ('<w:style w:type="paragraph" w:customStyle="1" w:styleId="Label"><w:name w:val="Label"/>'
             '<w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/></w:pPr><w:rPr><w:b/></w:rPr></w:style>')
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rId1", REL + "styles", "styles.xml", False),
            ("rId2", REL + "settings", "settings.xml", False),
        ]),
        "word/document.xml": document(),
        "word/styles.xml": styles().replace("</w:styles>", label + "</w:styles>"),
        "word/settings.xml": settings(15),
    }
    print(write("table-keeps.docx", parts))
    main_placed()


if __name__ == "__main__":
    main()
