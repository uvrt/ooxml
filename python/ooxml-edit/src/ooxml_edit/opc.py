"""The OPC package, readable *and* writable.

Format-neutral: this is the container every OOXML format shares, and nothing here knows what
the document inside it means.  Format layers subclass :class:`OpcPackage` to add their entry
points.

An OOXML file is a ZIP of XML "parts" wired together by relationship files.  This module owns
the container: it keeps every part's original bytes (as the compressed ZIP entry they were read
from, inflated when they are read), hands out lazily-parsed lxml trees for the parts a caller
wants to change, and writes the archive back.

The losslessness rule lives here and is worth stating plainly:

    A part that was never parsed is written back as the exact bytes that were read.

Only parts whose tree was actually touched get re-serialized.  Everything else -- media,
embedded packages, thumbnails, parts for features this library has never heard of -- passes
through untouched, so an open/save round trip with no edits is byte-identical.  That is
checked by ``tests/test_opc.py``, and by every format layer across its own corpus, and it is
the property the entire editing layer rests on.
"""

from __future__ import annotations

import copy
import io
import os
from contextlib import contextmanager
import posixpath
import threading
import zipfile
from dataclasses import dataclass
from typing import Any, BinaryIO, Callable, Iterable, Iterator

from lxml import etree

# The package's names and path rule are ooxml-common's, the reader's: one spelling for both.
# REL_IMAGE is re-exported: ooxml_edit.opc.REL_IMAGE is public (tests/test_public_api.py).
from ooxml_common.opc import (CONTENT_TYPES_PART, REL_IMAGE as REL_IMAGE, REL_OFFICE_DOCUMENT,
                              normalize_part_path)
from ooxml_common.zipparts import PartStore, RawEntry, ZipPart, read_parts

from .xml import Element, parse_xml, serialize

CONTENT_TYPES_NS = "http://schemas.openxmlformats.org/package/2006/content-types"
#: The declaration Office writes ``[Content_Types].xml`` with, line end included.
CONTENT_TYPES_DECLARATION = b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'

RELS_NS = "http://schemas.openxmlformats.org/package/2006/relationships"

#: A part kept compressed that is larger than this is parsed from its stream (the same
#: tree), never inflated whole first.
PARSE_STREAMED = 1 << 20

#: ``OOXML_EDIT_VERIFY_SERIALIZED=1``: every kept serialization of an edited part is compared
#: with the tree written out again (a test run's check that nothing changes a tree unseen).
VERIFY_SERIALIZED = "OOXML_EDIT_VERIFY_SERIALIZED"


def _verify_serialized() -> bool:
    return os.environ.get(VERIFY_SERIALIZED, "").strip() not in ("", "0")
#: An untouched part larger than this is saved a piece at a time -- where the compressor
#: writes the same bytes for a part given in pieces as given whole
#: (:func:`pieces_compress_alike`); else, and for a smaller part, it is inflated whole and
#: written as before.
SAVE_STREAMED = 1 << 20

_ALIKE: "bool | None" = None


def pieces_compress_alike() -> bool:
    """Whether this Python's deflate writes the same bytes for data given in pieces as
    given at once, as zlib does (its output does not depend on how its input is split
    without a flush).  zlib-ng -- CPython's own on Windows from 3.14, and some systems'
    zlib -- does not: there a part is compressed whole, so the bytes saved are always
    those of before.  zlib-ng is known by its version strings; any other deflate is also
    tried once, on made-up XML."""
    global _ALIKE
    if _ALIKE is None:
        import random
        import zlib

        names = " ".join(str(getattr(zlib, name, "")) for name in
                         ("ZLIB_VERSION", "ZLIB_RUNTIME_VERSION", "ZLIBNG_VERSION"))
        if hasattr(zlib, "ZLIBNG_VERSION") or "ng" in names.lower():
            # zlib-ng (or zlib-ng's zlib-compatible build): a sample may compress alike
            # while a part does not, as CI's Windows legs showed.
            _ALIKE = False
            return _ALIKE

        rng = random.Random(46)
        sample = b"".join(b'<row r="%d"><c r="A%d" s="%d"><v>%d</v></c></row>'
                          % (n, n, rng.randrange(9), rng.randrange(10 ** 6))
                          for n in range(4_000))
        whole = zlib.compressobj(zlib.Z_DEFAULT_COMPRESSION, zlib.DEFLATED, -15)
        expected = whole.compress(sample) + whole.flush()
        pieces = zlib.compressobj(zlib.Z_DEFAULT_COMPRESSION, zlib.DEFLATED, -15)
        got = b"".join(pieces.compress(sample[k:k + 65_536])
                       for k in range(0, len(sample), 65_536)) + pieces.flush()
        _ALIKE = got == expected
    return _ALIKE


@dataclass(frozen=True)
class Relationship:
    id: str
    type: str
    target: str
    #: Normalised package path, or ``None`` for External targets (hyperlinks).
    target_part: str | None
    is_external: bool


