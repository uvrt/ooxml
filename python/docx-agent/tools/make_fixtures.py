#!/usr/bin/env python3
"""Write docx-agent's own fixtures, ``tests/fixtures/generated/*.docx``.

The reused fixtures (docx2svg's, samplelib's, wordto's) have no ``w14:paraId`` at all and
almost none of the markup a reader of text has to walk.  These two documents are written
here, by hand, so their licence is this repository's and every id edge case is present on
purpose:

``ids-and-markup.docx`` (compatibility mode 15)
    paraIds as Word writes them, missing, repeated three times, out of range, and one
    without a textId; a hyperlink, a complex field and a simple field (cached results), an
    inline and a block-level content control, an insertion and a deletion by another
    author, a move, a bookmark across two paragraphs, a tab, line and page breaks, a
    symbol and the special hyphens; a table with a horizontal and a vertical merge and
    paraIds on its rows; a header and a footer; a footnote; a comment.

``mode14.docx``
    a few paragraphs with paraIds in compatibility mode 14, to hold edits to a document
    below mode 15 (which an edit must never upgrade).

``controls.docx`` (compatibility mode 15)
    content controls of every kind as Word 365 writes them: plain text showing its
    placeholder, rich text with a title and a tag, a drop-down list (items whose display
    text and value differ), a combo box holding text of its own, a date in its picture, a
    checked check box, content-locked and control-locked ones, one holding a tracked
    insertion, a data-bound one (its custom XML part), a block-level one of two paragraphs
    with a bookmark displaced around it, one in a table cell, and one in the header.

``lists-and-styles.docx`` (compatibility mode 15)
    what E1's edits work on: a template localised to Dutch (``Kop1`` is "heading 1",
    ``Standaard`` is Normal, ``Zwaar`` is Strong), numbered lists over one abstract
    definition -- one continuing, one restarted with a ``w:startOverride`` -- a list a
    paragraph style gives (``Lijstopsomteken``, List Bullet), an unused list instance, a
    paragraph whose formatting changes mid-word with runs split by rsids and proofing marks,
    an external and an internal hyperlink, a bookmark, and an inline picture (a PNG written
    here).

``review-follow.docx`` (compatibility mode 15)
    what a review settles beyond its records: data-bound controls showing tracked changes
    (text, a drop-down list, a date whose node lacks its time), and a table with tracked
    cell insertions that a comment describing them is anchored in, beside an ordinary
    comment.
``own-tracked-note.docx`` (compatibility mode 15)
    a footnote of three paragraphs, it and its reference tracked insertions by the
    differential harness's own author ("Writer A"): a tracked edit of the note deletes those
    insertions outright, and the edit's new records reuse their ids as docx-agent numbers
    them.

Deterministic: the zip entries carry a fixed timestamp, so running this again writes the
same bytes.

    python tools/make_fixtures.py
"""

from __future__ import annotations

import zipfile
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "generated"

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
NS = f'xmlns:w="{W}" xmlns:r="{R}" xmlns:w14="{W14}" xmlns:mc="{MC}" mc:Ignorable="w14"'
DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
WML = "application/vnd.openxmlformats-officedocument.wordprocessingml"
DATE = (2026, 10, 3, 12, 0, 0)
AUTHOR = 'w:author="Reviewer" w:date="2026-10-01T09:00:00Z"'


def p(text: str = "", para: str | None = None, textid: str | None = "77777777", *,
      style: str | None = None, inner: str | None = None) -> str:
    ids = ""
    if para is not None:
        ids = f' w14:paraId="{para}"' + (f' w14:textId="{textid}"' if textid else "")
    props = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""
    body = inner if inner is not None else (f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else "")
    return f"<w:p{ids}>{props}{body}</w:p>"


def r(text: str, props: str = "") -> str:
    return f'<w:r>{f"<w:rPr>{props}</w:rPr>" if props else ""}<w:t xml:space="preserve">{text}</w:t></w:r>'


