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
    holder = next(tree.iter(MC + "AlternateContent"))
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
