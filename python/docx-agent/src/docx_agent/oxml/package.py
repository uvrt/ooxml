"""The ``.docx`` package: ooxml-edit's OPC container plus WordprocessingML's entry points.

Reading, editing and losslessly writing parts is :mod:`ooxml_edit.opc`'s; this module only
knows where a Word document keeps its main part, its stories (headers, footers, notes,
comments), its styles, numbering and settings, and which content types make a package a
document, a macro-enabled document or a template.
"""

from __future__ import annotations

import posixpath
import re

from lxml import etree

from . import xml as _xml  # noqa: F401  registers the w: namespaces before qn() is used
from ooxml_edit.opc import (  # noqa: F401  (re-exported)
    CONTENT_TYPES_PART,
    REL_OFFICE_DOCUMENT,
    OpcPackage,
    Relationship,
    normalize_part_path,
    rels_path_for,
)
from .xml import find, qn
from ooxml_common.kinds import MAIN_CONTENT_TYPES as _ALL_MAIN_CONTENT_TYPES

_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"
REL_STYLES = _REL + "styles"
REL_NUMBERING = _REL + "numbering"
REL_SETTINGS = _REL + "settings"
REL_HEADER = _REL + "header"
REL_FOOTER = _REL + "footer"
REL_FOOTNOTES = _REL + "footnotes"
REL_ENDNOTES = _REL + "endnotes"
REL_COMMENTS = _REL + "comments"

#: The main part's content type for each kind of Word package
#: (:data:`ooxml_common.kinds.MAIN_CONTENT_TYPES`, Word's share).
MAIN_CONTENT_TYPES: dict[str, str] = {kind: _ALL_MAIN_CONTENT_TYPES[kind] for kind in ("docx", "dotx", "docm", "dotm")}

#: Relationship types of the parts that hold stories, in the order they are listed.
STORY_RELATIONSHIPS = (REL_HEADER, REL_FOOTER, REL_FOOTNOTES, REL_ENDNOTES, REL_COMMENTS)

#: The compatibility mode a document has when its settings name none: Word 2007's.
DEFAULT_COMPATIBILITY_MODE = 12