def ids_and_markup() -> dict[str, str]:
    body = "".join([
        p("Identifiers and markup", "1A2B3C01", style="Heading1"),
        p(inner=r("Plain ") + r("bold", "<w:b/>") + r(" and ") + r("italic", "<w:i/>") + r(" runs."),
          para="1A2B3C02"),
        p("A paragraph with no paraId."),
        p("First of three repeats.", "2B3C4D05"),
        p("Second of three repeats.", "2B3C4D05"),
        p("Third of three repeats.", "2B3C4D05"),
        p("Out of range paraId.", "80000001"),
        p("A paraId without a textId.", "1A2B3C08", None),
        # A hyperlink (external) around two runs.
        p(inner=r("Visit ") + '<w:hyperlink r:id="rIdLink" w:history="1">'
          + r("the ", '<w:rStyle w:val="Hyperlink"/>') + r("site", '<w:rStyle w:val="Hyperlink"/><w:b/>')
          + "</w:hyperlink>" + r(" today."), para="1A2B3C09"),
        # A complex field: the instruction is not text, the result is.
        p(inner=r("Page ") + '<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
          + '<w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>'
          + '<w:r><w:fldChar w:fldCharType="separate"/></w:r>' + r("1")
          + '<w:r><w:fldChar w:fldCharType="end"/></w:r>' + r(" of the document."), para="1A2B3C0A"),
        # A simple field.
        p(inner=r("Author: ") + '<w:fldSimple w:instr=" AUTHOR \\* MERGEFORMAT ">' + r("Someone")
          + "</w:fldSimple>", para="1A2B3C0B"),
        # An inline content control.
        p(inner=r("Name: ") + '<w:sdt><w:sdtPr><w:id w:val="101"/><w:text/></w:sdtPr><w:sdtContent>'
          + r("Ada Lovelace") + "</w:sdtContent></w:sdt>" + r("."), para="1A2B3C0C"),
        # Tracked changes by another author: the current view has the insertion, not the deletion.
        p(inner=r("Revenue grew ") + f'<w:del w:id="201" {AUTHOR}><w:r><w:delText>12</w:delText></w:r></w:del>'
          + f'<w:ins w:id="202" {AUTHOR}>' + r("14") + "</w:ins>" + r("% this year."), para="1A2B3C0D"),
        # A move: from here ...
        p(inner=r("Moved from: ") + f'<w:moveFromRangeStart w:id="203" w:name="move1" {AUTHOR}/>'
          + f'<w:moveFrom w:id="204" {AUTHOR}><w:r><w:t>travelling words</w:t></w:r></w:moveFrom>'
          + '<w:moveFromRangeEnd w:id="203"/>', para="1A2B3C0E"),
        # ... to here.
        p(inner=r("Moved to: ") + f'<w:moveToRangeStart w:id="205" w:name="move1" {AUTHOR}/>'
          + f'<w:moveTo w:id="206" {AUTHOR}><w:r><w:t>travelling words</w:t></w:r></w:moveTo>'
          + '<w:moveToRangeEnd w:id="205"/>', para="1A2B3C0F"),
        # A bookmark across two paragraphs, and a footnote reference.
        p(inner='<w:bookmarkStart w:id="0" w:name="span"/>' + r("A bookmark starts here"),
          para="1A2B3C10"),
        p(inner=r("and ends here.") + '<w:bookmarkEnd w:id="0"/>'
          + '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="1"/></w:r>',
          para="1A2B3C11"),
        # Special characters.
        p(inner='<w:r><w:t>Tab</w:t><w:tab/><w:t>then line</w:t><w:br/><w:t>break, non</w:t>'
          '<w:noBreakHyphen/><w:t>breaking, soft</w:t><w:softHyphen/><w:t xml:space="preserve">hyphen, symbol </w:t>'
          '<w:sym w:font="Symbol" w:char="F061"/><w:t>.</w:t></w:r>', para="1A2B3C12"),
        # A comment on a word.
        p(inner=r("A ") + '<w:commentRangeStart w:id="1"/>' + r("commented")
          + '<w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r>' + r(" word."),
          para="1A2B3C13"),
        # A block-level content control.
        '<w:sdt><w:sdtPr><w:id w:val="102"/><w:richText/></w:sdtPr><w:sdtContent>'
        + p("Inside a block content control.", "1A2B3C14") + "</w:sdtContent></w:sdt>",
        # A table: a horizontal merge in the first row, a vertical one in the first column.
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/>'
        '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" '
        'w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/>'
        '<w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
        '<w:tr w14:paraId="3C4D5E01" w14:textId="77777777">'
        '<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge w:val="restart"/></w:tcPr>'
        + p("Merged down", "3C4D5E02") + '</w:tc>'
        '<w:tc><w:tcPr><w:tcW w:w="6000" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr>'
        + p("Merged across", "3C4D5E03") + '</w:tc></w:tr>'
        '<w:tr w14:paraId="3C4D5E04" w14:textId="77777777">'
        '<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge/></w:tcPr>' + p("", "3C4D5E05") + '</w:tc>'
        '<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>' + p("B2", "3C4D5E06") + '</w:tc>'
        '<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>' + p("C2", "3C4D5E07") + '</w:tc></w:tr>'
        '</w:tbl>',
        p("After the table.", "1A2B3C15"),
        p(inner='<w:r><w:br w:type="page"/></w:r>' + r("Second page."), para="1A2B3C16"),
        p("The last paragraph.", "1A2B3C17"),
        '<w:sectPr><w:headerReference w:type="default" r:id="rIdHeader"/>'
        '<w:footerReference w:type="default" r:id="rIdFooter"/>'
        '<w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" '
        'w:footer="708" w:gutter="0"/><w:cols w:space="708"/></w:sectPr>',
    ])
    document = f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>"
    header = f"{DECL}<w:hdr {NS}>" + p("A header.", "4D5E6F01", style="Header") + "</w:hdr>"
    footer = (f"{DECL}<w:ftr {NS}>"
              + p(inner=r("Page ") + '<w:fldSimple w:instr=" PAGE ">' + r("1") + "</w:fldSimple>",
                  para="4D5E6F02") + "</w:ftr>")
    footnotes = (
        f"{DECL}<w:footnotes {NS}>"
        '<w:footnote w:type="separator" w:id="-1">' + p(inner='<w:r><w:separator/></w:r>', para="5E6F7001")
        + '</w:footnote><w:footnote w:type="continuationSeparator" w:id="0">'
        + p(inner='<w:r><w:continuationSeparator/></w:r>', para="5E6F7002") + '</w:footnote>'
        '<w:footnote w:id="1"><w:p w14:paraId="5E6F7003" w14:textId="77777777"><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr>'
        '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r>'
        + r(" A footnote.") + "</w:p></w:footnote></w:footnotes>"
    )
    comments = (f"{DECL}<w:comments {NS}>"
                '<w:comment w:id="1" w:author="Reviewer" w:date="2026-10-01T09:00:00Z" w:initials="RV">'
                + p(inner='<w:r><w:annotationRef/></w:r>' + r("A comment."), para="6F708001")
                + "</w:comment></w:comments>")
    return {
        "word/document.xml": document,
        "word/header1.xml": header,
        "word/footer1.xml": footer,
        "word/footnotes.xml": footnotes,
        "word/comments.xml": comments,
    }


