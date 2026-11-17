"""A made-up long report with many tracked changes and comments, for the tracked-report bench
(``tools/tracked_report_bench.py``) and for timing one edit on it.

    python tools/tracked_report.py out.docx [--chapters 44] [--seed 1]
    python tools/tracked_report.py out.docx --accepted     # the same text, no revisions or comments
    python tools/tracked_report.py out.docx --fields       # with a table of contents and page references
    python tools/tracked_report.py out.docx --figures      # with charts, pictures and shapes

Each chapter: a level-1 heading, six sections (a level-2 heading, six body paragraphs of about
80 words, a table of ten rows by four columns), then an "End of chapter" paragraph.  Every
other table is formatted by hand (shading, bold, size and colour on its runs); the rest take
the ``TableGrid`` style.  Revisions from four authors: inserted and deleted runs, run and
paragraph formatting changes in the body and in table cells, and a tracked row in some tables;
a comment on one paragraph of each section.  With ``--accepted`` the same words with every
revision accepted and no comments.  With ``--fields``: a table of contents (a ``TOC`` field, its
result not yet made) after the title, a bookmark on each chapter's heading, and in each
chapter's end paragraph a ``PAGEREF`` to the next chapter's heading -- the fields a
``word_fields`` update lays the document out for.  With ``--figures``: in each chapter a bar
chart (its own chart part) after the second section's table, a picture after the fourth's, and
a coloured rectangle floating behind the chapter's heading, each with a caption where it is
inline -- the drawings a page's painter draws.  Words, authors and dates are made up; the output is the
same for the same arguments.  About 340 pages at the default 44 chapters (A4, Calibri 11).
"""

from __future__ import annotations

import argparse
import io
import random
import zipfile
from xml.sax.saxutils import escape

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
NS = (f'xmlns:w="{W}" xmlns:r="{R}" xmlns:w14="{W14}" xmlns:mc="{MC}" xmlns:wp="{WP}" xmlns:a="{A}" '
      'mc:Ignorable="w14"')
DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
WML = "application/vnd.openxmlformats-officedocument.wordprocessingml"
STAMP = (2026, 11, 1, 9, 0, 0)

WORDS = ("lantern meadow river stone harbour quiet orchard copper valley signal morning ledger window "
         "garden timber bridge market evening canvas thread pebble summit willow amber cedar heron "
         "pine falcon granite tide compass ember sparrow maple fern linen pewter cobalt saffron").split()
AUTHORS = ("Ada Morgan", "Ben Ortiz", "Cleo Hart", "Dev Rao")
SECTIONS = 6
PARAGRAPHS = 6
ROWS = 10
COLUMNS = 4


class _Ids:
    """Paragraph ids (``w14:paraId``) and revision/comment ids, unique and repeatable."""

    def __init__(self, rng: random.Random) -> None:
        self.rng = rng
        self.seen: set[int] = set()
        self.revision = 0

    def para(self) -> str:
        while True:
            value = self.rng.randrange(0x10000000, 0x7FFFFFFF)
            if value not in self.seen:
                self.seen.add(value)
                return f"{value:08X}"

    def next_revision(self) -> int:
        self.revision += 1
        return self.revision


def _stamp(rng: random.Random) -> str:
    author = rng.choice(AUTHORS)
    return f'w:author="{author}" w:date="2026-10-{rng.randint(1, 28):02d}T{rng.randint(8, 17):02d}:00:00Z"'


def _words(rng: random.Random, n: int) -> str:
    return " ".join(rng.choice(WORDS) for _ in range(n))


def _run(text: str, props: str = "") -> str:
    return f'<w:r>{f"<w:rPr>{props}</w:rPr>" if props else ""}<w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


