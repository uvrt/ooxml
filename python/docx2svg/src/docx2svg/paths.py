"""Element paths: what ``data-docx-path`` and every layout object's ``path`` name.

Every element the SVG draws from the document carries ``data-docx-path``, and the layout's
lines, spans, rules and drawings (``ooxml_core.LaidPages.page``) carry the same string.  A
path is a chain of steps from its part's root, counted as the Word model counts them when it
reads the part (``ooxml_word_model::read::paths``), so that :func:`resolve_path` finds the
element again in any ElementTree-compatible tree of the same part -- the standard
library's, or lxml's -- and a caller that edits the document can find what was drawn:

* ``w:name[k]``: the k-th child with that **local name**, whatever its namespace (the
  prefix written is always ``w:`` for WordprocessingML's own steps), comments and
  processing instructions not counted -- XPath's ``*[local-name()='name'][k]``;
* ``w:tr[k]`` and ``w:tc[k]``: the k-th row of a table, the k-th cell of a row, counted
  **through** row- and cell-level content controls (``w:sdt/w:sdtContent``) and
  ``w:customXml``, and counting rows deleted as a revision (which are not drawn);
* ``w:footnote[@w:id=N]``, ``w:endnote[@w:id=N]``: a note, by its id;
* ``wp:anchor[k]`` after a run: its k-th floating drawing, ``wp:inline[k]`` its k-th
  inline DrawingML object (a ``w:drawing`` or the ``mc:Choice`` of an
  ``mc:AlternateContent`` holding one); a group's members (``wps:wsp[k]``,
  ``pic:pic[k]``, ``wpg:grpSp[k]``, ``wpg:graphicFrame[k]``) are counted among the
  group's own children;
* a step without an index (``w:body``, ``wps:txbx``, ``w:txbxContent``): the first
  element with that local name below (the part's root itself when it is the first step:
  ``w:hdr``, ``w:ftr``, ``w:footnotes``);
* ``label``: the list label drawn for the paragraph before it (resolves to the paragraph);
* ``w:sectPr[k]/w:cols`` (a column separator): the k-th section's ``w:cols``, the
  sections' ``w:sectPr`` taken in document order.

The part a path is in is the main document part for the body, the story's part for a
header or footer (``data-docx-part``), and the footnotes or endnotes part for a path that
starts at a note.

A few paths name something the document does not hold, and resolve to ``None``: Word's
own footnote separator where the document has none (``w:footnotes/separator``).
"""

from __future__ import annotations


def resolve_path(root, path: str):
    """The element ``path`` names in a part whose root element is ``root``, or ``None``.

    ``root`` is the part's root element (``w:document``, ``w:hdr``, ``w:ftr``,
    ``w:footnotes``, ``w:endnotes``) as the standard library's ElementTree or lxml parses
    it.  The path is resolved by the Word model in Rust (``ooxml_core.element_path``, the
    rules above), which gives the element's place among its parents' child elements; a path
    the part does not hold (an edited tree, a synthetic step) resolves to ``None``."""
    if root is None or not path:
        return None
    from . import rust

    if hasattr(root, "getroottree"):
        import importlib

        data = importlib.import_module("lxml.etree").tostring(root)
    else:
        import xml.etree.ElementTree as ElementTree

        data = ElementTree.tostring(root)
    place = rust.core().element_path(data, path)
    if place is None:
        return None
    node = root
    for index in place:
        elements = [c for c in node if isinstance(c.tag, str)]
        if index >= len(elements):
            return None
        node = elements[index]
    return node


__all__ = ["resolve_path"]