def sdt(properties: str, content: str, end: bool = False) -> str:
    """A content control: ``w:sdtPr`` holding ``properties``, then ``content``."""
    return (f"<w:sdt><w:sdtPr>{properties}</w:sdtPr>" + ("<w:sdtEndPr/>" if end else "")
            + f"<w:sdtContent>{content}</w:sdtContent></w:sdt>")


GOTHIC = '<w:rFonts w:ascii="MS Gothic" w:eastAsia="MS Gothic" w:hAnsi="MS Gothic" w:hint="eastAsia"/>'
CHECKBOX = ('<w14:checkbox><w14:checked w14:val="1"/><w14:checkedState w14:val="2612" w14:font="MS Gothic"/>'
            '<w14:uncheckedState w14:val="2610" w14:font="MS Gothic"/></w14:checkbox>')
STORE = "{5C1A2B3D-0000-4000-8000-00000000C0DE}"


def controls() -> dict[str, str]:
    """Every kind of content control, as Word 365 writes them, and where they stand."""
    items = ('<w:listItem w:displayText="One" w:value="a"/><w:listItem w:displayText="Two" w:value="b"/>'
             '<w:listItem w:displayText="Three" w:value="c"/>')
    body = "".join([
        p("Content controls", "7A000001", style="Heading1"),
        p(inner=r("Name: ") + sdt('<w:rPr><w:rStyle w:val="PlaceholderText"/></w:rPr><w:id w:val="301"/>'
                                  '<w:showingPlcHdr/><w:text/>',
                                  r("Click or tap here to enter text.", '<w:rStyle w:val="PlaceholderText"/>'), True)
          + r("."), para="7A000002"),
        p(inner=r("Notes: ") + sdt('<w:alias w:val="Notes"/><w:tag w:val="notes"/><w:id w:val="302"/><w:richText/>',
                                   r("Bold", "<w:b/>") + r(" and plain")), para="7A000003"),
        p(inner=r("Choice: ") + sdt(f'<w:id w:val="303"/><w:dropDownList>{items}</w:dropDownList>', r("One")),
          para="7A000004"),
        p(inner=r("Colour: ") + sdt('<w:id w:val="304"/><w:comboBox><w:listItem w:displayText="Red" w:value="Red"/>'
                                    '<w:listItem w:displayText="Green" w:value="Green"/></w:comboBox>', r("Blue")),
          para="7A000005"),
        p(inner=r("Date: ") + sdt('<w:id w:val="305"/><w:date w:fullDate="2026-01-15T00:00:00Z">'
                                  '<w:dateFormat w:val="d MMMM yyyy"/><w:lid w:val="en-GB"/>'
                                  '<w:storeMappedDataAs w:val="dateTime"/><w:calendar w:val="gregorian"/></w:date>',
                                  r("15 January 2026")), para="7A000006"),
        p(inner=r("Done: ") + sdt(f'<w:rPr>{GOTHIC}</w:rPr><w:id w:val="306"/>{CHECKBOX}',
                                  r("\u2612", GOTHIC), True), para="7A000007"),
        p(inner=r("Locked: ") + sdt('<w:id w:val="307"/><w:lock w:val="contentLocked"/><w:text/>', r("Fixed"))
          + r(" and ") + sdt('<w:id w:val="308"/><w:lock w:val="sdtLocked"/><w:text/>', r("Kept")), para="7A000008"),
        p(inner=r("Reviewed: ") + sdt('<w:id w:val="309"/><w:text/>',
                                      f'<w:ins w:id="401" {AUTHOR}>' + r("Inserted words") + "</w:ins>"),
          para="7A000009"),
        p(inner=r("Bound: ") + sdt(f'<w:id w:val="310"/><w:dataBinding w:prefixMappings="xmlns:ns0=\'urn:fixture\'" '
                                   f'w:xpath="/ns0:root[1]/ns0:client[1]" w:storeItemID="{STORE}"/><w:text/>',
                                   r("Acme")), para="7A00000A"),
        '<w:bookmarkStart w:id="20" w:name="around" w:displacedByCustomXml="next"/>'
        + sdt('<w:alias w:val="Summary"/><w:id w:val="311"/><w:richText/>',
              p("First paragraph inside.", "7A00000B") + p("Second paragraph inside.", "7A00000C"))
        + '<w:bookmarkEnd w:id="20" w:displacedByCustomXml="prev"/>',
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
        '<w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>'
        '<w:tr w14:paraId="7B000001" w14:textId="77777777"><w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>'
        + p("Item", "7B000002") + '</w:tc><w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>'
        + p(inner=sdt('<w:id w:val="312"/><w:text/>', r("In a cell")), para="7B000003") + '</w:tc></w:tr>'
        + '</w:tbl>',
        p("After the controls.", "7A00000D"),
        '<w:sectPr><w:headerReference w:type="default" r:id="rIdHeader"/><w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" '
        'w:gutter="0"/></w:sectPr>',
    ])
    header = (f"{DECL}<w:hdr {NS}>" + p(inner=r("Client ") + sdt('<w:id w:val="314"/><w:text/>', r("Header name")),
                                        para="7C000001", style="Header") + "</w:hdr>")
    item = f'{DECL}<ns0:root xmlns:ns0="urn:fixture"><ns0:client>Acme</ns0:client></ns0:root>'
    props = (f'{DECL}<ds:datastoreItem ds:itemID="{STORE}" '
             'xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml">'
             '<ds:schemaRefs/></ds:datastoreItem>')
    return {
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
        "word/header1.xml": header,
        "customXml/item1.xml": item,
        "customXml/itemProps1.xml": props,
        "customXml/_rels/item1.xml.rels": relationships([
            ("rId1", REL + "customXmlProps", "itemProps1.xml", False)]),
    }