class OpcPackage:
    """An open OOXML package: original bytes per part, plus lxml trees for the parts being edited.

    A part read from the ZIP stays its compressed entry until it is replaced
    (:class:`~ooxml_common.zipparts.PartStore`): :meth:`read` inflates it at each call,
    :meth:`open_part` a piece at a time, :meth:`tree` once to parse it, and :meth:`write`
    copies it through a piece at a time -- the package never holds an untouched part
    inflated."""

    def __init__(self, entries: list[zipfile.ZipInfo],
                 parts: "PartStore | dict[str, bytes]") -> None:
        #: Preserved so ``save`` reproduces the original entry order and compression.  Writing
        #: parts in a different order is legal but makes every diff of a saved file useless.
        self._entries = entries
        self._parts = parts if isinstance(parts, PartStore) else PartStore(parts)
        self._trees: dict[str, Element] = {}
        self._dirty: set[str] = set()
        self._rels_cache: dict[str, dict[str, Relationship]] = {}
        #: What was read, so undo can put back a part that was replaced or remove one that was
        #: added.  Shares the parts (compressed or not) with ``_parts``; nothing is copied.
        self._original = self._parts.copy()
        #: Parts written as raw bytes since opening (added, replaced or removed), not as
        #: trees -- in the order they were first changed, which is the order new zip entries
        #: were appended in, so redo can append them in that order again.
        self._raw_changes: dict[str, None] = {}
        #: 0.117.0: an edited part's bytes, written out once while its tree does not change
        #: (:meth:`keep_serialized`): what a version's snapshot, its save and its readers
        #: share -- one serialization per version, not one per reader.
        self._serialized: dict[str, bytes] = {}
        self._keep_serialized = False
        #: How many :meth:`serializing` scopes are open: bytes are kept only inside one.
        self._serial_scopes = 0
        #: 0.117.0: each edited part's last snapshot (:class:`Pieces`), whose pieces the
        #: next one shares where the bytes are the same: a version costs what it changed.
        self._pieces: dict[str, "Pieces"] = {}
        #: 0.119.0: the edited parts whose last snapshot (``_pieces``) is their tree's bytes
        #: now -- taken once the history said the state is made (:meth:`keep_serialized`),
        #: and nothing changed the part since -- inside a :meth:`serializing` scope.  The
        #: next snapshot shares it whole, and the part's bytes are those pieces: one
        #: serialization per version.  Kept from one call to the next where the scope says
        #: so (``keep_settled``: the tool layer's calls, whose documents change between
        #: calls only through the history or with :meth:`mark_dirty`); else forgotten when
        #: the scope ends.  The history's next checkpoint or batch, :meth:`mark_dirty`,
        #: putting a tree or a part in place, restoring, and :meth:`tree` asked outside a
        #: scope each forget it too.
        self._settled: set[str] = set()
        self._settling = False
        self._keep_settled = False
        self._restoring_settled: frozenset[str] = frozenset()
        #: 0.119.0: each part's changes counted (marked dirty, a tree or a part put in place,
        #: restored), and the count at its last snapshot: inside a scope that keeps settled
        #: snapshots (the tool layer's calls, where a tree changes only through the history
        #: or marked dirty), a snapshot of a part not changed since its last one is that one
        #: -- a batch's states before each op are not written out again where nothing changed.
        self._changes: dict[str, int] = {}
        self._snapshot_changes: dict[str, int] = {}

    # -- loading ---------------------------------------------------------------------------

    @classmethod
    def open(cls, source: str | os.PathLike[str] | bytes | BinaryIO):
        entries, parts = read_parts(source, normalize_part_path)
        return cls(entries, parts)

    # -- part access -----------------------------------------------------------------------

    @property
    def part_names(self) -> list[str]:
        return list(self._parts)

    def has_part(self, path: str) -> bool:
        return normalize_part_path(path) in self._parts

    def read(self, path: str) -> bytes | None:
        """Current bytes of a part: the live tree if it was edited, the original otherwise."""
        path = normalize_part_path(path)
        if path in self._dirty:
            return self._serialize(path)
        return self._parts.get(path)

    def _serialize(self, path: str) -> bytes:
        """An edited part's tree written out: kept while :meth:`keep_serialized` says the
        trees do not change, so the version's every reader shares one serialization."""
        kept = self._serialized.get(path)
        if kept is not None:
            if _verify_serialized():
                assert kept == serialize(self._trees[path]), f"{path}: kept bytes are not the tree's"
            return kept
        if path in self._settled:
            data = bytes(self._pieces[path])
            if _verify_serialized():
                assert data == serialize(self._trees[path]), f"{path}: settled bytes are not the tree's"
        else:
            data = self._write_tree(path, self._trees[path])
        if self._keep_serialized:
            self._serialized[path] = data
        return data

    def _write_tree(self, path: str, root: Element) -> bytes:
        """An edited part's tree written out (:func:`~.xml.serialize`): what every read of
        its bytes, snapshot and save writes.  A format may write a part from its last bytes,
        only what changed written again (0.120.0), as long as the bytes are the tree's;
        ``OOXML_EDIT_VERIFY_SERIALIZED=1`` makes it compare them with the tree written out
        whole."""
        return serialize(root)

    def keep_serialized(self, keep: bool) -> None:
        """Whether an edited part's bytes are kept once written out (0.117.0): ``True`` only
        while no tree changes -- the history says so between one state change and the next
        (:class:`~.history.History`: after an outermost batch closes or an undo, and never
        while one is open) -- and only inside a :meth:`serializing` scope; ``False`` forgets
        every kept part.  Marking a part dirty, putting a tree or a part in place, or
        restoring forgets that part's (or every) kept bytes.

        0.119.0: ``keep`` also says whether a snapshot taken now is the state the history
        made (``_settled``, kept across calls until the next change); ``False`` forgets
        those too."""
        self._keep_serialized = keep and self._serial_scopes > 0
        self._settling = keep
        if not keep:
            self._serialized.clear()
            self._settled.clear()

    @contextmanager
    def serializing(self, keep_settled: bool = False) -> Iterator[None]:
        """A stretch in which nothing changes a tree but through the history (0.117.0): the
        tool layer's one call on the document, its lock held -- its change, its undo step,
        its commit and its readers share one serialization of the version it leaves.  Outside
        one (an application editing trees between calls, as it may), nothing is kept: every
        read writes the trees out again, as before."""
        if not self._serial_scopes:
            self._keep_settled = keep_settled
        self._serial_scopes += 1
        try:
            yield
        finally:
            self._serial_scopes -= 1
            if not self._serial_scopes:
                # The bytes kept for readers go; a settled snapshot stays where the scope
                # says nothing changes a tree between it and the next one but through the
                # history or with mark_dirty (``keep_settled``, 0.119.0: the tool layer's
                # documents), else it goes too.
                self._keep_serialized = False
                self._serialized.clear()
                if not self._keep_settled:
                    self._settled.clear()

    def open_part(self, path: str) -> BinaryIO | None:
        """A binary file over a part's current bytes: an untouched part inflated a piece at
        a time as it is read, so a parser fed from it never holds the part whole; an
        edited one's tree written out.  ``None`` for no such part."""
        path = normalize_part_path(path)
        if path in self._dirty:
            return io.BytesIO(self._serialize(path))
        return self._parts.open(path)

    def part_size(self, path: str) -> int | None:
        """A part's current size in bytes -- an untouched one's without inflating it;
        ``None`` for no such part."""
        path = normalize_part_path(path)
        if path in self._dirty:
            return len(self._serialize(path))
        return self._parts.size(path)

    def part_entry(self, path: str) -> ZipPart | None:
        """The part's compressed ZIP entry while it is untouched
        (:class:`~ooxml_common.zipparts.ZipPart`: read it whole, or open it from any
        offset); ``None`` once it is edited or replaced, or for no such part."""
        path = normalize_part_path(path)
        if path in self._dirty:
            return None
        stored = self._parts.stored(path)
        return stored if isinstance(stored, ZipPart) else None

    def tree(self, path: str) -> Element | None:
        """Parsed root of a part, cached.  Parsing alone does not mark the part dirty."""
        path = normalize_part_path(path)
        if not self._serial_scopes:
            # 0.119.0: a tree handed out outside a call (an application's, which may change it
            # without the history) is written out again for its next snapshot.
            self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        cached = self._trees.get(path)
        if cached is not None:
            return cached
        stored = self._parts.stored(path)
        if stored is None:
            return None
        if isinstance(stored, ZipPart):
            # A large part is parsed as it is inflated, never held whole beside its tree.
            root = parse_xml(stored.open() if stored.size > PARSE_STREAMED else stored.read())
        else:
            root = parse_xml(stored)
        self._trees[path] = root
        return root

    def mark_dirty(self, path: str) -> None:
        """Record that a part's tree was changed, so ``save`` re-serializes it.

        Callers must invoke this after mutating a tree.  It is deliberately explicit: lxml
        gives no change notification, and inferring dirtiness by re-serializing and comparing
        would defeat the point of not re-serializing.
        """
        path = normalize_part_path(path)
        if path not in self._trees:
            raise KeyError(f"part {path!r} was never parsed, so it cannot be dirty")
        self._dirty.add(path)
        self._serialized.pop(path, None)
        self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        # A rels part that changed invalidates its cached relationships.
        self._rels_cache.pop(part_for_rels(path), None)

    def set_tree(self, path: str, root: Element) -> None:
        """Make ``root`` the part's tree, marked dirty: what :meth:`restore` does for one
        part.  For an edit that built the part's new content elsewhere and installs it whole
        (round 45: a change prepared on a copy of the document, :mod:`.tools.blocks`).  The
        part must exist."""
        path = normalize_part_path(path)
        if path not in self._parts:
            raise KeyError(f"no part {path!r}")
        self._trees[path] = root
        self._dirty.add(path)
        self._serialized.pop(path, None)
        self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        self._rels_cache.pop(part_for_rels(path), None)

    @property
    def dirty_parts(self) -> frozenset[str]:
        return frozenset(self._dirty)

    def changed_parts(self) -> frozenset[str]:
        """Parts that differ from what was read: edited trees, and parts added, replaced or
        removed.  Empty again once every change is undone."""
        return frozenset(self._dirty) | frozenset(self._raw_changes)

    def opened(self) -> "OpcPackage":
        """The package as it was read, before any change -- a separate, read-only copy."""
        return type(self)(list(self._entries), self._original.copy())

    def replace_part(self, path: str, data: bytes) -> None:
        """Overwrite (or add) a part with raw bytes, dropping any parsed tree for it.

        Raw writes are part of the undo snapshot, so undoing an edit that added a part removes
        it again and the package saves byte-identical to before.
        """
        path = normalize_part_path(path)
        new = path not in self._parts
        self._parts[path] = data
        self._trees.pop(path, None)
        self._dirty.discard(path)
        self._serialized.pop(path, None)
        self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        self._rels_cache.pop(part_for_rels(path), None)
        self._raw_changes[path] = None
        if new:
            self._ensure_entry(path)

    def add_part(self, path: str, data: bytes, content_type: str | None = None, *,
                 override: bool = False) -> str:
        """Add a new part, declaring its content type when one is given.

        ``content_type`` is registered as a ``Default`` for the part's extension when the
        extension has none yet -- the convention for media -- and as an ``Override`` for the
        part name when the extension's ``Default`` gives another type.  ``override=True``
        asks for an ``Override`` instead of a new ``Default``.  When the extension's
        ``Default`` already gives ``content_type`` nothing is declared, ``override`` or not:
        Word writes no ``Override`` that repeats a ``Default``.
        """
        path = normalize_part_path(path)
        if path in self._parts:
            raise ValueError(f"part {path!r} already exists")
        self.replace_part(path, data)
        if content_type is not None:
            if self._default_type(path) == content_type:
                # A stale Override for this name (a part removed and added again) would
                # still win over the Default; it goes, and nothing new is written.
                self._remove_override(path)
            else:
                self.declare_content_type(path, content_type, override=override)
        return path

    def remove_part(self, path: str) -> None:
        """Remove a part, its relationships part and its content-type ``Override``.

        Nothing checks that the part is unreferenced -- that is :meth:`reap`'s job, and the
        safe way in.  Removal is part of the undo snapshot like any raw write: the zip entry
        keeps its place, so undoing puts the part back exactly where it was.
        """
        path = normalize_part_path(path)
        if path not in self._parts:
            raise KeyError(f"no part {path!r}")
        rels = rels_path_for(path)
        if rels != path and rels in self._parts:
            self.remove_part(rels)
        del self._parts[path]
        self._trees.pop(path, None)
        self._dirty.discard(path)
        self._serialized.pop(path, None)
        self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        self._rels_cache.pop(part_for_rels(path), None)
        self._rels_cache.pop(path, None)
        self._raw_changes[path] = None
        self._remove_override(path)

    def copy_part(self, source: str, share: Callable[[Relationship], bool],
                  mapping: dict[str, str] | None = None) -> str:
        """Copy ``source`` to a new part beside it, with its relationships; returns the copy.

        Each internal relationship of the original is followed: to a part already in
        ``mapping`` (old name -> new name, filled in as parts are copied, and seedable by the
        caller) it is re-pointed at the copy; where ``share(relationship)`` says the target may
        be shared, the copy relates to the same part; anything else is copied in turn, the same
        way.  Relationship ids are kept, so the copied XML needs no rewriting.  The copy is
        named like the original with the lowest free number (``item3.xml`` -> ``item7.xml``)
        and gets the original's content type.
        """
        source = normalize_part_path(source)
        mapping = {} if mapping is None else mapping
        data = self.read(source)
        if data is None:
            raise KeyError(f"no part {source!r}")
        new = self.unused_part_name(numbered_template(source))
        mapping[source] = new
        content_type = self.content_type(source)
        self.add_part(new, data, content_type, override=self._has_override(source))

        rels = rels_path_for(source)
        root = self.tree(rels)
        if root is None:
            return new
        copied = parse_xml(serialize(root))
        base = posixpath.dirname(new)
        for node, relationship in zip(_relationship_nodes(copied), self._relationship_list(source)):
            if relationship is None or relationship.is_external or relationship.target_part is None:
                continue
            target = relationship.target_part
            if target in mapping:
                new_target = mapping[target]
            elif share(relationship) or not self.has_part(target):
                continue  # same part, and the same (relative or absolute) Target still reaches it
            else:
                new_target = self.copy_part(target, share, mapping)
            node.set("Target", posixpath.relpath(new_target, base or "."))
        self.add_part(rels_path_for(new), serialize(copied), self.content_type(rels))
        return new

    # -- packages inside packages ----------------------------------------------------------

    def open_embedded(self, path: str) -> "OpcPackage":
        """A part that is itself an OPC package (an embedded object, say), opened for editing.

        The nested package is an ordinary :class:`OpcPackage` read from the part's current
        bytes, with the same losslessness: whatever is not edited inside it is written back
        as the bytes that were read.  Changes reach this package only through
        :meth:`replace_embedded`.
        """
        data = self.read(path)
        if data is None:
            raise KeyError(f"no part {normalize_part_path(path)!r}")
        return OpcPackage.open(data)

    def replace_embedded(self, path: str, package: "OpcPackage") -> bool:
        """Store an edited nested package back into its part; returns whether anything changed.

        A nested package with no edits is left alone -- not even rewritten -- so the part
        stays byte-identical.  The write is a raw part replacement, so it is part of the undo
        snapshot like any other: undoing it puts the original bytes back.
        """
        if not package.dirty_parts and not package._raw_changes:
            return False
        data = package.to_bytes()
        if data == self.read(path):
            return False
        self.replace_part(path, data)
        return True

    def unused_part_name(self, template: str) -> str:
        """``template`` with ``{n}`` replaced by the lowest number not already a part."""
        index = 1
        while normalize_part_path(template.format(n=index)) in self._parts:
            index += 1
        return normalize_part_path(template.format(n=index))

    def find_part_with_bytes(self, data: bytes, directory: str) -> str | None:
        """An existing part under ``directory`` holding exactly ``data`` -- for de-duplication."""
        prefix = normalize_part_path(directory).rstrip("/") + "/"
        for path in self._parts:
            if path.startswith(prefix) and path not in self._dirty \
                    and self._parts.size(path) == len(data) and self._parts[path] == data:
                return path
        return None

    # -- content types ---------------------------------------------------------------------

    def content_type(self, path: str) -> str | None:
        """The declared content type of a part: its Override, else its extension's Default."""
        path = normalize_part_path(path)
        root = self.tree(CONTENT_TYPES_PART)
        if root is None:
            return None
        extension = path.rsplit(".", 1)[-1].lower() if "." in path else ""
        default = None
        for node in root:
            if node.tag == "{%s}Override" % CONTENT_TYPES_NS:
                if normalize_part_path(node.get("PartName") or "") == path:
                    return node.get("ContentType")
            elif node.tag == "{%s}Default" % CONTENT_TYPES_NS:
                if (node.get("Extension") or "").lower() == extension:
                    default = node.get("ContentType")
        return default

    def declare_content_type(self, path: str, content_type: str, *, override: bool = False) -> None:
        """Make ``path`` resolve to ``content_type``, touching ``[Content_Types].xml`` only if
        it does not already.  ``override`` forces a per-part ``Override`` even when the
        extension's ``Default`` already says the same thing.

        A part that already has an ``Override`` keeps it, with its ``ContentType`` changed --
        a template's main part becoming a document's, say -- rather than gaining a second one
        that :meth:`content_type` would never read.  Duplicate ``Override`` elements for the
        part are removed.  An extension's ``Default`` is never changed: other parts share it.
        """
        path = normalize_part_path(path)
        if self.content_type(path) == content_type and not (override and not self._has_override(path)):
            return
        root = self.tree(CONTENT_TYPES_PART)
        if root is None:
            raise ValueError("the package has no [Content_Types].xml")
        existing = self._overrides(path)
        if existing:
            existing[0].set("ContentType", content_type)
            for node in existing[1:]:
                _remove_keeping_layout(node)
            self.mark_dirty(CONTENT_TYPES_PART)
            return
        extension = path.rsplit(".", 1)[-1].lower() if "." in path else ""
        has_default = self._default_node(path) is not None
        if extension and not has_default and not override:
            node = etree.Element("{%s}Default" % CONTENT_TYPES_NS)
            node.set("Extension", extension)
            node.set("ContentType", content_type)
            defaults = [n for n in root if n.tag == "{%s}Default" % CONTENT_TYPES_NS]
            if defaults:
                defaults[-1].addnext(node)
            else:
                root.insert(0, node)
        else:
            node = etree.SubElement(root, "{%s}Override" % CONTENT_TYPES_NS)
            node.set("PartName", "/" + path)
            node.set("ContentType", content_type)
        _match_tail(node)
        self.mark_dirty(CONTENT_TYPES_PART)

    def content_types_with(self, path: str, content_type: str, *, override: bool = True) -> bytes:
        """``[Content_Types].xml`` as :meth:`declare_content_type` would leave it, serialized
        as Office writes that part -- without changing the package.

        For writing a copy of the package as something else -- the main part retyped, say,
        when a template is saved under a document's name -- through :meth:`save`'s
        ``replacements``, while the open package stays what it is::

            package.save(target, {CONTENT_TYPES_PART: package.content_types_with(main, wanted)})

        ``override`` defaults to ``True`` here: a part retyped this way is one whose type
        must not depend on its extension's ``Default``.
        """
        root = self.tree(CONTENT_TYPES_PART)
        if root is None:
            raise ValueError("the package has no [Content_Types].xml")
        was_dirty = CONTENT_TYPES_PART in self._dirty
        self._trees[CONTENT_TYPES_PART] = copy.deepcopy(root)
        try:
            self.declare_content_type(path, content_type, override=override)
            edited = self._trees[CONTENT_TYPES_PART]
        finally:
            self._trees[CONTENT_TYPES_PART] = root
            if not was_dirty:
                self._dirty.discard(CONTENT_TYPES_PART)
        return CONTENT_TYPES_DECLARATION + etree.tostring(edited, encoding="UTF-8")

    def _default_node(self, path: str) -> Element | None:
        """The ``Default`` for ``path``'s extension, if any."""
        root = self.tree(CONTENT_TYPES_PART)
        if root is None or "." not in path:
            return None
        extension = path.rsplit(".", 1)[-1].lower()
        for node in root:
            if node.tag == "{%s}Default" % CONTENT_TYPES_NS \
                    and (node.get("Extension") or "").lower() == extension:
                return node
        return None

    def _default_type(self, path: str) -> str | None:
        node = self._default_node(normalize_part_path(path))
        return None if node is None else node.get("ContentType")

    def _prune_defaults(self, extensions: Iterable[str]) -> None:
        """Remove the ``Default`` of each of ``extensions`` that no part has any more."""
        root = self.tree(CONTENT_TYPES_PART)
        if root is None:
            return
        in_use = {path.rsplit(".", 1)[-1].lower() for path in self._parts if "." in path}
        unused = {extension.lower() for extension in extensions} - in_use
        nodes = [node for node in root
                 if node.tag == "{%s}Default" % CONTENT_TYPES_NS
                 and (node.get("Extension") or "").lower() in unused]
        for node in nodes:
            _remove_keeping_layout(node)
        if nodes:
            self.mark_dirty(CONTENT_TYPES_PART)

    def _overrides(self, path: str) -> list[Element]:
        root = self.tree(CONTENT_TYPES_PART)
        if root is None:
            return []
        return [node for node in root
                if node.tag == "{%s}Override" % CONTENT_TYPES_NS
                and normalize_part_path(node.get("PartName") or "") == path]

    def _has_override(self, path: str) -> bool:
        return bool(self._overrides(normalize_part_path(path)))

    def _remove_override(self, path: str) -> None:
        nodes = self._overrides(path)
        for node in nodes:
            _remove_keeping_layout(node)
        if nodes:
            self.mark_dirty(CONTENT_TYPES_PART)

    # -- relationships ---------------------------------------------------------------------

    def relationships(self, part_path: str) -> dict[str, Relationship]:
        """Relationships declared by ``part_path``, keyed by relationship id."""
        part_path = normalize_part_path(part_path)
        cached = self._rels_cache.get(part_path)
        if cached is not None:
            return cached

        result: dict[str, Relationship] = {}
        root = self.tree(rels_path_for(part_path))
        if root is not None:
            base = posixpath.dirname(part_path)
            for node in root.findall("{%s}Relationship" % RELS_NS):
                rel_id = node.get("Id")
                rel_type = node.get("Type")
                target = node.get("Target")
                if rel_id is None or rel_type is None or target is None:
                    continue
                external = (node.get("TargetMode") or "") == "External"
                result[rel_id] = Relationship(
                    id=rel_id,
                    type=rel_type,
                    target=target,
                    target_part=None if external else resolve_target(base, target),
                    is_external=external,
                )
        self._rels_cache[part_path] = result
        return result

    def related_part(self, part_path: str, rel_id: str | None) -> str | None:
        if rel_id is None:
            return None
        relationship = self.relationships(part_path).get(rel_id)
        if relationship is None or relationship.is_external:
            return None
        return relationship.target_part

    def related_parts_of_type(self, part_path: str, rel_type: str) -> list[str]:
        return [
            rel.target_part
            for rel in self.relationships(part_path).values()
            if rel.type == rel_type and rel.target_part is not None
        ]

    def add_relationship(self, part_path: str, rel_type: str, target_part: str) -> str:
        """Relate ``part_path`` to ``target_part``; returns the relationship id.

        An existing relationship of the same type to the same target is reused, as Office
        does, rather than duplicated.
        """
        part_path = normalize_part_path(part_path)
        target_part = normalize_part_path(target_part)
        for rel in self.relationships(part_path).values():
            if rel.type == rel_type and rel.target_part == target_part and not rel.is_external:
                return rel.id
        root = self._rels_root(part_path)
        rel_id = self.next_rel_id(part_path)
        base = posixpath.dirname(part_path)
        node = etree.SubElement(root, "{%s}Relationship" % RELS_NS)
        node.set("Id", rel_id)
        node.set("Type", rel_type)
        node.set("Target", posixpath.relpath(target_part, base or "."))
        _match_tail(node)
        self.mark_dirty(rels_path_for(part_path))
        return rel_id

    def add_external_relationship(self, part_path: str, rel_type: str, target: str) -> str:
        """Relate ``part_path`` to an address outside the package (a hyperlink, say).

        An existing external relationship of the same type and address is reused.
        """
        part_path = normalize_part_path(part_path)
        for rel in self.relationships(part_path).values():
            if rel.type == rel_type and rel.is_external and rel.target == target:
                return rel.id
        root = self._rels_root(part_path)
        rel_id = self.next_rel_id(part_path)
        node = etree.SubElement(root, "{%s}Relationship" % RELS_NS)
        node.set("Id", rel_id)
        node.set("Type", rel_type)
        node.set("Target", target)
        node.set("TargetMode", "External")
        _match_tail(node)
        self.mark_dirty(rels_path_for(part_path))
        return rel_id

    def remove_relationship(self, part_path: str, rel_id: str) -> None:
        """Delete one relationship.  The target part is left alone; see :meth:`release`."""
        part_path = normalize_part_path(part_path)
        root = self.tree(rels_path_for(part_path))
        if root is None:
            raise KeyError(f"{part_path!r} has no relationships")
        for node in _relationship_nodes(root):
            if node.get("Id") == rel_id:
                _remove_keeping_layout(node)
                self.mark_dirty(rels_path_for(part_path))
                return
        raise KeyError(f"{part_path!r} has no relationship {rel_id!r}")

    def referenced_values(self, part_path: str) -> set[str]:
        """Every attribute value in a part's XML -- what a relationship id is proved unused
        against.

        Deliberately blunt.  Relationship ids are spelled in many attributes (``r:id``,
        ``r:embed``, ``r:link``, ``r:pict``, a legacy ``o:relid``...), and an id that merely
        *might* be referenced must be kept, so any attribute holding the id's exact text
        counts.  Over-counting costs at most an unreaped relationship; under-counting costs
        a repair prompt.
        """
        root = self.tree(part_path)
        if root is None:
            return set()
        return set(root.xpath("//@*"))

    def relationship_sources(self, target_part: str) -> list[tuple[str, Relationship]]:
        """``(source part, relationship)`` for every relationship in the package that targets
        ``target_part`` -- including ones declared by parts nothing else reaches."""
        target_part = normalize_part_path(target_part)
        found = []
        for owner in self._relationship_owners():
            for rel in self.relationships(owner).values():
                if rel.target_part == target_part:
                    found.append((owner, rel))
        return found

    def release(self, part_path: str, rel_ids: Iterable[str]) -> list[str]:
        """Drop relationships ``part_path`` no longer references, then :meth:`reap` their targets.

        Call it after an edit removed references -- a deleted picture, a replaced image fill.
        A relationship is removed only if its id no longer appears anywhere in the part (see
        :meth:`referenced_values`), so it is safe to pass ids that are still in use.  A
        relationships part left with no relationship is removed too, as Word does: it writes
        no empty one.  Returns the parts removed -- reaped targets, not that relationships
        part.
        """
        part_path = normalize_part_path(part_path)
        relationships = self.relationships(part_path)
        wanted = [rel_id for rel_id in dict.fromkeys(rel_ids) if rel_id in relationships]
        if not wanted:
            return []
        still_used = self.referenced_values(part_path)
        targets = []
        released = False
        for rel_id in wanted:
            if rel_id in still_used:
                continue
            rel = relationships[rel_id]
            self.remove_relationship(part_path, rel_id)
            released = True
            if rel.target_part is not None:
                targets.append(rel.target_part)
        rels = rels_path_for(part_path)
        if released and not _relationship_nodes(self.tree(rels)):
            self.remove_part(rels)
        return self.reap(targets)

    def reap(self, candidates: Iterable[str]) -> list[str]:
        """Remove the candidate parts -- and what only they lead to -- that nothing else uses.

        The proof is package-wide.  Starting from the candidates, everything they relate to is
        gathered; then any part still targeted by a relationship from *outside* that set --
        from any part in the package, the package root included, reachable or not -- is
        dropped from it, repeatedly, until it is stable.  What remains is referenced only from
        within itself (a page and its notes, which point at each other), and is removed with
        its relationships parts and content-type overrides.  An extension's ``Default`` goes
        too once no part has that extension (the last picture of a kind removed), since Word
        writes none for an extension the package does not use.  Returns the removed parts.
        """
        doomed: set[str] = set()
        pending = [normalize_part_path(path) for path in candidates]
        while pending:
            path = pending.pop()
            if path in doomed or path not in self._parts or path == CONTENT_TYPES_PART:
                continue
            doomed.add(path)
            for rel in self.relationships(path).values():
                if rel.target_part is not None:
                    pending.append(rel.target_part)
        if not doomed:
            return []

        incoming: dict[str, set[str]] = {path: set() for path in doomed}
        for owner in self._relationship_owners():
            for rel in self.relationships(owner).values():
                if rel.target_part in incoming:
                    incoming[rel.target_part].add(owner)
        changed = True
        while changed:
            changed = False
            for path in list(doomed):
                if any(source not in doomed for source in incoming[path]):
                    doomed.discard(path)
                    changed = True

        removed = sorted(doomed)
        for path in removed:
            if path in self._parts:
                self.remove_part(path)
        self._prune_defaults({path.rsplit(".", 1)[-1] for path in removed if "." in path})
        return removed

    def unreachable_parts(self) -> set[str]:
        """Parts no chain of relationships from the package root reaches (diagnostics, tests).

        Relationships parts count as reached when their owner is, and the content-types part
        always is.
        """
        reached = {""}
        pending = [""]
        while pending:
            for rel in self.relationships(pending.pop()).values():
                target = rel.target_part
                if target is not None and target not in reached and target in self._parts:
                    reached.add(target)
                    pending.append(target)
        return {
            path for path in self._parts
            if path != CONTENT_TYPES_PART and path not in reached
            and not (part_for_rels(path) != path and part_for_rels(path) in reached)
        }

    def orphaned_parts(self) -> list[str]:
        """Parts nothing uses, in name order: those no chain of relationships from the
        package root reaches (:meth:`unreachable_parts`; a page and its notes that point
        only at each other count), and those no content type is declared for that no
        relationship targets (a stray zip entry such as ``[trash]/0000.dat``, which no
        application reads as a part).  A relationships part is left out when its owner is
        listed: it goes with it."""
        found = self.unreachable_parts()
        targets = {rel.target_part for owner in self._relationship_owners()
                   for rel in self.relationships(owner).values() if rel.target_part is not None}
        for path in self._parts:
            if (path != CONTENT_TYPES_PART and path not in targets and part_for_rels(path) == path
                    and self.content_type(path) is None):
                found.add(path)
        return sorted(path for path in found
                      if not (part_for_rels(path) != path and part_for_rels(path) in found))

    def drop_orphans(self) -> list[tuple[str, int]]:
        """Remove :meth:`orphaned_parts` -- each with its relationships part and its
        content-type ``Override``, and an extension's ``Default`` once no part has that
        extension -- and return ``(part, bytes)`` for every part removed, relationships
        parts included, in name order.  Like :meth:`remove_part`, part of the undo snapshot."""
        removed: list[tuple[str, int]] = []
        for path in self.orphaned_parts():
            if path not in self._parts:
                continue
            rels = rels_path_for(path)
            for gone in ([path, rels] if rels != path and rels in self._parts else [path]):
                removed.append((gone, self.part_size(gone) or 0))
            self.remove_part(path)
        self._prune_defaults({path.rsplit(".", 1)[-1] for path, _ in removed if "." in path})
        return sorted(removed)

    def _relationship_owners(self) -> list[str]:
        return [part_for_rels(path) for path in self._parts
                if part_for_rels(path) != path or path == "_rels/.rels"]

    def _relationship_list(self, part_path: str) -> list[Relationship | None]:
        """The part's relationships in document order, ``None`` for a malformed node."""
        root = self.tree(rels_path_for(part_path))
        relationships = self.relationships(part_path)
        return [relationships.get(node.get("Id") or "") for node in _relationship_nodes(root)]

    def _rels_root(self, part_path: str) -> Element:
        rels_path = rels_path_for(part_path)
        if not self.has_part(rels_path):
            self.add_part(
                rels_path,
                b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
                b'<Relationships xmlns="%s"/>' % RELS_NS.encode(),
                "application/vnd.openxmlformats-package.relationships+xml",
            )
        root = self.tree(rels_path)
        assert root is not None
        return root

    def next_rel_id(self, part_path: str) -> str:
        """An ``rIdN`` not already used by ``part_path``."""
        used = self.relationships(part_path)
        index = len(used) + 1
        while f"rId{index}" in used:
            index += 1
        return f"rId{index}"

    def main_document_part(self) -> str | None:
        """Target of the package-level ``officeDocument`` relationship, if any."""
        for rel in self.relationships("").values():
            if rel.type == REL_OFFICE_DOCUMENT and rel.target_part is not None:
                return rel.target_part
        return None

    # -- snapshots (undo) ------------------------------------------------------------------

    def snapshot(self) -> "PackageSnapshot":
        """Current bytes of every edited part.

        Only edited parts are captured: everything else is still the original bytes and cannot
        have changed, so a snapshot costs one serialization per edited part -- tens of KB.
        Parts written raw (added media, say) are captured by reference, not copied.
        """
        return PackageSnapshot(
            trees={path: self._snapshot_of(path) for path in self._dirty},
            raw={path: self._parts.stored(path) for path in self._raw_changes},
        )

    def _snapshot_of(self, path: str) -> "Pieces":
        """An edited part's bytes now as :class:`Pieces`: the pieces of its last snapshot
        where the bytes are the same at the start and the end, only the stretch between
        copied (0.117.0) -- every version of a long part held at the cost of its change."""
        if path in self._settled or (self._serial_scopes and self._keep_settled and path in self._pieces
                                     and self._snapshot_changes.get(path) == self._changes.get(path, 0)):
            found = self._pieces[path]
            if _verify_serialized():
                assert bytes(found) == serialize(self._trees[path]), f"{path}: settled bytes are not the tree's"
            return found
        found = Pieces.of(self._serialize(path), self._pieces.get(path))
        self._pieces[path] = found
        self._snapshot_changes[path] = self._changes.get(path, 0)
        if self._settling and self._serial_scopes:
            self._settled.add(path)
        return found

    def restore(self, snapshot: "PackageSnapshot") -> None:
        """Reset every edited part to a previous snapshot.

        Parts that were edited then *and* now are reset to the snapshot; parts edited since are
        dropped back to their original bytes, and parts added since are removed.
        """
        self._serialized.clear()
        # (0.119.0: the parts whose last snapshot was their tree's bytes, for a format that
        # tells the restored tree's bytes from what it put back, _restored_tree)
        self._restoring_settled = frozenset(self._settled)
        self._settled.clear()
        for path in set(snapshot.trees) | set(self._dirty):
            self._changes[path] = self._changes.get(path, 0) + 1
        # (0.118.0: each tree and its last snapshot's bytes as they were, for a format that
        # puts back only what differs, _restored_tree)
        current_trees = {path: self._trees.get(path) for path in snapshot.trees if path in self._dirty}
        current_pieces = {path: self._pieces.get(path) for path in current_trees}
        # Raw parts first: a tree restored below may belong to a part that was re-added.
        for path in list(snapshot.raw) + [p for p in self._raw_changes if p not in snapshot.raw]:
            if path in snapshot.raw:
                data = snapshot.raw[path]  # None: the part did not exist then
            else:
                data = self._original.stored(path)  # changed since: back to what was read
            if data is not None:
                self._set_raw(path, data)
            else:
                self._parts.pop(path, None)
                if path not in self._original:
                    # An added part leaves no trace; a removed original keeps its entry, so
                    # putting it back restores the archive's order too.
                    self._entries = [e for e in self._entries
                                     if normalize_part_path(e.filename) != path]
            self._trees.pop(path, None)
            self._rels_cache.pop(part_for_rels(path), None)
        self._raw_changes = dict.fromkeys(snapshot.raw)

        for path in self._dirty - set(snapshot.trees):
            self._trees.pop(path, None)
            self._rels_cache.pop(part_for_rels(path), None)
        self._dirty = set()
        for path, kept in snapshot.trees.items():
            self._trees[path] = self._restored_tree(path, kept, current_pieces.get(path), current_trees.get(path))
            self._dirty.add(path)
            self._rels_cache.pop(part_for_rels(path), None)
            if isinstance(kept, Pieces):
                self._pieces[path] = kept
            else:
                self._settled.discard(path)
        self._restoring_settled = frozenset()

    def _restored_tree(self, path: str, kept: Any, pieces: "Pieces | None", tree: "Element | None") -> Element:
        """The tree of a part put back to ``kept`` (a snapshot's bytes) by :meth:`restore`:
        parsed whole here.  ``pieces`` and ``tree`` are the part's last snapshot's bytes and
        its tree before (``None``: none): a format may put back only the elements whose bytes
        differ, as long as the tree it gives writes out as ``kept`` (0.118.0).  One that
        knows the tree writes out as ``kept`` may add ``path`` to ``_settled`` (0.119.0):
        the part is not written out again for its next snapshot."""
        return parse_xml(bytes(kept))

    # -- snapshots part by part (scoped undo) ----------------------------------------------

    def snapshot_value(self, snapshot: "PackageSnapshot", path: str) -> bytes | None:
        """A part's bytes in ``snapshot``; ``None`` when it did not exist then."""
        path = normalize_part_path(path)
        if path in snapshot.trees:
            return bytes(snapshot.trees[path])
        if path in snapshot.raw:
            return _bytes(snapshot.raw[path])
        return self._original.get(path)

    def changed_between(self, before: "PackageSnapshot",
                        after: "PackageSnapshot") -> frozenset[str]:
        """The parts whose bytes differ between two snapshots of this package.

        A part neither snapshot captured is still the original bytes in both.  A part edited
        back to bytes that serialize differently from the original counts as changed: the
        answer may be too wide, never too narrow."""
        paths = set(before.trees) | set(before.raw) | set(after.trees) | set(after.raw)
        return frozenset(path for path in paths if not self._same_in(before, after, path))

    def _same_in(self, before: "PackageSnapshot", after: "PackageSnapshot", path: str) -> bool:
        """Whether a part's bytes are the same in two snapshots -- the same compressed
        entry in both without inflating it."""
        if path not in before.trees and path not in after.trees:
            first = before.raw[path] if path in before.raw else self._original.stored(path)
            second = after.raw[path] if path in after.raw else self._original.stored(path)
            if first is second:
                return True
        if path in before.trees and path in after.trees:
            return before.trees[path] == after.trees[path]       # pieces shared: not joined
        return self.snapshot_value(before, path) == self.snapshot_value(after, path)

    def snapshot_with(self, target: "PackageSnapshot", source: "PackageSnapshot",
                      paths: Iterable[str]) -> "PackageSnapshot":
        """``target`` with the parts ``paths`` as they are in ``source``: each part's place
        in ``source`` (edited tree, raw bytes, or the original) is copied over."""
        trees = dict(target.trees)
        raw = dict(target.raw)
        for path in paths:
            path = normalize_part_path(path)
            trees.pop(path, None)
            raw.pop(path, None)
            if path in source.raw:
                raw[path] = source.raw[path]
            if path in source.trees:
                trees[path] = source.trees[path]
        return PackageSnapshot(trees=trees, raw=raw)

    def reachable_parts(self, start: str, follow: Callable[[Relationship], bool]
                        ) -> list[str]:
        """``start`` and the parts reached from it through relationships that ``follow``
        accepts, each with its relationships part: what a scope such as one page owns.
        External targets and missing parts are skipped."""
        start = normalize_part_path(start)
        seen: list[str] = []
        queue = [start]
        while queue:
            path = queue.pop(0)
            if path in seen or path not in self._parts:
                continue
            seen.append(path)
            for rel in self.relationships(path).values():
                if rel.target_part is not None and not rel.is_external and follow(rel):
                    queue.append(rel.target_part)
        parts: list[str] = []
        for path in seen:
            # The relationships part even when it does not exist yet: an edit may add it.
            parts += [path, rels_path_for(path)]
        return parts

    def _set_raw(self, path: str, data: "bytes | ZipPart") -> None:
        self._ensure_entry(path)
        self._serialized.pop(path, None)
        self._settled.discard(path)
        self._changes[path] = self._changes.get(path, 0) + 1
        self._parts[path] = data

    def _ensure_entry(self, path: str) -> None:
        """A zip entry for ``path``: the one it was read with, or a new one at the end."""
        if not any(normalize_part_path(entry.filename) == path for entry in self._entries):
            self._entries.append(_new_entry(path))

    # -- saving ----------------------------------------------------------------------------

    def _as_read(self, path: str, data: bytes | None) -> "ZipPart | RawEntry | None":
        """The entry ``path`` was read with, where ``data`` -- what a save would write for
        it -- is the bytes read: an edit undone by another, a derived part brought up to
        date to what it held.  The sizes are compared first, so a part that grew or shrank
        is not inflated."""
        entry = self._original.compressed(path)
        if data is None or entry is None or not entry.copyable or entry.size != len(data):
            return None
        return entry if self._original[path] == data else None

    def to_bytes(self, replacements: dict[str, bytes] | None = None, *,
                 recompress: bool = False) -> bytes:
        buffer = io.BytesIO()
        self.write(buffer, replacements, recompress=recompress)
        return buffer.getvalue()

    def save(self, target: str | os.PathLike[str],
             replacements: dict[str, bytes] | None = None, *, recompress: bool = False) -> None:
        with open(os.fspath(target), "wb") as handle:
            self.write(handle, replacements, recompress=recompress)

    def write(self, target: BinaryIO, replacements: dict[str, bytes] | None = None, *,
              recompress: bool = False) -> None:
        """Write the package, reproducing entry order, timestamps and compression.

        ``replacements`` (part name -> bytes) are written instead of the parts' own bytes
        without changing the package: what a format layer derives at save time.  Only
        existing parts can be replaced.

        A part still as it was read is copied as its entry was, byte for byte -- its local
        header, its data (never inflated or compressed again), its data descriptor if it
        has one, and its central directory record but for the offset.  So a package saved
        without a change is the bytes read, and a saved entry keeps its MS-FSSHTTPD chunk
        signature: a co-authoring host is sent only the entries that changed.  A part
        edited, added or replaced is compressed as before, by its entry's method -- unless
        its bytes are the ones read (a derived part brought up to date to the same
        content, an edit undone by another) and its header holds its sizes (no data
        descriptor, no needless ZIP64: :func:`~ooxml_common.zipparts.local_record`): then
        it is copied as read.  ``recompress`` (a host's choice; no tool sets it) compresses every part
        again instead, as before 0.101.0: the bytes zipfile writes for the parts' bytes.
        """
        replacements = {normalize_part_path(k): v for k, v in (replacements or {}).items()}
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
            for info in self._entries:
                path = normalize_part_path(info.filename)
                data = replacements.get(path) if path in self._parts else None
                untouched = None
                if not recompress and path in self._parts \
                        and (data is not None or path in self._dirty
                             or self._parts.compressed(path) is None):
                    # written anew -- unless it is the bytes read
                    if data is None:
                        data = self.read(path)
                    as_read = self._as_read(path, data)
                    if as_read is not None:
                        _copy_entry(archive, info, as_read, as_read=True)
                        continue
                if data is None and path not in self._dirty and not recompress:
                    entry_read = self._parts.compressed(path)
                    if entry_read is not None:
                        _copy_entry(archive, info, entry_read,
                                    as_read=entry_read is self._original.compressed(path))
                        continue
                if data is None and path not in self._dirty:
                    stored = self._parts.stored(path)
                    if isinstance(stored, ZipPart) and stored.size > SAVE_STREAMED \
                            and pieces_compress_alike():
                        untouched = stored
                    elif isinstance(stored, ZipPart):
                        data = stored.read()
                    else:
                        data = stored
                elif data is None:
                    data = self.read(path)
                if data is None and untouched is None:
                    continue
                entry = zipfile.ZipInfo(info.filename, date_time=info.date_time)
                entry.compress_type = info.compress_type
                entry.external_attr = info.external_attr
                entry.create_system = info.create_system
                if untouched is None:
                    assert data is not None
                    archive.writestr(entry, data)
                    continue
                # An untouched part goes through a piece at a time, never inflated whole:
                # the bytes written are writestr's (its size known before, as writestr
                # sets it; the compressor fed the same bytes in pieces).
                entry.file_size = untouched.size
                with archive.open(entry, "w") as out:
                    for piece in untouched.chunks():
                        out.write(piece)


