"""Made-up Word documents and what a test reads of them, through the toolbox only (P4e: no
Python view): a blank package from the Rust core, a body of raw WordprocessingML put into one,
and a document's blocks, ids and validation read with ``word_read`` and ``check``.

Must not: import pytest or docx-agent's library (the gates' environment has no pytest, and
the library has no document API).
"""

from __future__ import annotations

import io
import re
import zipfile
from datetime import datetime, timezone
from typing import Any

#: When the blank documents were made: fixed, so their bytes are the same each run.
CREATED = datetime(2026, 10, 4, tzinfo=timezone.utc)

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_W = "{%s}" % W
_BLOCK = re.compile(r"^<!-- ((?:p|t):[0-9A-Fa-f]+)(?: ([^>]*?))? -->\n(.*?)(?=^<!-- (?:p|t):|\Z)", re.M | re.S)


def blank(*, page: str = "A4", landscape: bool = False, language: str = "en-US",
          separators: tuple[str, str] = (".", ","), title: str | None = None, author: str | None = None,
          created: datetime = CREATED) -> bytes:
    """A new blank document's package as the Rust core makes it (``ooxml_core.new_document``,
    what ``new_document`` writes)."""
    import ooxml_core

    stamp = created.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return bytes(ooxml_core.new_document(page=(page, landscape), language=language, separators=separators,
                                         properties=(title, author, stamp)))


def replaced(data: bytes, parts: dict[str, Any]) -> bytes:
    """``data`` with each named part's bytes replaced: a value, or a function of the part's
    text giving its new text."""
    out = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as target:
        for info in source.infolist():
            content = source.read(info.filename)
            given = parts.get(info.filename)
            if callable(given):
                content = given(content.decode("utf-8")).encode("utf-8")
            elif given is not None:
                content = given if isinstance(given, bytes) else given.encode("utf-8")
            target.writestr(info, content)
    return out.getvalue()


def with_body(body: str, data: bytes | None = None) -> bytes:
    """A blank document (or ``data``) whose body is ``body`` (WordprocessingML with the ``w:``
    prefix) before its section properties."""

    def put(xml: str) -> str:
        start = xml.index("<w:body>") + len("<w:body>")
        end = xml.rindex("<w:sectPr")
        return xml[:start] + body + xml[end:]

    return replaced(data if data is not None else blank(), {"word/document.xml": put})


def paragraph(text: str = "", props: str = "") -> str:
    """A paragraph of one run of ``text`` (and ``props``, its ``w:pPr`` content)."""
    run = f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>' if text else ""
    return f"<w:p>{f'<w:pPr>{props}</w:pPr>' if props else ''}{run}</w:p>"


def call(box: Any, session: Any, tool: str, **arguments: Any) -> Any:
    """``tool``'s result, which must be ok."""
    result = box.dispatch(session, tool, arguments)
    assert result.ok, result.to_json()
    return result


def read(box: Any, session: Any, doc: str, **arguments: Any) -> str:
    """The whole of ``word_read``'s Markdown, every page of it."""
    out, cursor = [], None
    while True:
        result = call(box, session, "word_read", doc=doc, **({"cursor": cursor} if cursor else {}), **arguments)
        out.append(result.data)
        cursor = result.next_cursor
        if not cursor:
            return "".join(out)


def blocks(box: Any, session: Any, doc: str, **arguments: Any) -> list[tuple[str, str]]:
    """The body's blocks as ``word_read`` says them: (id, Markdown text), in order; an empty
    paragraph's text is ``""``."""
    return [(found.group(1), found.group(3).strip()) for found in _BLOCK.finditer(read(box, session, doc, **arguments))]


def ids(box: Any, session: Any, doc: str) -> dict[str, str]:
    """The id of each block by its text (the first block of a text)."""
    out: dict[str, str] = {}
    for block, text in blocks(box, session, doc):
        out.setdefault(text, block)
    return out


def paragraph_ids(box: Any, session: Any, doc: str) -> list[str]:
    """The body's paragraphs' ids, in order (tables left out)."""
    return [block for block, _ in blocks(box, session, doc) if block.startswith("p:")]


def validation(box: Any, session: Any, doc: str) -> dict[str, Any]:
    """``check``'s ``validate``: the problems now against those it opened with."""
    return call(box, session, "check", doc=doc, include=["validate"]).data["validate"]


def saved(session: Any, doc: str) -> bytes:
    """The document's bytes as they are now (the session's save of its version, which changes
    nothing)."""
    return session.entry(doc).snapshot()


def tree(data: bytes, name: str = "word/document.xml") -> Any:
    """A part of a package, parsed."""
    from lxml import etree

    return etree.fromstring(part(data, name).encode("utf-8"))


def body_blocks(root: Any) -> list[Any]:
    """The body's paragraphs and tables in order, as ``word_read`` lists them (a block-level
    content control's are its content's)."""
    out: list[Any] = []

    def walk(parent: Any) -> None:
        for child in parent:
            if child.tag in (_W + "p", _W + "tbl"):
                out.append(child)
            elif child.tag in (_W + "sdt", _W + "sdtContent", _W + "customXml"):
                walk(child)

    walk(root.find(_W + "body"))
    return out