FOLLOW_STORE = "{5C1A2B3D-0000-4000-8000-00000000F011}"


def review_follow() -> dict[str, str]:
    """What a review settles beyond its records: data-bound controls showing tracked
    changes (text, a drop-down list, a date whose node lacks its time), and a table with a
    tracked cell insertion that a comment written to describe it is anchored in, beside
    an ordinary comment."""
    def bound(node: str) -> str:
        # prefix "fl", not Word's "ns0": test_ids reads a stray ns0 near the head as a leak
        return (f'<w:dataBinding w:prefixMappings="xmlns:fl=\'urn:follow\'" w:xpath="/fl:root[1]/fl:{node}[1]" '
                f'w:storeItemID="{FOLLOW_STORE}"/>')

    items = '<w:listItem w:displayText="One" w:value="a"/><w:listItem w:displayText="Two" w:value="b"/>'
    deleted = '<w:del w:id="{0}" ' + AUTHOR + '><w:r><w:delText xml:space="preserve">{1}</w:delText></w:r></w:del>'
    cell = '<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>{0}</w:tcPr>{1}</w:tc>'
    body = "".join([
        p("Review follow-ups", "7D000001", style="Heading1"),
        # plain text and a plain table first: what an edit set over the document works on
        p("Plain paragraphs come first in this document.", "7D000010"),
        p("Another plain paragraph follows the first one.", "7D000011"),
        p("A third plain paragraph keeps the opening going.", "7D000012"),
        p("Fourth plain words stand here for edits.", "7D000013"),
        p("Fifth plain words wait to be deleted.", "7D000014"),
        p("Sixth plain paragraph closes the opening.", "7D000015"),
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
        '<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
        '<w:tr w14:paraId="7E000011" w14:textId="77777777">'
        + cell.format("", p("North", "7E000012")) + cell.format("", p("South", "7E000013")) + "</w:tr>"
        '<w:tr w14:paraId="7E000014" w14:textId="77777777">'
        + cell.format("", p("East", "7E000015")) + cell.format("", p("West", "7E000016")) + "</w:tr></w:tbl>",
        p(inner=r("Client: ") + sdt('<w:id w:val="320"/>' + bound("client") + "<w:text/>",
                                    r("Acme") + f'<w:ins w:id="501" {AUTHOR}>' + r(" Corp") + "</w:ins>"
                                    + deleted.format(502, " Ltd")), para="7D000002"),
        p(inner=r("Grade: ") + sdt(f'<w:id w:val="321"/>{bound("grade")}<w:dropDownList>{items}</w:dropDownList>',
                                   deleted.format(503, "One") + f'<w:ins w:id="504" {AUTHOR}>' + r("Two") + "</w:ins>"),
          para="7D000003"),
        p(inner=r("Due: ") + sdt(f'<w:id w:val="322"/>{bound("due")}<w:date w:fullDate="2026-03-02T00:00:00Z">'
                                 '<w:dateFormat w:val="d-M-yyyy"/><w:lid w:val="en-GB"/>'
                                 '<w:storeMappedDataAs w:val="dateTime"/><w:calendar w:val="gregorian"/></w:date>',
                                 r("2-3-2026")) + f'<w:ins w:id="505" {AUTHOR}>' + r(" (moved)") + "</w:ins>",
          para="7D000004"),
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
        '<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
        '<w:tr w14:paraId="7E000001" w14:textId="77777777">'
        + cell.format("", p(inner='<w:commentRangeStart w:id="1"/>' + r("Name")
                            + '<w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r>', para="7E000002"))
        + cell.format(f'<w:cellIns w:id="506" {AUTHOR}/>', p("Added", "7E000003")) + "</w:tr>"
        '<w:tr w14:paraId="7E000004" w14:textId="77777777">'
        + cell.format("", p(inner=r("Row ") + '<w:commentRangeStart w:id="2"/>' + r("two")
                            + '<w:commentRangeEnd w:id="2"/><w:r><w:commentReference w:id="2"/></w:r>'
                            + f'<w:ins w:id="508" {AUTHOR}>' + r(" and more") + "</w:ins>", para="7E000005"))
        + cell.format(f'<w:cellIns w:id="507" {AUTHOR}/>', p("Added too", "7E000006")) + "</w:tr></w:tbl>",
        p("The document ends with this paragraph.", "7D000016"),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
        'w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
    ])

    def comment(number: int, text: str, para: str) -> str:
        return (f'<w:comment w:id="{number}" w:author="Reviewer" w:date="2026-10-01T09:00:00Z" w:initials="RV">'
                + p(inner='<w:r><w:annotationRef/></w:r>' + r(text), para=para) + "</w:comment>")

    comments = (f"{DECL}<w:comments {NS}>"
                + comment(1, "Tracked structural change: a column inserted.", "7F000001")
                + comment(2, "An ordinary remark.", "7F000002") + "</w:comments>")
    # each node what its control shows now (Word keeps them so); a review changes them
    item = (f'{DECL}<ns0:root xmlns:ns0="urn:follow"><ns0:client>Acme Corp</ns0:client><ns0:grade>b</ns0:grade>'
            '<ns0:due>2026-03-02T00:00:00</ns0:due></ns0:root>')
    props = (f'{DECL}<ds:datastoreItem ds:itemID="{FOLLOW_STORE}" '
             'xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml">'
             '<ds:schemaRefs/></ds:datastoreItem>')
    return {
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
        "word/comments.xml": comments,
        "customXml/item1.xml": item,
        "customXml/itemProps1.xml": props,
        "customXml/_rels/item1.xml.rels": relationships([
            ("rId1", REL + "customXmlProps", "itemProps1.xml", False)]),
    }