def _copy_entry(archive: zipfile.ZipFile, info: zipfile.ZipInfo, entry: "ZipPart | RawEntry", *,
                as_read: bool = False) -> None:
    """Append ``entry`` -- a part's compressed data as it was read -- to ``archive`` as it
    is, under ``info``'s name, time and attributes.  Where it is the entry ``info`` was read
    with (``as_read``) and kept its local record (:attr:`ZipPart.header`), its header and
    data descriptor are written as they are, and the central directory record is
    ``info``'s as read but for the header's offset (zipfile still adds ZIP64's where an
    offset needs it).  Else the header is made anew, as zipfile makes one for an entry
    whose sizes are known: the entry's method, size, compressed size and CRC-32, its flags
    that describe the data (no data descriptor; the UTF-8 flag from the name), no extra
    field (ZIP64's only where a size or offset needs it, which zipfile adds, here and in
    the central directory)."""
    if as_read and entry.header is not None:
        out = copy.copy(info)
        _append(archive, out, (entry.header, entry.compressed, entry.descriptor))
        return
    out = zipfile.ZipInfo(info.filename, date_time=info.date_time)
    out.compress_type = entry.method
    # As zipfile gives an entry it writes (its _open_to_write): rw------- when none.
    out.external_attr = info.external_attr or 0o600 << 16
    out.create_system = info.create_system
    out.flag_bits = entry.flags
    out.file_size = entry.size
    out.compress_size = entry.compressed_size
    out.CRC = entry.crc
    _append(archive, out, (None, entry.compressed, b""))