class Report:
    """Builds the body, the comments part and the counts."""

    def __init__(self, seed: int, accepted: bool) -> None:
        self.rng = random.Random(seed)
        self.ids = _Ids(random.Random(seed + 1))
        self.accepted = accepted
        self.comments: list[str] = []
        self.counts = {"insertions": 0, "deletions": 0, "formatting": 0, "rows": 0, "comments": 0,
                       "paragraphs": 0, "tables": 0}
        #: With figures: each figure's part name, its relationship, content type and bytes.
        self.figures: list[tuple[str, str, str, bytes]] = []

    # -- runs carrying revisions --------------------------------------------------------------
    def _tracked_runs(self, text_words: int, props: str = "", rate: float = 1.0) -> str:
        """Plain runs with some inserted, deleted and reformatted ones among them."""
        rng = self.rng
        out: list[str] = []
        left = text_words
        while left > 0:
            n = min(left, rng.randint(6, 18))
            left -= n
            words = _words(rng, n) + " "
            roll = rng.random() / rate
            if self.accepted:
                if roll < 0.12:       # a deleted run is gone once accepted
                    continue
                out.append(_run(words, props))
            elif roll < 0.10:
                self.counts["insertions"] += 1
                out.append(f'<w:ins w:id="{self.ids.next_revision()}" {_stamp(rng)}>{_run(words, props)}</w:ins>')
            elif roll < 0.12:
                self.counts["deletions"] += 1
                out.append(f'<w:del w:id="{self.ids.next_revision()}" {_stamp(rng)}><w:r>'
                           f'{f"<w:rPr>{props}</w:rPr>" if props else ""}'
                           f'<w:delText xml:space="preserve">{escape(words)}</w:delText></w:r></w:del>')
            elif roll < 0.16:
                self.counts["formatting"] += 1
                out.append(_run(words, props + f'<w:b/><w:rPrChange w:id="{self.ids.next_revision()}" '
                                f'{_stamp(rng)}><w:rPr>{props}</w:rPr></w:rPrChange>'))
            else:
                out.append(_run(words, props))
        return "".join(out)

    def _paragraph(self, inner: str, style: str | None = None, ppr_extra: str = "",
                   changed: bool = False) -> str:
        self.counts["paragraphs"] += 1
        style_xml = f'<w:pStyle w:val="{style}"/>' if style else ""
        change = ""
        if changed and not self.accepted:
            self.counts["formatting"] += 1
            change = (f'<w:pPrChange w:id="{self.ids.next_revision()}" {_stamp(self.rng)}><w:pPr>'
                      f'{style_xml}</w:pPr></w:pPrChange>')
            ppr_extra = ppr_extra + '<w:jc w:val="both"/>'
        props = f"<w:pPr>{style_xml}{ppr_extra}{change}</w:pPr>" if (style_xml or ppr_extra or change) else ""
        return f'<w:p w14:paraId="{self.ids.para()}" w14:textId="77777777">{props}{inner}</w:p>'

    def _commented(self, inner: str) -> str:
        if self.accepted:
            return inner
        cid = len(self.comments)
        self.counts["comments"] += 1
        rng = self.rng
        author = rng.choice(AUTHORS)
        initials = "".join(part[0] for part in author.split())
        self.comments.append(
            f'<w:comment w:id="{cid}" w:author="{author}" w:date="2026-10-{rng.randint(1, 28):02d}T10:00:00Z" '
            f'w:initials="{initials}"><w:p w14:paraId="{self.ids.para()}" w14:textId="77777777">'
            '<w:pPr><w:pStyle w:val="CommentText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="CommentReference"/>'
            f'</w:rPr><w:annotationRef/></w:r>{_run(_words(rng, rng.randint(6, 20)).capitalize() + "?")}'
            '</w:p></w:comment>')
        return (f'<w:commentRangeStart w:id="{cid}"/>{inner}<w:commentRangeEnd w:id="{cid}"/>'
                f'<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="{cid}"/></w:r>')

    # -- blocks -----------------------------------------------------------------------------
    def _table(self, chapter: int, section: int) -> str:
        rng = self.rng
        self.counts["tables"] += 1
        by_hand = (chapter * SECTIONS + section) % 2 == 1
        widths = [2600, 2100, 2100, 2226]
        grid = "".join(f'<w:gridCol w:w="{w}"/>' for w in widths)
        if by_hand:
            tblpr = ('<w:tblPr><w:tblW w:w="9026" w:type="dxa"/><w:tblBorders>'
                     + "".join(f'<w:{side} w:val="single" w:sz="4" w:space="0" w:color="8EAADB"/>'
                               for side in ("top", "left", "bottom", "right", "insideH", "insideV"))
                     + '</w:tblBorders><w:tblLook w:val="0000"/></w:tblPr>')
        else:
            tblpr = ('<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="9026" w:type="dxa"/>'
                     '<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" '
                     'w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr>')
        rows = []
        for r in range(ROWS):
            header = r == 0
            cells = []
            for c in range(COLUMNS):
                text = (f"Column {c + 1}" if header else
                        (f"Row {r} {_words(rng, 2)}" if c == 0 else
                         (str(rng.randint(1, 999)) if c % 2 else _words(rng, rng.randint(1, 4)))))
                if by_hand:
                    shade = ('<w:shd w:val="clear" w:color="auto" w:fill="1F4E79"/>' if header else
                             ('<w:shd w:val="clear" w:color="auto" w:fill="DEEAF6"/>' if r % 2 else ""))
                    props = ('<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:cs="Arial"/>'
                             + ('<w:b/><w:color w:val="FFFFFF"/>' if header else '<w:color w:val="1F3864"/>')
                             + '<w:sz w:val="18"/><w:szCs w:val="18"/>')
                    ppr = '<w:spacing w:before="40" w:after="40"/>' + ('<w:jc w:val="right"/>' if c % 2 else "")
                else:
                    shade, props, ppr = "", "", ""
                tcpr = f'<w:tcPr><w:tcW w:w="{widths[c]}" w:type="dxa"/>{shade}</w:tcPr>'
                roll = rng.random()
                if self.accepted or header or roll > 0.18:
                    inner = _run(text, props)
                elif roll < 0.08:
                    self.counts["insertions"] += 1
                    inner = f'<w:ins w:id="{self.ids.next_revision()}" {_stamp(rng)}>{_run(text, props)}</w:ins>'
                elif roll < 0.12:
                    self.counts["deletions"] += 1
                    inner = (_run(text, props) + f'<w:del w:id="{self.ids.next_revision()}" {_stamp(rng)}><w:r>'
                             f'{f"<w:rPr>{props}</w:rPr>" if props else ""}<w:delText xml:space="preserve"> '
                             f'{escape(_words(rng, 2))}</w:delText></w:r></w:del>')
                else:
                    self.counts["formatting"] += 1
                    inner = _run(text, props + f'<w:i/><w:rPrChange w:id="{self.ids.next_revision()}" '
                                 f'{_stamp(rng)}><w:rPr>{props}</w:rPr></w:rPrChange>')
                style = "TableText"
                cells.append(f"<w:tc>{tcpr}{self._paragraph(inner, style, ppr)}</w:tc>")
            trpr = "<w:trPr><w:tblHeader/></w:trPr>" if header else ""
            if not header and not self.accepted and r == ROWS - 1 and rng.random() < 0.3:
                self.counts["rows"] += 1
                trpr = f'<w:trPr><w:ins w:id="{self.ids.next_revision()}" {_stamp(rng)}/></w:trPr>'
            rows.append(f"<w:tr>{trpr}{''.join(cells)}</w:tr>")
        return f"<w:tbl>{tblpr}<w:tblGrid>{grid}</w:tblGrid>{''.join(rows)}</w:tbl>"

    def _figure(self, part: str, kind: str, data: bytes) -> str:
        rid = f"rIdF{len(self.figures) + 1}"
        self.figures.append((part, rid, kind, data))
        return rid

    def body(self, chapters: int, fields: bool = False, figures: bool = False) -> str:
        rng = self.rng
        out: list[str] = [self._paragraph(_run("A made-up report"), "Title")]
        if fields:
            out.append(self._paragraph(_field(' TOC \\o "1-2" \\h \\z \\u ', "Contents to be made.")))
        for ch in range(chapters):
            heading = _run(f"Chapter {ch + 1} {_words(rng, 2).title()}")
            if fields:
                heading = (f'<w:bookmarkStart w:id="{9000 + ch}" w:name="chapter{ch + 1}"/>{heading}'
                           f'<w:bookmarkEnd w:id="{9000 + ch}"/>')
            if figures:
                heading += _shape_run(3 * ch + 1, rng.choice(("DEEAF6", "E2EFDA", "FFF2CC")))
            out.append(self._paragraph(heading, "Heading1"))
            for s in range(SECTIONS):
                out.append(self._paragraph(_run(f"Section {ch + 1}.{s + 1} {_words(rng, 3).title()}"), "Heading2"))
                for k in range(PARAGRAPHS):
                    inner = self._tracked_runs(rng.randint(60, 100))
                    if k == 2:
                        inner = self._commented(inner)
                    out.append(self._paragraph(inner, changed=rng.random() < 0.05))
                out.append(self._paragraph(_run(f"Table {ch + 1}.{s + 1} {_words(rng, 3)}"), "Caption"))
                out.append(self._table(ch, s))
                if figures and s == 1:
                    title = f"Figure {ch + 1}.1 {_words(rng, 2).title()}"
                    rid = self._figure(f"word/charts/chart{ch + 1}.xml", CHART_TYPE,
                                       _chart_part(rng, title).encode())
                    out.append(self._paragraph(_chart_run(rid, 3 * ch + 2)))
                    out.append(self._paragraph(_run(title), "Caption"))
                if figures and s == 3:
                    rgb = (rng.randint(40, 160), rng.randint(60, 160), rng.randint(90, 200))
                    rid = self._figure(f"word/media/image{ch + 1}.png", "image/png", _png(90, 60, rgb))
                    out.append(self._paragraph(_picture_run(rid, 3 * ch + 3)))
                    out.append(self._paragraph(_run(f"Figure {ch + 1}.2 {_words(rng, 2).title()}"), "Caption"))
            end = _run(f"End of chapter {ch + 1}.")
            if fields and ch + 1 < chapters:
                end += _run(" The next chapter starts on page ") + _field(f" PAGEREF chapter{ch + 2} \\h ", "1") + _run(".")
            out.append(self._paragraph(end))
        return "".join(out)