def elements(box: Any, session: Any, doc: str) -> dict[str, Any]:
    """The body's blocks' elements by their ids (``word_read``'s order and the body's)."""
    listed = [block for block, _ in blocks(box, session, doc)]
    found = body_blocks(tree(saved(session, doc)))
    assert len(listed) == len(found), (listed, len(found))
    return dict(zip(listed, found))


def text_of(element: Any) -> str:
    """A paragraph's text in the final view (insertions in, deletions out): a tab ``\\t``,
    a line break ``\\v``."""
    out = []
    for node in element.iter(_W + "t", _W + "tab", _W + "br", _W + "cr", _W + "noBreakHyphen"):
        if any(a.tag in (_W + "del", _W + "moveFrom") for a in node.iterancestors()):
            continue
        if node.tag == _W + "t":
            out.append(node.text or "")
        elif node.tag == _W + "tab":
            if node.getparent().tag == _W + "r":
                out.append("\t")
        elif node.tag == _W + "noBreakHyphen":
            out.append("\u2011")
        else:
            out.append("\v")
    return "".join(out)


def rows_of(table: Any) -> list[Any]:
    """A table's own rows (a row Word lists as deleted included)."""
    return [row for row in table if row.tag == _W + "tr"] + [row for sdt in table.iter(_W + "sdtContent")
                                                              if sdt.getparent().getparent() is table
                                                              for row in sdt if row.tag == _W + "tr"]


def cells_of(row: Any) -> list[Any]:
    """A row's cells, its own (not a nested table's)."""
    return [tc for tc in row if tc.tag == _W + "tc"] + [tc for sdt in row.iter(_W + "sdtContent")
                                                       for tc in sdt if tc.tag == _W + "tc"]


def cell_text(cell: Any) -> str:
    """A cell's paragraphs' texts, a line each (``\\n`` between)."""
    return "\n".join(text_of(p) for p in cell if p.tag == _W + "p")


W14_PARA = "{http://schemas.microsoft.com/office/word/2010/wordml}paraId"


def find(root: Any, block: str) -> Any:
    """The paragraph (``p:<paraId>``) or table (``t:<its first row's paraId>``) an id names,
    anywhere in a part: ``None`` where there is none."""
    kind, _, hex_id = block.partition(":")
    tag = _W + ("p" if kind == "p" else "tr")
    for node in root.iter(tag):
        if node.get(W14_PARA, "").upper() == hex_id.upper():
            if kind == "p":
                return node
            table = node.getparent()
            while table is not None and table.tag != _W + "tbl":
                table = table.getparent()
            if table is not None and rows_of(table) and rows_of(table)[0] is node:
                return table
    return None


def element(session: Any, doc: str, block: str, name: str = "word/document.xml") -> Any:
    """The element a block id names in the document as it is now (must be there)."""
    found = find(tree(saved(session, doc), name), block)
    assert found is not None, f"no {block} in {doc}"
    return found


def table_texts(session: Any, doc: str, table: str) -> list[list[str]]:
    """A table's cells' texts, row by row."""
    return [[cell_text(cell) for cell in cells_of(row)] for row in rows_of(element(session, doc, table))]


def tables(session: Any, doc: str) -> list[Any]:
    """The body's tables (nested ones left out)."""
    return [block for block in body_blocks(tree(saved(session, doc))) if block.tag == _W + "tbl"]


def paragraphs(session: Any, doc: str) -> list[tuple[str, str]]:
    """The body's paragraphs (in tables too, in order) as (id, final text)."""
    root = tree(saved(session, doc))
    return [(f"p:{p.get(W14_PARA)}", text_of(p)) for p in root.find(_W + "body").iter(_W + "p")]


def revisions(session: Any, doc: str) -> list[Any]:
    """The tracked-change records of the document part."""
    marks = ("ins", "del", "moveFrom", "moveTo", "rPrChange", "pPrChange", "tblPrChange", "trPrChange",
             "tcPrChange", "sectPrChange", "numberingChange", "cellIns", "cellDel", "cellMerge")
    return list(tree(saved(session, doc)).iter(*(_W + m for m in marks)))


def part(data: bytes, name: str = "word/document.xml") -> str:
    """A part of a package, as text."""
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        return package.read(name).decode("utf-8")


def chart_part(data: bytes, drawing: str, story: str = "word/document.xml") -> str:
    """The chart part of the drawing ``d:<docPr id>`` in ``story``."""
    import posixpath

    root = tree(data, story)
    number = drawing.partition(":")[2]
    frame = next(d for d in root.iter("{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}docPr")
                 if d.get("id") == number).getparent()
    chart = next(frame.iter("{http://schemas.openxmlformats.org/drawingml/2006/chart}chart"))
    rel = chart.get("{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id")
    folder, name = posixpath.split(story)
    rels = tree(data, f"{folder}/_rels/{name}.rels")
    target = next(r.get("Target") for r in rels if r.get("Id") == rel)
    return posixpath.normpath(posixpath.join(folder, target))