def _append(archive: zipfile.ZipFile, out: zipfile.ZipInfo, pieces: tuple) -> None:
    """Write an entry -- its local header (``None``: ``out``'s, made by zipfile), its
    compressed data and its data descriptor -- at the end of ``archive``'s entries, and
    list ``out`` for its central directory."""
    header, compressed, descriptor = pieces
    fp = archive.fp
    assert fp is not None
    if getattr(archive, "_seekable", True):
        fp.seek(archive.start_dir)
    out.header_offset = fp.tell()
    fp.write(out.FileHeader(None) if header is None else header)
    fp.write(compressed)
    fp.write(descriptor)
    archive.start_dir = fp.tell()
    archive.filelist.append(out)
    archive.NameToInfo[out.filename] = out


class Pieces:
    """A part's bytes in a snapshot, as pieces (0.117.0): the pieces of the snapshot before
    shared where the bytes are the same, so a long part's many versions are held at the
    cost of what each changed, not a copy each.  ``bytes(pieces)`` is the bytes; two are
    equal when their bytes are (shared pieces found the same objects, without joining)."""

    __slots__ = ("pieces", "size")

    #: A new piece's size: the stretch an edit changed is copied in pieces of this.
    SIZE = 1 << 18

    def __init__(self, pieces: tuple[bytes, ...]) -> None:
        self.pieces = pieces
        self.size = sum(len(piece) for piece in pieces)

    @classmethod
    def of(cls, data: bytes, before: "Pieces | None" = None) -> "Pieces":
        """``data`` as pieces, sharing ``before``'s that are the same bytes at the same place
        from the start, and from the end.  (0.120.0: ``data`` is what the pieces join to,
        kept beside the latest few, :func:`_joined`.)"""
        found = cls._of(data, before)
        _keep_joined(found, data)
        return found

    @classmethod
    def _of(cls, data: bytes, before: "Pieces | None" = None) -> "Pieces":
        size = cls.SIZE
        if before is None or not before.pieces:
            return cls(tuple(data[k:k + size] for k in range(0, len(data), size)) or (b"",))
        old = before.pieces
        head, at = 0, 0
        # (bytes.startswith/endswith at an offset: compared in place, 0.119.0)
        while head < len(old) and data.startswith(old[head], at):
            at += len(old[head])
            head += 1
        if head == len(old) and at == len(data):
            return before
        tail, end = 0, len(data)
        while tail < len(old) - head and end - len(old[-1 - tail]) >= at \
                and data.endswith(old[-1 - tail], at, end):
            end -= len(old[-1 - tail])
            tail += 1
        middle = tuple(data[k:min(k + size, end)] for k in range(at, end, size))
        return cls(old[:head] + middle + (old[len(old) - tail:] if tail else ()))

    def __bytes__(self) -> bytes:
        if len(self.pieces) == 1:
            return self.pieces[0]
        found = _joined(self)
        if found is None:
            return b"".join(self.pieces)
        if _verify_serialized():
            assert found == b"".join(self.pieces), "kept bytes are not the pieces'"
        return found

    def __len__(self) -> int:
        return self.size

    def __eq__(self, other: object) -> bool:
        if isinstance(other, Pieces):
            if other is self or other.pieces is self.pieces:
                return True
            if other.size != self.size:
                return False
            if len(other.pieces) == len(self.pieces) and all(a is b or a == b for a, b in zip(self.pieces, other.pieces)):
                return True
            # 0.119.0: the pieces both share at the start and at the end (the same objects,
            # so the same bytes) left out, only the rest joined and compared.
            mine, theirs = self.pieces, other.pieces
            head = 0
            while head < len(mine) and head < len(theirs) and mine[head] is theirs[head]:
                head += 1
            tail = 0
            while tail < len(mine) - head and tail < len(theirs) - head \
                    and mine[-1 - tail] is theirs[-1 - tail]:
                tail += 1
            return b"".join(mine[head:len(mine) - tail]) == b"".join(theirs[head:len(theirs) - tail])
        if isinstance(other, (bytes, bytearray, memoryview)):
            return self.size == len(other) and bytes(self) == other
        return NotImplemented

    __hash__ = None  # type: ignore[assignment]

    def __repr__(self) -> str:
        return f"Pieces({self.size} bytes in {len(self.pieces)})"