#: A figure's extent, EMU (6 in by 3 in for a chart, 3 in by 2 in for a picture).
CHART_EXTENT = (5486400, 2743200)
PICTURE_EXTENT = (2743200, 1828800)
CHART_TYPE = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
WPS = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"


def _png(width: int, height: int, rgb: tuple[int, int, int]) -> bytes:
    """A small PNG of one colour, a lighter band across its middle."""
    import struct
    import zlib

    rows = []
    for y in range(height):
        shade = rgb if abs(y - height // 2) > height // 6 else tuple(min(255, c + 80) for c in rgb)
        rows.append(b"\x00" + bytes(shade) * width)

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(b"".join(rows)))
            + chunk(b"IEND", b""))


def _chart_part(rng: random.Random, title: str) -> str:
    """A clustered bar chart of three series over four categories, its values cached."""
    C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
    cats = ["North", "South", "East", "West"]

    def series(k: int) -> str:
        values = "".join(f'<c:pt idx="{i}"><c:v>{rng.randint(10, 90)}</c:v></c:pt>' for i in range(len(cats)))
        names = "".join(f'<c:pt idx="{i}"><c:v>{c}</c:v></c:pt>' for i, c in enumerate(cats))
        return (f'<c:ser><c:idx val="{k}"/><c:order val="{k}"/><c:tx><c:v>Series {k + 1}</c:v></c:tx>'
                f'<c:cat><c:strLit><c:ptCount val="{len(cats)}"/>{names}</c:strLit></c:cat>'
                f'<c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="{len(cats)}"/>{values}'
                '</c:numLit></c:val></c:ser>')

    return (f'{DECL}<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}"><c:chart>'
            f'<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>{escape(title)}</a:t></a:r></a:p></c:rich></c:tx>'
            '<c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/><c:plotArea><c:layout/>'
            '<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>'
            + "".join(series(k) for k in range(3))
            + '<c:gapWidth val="150"/><c:axId val="1"/><c:axId val="2"/></c:barChart>'
            '<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/>'
            '<c:axPos val="b"/><c:crossAx val="2"/></c:catAx>'
            '<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/>'
            '<c:axPos val="l"/><c:majorGridlines/><c:crossAx val="1"/></c:valAx></c:plotArea>'
            '<c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/></c:chart>'
            '</c:chartSpace>')