#: The author and date of the harness's tracked edits (word_edits_diff.py's TRACKING).
WRITER = 'w:author="Writer A" w:date="2026-11-17T10:00:00Z"'


def own_tracked_note() -> dict[str, str]:
    """A footnote of three paragraphs, it and its reference inserted as tracked changes by
    the harness's own author: editing it tracked deletes those insertions outright."""
    body = "".join([
        p("A note of one's own", "7F100001", style="Heading1"),
        p(inner=r("The claim") + f'<w:ins w:id="509" {WRITER}><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/>'
          '</w:rPr><w:footnoteReference w:id="1"/></w:r></w:ins>' + r(" stands here."), para="7F100002"),
        p("After the note.", "7F100003"),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
        'w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
    ])

    def noted(para: str, mark: int | None, record: int, inner: str) -> str:
        marked = f'<w:rPr><w:ins w:id="{mark}" {WRITER}/></w:rPr>' if mark is not None else ""
        return (f'<w:p w14:paraId="{para}" w14:textId="77777777"><w:pPr><w:pStyle w:val="FootnoteText"/>{marked}'
                f'</w:pPr><w:ins w:id="{record}" {WRITER}>{inner}</w:ins></w:p>')

    footnotes = (
        f"{DECL}<w:footnotes {NS}>"
        '<w:footnote w:type="separator" w:id="-1">' + p(inner='<w:r><w:separator/></w:r>', para="7F100101")
        + '</w:footnote><w:footnote w:type="continuationSeparator" w:id="0">'
        + p(inner='<w:r><w:continuationSeparator/></w:r>', para="7F100102") + '</w:footnote><w:footnote w:id="1">'
        + noted("7F100103", 511, 510, '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/>'
                '</w:r>' + r(" First"))
        + noted("7F100104", 513, 512, r("Second")) + noted("7F100105", None, 514, r("Third"))
        + "</w:footnote></w:footnotes>"
    )
    return {"word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
            "word/footnotes.xml": footnotes}