#: 0.120.0: the bytes the latest few :class:`Pieces` made (:meth:`Pieces.of`) join to,
#: the very ``bytes`` they were made of -- an edit's state before and after, read whole by
#: what the edit's result says (``Change.before``/``after``), are not joined again.  Held for
#: the latest :data:`_JOINED_KEPT` only (a long part's every version would be its copy).
_JOINED: "list[tuple[Pieces, bytes]]" = []
_JOINED_KEPT = 4
#: Round 58: and within this many bytes (a long report's main part is tens of megabytes):
#: the latest two -- an edit's before and after -- kept whatever their size.
_JOINED_BYTES = 64 * 2 ** 20
_JOINED_LOCK = threading.Lock()


def _keep_joined(pieces: "Pieces", data: bytes) -> None:
    if len(pieces.pieces) < 2 or not isinstance(data, bytes):
        return
    with _JOINED_LOCK:
        for k, (kept, _data) in enumerate(_JOINED):
            if kept is pieces:
                del _JOINED[k]
                break
        _JOINED.append((pieces, data))
        del _JOINED[:-_JOINED_KEPT]
        total = sum(len(kept) for _, kept in _JOINED)
        while len(_JOINED) > 2 and total > _JOINED_BYTES:
            total -= len(_JOINED.pop(0)[1])


