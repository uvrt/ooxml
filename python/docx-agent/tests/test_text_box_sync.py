"""A text box's VML fallback after an edit (lane P2b6): synced when the box's content
changed, in a lone edit and in a batch (every tool call is one); the ids stamping gives its
paragraphs are no change on their own -- Word reads ``mc:Choice`` and writes the fallback
again on saving, and older readers ignore them -- so an edit elsewhere in an unstamped
document leaves the fallback as it was."""

from __future__ import annotations

from pathlib import Path

from lxml import etree

from docx_agent import Document

FIXTURE = Path(__file__).parent / "fixtures" / "generated" / "charts" / "chart-places.docx"
MC = "{http://schemas.openxmlformats.org/markup-compatibility/2006}"
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
PARA_ID = "{http://schemas.microsoft.com/office/word/2010/wordml}paraId"


def _box(document: Document) -> tuple[etree._Element, etree._Element]:
    """The text box's choice and fallback contents (the fixture has one)."""
    tree = document.package.tree(document.package.document_part())
    holder = next(h for h in tree.iter(MC + "AlternateContent")
                  if next(h.find(MC + "Choice").iter(W + "txbxContent"), None) is not None)
    choice = next(holder.find(MC + "Choice").iter(W + "txbxContent"))
    fallback = next(holder.find(MC + "Fallback").iter(W + "txbxContent"))
    return choice, fallback


def _box_paragraph(document: Document) -> str:
    """The id of the box's first paragraph."""
    choice, _ = _box(document)
    first = choice.find(W + "p")
    index = document._index(document.package.document_part())
    return next(e.id for e in index.paragraphs if e.element is first)


def _outside(document: Document) -> str:
    """The id of the body's first paragraph (not in the box)."""
    return document._index(document.package.document_part()).paragraphs[0].id


def _canonical(element: etree._Element) -> bytes:
    """``element`` wherever its namespaces are declared (exclusive canonical form)."""
    return etree.tostring(element, method="c14n", exclusive=True)


def test_stamping_alone_leaves_the_fallback():
    document = Document.open(FIXTURE)
    before = etree.tostring(_box(document)[1])
    document.set_text_paragraphs(_outside(document), "One\nTwo")  # a batch
    choice, fallback = _box(document)
    assert choice.find(W + "p").get(PARA_ID)
    assert etree.tostring(fallback) == before
    document = Document.open(FIXTURE)
    document._set_text(_outside(document), "Lone")  # a lone edit
    assert etree.tostring(_box(document)[1]) == before


def test_a_text_change_syncs_the_fallback():
    for batch in (True, False):
        document = Document.open(FIXTURE)
        target = _box_paragraph(document)
        if batch:
            with document.batch():
                document._set_text(target, "Changed")
        else:
            document._set_text(target, "Changed")
        choice, fallback = _box(document)
        assert "".join(fallback.itertext()).startswith("Changed")
        ids = [[p.get(PARA_ID) for p in c.iter(W + "p")] for c in (choice, fallback)]
        assert ids[0] == ids[1] and all(ids[0])
        assert "".join(choice.itertext()) == "".join(fallback.itertext())


def test_a_moved_block_leaves_its_box_fallback():
    """Lane P4b6: moving the block that holds the box, before or after the others, is no
    change to the box (lxml moves namespace declarations onto the moved block; they are
    not content), so its fallback stays as it was while its choice is stamped."""
    moves = ({"after": "last"}, {"before": "first"})
    for move in moves:
        document = Document.open(FIXTURE)
        before = _canonical(_box(document)[1])
        blocks = [b["id"] for b in document.blocks()]
        holder = blocks[1]  # the fixture's second block holds the box
        body = document.package.tree(document.package.document_part()).find(W + "body")
        assert any(e is _box(document)[0] for e in body[1].iter(W + "txbxContent"))
        others = [b for b in blocks if b != holder]
        where = {k: others[-1] if v == "last" else others[0] for k, v in move.items()}
        document.move_blocks(holder, **where)
        choice, fallback = _box(document)
        assert choice.find(W + "p").get(PARA_ID)
        assert _canonical(fallback) == before, move  # declarations moved, nothing else