def styles() -> str:
    def style(kind: str, sid: str, name: str, extra: str = "", based: str | None = "Normal",
              after: str | None = None) -> str:
        # CT_Style is a sequence: name, basedOn, next, ..., qFormat, ..., pPr, rPr.
        basis = f'<w:basedOn w:val="{based}"/>' if based else ""
        following = f'<w:next w:val="{after}"/>' if after else ""
        return (f'<w:style w:type="{kind}" w:styleId="{sid}"><w:name w:val="{name}"/>{basis}'
                f'{following}<w:qFormat/>{extra}</w:style>')

    return (
        f'{DECL}<w:styles xmlns:w="{W}">'
        '<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" '
        'w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/>'
        '<w:lang w:val="en-GB" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>'
        '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr>'
        '</w:pPrDefault></w:docDefaults>'
        '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>'
        '<w:qFormat/></w:style>'
        '<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont">'
        '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/>'
        '<w:unhideWhenUsed/></w:style>'
        '<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>'
        '<w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/>'
        '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar>'
        '</w:tblPr></w:style>'
        + style("paragraph", "Heading1", "heading 1",
                '<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="0"/>'
                '<w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr>',
                after="Normal")
        + style("paragraph", "Header", "header")
        + style("paragraph", "FootnoteText", "footnote text",
                '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
                '<w:rPr><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr>')
        + style("character", "FootnoteReference", "footnote reference",
                '<w:rPr><w:vertAlign w:val="superscript"/></w:rPr>', based="DefaultParagraphFont")
        + style("character", "Hyperlink", "Hyperlink",
                '<w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr>', based="DefaultParagraphFont")
        + '<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/>'
        '<w:basedOn w:val="TableNormal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
        '<w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
        '<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style>'
        "</w:styles>"
    )


def settings(mode: int, notes: bool = False) -> str:
    separators = '<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>' if notes else ""
    return (f'{DECL}<w:settings xmlns:w="{W}"><w:defaultTabStop w:val="708"/>{separators}'
            '<w:compat><w:compatSetting w:name="compatibilityMode" '
            f'w:uri="http://schemas.microsoft.com/office/word" w:val="{mode}"/></w:compat>'
            "</w:settings>")


def content_types(extra: dict[str, str]) -> str:
    overrides = "".join(f'<Override PartName="/{part}" ContentType="{kind}"/>' for part, kind in extra.items())
    return (f'{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>' + overrides + "</Types>")