def forget_joined() -> None:
    """Drop the joined bytes kept (:data:`_JOINED`), when a document is closed (round 58,
    Q3): they are kept by their pieces, not by document, so a closed document's would stay
    until other edits pushed them out -- up to 64 MB of a long part's states.  Another
    document's are joined again when next asked."""
    with _JOINED_LOCK:
        _JOINED.clear()


def _joined(pieces: "Pieces") -> "bytes | None":
    with _JOINED_LOCK:
        for kept, data in reversed(_JOINED):
            if kept is pieces:
                return data
    return None


@dataclass(frozen=True)
class PackageSnapshot:
    """What :meth:`OpcPackage.snapshot` captures: edited trees as bytes (0.117.0: as
    :class:`Pieces`, ``bytes()`` of which is the bytes), raw parts by value."""

    trees: "dict[str, bytes | Pieces]"
    #: A part's bytes, or its compressed entry (:class:`~ooxml_common.zipparts.ZipPart`)
    #: when it is one read from the ZIP; ``None``: it did not exist.
    raw: "dict[str, bytes | ZipPart | None]"


def _bytes(value: "bytes | ZipPart | None") -> bytes | None:
    return value.read() if isinstance(value, ZipPart) else value


def _new_entry(path: str) -> zipfile.ZipInfo:
    entry = zipfile.ZipInfo(path, date_time=(1980, 1, 1, 0, 0, 0))
    # ZipInfo records the platform it was made on (0 on Windows, 3 elsewhere) in every
    # header, so a package's bytes would differ by platform.  Pin it: the same edit writes
    # the same bytes everywhere.
    entry.create_system = 3
    # XML compresses well; media is usually compressed already.
    if path.endswith((".xml", ".rels")):
        entry.compress_type = zipfile.ZIP_DEFLATED
    return entry