def _theme() -> str:
    """A theme with Office's colours and faces, which the charts' series and text take."""
    colours = (("dk1", "000000"), ("lt1", "FFFFFF"), ("dk2", "0E2841"), ("lt2", "E8E8E8"), ("accent1", "156082"),
               ("accent2", "E97132"), ("accent3", "196B24"), ("accent4", "0F9ED5"), ("accent5", "A02B93"),
               ("accent6", "4EA72E"), ("hlink", "467886"), ("folHlink", "96607D"))
    scheme = "".join(f'<a:{k}><a:srgbClr val="{v}"/></a:{k}>' for k, v in colours)
    fonts = ('<a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
             '<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>')
    fill = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
    line = '<a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>'
    return (f'{DECL}<a:theme xmlns:a="{A}" name="Office Theme"><a:themeElements>'
            f'<a:clrScheme name="Office">{scheme}</a:clrScheme><a:fontScheme name="Office">{fonts}</a:fontScheme>'
            f'<a:fmtScheme name="Office"><a:fillStyleLst>{fill * 3}</a:fillStyleLst><a:lnStyleLst>{line * 3}'
            '</a:lnStyleLst><a:effectStyleLst>' + '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3
            + f'</a:effectStyleLst><a:bgFillStyleLst>{fill * 3}</a:bgFillStyleLst></a:fmtScheme>'
            '</a:themeElements></a:theme>')