def relationships(rels: list[tuple[str, str, str, bool]]) -> str:
    items = "".join(
        f'<Relationship Id="{rid}" Type="{kind}" Target="{target}"'
        + (' TargetMode="External"' if external else "") + "/>"
        for rid, kind, target, external in rels)
    return (f'{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f"{items}</Relationships>")


def png(width: int = 4, height: int = 4, color: tuple[int, int, int] = (40, 120, 200)) -> bytes:
    """A small one-colour PNG, written here."""
    import struct
    import zlib

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(color) * width for _ in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
WP14 = "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"


def lists_and_styles() -> dict[str, str]:
    def listed(text: str, para: str, num: int, level: int = 0, style: str = "Lijstalinea") -> str:
        return (f'<w:p w14:paraId="{para}" w14:textId="77777777"><w:pPr><w:pStyle w:val="{style}"/>'
                f'<w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr>'
                f'{r(text)}</w:p>')

    picture = (
        f'<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0" xmlns:wp="{WP}" '
        f'xmlns:wp14="{WP14}" wp14:anchorId="2A3B4C01" wp14:editId="2A3B4C02">'
        '<wp:extent cx="457200" cy="457200"/><wp:docPr id="1" name="Picture 1" descr="A blue square"/>'
        f'<a:graphic xmlns:a="{A}"><a:graphicData uri="{PIC}"><pic:pic xmlns:pic="{PIC}">'
        '<pic:nvPicPr><pic:cNvPr id="1" name="square.png"/><pic:cNvPicPr/></pic:nvPicPr>'
        '<pic:blipFill><a:blip r:embed="rIdImage"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
        '<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="457200"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>'
        '</wp:inline></w:drawing></w:r>')
    body = "".join([
        p("Lijsten en stijlen", "5A000001", style="Kop1"),
        # Formatting that changes mid-word, runs split by rsids and a proofing mark.
        p(inner='<w:r w:rsidR="00A1B2C3"><w:t xml:space="preserve">Revenue grew </w:t></w:r>'
          '<w:r w:rsidR="00D4E5F6"><w:t xml:space="preserve">by </w:t></w:r>'
          + r("four", "<w:b/>") + r("teen", '<w:b/><w:i/>') + '<w:proofErr w:type="spellStart"/>'
          + r(" percnt", "") + '<w:proofErr w:type="spellEnd"/>' + r(" in the ")
          + r("third", '<w:color w:val="C00000"/><w:u w:val="single"/>') + r(" quarter."),
          para="5A000002"),
        listed("First item", "5A000003", 1),
        listed("Second item", "5A000004", 1),
        listed("A sub-item", "5A000005", 1, 1),
        p("Between the lists.", "5A000006"),
        listed("Restarted at one", "5A000007", 2),
        listed("Continues the restart", "5A000008", 2),
        p("Bullet from a style", "5A000009", style="Lijstopsomteken"),
        p("Another bullet from the style", "5A00000A", style="Lijstopsomteken"),
        p(inner=r("See ") + '<w:hyperlink r:id="rIdSite" w:history="1">'
          + r("the website", '<w:rStyle w:val="Hyperlink"/>') + "</w:hyperlink>"
          + r(" or ") + '<w:hyperlink w:anchor="target" w:history="1">'
          + r("the target", '<w:rStyle w:val="Hyperlink"/>') + "</w:hyperlink>" + r("."), para="5A00000B"),
        p(inner='<w:bookmarkStart w:id="0" w:name="target"/>' + r("The target paragraph")
          + '<w:bookmarkEnd w:id="0"/>' + r(" ends here."), para="5A00000C"),
        p(inner=r("A picture: ") + picture + r(" and text after it."), para="5A00000D"),
        p("The last paragraph.", "5A00000E"),
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>'
        '<w:cols w:space="708"/></w:sectPr>',
    ])
    numbering = (
        f'{DECL}<w:numbering xmlns:w="{W}">'
        '<w:abstractNum w:abstractNumId="0"><w:nsid w:val="1F2E3D4C"/><w:multiLevelType w:val="hybridMultilevel"/>'
        '<w:tmpl w:val="0413000F"/>'
        '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>'
        '<w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl>'
        '<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2."/>'
        '<w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl>'
        "</w:abstractNum>"
        '<w:abstractNum w:abstractNumId="1"><w:nsid w:val="5B6C7D8E"/><w:multiLevelType w:val="singleLevel"/>'
        '<w:tmpl w:val="04130001"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/>'
        '<w:pStyle w:val="Lijstopsomteken"/><w:lvlText w:val=""/><w:lvlJc w:val="left"/>'
        '<w:pPr><w:ind w:left="360" w:hanging="360"/></w:pPr>'
        '<w:rPr><w:rFonts w:ascii="Symbol" w:hAnsi="Symbol" w:hint="default"/></w:rPr></w:lvl></w:abstractNum>'
        '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'
        '<w:num w:numId="2"><w:abstractNumId w:val="0"/><w:lvlOverride w:ilvl="0">'
        '<w:startOverride w:val="1"/></w:lvlOverride></w:num>'
        '<w:num w:numId="3"><w:abstractNumId w:val="1"/></w:num>'
        '<w:num w:numId="4"><w:abstractNumId w:val="0"/></w:num>'
        "</w:numbering>"
    )
    return {"word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
            "word/numbering.xml": numbering}


def dutch_styles() -> str:
    """A template localised to Dutch: the style ids Dutch Word writes, the English names."""
    def style(kind: str, sid: str, name: str, extra: str = "", based: str | None = "Standaard") -> str:
        basis = f'<w:basedOn w:val="{based}"/>' if based else ""
        return (f'<w:style w:type="{kind}" w:styleId="{sid}"><w:name w:val="{name}"/>{basis}'
                f'<w:qFormat/>{extra}</w:style>')

    return (
        f'{DECL}<w:styles xmlns:w="{W}">'
        '<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" '
        'w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/>'
        '<w:lang w:val="nl-NL" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>'
        '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr>'
        '</w:pPrDefault></w:docDefaults>'
        '<w:style w:type="paragraph" w:default="1" w:styleId="Standaard"><w:name w:val="Normal"/>'
        '<w:qFormat/></w:style>'
        '<w:style w:type="character" w:default="1" w:styleId="Standaardalinea-lettertype">'
        '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/>'
        '<w:unhideWhenUsed/></w:style>'
        '<w:style w:type="table" w:default="1" w:styleId="Standaardtabel"><w:name w:val="Normal Table"/>'
        '<w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/>'
        '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar>'
        '</w:tblPr></w:style>'
        + style("paragraph", "Kop1", "heading 1",
                '<w:pPr><w:keepNext/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/></w:pPr>'
                '<w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr>')
        + style("paragraph", "Lijstalinea", "List Paragraph", '<w:pPr><w:ind w:left="720"/><w:contextualSpacing/></w:pPr>')
        + style("paragraph", "Lijstopsomteken", "List Bullet",
                '<w:pPr><w:numPr><w:numId w:val="3"/></w:numPr><w:contextualSpacing/></w:pPr>')
        + style("character", "Zwaar", "Strong", "<w:rPr><w:b/><w:bCs/></w:rPr>", based="Standaardalinea-lettertype")
        + style("character", "Nadruk", "Emphasis", "<w:rPr><w:i/><w:iCs/></w:rPr>", based="Standaardalinea-lettertype")
        + style("character", "Hyperlink", "Hyperlink",
                '<w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr>', based="Standaardalinea-lettertype")
        + "</w:styles>"
    )


def write(name: str, parts: dict[str, str]) -> Path:
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / name
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for part, data in parts.items():
            archive.writestr(zipfile.ZipInfo(part, date_time=DATE),
                             data if isinstance(data, bytes) else data.encode("utf-8"),
                             compress_type=zipfile.ZIP_DEFLATED)
    return path


def main() -> None:
    story = ids_and_markup()
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/header1.xml": WML + ".header+xml",
            "word/footer1.xml": WML + ".footer+xml",
            "word/footnotes.xml": WML + ".footnotes+xml",
            "word/comments.xml": WML + ".comments+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rIdStyles", REL + "styles", "styles.xml", False),
            ("rIdSettings", REL + "settings", "settings.xml", False),
            ("rIdHeader", REL + "header", "header1.xml", False),
            ("rIdFooter", REL + "footer", "footer1.xml", False),
            ("rIdFootnotes", REL + "footnotes", "footnotes.xml", False),
            ("rIdComments", REL + "comments", "comments.xml", False),
            ("rIdLink", REL + "hyperlink", "https://example.com/", True),
        ]),
        **story,
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15, notes=True),
    }
    print(write("ids-and-markup.docx", parts))

    body = "".join([
        p("A document in compatibility mode 14.", "0A0B0C01", style="Heading1"),
        p(inner=r("It has ") + r("formatted", "<w:b/>") + r(" runs and paraIds."), para="0A0B0C02"),
        p("No edit may upgrade its mode.", "0A0B0C03"),
        '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>',
    ])
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
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>",
        "word/styles.xml": styles(),
        "word/settings.xml": settings(14),
    }
    print(write("mode14.docx", parts))

    story = lists_and_styles()
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/numbering.xml": WML + ".numbering+xml",
        }).replace('<Default Extension="xml"', '<Default Extension="png" ContentType="image/png"/>'
                   '<Default Extension="xml"'),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rId1", REL + "styles", "styles.xml", False),
            ("rId2", REL + "settings", "settings.xml", False),
            ("rId3", REL + "numbering", "numbering.xml", False),
            ("rIdImage", REL + "image", "media/image1.png", False),
            ("rIdSite", REL + "hyperlink", "https://example.com/report", True),
        ]),
        **story,
        "word/styles.xml": dutch_styles(),
        "word/settings.xml": settings(15),
        "word/media/image1.png": png(),
    }
    print(write("lists-and-styles.docx", parts))

    story = controls()
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/header1.xml": WML + ".header+xml",
            "customXml/itemProps1.xml": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rIdStyles", REL + "styles", "styles.xml", False),
            ("rIdSettings", REL + "settings", "settings.xml", False),
            ("rIdHeader", REL + "header", "header1.xml", False),
            ("rIdCustom", REL + "customXml", "../customXml/item1.xml", False),
        ]),
        **story,
        "word/styles.xml": styles().replace(
            "</w:styles>", '<w:style w:type="character" w:styleId="PlaceholderText"><w:name w:val="Placeholder Text"/>'
            '<w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:color w:val="666666"/></w:rPr></w:style></w:styles>'),
        "word/settings.xml": settings(15),
    }
    print(write("controls.docx", parts))

    story = review_follow()
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/comments.xml": WML + ".comments+xml",
            "customXml/itemProps1.xml": "application/vnd.openxmlformats-officedocument.customXmlProperties+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rIdStyles", REL + "styles", "styles.xml", False),
            ("rIdSettings", REL + "settings", "settings.xml", False),
            ("rIdComments", REL + "comments", "comments.xml", False),
            ("rIdCustom", REL + "customXml", "../customXml/item1.xml", False),
        ]),
        **story,
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15),
    }
    print(write("review-follow.docx", parts))

    story = own_tracked_note()
    parts = {
        "[Content_Types].xml": content_types({
            "word/document.xml": WML + ".document.main+xml",
            "word/styles.xml": WML + ".styles+xml",
            "word/settings.xml": WML + ".settings+xml",
            "word/footnotes.xml": WML + ".footnotes+xml",
        }),
        "_rels/.rels": relationships([("rId1", REL + "officeDocument", "word/document.xml", False)]),
        "word/_rels/document.xml.rels": relationships([
            ("rIdStyles", REL + "styles", "styles.xml", False),
            ("rIdSettings", REL + "settings", "settings.xml", False),
            ("rIdFootnotes", REL + "footnotes", "footnotes.xml", False),
        ]),
        **story,
        "word/styles.xml": styles(),
        "word/settings.xml": settings(15, notes=True),
    }
    print(write("own-tracked-note.docx", parts))


if __name__ == "__main__":
    main()
