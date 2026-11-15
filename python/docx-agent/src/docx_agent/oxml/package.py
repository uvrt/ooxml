"""The ``.docx`` package: ooxml-edit's OPC container plus WordprocessingML's entry points.

Reading, editing and losslessly writing parts is :mod:`ooxml_edit.opc`'s; this module only
knows where a Word document keeps its main part, its stories (headers, footers, notes,
comments), its styles, numbering and settings, and which content types make a package a
document, a macro-enabled document or a template.
"""

from __future__ import annotations

import posixpath

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
        found = _restore_body(tree, bytes(pieces), target)
        if found is None:
            incremental.STATS["restore: whole"] += 1
            return super()._restored_tree(path, kept, pieces, tree)
        written = _serialize(tree)
        if written != target:
            incremental.STATS["restore: whole (written otherwise)"] += 1
            return super()._restored_tree(path, kept, pieces, tree)
        incremental.STATS["restore: per block"] += 1
        if self._serial_scopes:
            self._serialized[path] = written
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


def _restore_body(root, current: bytes, target: bytes):
    """Put ``root`` (whose bytes are ``current``) to ``target`` by replacing the body's
    children between the top-level blocks the two have alike at the start and at the end;
    ``None`` where that cannot be done so (the caller parses ``target`` whole)."""
    from ooxml_edit.xml import parse_xml

    from ..revalidate import _scan
    from ..tools.touched import _common_prefix, _common_suffix

    found = _scan(target)
    if found is None:
        return None
    spans, (body_at, body_end), _ = found
    prefix = _common_prefix(current, target)
    suffix = _common_suffix(current, target, min(len(current), len(target)) - prefix)
    if prefix < body_at or len(target) - suffix > body_end or len(current) - suffix > body_end + len(current) - len(target):
        return None
    n = len(spans)
    before = 0
    while before < n and spans[before][1] <= prefix:
        before += 1
    after = 0
    while after < n - before and spans[n - 1 - after][0] >= len(target) - suffix:
        after += 1
    body = root.find(qn("w:body"))
    if body is None:
        return None
    blocks = [child for child in body if child.tag in _BLOCK_TAGS]
    begin = spans[before - 1][1] if before else body_at
    stop = spans[n - after][0] if after else body_end
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
    for k, node in enumerate(list(new_body)):
        body.insert(lo + k, node)
    return root


_BLOCK_TAGS = frozenset(qn(t) for t in ("w:p", "w:tbl", "w:sdt", "w:customXml"))