def _relationship_nodes(root: Element | None) -> list[Element]:
    return [] if root is None else root.findall("{%s}Relationship" % RELS_NS)


def _remove_keeping_layout(node: Element) -> None:
    """Remove a child of a pretty-printed list without leaving its indentation behind."""
    parent = node.getparent()
    if parent is None:
        return
    previous = node.getprevious()
    if node.getnext() is None and previous is not None:
        # The last child's tail is the parent's closing indentation: hand it on.
        previous.tail = node.tail
    elif previous is None and node.getnext() is not None:
        pass  # parent.text already indents the next child
    parent.remove(node)


def _match_tail(node: Element) -> None:
    """Indent a new child like its siblings, for parts written pretty-printed."""
    previous = node.getprevious()
    if previous is None or not previous.tail or previous.tail.strip():
        return
    if node.getnext() is None:
        # The old last child's tail was the parent's closing indentation: it moves to the new
        # last child, and the old one takes the between-siblings indentation.
        node.tail = previous.tail
        before = previous.getprevious()
        if before is not None and before.tail and not before.tail.strip():
            previous.tail = before.tail
    else:
        node.tail = previous.tail


# -- path helpers ------------------------------------------------------------------------


def rels_path_for(part_path: str) -> str:
    """``dir/part.xml`` -> ``dir/_rels/part.xml.rels``."""
    directory, _, name = normalize_part_path(part_path).rpartition("/")
    return f"{directory}/_rels/{name}.rels" if directory else f"_rels/{name}.rels"