class WordPackage(OpcPackage):
    """An open Word package (``.docx``, ``.docm``, ``.dotx``, ``.dotm``)."""

    #: 0.1.118: the main part's bytes as the tool layer last read them, with its top-level
    #: blocks' spans and the body's (``tools/touched``): where an undo finds the blocks it
    #: puts back without reading every block's tags (``None``: not known).
    _body_spans: "tuple[bytes, list, tuple[int, int]] | None" = None

    def _restored_tree(self, path, kept, pieces, tree):
        """The main part put back (an undo, a redo, a rolled back batch) by replacing only
        the body's top-level elements whose bytes differ, the others kept as they are -- the
        same elements, so what is kept per block (the body's index, the Rust mirror's units)
        stays theirs (0.1.116).  The tree is written out and compared with ``kept``: where it
        does not write out the same, it is parsed whole, as before."""
        from .. import incremental

        if tree is None or pieces is None or not incremental.enabled() or path != self.document_part():
            return super()._restored_tree(path, kept, pieces, tree)
        target = bytes(kept)
        # 0.1.118: where the tree's bytes now are its last snapshot's (``settled``), the
        # restored tree's bytes are told from the blocks put in alone (_restore_body's
        # ``written``): the part is not written out whole; else (a batch rolled back, its
        # changes not snapshotted) it is, as before.
        settled = path in getattr(self, "_restoring_settled", ())
        current = bytes(pieces)
        held = self._body_spans
        hint = (held[1], held[2]) if held is not None and len(held[0]) == len(current) and held[0] == current \
            else None
        found = _restore_body(tree, current, target, settled, hint)
        if found is None:
            incremental.STATS["restore: whole"] += 1
            return super()._restored_tree(path, kept, pieces, tree)
        written = found[1]
        if written is None:
            written = _serialize(tree)
            if written != target:
                incremental.STATS["restore: whole (written otherwise)"] += 1
                return super()._restored_tree(path, kept, pieces, tree)
            incremental.STATS["restore: per block"] += 1
        else:
            incremental.STATS["restore: per block, written per block"] += 1
            if incremental.verifying() and _serialize(tree) != target:
                raise incremental.Mismatch("restore: the blocks put back do not write out as the snapshot")
        if self._serial_scopes:
            self._serialized[path] = written
        self._settled.add(path)
        return tree

    def document_part(self) -> str:
        """The main document part (``word/document.xml`` in every file Word writes)."""
        main = self.main_document_part()
        if main is not None:
            return main
        if self.has_part("word/document.xml"):
            return "word/document.xml"
        raise ValueError("not a Word package: no main document part")

    @property
    def kind(self) -> str | None:
        """``docx``, ``docm``, ``dotx`` or ``dotm``, from the main part's content type."""
        content_type = self.content_type(self.document_part())
        for kind, known in MAIN_CONTENT_TYPES.items():
            if content_type == known:
                return kind
        return None

    def story_parts(self) -> list[str]:
        """Every part related to the main part that holds a story, in relationship order:
        headers, footers, footnotes, endnotes, comments."""
        main = self.document_part()
        out: list[str] = []
        for rel_type in STORY_RELATIONSHIPS:
            for part in self.related_parts_of_type(main, rel_type):
                if part not in out and self.has_part(part):
                    out.append(part)
        return out

    def _related(self, rel_type: str) -> str | None:
        parts = self.related_parts_of_type(self.document_part(), rel_type)
        return parts[0] if parts and self.has_part(parts[0]) else None

    def styles_part(self) -> str | None:
        return self._related(REL_STYLES)

    def numbering_part(self) -> str | None:
        return self._related(REL_NUMBERING)

    def settings_part(self) -> str | None:
        return self._related(REL_SETTINGS)

    def compatibility_mode(self) -> int:
        """``w:compatSetting w:name="compatibilityMode"`` from the settings, or 12 (Word
        2007's mode) when the document names none."""
        part = self.settings_part()
        root = self.tree(part) if part is not None else None
        compat = find(root, "w:compat")
        if compat is not None:
            for setting in compat.findall(qn("w:compatSetting")):
                if setting.get(qn("w:name")) == "compatibilityMode":
                    try:
                        return int(setting.get(qn("w:val")) or "")
                    except ValueError:
                        break
        return DEFAULT_COMPATIBILITY_MODE


def story_name(package: WordPackage, part: str) -> str:
    """The name a part's story goes by in ids: ``body`` for the main part, else the part's
    stem (``header2``, ``footnotes``, ``comments``)."""
    if part == package.document_part():
        return "body"
    return posixpath.splitext(posixpath.basename(part))[0]


def _serialize(root):
    from ooxml_edit.xml import serialize

    return serialize(root)


