#!/usr/bin/env python3
"""Write ``tests/fixtures/generated/table-keeps.docx``: tables kept on one page by their rows'
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


if __name__ == "__main__":
    main()
