"""What a test reads of a Word document's layout and blocks through the toolbox (P4e: no
Python view): block and table ids as ``word_read`` names them, where ``word_inspect`` places a
block, a paragraph's effective properties, a document made of Markdown alone, its sections'
body heights, and raw numbering edits a tool cannot make.

Must not: import pytest or docx-agent's library.
"""

from __future__ import annotations

import re
from typing import Any

import word_docs

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W = "{%s}" % W_NS


def _ok(box: Any, session: Any, tool: str, /, **arguments: Any) -> Any:
    result = box.dispatch(session, tool, arguments)
    assert result.ok, (tool, result.error and result.error.to_json())
    return result


def ids_of(session, doc: str) -> list[str]:
    """The body's paragraphs' ids, in order (in tables too)."""
    return [found for found, _ in word_docs.paragraphs(session, doc)]


_BLOCK_ID = re.compile(r"<!-- ((?:p|t)[:@][^\s>]+)")


def _top_level(markdown: str) -> str:
    """``word_read``'s Markdown less the inside of the tables it writes as HTML (their cells'
    blocks are not the body's)."""
    out, depth = [], 0
    for piece in re.split(r"(<table\b[^>]*>|</table>)", markdown):
        if piece.startswith("<table"):
            depth += 1
        elif piece == "</table>":
            depth -= 1
        elif depth == 0:
            out.append(piece)
    return "".join(out)


def block_ids(box, session, doc: str) -> list[str]:
    """The body's blocks' ids as ``word_read`` names them (paraIds, or places where a
    block has none), in order: a list item's id ends its line."""
    return _BLOCK_ID.findall(_top_level(word_docs.read(box, session, doc)))


def table_ids(box, session, doc: str) -> list[str]:
    """The body's tables' ids, in order (read in the markup view: every table the body holds,
    one a tracked change deletes too)."""
    return [block for block in _BLOCK_ID.findall(_top_level(word_docs.read(box, session, doc, view="markup")))
            if block.startswith("t")]


def table_element(box, session, doc: str, table: str):
    """A body table's element as the document is now."""
    return word_docs.tables(session, doc)[table_ids(box, session, doc).index(table)]


def where(box, session, doc: str, block: str) -> list[dict]:
    """Where the layout places a block: its placements (``page``, ``top``, ``bottom``...)."""
    return _ok(box, session, "word_inspect", doc=doc, target=block, layout=True).data["blocks"][0]["placements"]


def pages_of(box, session, doc: str, block: str) -> list[int]:
    """The pages a block is drawn on."""
    return sorted({placed["page"] for placed in where(box, session, doc, block)})


def effective(box, session, doc: str, paragraph: str) -> dict:
    """A paragraph's effective properties, as ``word_inspect`` gives them."""
    return _ok(box, session, "word_inspect", doc=doc, target=paragraph).data["blocks"][0]["effective"]


def written(box, session, markdown: str, name: str = "made.docx") -> str:
    """A new blank document holding ``markdown`` alone (its empty paragraph taken out, but
    after a table, which Word needs a paragraph after)."""
    doc = session.open(word_docs.blank(), name)
    _ok(box, session, "word_insert_markdown", doc=doc, at="end", markdown=markdown)
    listed = word_docs.blocks(box, session, doc)
    for k, (block, text) in enumerate(listed):
        if not text and not (k and listed[k - 1][0].startswith("t")):
            _ok(box, session, "word_delete", doc=doc, target=block)
    return doc


def section_bodies(session, doc: str) -> list[float]:
    """Each section's body height in points: its page height less its top and bottom margins
    (in document order, the body's last section last)."""
    root = word_docs.tree(word_docs.saved(session, doc))
    w = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
    out = []
    for properties in root.iter(w + "sectPr"):
        height = int(properties.find(w + "pgSz").get(w + "h"))
        margins = properties.find(w + "pgMar")
        top, bottom = (abs(int(margins.get(w + side))) for side in ("top", "bottom"))
        out.append(round((height - top - bottom) / 20, 2))
    return out


def text_width() -> int:
    """The text column's width of a new blank document's page, in twips."""
    section = word_docs.part(word_docs.blank())
    page = int(re.search(r'<w:pgSz w:w="(\d+)"', section).group(1))
    left, right = (int(re.search(rf'w:{side}="(\d+)"', section).group(1)) for side in ("left", "right"))
    return page - left - right


def with_strays(data: bytes, after: str, items: list[tuple[str, int]], style: str | None = None,
                abstract: str = "1") -> tuple[bytes, list[str]]:
    """``data`` with paragraphs inserted after the paragraph ``after``, each of
    ``(text, level)`` in a list of its own (a new instance of the numbering definition
    ``abstract``, as a writer's new items came out, which no tool makes): the package and
    their ids."""
    from lxml import etree

    root = word_docs.tree(data)
    numbering = word_docs.tree(data, "word/numbering.xml")
    anchor = word_docs.find(root, after)
    ids = []
    for k, (text, level) in enumerate(items):
        number, para = str(90 + k), f"7A00{k:04X}"
        num = etree.SubElement(numbering, W + "num", {W + "numId": number})
        etree.SubElement(num, W + "abstractNumId", {W + "val": abstract})
        styled = f'<w:pStyle w:val="{style}"/>' if style else ""
        paragraph = etree.fromstring(
            f'<w:p xmlns:w="{W_NS}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" '
            f'w14:paraId="{para}" w14:textId="77777777"><w:pPr>{styled}<w:numPr><w:ilvl w:val="{level}"/>'
            f'<w:numId w:val="{number}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>')
        anchor.addnext(paragraph)
        anchor = paragraph
        ids.append(f"p:{para}")

    def xml(node: Any) -> bytes:
        return etree.tostring(node, xml_declaration=True, encoding="UTF-8", standalone=True)

    return word_docs.replaced(data, {"word/document.xml": xml(root), "word/numbering.xml": xml(numbering)}), ids