def _inline(rid: str, extent: tuple[int, int], number: int, graphic: str) -> str:
    cx, cy = extent
    return (f'<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/>'
            f'<wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="{number}" name="Figure {number}"/>'
            f'<a:graphic>{graphic}</a:graphic></wp:inline></w:drawing></w:r>')


def _chart_run(rid: str, number: int) -> str:
    C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
    return _inline(rid, CHART_EXTENT, number, f'<a:graphicData uri="{C}"><c:chart xmlns:c="{C}" r:id="{rid}"/>'
                   '</a:graphicData>')


def _picture_run(rid: str, number: int) -> str:
    PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
    cx, cy = PICTURE_EXTENT
    return _inline(rid, PICTURE_EXTENT, number, (
        f'<a:graphicData uri="{PIC}"><pic:pic xmlns:pic="{PIC}"><pic:nvPicPr><pic:cNvPr id="{number}" '
        f'name="picture{number}.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/>'
        '<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/>'
        f'<a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>'
        '</pic:pic></a:graphicData>'))


def _shape_run(number: int, colour: str) -> str:
    """A rounded rectangle floating behind its paragraph, filled and outlined, no text."""
    cx, cy = 5486400, 457200
    return (f'<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" '
            f'relativeHeight="{251658240 + number}" behindDoc="1" locked="0" layoutInCell="1" allowOverlap="1">'
            '<wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset>'
            '</wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>-45720</wp:posOffset>'
            f'</wp:positionV><wp:extent cx="{cx}" cy="{cy}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/>'
            f'<wp:docPr id="{number}" name="Band {number}"/><a:graphic><a:graphicData uri="{WPS}">'
            f'<wps:wsp xmlns:wps="{WPS}"><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/>'
            f'<a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>'
            f'<a:solidFill><a:srgbClr val="{colour}"/></a:solidFill><a:ln w="12700"><a:solidFill>'
            '<a:srgbClr val="1F3864"/></a:solidFill></a:ln></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData>'
            '</a:graphic></wp:anchor></w:drawing></w:r>')


def _field(instruction: str, result: str) -> str:
    """A complex field's runs: its instruction and the result as last made."""
    def mark(kind: str) -> str:
        return f'<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>'

    return (mark("begin") + f'<w:r><w:instrText xml:space="preserve">{escape(instruction)}</w:instrText></w:r>'
            + mark("separate") + _run(result) + mark("end"))


def _styles() -> str:
    def style(sid: str, name: str, extra: str = "", based: str | None = "Normal", kind: str = "paragraph") -> str:
        basis = f'<w:basedOn w:val="{based}"/>' if based else ""
        return f'<w:style w:type="{kind}" w:styleId="{sid}"><w:name w:val="{name}"/>{basis}<w:qFormat/>{extra}</w:style>'

    borders = "".join(f'<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>'
                      for side in ("top", "left", "bottom", "right", "insideH", "insideV"))
    return (
        f'{DECL}<w:styles xmlns:w="{W}">'
        '<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" '
        'w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/>'
        '<w:lang w:val="en-GB" w:eastAsia="en-US" w:bidi="ar-SA"/></w:rPr></w:rPrDefault>'
        '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr>'
        '</w:pPrDefault></w:docDefaults>'
        '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>'
        '<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont">'
        '<w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style>'
        '<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/>'
        '<w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/>'
        '<w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>'
        + style("Title", "Title", '<w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:sz w:val="56"/></w:rPr>')
        + style("Heading1", "heading 1", '<w:pPr><w:keepNext/><w:pageBreakBefore/><w:spacing w:before="240" '
                'w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr>')
        + style("Heading2", "heading 2", '<w:pPr><w:keepNext/><w:spacing w:before="200" w:after="80"/>'
                '<w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr>')
        + style("Caption", "caption", '<w:pPr><w:keepNext/><w:spacing w:after="60"/></w:pPr>'
                '<w:rPr><w:i/><w:sz w:val="18"/></w:rPr>')
        + style("TableText", "Table Text", '<w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
                '<w:rPr><w:sz w:val="20"/></w:rPr>')
        + style("TableTextBold", "Table Text Bold", '<w:rPr><w:b/><w:sz w:val="20"/></w:rPr>', based="TableText")
        + style("TableNumber", "Table Number", '<w:pPr><w:jc w:val="right"/></w:pPr>', based="TableText")
        + style("CommentText", "annotation text", '<w:pPr><w:spacing w:line="240" w:lineRule="auto"/></w:pPr>'
                '<w:rPr><w:sz w:val="20"/></w:rPr>')
        + style("CommentReference", "annotation reference", '<w:rPr><w:sz w:val="16"/></w:rPr>',
                based="DefaultParagraphFont", kind="character")
        + '<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/>'
        '<w:basedOn w:val="TableNormal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>'
        f'<w:tblPr><w:tblBorders>{borders}</w:tblBorders></w:tblPr></w:style>'
        "</w:styles>")


