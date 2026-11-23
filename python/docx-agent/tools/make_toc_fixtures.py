#!/usr/bin/env python3
"""Write ``crates/ooxml-oracle/fixtures/generated/toc-numbered.docx``: a table of contents over numbered
headings, every way a numbered heading's entry is written (round 32, R12: the number, a tab
and a left stop after it; or the number, a space and the text), written here by hand with
:mod:`make_fixtures`' parts (compatibility mode 15, paraIds on every paragraph):

- a table of contents (``TOC \\o "1-3" \\h \\z \\u``) whose entries are out of date, in
  one paragraph;
- ``heading 1`` to ``heading 3`` numbered over one multilevel list: level 1 ``1.`` followed
  by a tab (Word's default), level 2 ``1.1`` by a space, level 3 ``1.1.1`` by nothing;
  ``heading 1`` and ``heading 3`` by their styles, ``heading 2`` by each paragraph's own
  ``w:numPr`` (its style unnumbered, so a heading 2 an edit adds is not);
- a ``toc 2`` style of the document's own (indented, in capitals), the others Word's;
- a numbered body list of its own before the headings (its count is not theirs);
- a heading whose numbering is removed directly (``w:numId`` 0), and one in a table cell.

Deterministic: running it again writes the same bytes.

    python tools/make_toc_fixtures.py

Since docx-agent 0.2.0 (P4e) the checkout has no Python ``Document``: this is the record of
how the fixtures were made. Run it against the frozen reference, docx-agent's Python at the
commit crates/ooxml-oracle/reference.toml names (installed as the gates install it:
.github/actions/gates-reference, local-gates.sh), from a ``git archive`` of that commit.
"""

from __future__ import annotations

from make_fixtures import DECL, NS, REL, W, WML, content_types, p, relationships, settings, styles, write

_COUNTER = iter(range(1, 10_000))
#: A heading 2's own list level (its style has none).
_H2 = '<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>'


def _para(text: str, *, style: str | None = None, props: str = "", inner: str | None = None) -> str:
    """A paragraph with its own paraId."""
    para = f"6B{next(_COUNTER):06X}"
    own = (f'<w:pStyle w:val="{style}"/>' if style else "") + props
    body = inner if inner is not None else f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'
    return (f'<w:p w14:paraId="{para}" w14:textId="77777777">'
            f'{f"<w:pPr>{own}</w:pPr>" if own else ""}{body}</w:p>')


def _toc() -> str:
    """A table of contents with one stale entry, in one paragraph (begin to end)."""
    instruction = ' TOC \\o "1-3" \\h \\z \\u '
    head = ('<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            f'<w:r><w:instrText xml:space="preserve">{instruction}</w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            '<w:r><w:t xml:space="preserve">Out of date</w:t></w:r>'
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>')
    return _para("", inner=head)


def _cell(content: str) -> str:
    return f'<w:tc><w:tcPr><w:tcW w:w="9026" w:type="dxa"/></w:tcPr>{content}</w:tc>'


def document() -> str:
    body = "".join([
        p("Contents", "6B100001"),
        _toc(),
        _para("A list of its own, before the headings:"),
        _para("First item", style="ListNumber"),
        _para("Second item", style="ListNumber"),
        _para("Introduction", style="Heading1"),
        _para("Some text."),
        _para("Background", style="Heading2", props=_H2),
        _para("Details", style="Heading3"),
        _para("More details", style="Heading3"),
        _para("Not numbered", style="Heading3", props='<w:numPr><w:ilvl w:val="2"/><w:numId w:val="0"/></w:numPr>'),
        _para("Method", style="Heading1"),
        _para("Approach", style="Heading2", props=_H2),
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
        '<w:tblGrid><w:gridCol w:w="9026"/></w:tblGrid>'
        f'<w:tr>{_cell(_para("In a table", style="Heading2", props=_H2))}</w:tr></w:tbl>',
        _para("Results", style="Heading1"),
        _para("The end."),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
    ])
    return f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>"


def _level(ilvl: int, text: str, style: str | None, suffix: str | None) -> str:
    suff = f'<w:suff w:val="{suffix}"/>' if suffix else ""
    linked = f'<w:pStyle w:val="{style}"/>' if style else ""
    indent = 432 + 144 * ilvl
    return (f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="decimal"/>'
            f'{linked}{suff}<w:lvlText w:val="{text}"/><w:lvlJc w:val="left"/>'
            f'<w:pPr><w:ind w:left="{indent}" w:hanging="{indent}"/></w:pPr></w:lvl>')


def numbering() -> str:
    headings = ('<w:abstractNum w:abstractNumId="0"><w:nsid w:val="6B000001"/>'
                '<w:multiLevelType w:val="multilevel"/>'
                + _level(0, "%1.", "Heading1", None)
                + _level(1, "%1.%2", None, "space")
                + _level(2, "%1.%2.%3", "Heading3", "nothing")
                + "</w:abstractNum>")
    items = ('<w:abstractNum w:abstractNumId="1"><w:nsid w:val="6B000002"/>'
             '<w:multiLevelType w:val="singleLevel"/>'
             '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/>'
             '<w:pStyle w:val="ListNumber"/><w:lvlText w:val="%1)"/><w:lvlJc w:val="left"/>'
             '<w:pPr><w:ind w:left="360" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>')
    return (f'{DECL}<w:numbering xmlns:w="{W}">{headings}{items}'
            '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'
            '<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>')


def _styles() -> str:
    def numbered(sid: str, name: str, ilvl: int, size: int) -> str:
        return (f'<w:style w:type="paragraph" w:styleId="{sid}"><w:name w:val="{name}"/>'
                '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>'
                f'<w:pPr><w:keepNext/><w:numPr><w:ilvl w:val="{ilvl}"/><w:numId w:val="1"/></w:numPr>'
                f'<w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="{ilvl}"/></w:pPr>'
                f'<w:rPr><w:b/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr></w:style>')

    heading2 = ('<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/>'
                '<w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>'
                '<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="1"/></w:pPr>'
                '<w:rPr><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style>')
    own = (heading2 + numbered("Heading3", "heading 3", 2, 24)
           + '<w:style w:type="paragraph" w:styleId="ListNumber"><w:name w:val="List Number"/>'
           '<w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:style>'
           '<w:style w:type="paragraph" w:styleId="TOC2"><w:name w:val="toc 2"/><w:basedOn w:val="Normal"/>'
           '<w:next w:val="Normal"/><w:uiPriority w:val="39"/><w:unhideWhenUsed/>'
           '<w:pPr><w:spacing w:after="100"/><w:ind w:left="220"/></w:pPr><w:rPr><w:caps/></w:rPr></w:style>')
    text = styles()
    # heading 1 numbered by its style too: its list level before its spacing (CT_PPr's order)
    text = text.replace('<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/>',
                        '<w:pPr><w:keepNext/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'
                        '<w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/>', 1)
    assert 'w:numId w:val="1"' in text
    return text.replace("</w:styles>", own + "</w:styles>")


def main() -> None:
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/numbering.xml": WML + ".numbering+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rId1", REL + "styles", "styles.xml", False),
            ("rId2", REL + "settings", "settings.xml", False),
            ("rId3", REL + "numbering", "numbering.xml", False),
        ]),
        "word/document.xml": document(),
        "word/styles.xml": _styles(),
        "word/settings.xml": settings(15),
        "word/numbering.xml": numbering(),
    }
    print(write("toc-numbered.docx", parts))


if __name__ == "__main__":
    main()