def _restore_body(root, current: bytes, target: bytes, settled: bool = False, hint=None):
    """Put ``root`` (whose bytes are ``current``) to ``target`` by replacing the body's
    children between the top-level blocks the two have alike at the start and at the end;
    ``None`` where that cannot be done so (the caller parses ``target`` whole).  Else the
    root and, where ``settled`` says ``current`` is the tree's bytes, the bytes the tree
    writes out as when they are known from the children put in alone (0.1.118: ``target``;
    ``None``: write the tree out to know)."""
    from ooxml_edit.xml import parse_xml

    from ..revalidate import _scan
    from ..tools.touched import _blocks, _common_prefix, _common_suffix, _kept_counts

    prefix = _common_prefix(current, target)
    suffix = _common_suffix(current, target, min(len(current), len(target)) - prefix)
    if hint is not None:
        # 0.1.118: the blocks around the change from ``current``'s spans; only the stretch
        # between them is read in ``target``.
        spans, (body_at, body_end) = hint
        delta = len(target) - len(current)
        if prefix < body_at or len(current) - suffix > body_end:
            return None
        before, after = _kept_counts(spans, prefix, len(current) - suffix)
        body_end += delta
        begin = spans[before - 1][1] if before else body_at
        stop = spans[len(spans) - after][0] + delta if after else body_end
        if _blocks(target, begin, stop) is None:
            return None
    else:
        found = _scan(target)
        if found is None:
            return None
        spans, (body_at, body_end), _ = found
        if prefix < body_at or len(target) - suffix > body_end or len(current) - suffix > body_end + len(current) - len(target):
            return None
        n = len(spans)
        before = 0
        while before < n and spans[before][1] <= prefix:
            before += 1
        after = 0
        while after < n - before and spans[n - 1 - after][0] >= len(target) - suffix:
            after += 1
        begin = spans[before - 1][1] if before else body_at
        stop = spans[n - after][0] if after else body_end
    body = root.find(qn("w:body"))
    if body is None:
        return None
    blocks = [child for child in body if child.tag in _BLOCK_TAGS]
    head = target[:body_at]
    try:
        fragment = parse_xml(head + target[begin:stop] + target[body_end:])
    except Exception:  # noqa: BLE001 -- parsed whole by the caller
        return None
    new_body = fragment.find(qn("w:body"))
    if new_body is None:
        return None
    # The live body's children between the kept blocks (blocks and whatever else is there).
    children = list(body)
    first_kept = blocks[before - 1] if before else None
    last_kept = blocks[len(blocks) - after] if after else None
    if len(blocks) < before + after:
        return None
    lo = children.index(first_kept) + 1 if first_kept is not None else 0
    hi = children.index(last_kept) if last_kept is not None else len(children)
    for node in children[lo:hi]:
        body.remove(node)
    if first_kept is not None:
        first_kept.tail = new_body.text
    else:
        body.text = new_body.text
    inserted = list(new_body)
    for k, node in enumerate(inserted):
        body.insert(lo + k, node)
    written = None
    if settled:
        between = _in_place_run(root, (first_kept.tail if first_kept is not None else body.text), inserted)
        if between is not None and between == target[begin:stop]:
            written = target
    return root, written


#: A namespace declaration in a start tag.
_DECLARATION = re.compile(rb' xmlns(?::([^=\s]+))?="([^"]*)"')
_NAME = re.compile(rb"<[^\s/>]+")


def _in_place_run(root, text, nodes) -> bytes | None:
    """The bytes ``text`` (the text before the first node) and ``nodes`` (top-level
    children of the body, with their tails) are written as inside the whole part; ``None``
    where that is not told so (0.1.118, for :func:`_restore_body`)."""
    if text and not _plain_text(text):
        return None
    out = [text.encode() if text else b""]
    declared = _declared(root)
    for node in nodes:
        found = in_place(node, declared)
        if found is None:
            return None
        out.append(found)
    return b"".join(out)


def _plain_text(text: str) -> bool:
    return all(c in " \t\n" for c in text)


def _declared(root) -> frozenset:
    """The namespaces declared on the root and the body (the body's over the root's), as
    ``(prefix, uri)`` pairs of bytes (``b""`` for the default namespace)."""
    found: dict = {}
    body = root.find(qn("w:body"))
    for node in (root, body):
        if node is None:
            continue
        own = {k: v for k, v in node.nsmap.items()}
        parent = node.getparent()
        inherited = parent.nsmap if parent is not None else {}
        for prefix, uri in own.items():
            if inherited.get(prefix) != uri or parent is None:
                found[(prefix or "").encode()] = uri.encode()
    return frozenset(found.items())


def in_place(node, declared: frozenset) -> bytes | None:
    """``node``'s bytes (its tail included) as the whole part writes them, from lxml's
    serialization of the node alone: that declares, on the node, every namespace its
    ancestors do (``declared``); taken out again.  ``None`` where the node declares one of
    its own (the declarations are then not only the ancestors')."""
    data = etree.tostring(node, encoding="UTF-8", with_tail=True)
    name = _NAME.match(data)
    if name is None:
        return None
    at = name.end()
    found = set()
    while True:
        declaration = _DECLARATION.match(data, at)
        if declaration is None:
            break
        pair = (declaration.group(1) or b"", declaration.group(2))
        if pair in found:
            return None
        found.add(pair)
        at = declaration.end()
    if found != declared:
        return None
    return data[:name.end()] + data[at:]


_BLOCK_TAGS = frozenset(qn(t) for t in ("w:p", "w:tbl", "w:sdt", "w:customXml"))