def part_for_rels(rels_path: str) -> str:
    """Inverse of :func:`rels_path_for`; returns ``rels_path`` unchanged if it is not one."""
    directory, _, name = rels_path.rpartition("/")
    if not directory.endswith("_rels") or not name.endswith(".rels"):
        return rels_path
    owner_dir = directory[: -len("_rels")].rstrip("/")
    owner = name[: -len(".rels")]
    return f"{owner_dir}/{owner}" if owner_dir else owner


#: The old name, kept one release for the sibling that imports it (tests/test_public_api.py).
_part_for_rels = part_for_rels


def numbered_template(path: str) -> str:
    """``dir/item3.xml`` -> ``dir/item{n}.xml``: the naming pattern for a sibling copy."""
    import re

    directory, _, name = normalize_part_path(path).rpartition("/")
    stem, dot, extension = name.rpartition(".")
    if not dot:
        stem, extension = name, ""
    stem = re.sub(r"\d+$", "", stem).replace("{", "{{").replace("}", "}}")
    named = f"{stem}{{n}}" + (f".{extension}" if dot else "")
    return f"{directory}/{named}" if directory else named


def resolve_target(base: str, target: str) -> str:
    if target.startswith("/"):
        return normalize_part_path(target)
    return normalize_part_path(posixpath.join(base, target) if base else target)