def build(chapters: int = 44, seed: int = 1, accepted: bool = False, fields: bool = False,
          figures: bool = False) -> tuple[bytes, dict]:
    """The report's bytes, and what it holds (revisions, comments, paragraphs, tables, figures)."""
    report = Report(seed, accepted)
    body = report.body(chapters, fields, figures)
    section = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
               'w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
    types = {"word/document.xml": WML + ".document.main+xml", "word/styles.xml": WML + ".styles+xml",
             "word/settings.xml": WML + ".settings+xml"}
    rels = [("rId1", REL + "styles", "styles.xml"), ("rId2", REL + "settings", "settings.xml")]
    parts = {
        "word/document.xml": f"{DECL}<w:document {NS}><w:body>{body}{section}</w:body></w:document>",
        "word/styles.xml": _styles(),
        "word/settings.xml": (f'{DECL}<w:settings xmlns:w="{W}">'
                              + ("" if accepted else "<w:trackRevisions/>")
                              + '<w:defaultTabStop w:val="708"/><w:compat><w:compatSetting '
                              'w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" '
                              'w:val="15"/></w:compat></w:settings>'),
    }
    if report.comments:
        types["word/comments.xml"] = WML + ".comments+xml"
        rels.append(("rId3", REL + "comments", "comments.xml"))
        parts["word/comments.xml"] = f"{DECL}<w:comments {NS}>{''.join(report.comments)}</w:comments>"
    figure_parts: dict[str, bytes] = {}
    if report.figures:
        types["word/theme/theme1.xml"] = "application/vnd.openxmlformats-officedocument.theme+xml"
        rels.append(("rIdT", REL + "theme", "theme/theme1.xml"))
        figure_parts["word/theme/theme1.xml"] = _theme().encode()
    for part, rid, kind, data in report.figures:
        if kind != "image/png":
            types[part] = kind
        rels.append((rid, REL + ("chart" if kind == CHART_TYPE else "image"), part.removeprefix("word/")))
        figure_parts[part] = data
    overrides = "".join(f'<Override PartName="/{part}" ContentType="{kind}"/>' for part, kind in types.items())
    parts["[Content_Types].xml"] = (
        f'{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        + ('<Default Extension="png" ContentType="image/png"/>' if figure_parts else "")
        + f'<Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>')
    rel_xml = "".join(f'<Relationship Id="{i}" Type="{t}" Target="{target}"/>' for i, t, target in rels)
    pkg = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    parts["_rels/.rels"] = (f'{DECL}{pkg}<Relationship Id="rId1" Type="{REL}officeDocument" '
                            'Target="word/document.xml"/></Relationships>')
    parts["word/_rels/document.xml.rels"] = f"{DECL}{pkg}{rel_xml}</Relationships>"
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in ("[Content_Types].xml", "_rels/.rels", "word/document.xml", "word/_rels/document.xml.rels",
                     "word/styles.xml", "word/settings.xml", "word/comments.xml"):
            if name in parts:
                archive.writestr(zipfile.ZipInfo(name, date_time=STAMP), parts[name], zipfile.ZIP_DEFLATED)
        for name, data in figure_parts.items():
            archive.writestr(zipfile.ZipInfo(name, date_time=STAMP), data, zipfile.ZIP_DEFLATED)
    counts = dict(report.counts, revisions=report.ids.revision, figures=len(report.figures),
                  document_xml_bytes=len(parts["word/document.xml"].encode()))
    return buffer.getvalue(), counts


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out")
    ap.add_argument("--chapters", type=int, default=44)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--accepted", action="store_true", help="every revision accepted, no comments")
    ap.add_argument("--fields", action="store_true", help="a table of contents and page references")
    ap.add_argument("--figures", action="store_true", help="charts, pictures and shapes in each chapter")
    args = ap.parse_args()
    data, counts = build(args.chapters, args.seed, args.accepted, args.fields, args.figures)
    with open(args.out, "wb") as handle:
        handle.write(data)
    print({"bytes": len(data), **counts})


if __name__ == "__main__":
    main()
