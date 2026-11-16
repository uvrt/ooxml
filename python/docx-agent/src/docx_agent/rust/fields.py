"""``word_fields`` update read from the document's Rust owner (0.1.117).

An update computes each field's result from a layout of the document as it is at that
pass (:meth:`docx_agent.edit.fields.FieldOps._compute`): the page a paragraph is first
drawn on, each page's number, section and story kind.  Where the document has a Rust
owner that follows it (a check of a range opened it, :mod:`.check`), that layout is the
owner's: the state of the pass is handed to it as a change set -- the update's own state
between its passes, no version of the document's -- and read back
(``ooxml_core.field_pages``), instead of laying the document out in Python.  The results
are computed by the same Python code from the same numbers, so they are the same.

Anything this does not read the same falls back to docx2svg's layout, whole and exact, for
that round (counted in :data:`STATS`): no owner (or one that no longer follows: a document
the owner's mirror does not model), a field whose result needs more than page numbers
(``SECTIONPAGES``, ``STYLEREF``), a page number format or switch other than decimal, not
exactly one section, and a document of :data:`~docx_agent.tools._base.REFLOW_PAGES` pages
or fewer -- the next change's reflow of such a document compares the layout docx-agent's
own update keeps.  Of a longer one's, an update kept only what makes the next change's
reflow say "stale: call check": :func:`note_laid` keeps that much per version.  A document
with no owner gets one opened by the update's work ahead (:func:`ahead`), as a check of a
range opens it.  ``DOCX_AGENT_RUST=0`` (every Rust path) or ``DOCX_AGENT_RUST_FIELDS=0``
(this one only) turns it off.
"""
from __future__ import annotations

import json
import os
from collections import Counter
from typing import Any

#: The environment variable that turns this path off (``0``).
ENV = "DOCX_AGENT_RUST_FIELDS"

#: How each pass was read: ``served`` or why it fell back (tests and measurements; never in
#: results).
STATS: Counter = Counter()

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
#: The fields whose result needs a layout and that this path computes from page numbers.
_PAGES = frozenset({"PAGE", "NUMPAGES", "PAGEREF", "TOC"})
#: The fields whose result needs more of the layout than page numbers.
_NOT_SERVED = frozenset({"SECTIONPAGES", "STYLEREF"})
_KINDS = ("default", "first", "even")


def enabled() -> bool:
    return os.environ.get(ENV, "1").strip() != "0"


def owner_of(document: Any) -> Any:
    """The document's :class:`~.check.RustDoc` when this path may use it, else ``None``."""
    if not enabled():
        return None
    from .check import core

    if core() is None:
        return None
    doc = document.__dict__.get("_rust_doc")
    if doc is None or doc.stale is not None:
        return None
    return doc


def why_not(document: Any, found: list) -> str | None:
    """Why an update of ``found`` (every field it reads) is not read from the owner here
    (``None``: it may be)."""
    from docx2svg.fields import switches

    from .check import core

    if not enabled() or not hasattr(core(), "field_pages"):
        return "off"            # (turned off, or no ooxml-core 0.4.0: as before)
    if owner_of(document) is None:
        return "no owner" if document.__dict__.get("_rust_doc") is None else "owner stale"
    for item in found:
        keyword = item.keyword
        if keyword in _NOT_SERVED:
            return f"a {keyword} field"
        if keyword in ("PAGE", "NUMPAGES", "PAGEREF"):
            for kind, argument in switches(item.instruction):
                if not (kind == "*" and argument.lower() in ("mergeformat", "charformat")):
                    return "a page number switch"
    # One section (as the owner's mirror models), its page numbers decimal: the body's own
    # section properties the only ones (a tracked change's former ones count as another).
    body = document.package.tree(document.package.document_part()).find(_W + "body")
    found = list(body.iter(_W + "sectPr")) if body is not None else []
    if len(found) != 1 or found[0].getparent() is not body:
        return "not one section"
    numbering = found[0].find(_W + "pgNumType")
    if numbering is not None and (numbering.get(_W + "fmt") or "decimal") != "decimal":
        return "a page number format"
    return None


class _Info:
    __slots__ = ("number", "section", "story", "blank")

    def __init__(self, number: int, section: int, story: int, blank: bool) -> None:
        self.number, self.section, self.story, self.blank = number, section, story, blank


class _Page:
    __slots__ = ("number", "info")

    def __init__(self, number: int, info: _Info) -> None:
        self.number, self.info = number, info


class _Placed:
    __slots__ = ("page",)

    def __init__(self, page: int) -> None:
        self.page = page


class Layout:
    """The part of docx-agent's ``DocumentLayout`` a fields update reads, from one of the
    owner's snapshots: :attr:`pages` (each with its ``info``), :attr:`stopped` (never: the
    owner lays out the whole document) and :meth:`where` -- the page a paragraph is first
    drawn on (a body paragraph's from the owner; a header's or footer's: the first page that
    shows its part)."""

    stopped = None

    def __init__(self, document: Any, core: Any, key: str, seq: int) -> None:
        self._document, self._core, self._key, self._seq = document, core, key, seq
        self._first: dict[str, int | None] = {}
        self._stories: dict[str, str] | None = None
        answer = self._ask([], info=True)
        self.pages = [_Page(k, _Info(*row)) for k, row in enumerate(answer["info"])]

    def _ask(self, keys: list[str], info: bool = False) -> dict:
        text = self._core.field_pages(self._key, self._seq, keys, info)
        if text is None:
            raise _Gone()
        return json.loads(text)

    def prefetch(self, identifiers: list[str]) -> None:
        """The first pages of ``identifiers`` asked at once (a table of contents' headings)."""
        wanted = sorted({i for i in identifiers if i not in self._first})
        if wanted:
            for key, page in zip(wanted, self._ask(wanted)["first"]):
                self._first[key] = page

    def where(self, identifier: str) -> list:
        story = self._story_parts().get(identifier)
        if story is not None:
            page = self._story_page(story)
            return [_Placed(page)] if page is not None else []
        if identifier not in self._first:
            self.prefetch([identifier])
        page = self._first.get(identifier)
        return [_Placed(page)] if page is not None else []

    def _story_parts(self) -> dict[str, str]:
        """Each header and footer paragraph's id -> its part."""
        if self._stories is None:
            document = self._document
            out: dict[str, str] = {}
            for part in document._parts()[1:]:
                if document._story_of(part).startswith(("header", "footer")):
                    for entry in document._index(part).paragraphs:
                        out[entry.id] = part
            self._stories = out
        return self._stories

    def _story_page(self, part: str) -> int | None:
        """The first page (1-based) that shows header or footer ``part``: a page not blank
        whose kind of story (``first``, ``even``, ``default``: docx2svg's ``story_kind``)
        its section refers to it (stated, or inherited from a section before)."""
        document = self._document
        which = "header" if document._story_of(part).startswith("header") else "footer"
        parts: dict[tuple[int, int], str | None] = {}
        for page in self.pages:
            info = page.info
            if info.blank:
                continue
            key = (info.section, info.story)
            if key not in parts:
                node = document._inherited_reference(info.section, which, _KINDS[info.story])
                parts[key] = document._reference_part(node) if node is not None else None
            if parts[key] == part:
                return page.number + 1
        return None


class _Gone(Exception):
    """The snapshot asked for is no longer kept."""


def layout_for(document: Any, fields: list) -> Any:
    """The owner's layout of ``document`` as it is now (its state handed to the owner
    first, where it changed since) for an update reading ``fields`` (every field of the
    document), or ``None`` (counted in :data:`STATS`): laid out docx-agent's own way."""
    from ..tools._base import REFLOW_PAGES
    from .check import core, sync_state

    held = document.__dict__.get("_rust_doc")
    if held is not None:
        held.__dict__.pop("fields_laid", None)
    why = why_not(document, fields)
    if why is not None:
        if why != "off":
            STATS[f"fallback: {why}"] += 1
        return None
    doc = owner_of(document)
    c = core()
    try:
        seq = sync_state(document, doc)
    except Exception as why:  # noqa: BLE001 -- sync_state made the owner stale
        STATS[f"fallback: {type(why).__name__}"] += 1
        return None
    if seq is None:
        STATS["fallback: owner stale"] += 1
        return None
    try:
        layout = Layout(document, c, doc.key, seq)
    except Exception:  # noqa: BLE001 -- laid out docx-agent's own way
        STATS["fallback: snapshot"] += 1
        return None
    if len(layout.pages) <= REFLOW_PAGES:
        # A short document's next change reports its reflow from the layout kept: laid out
        # docx-agent's own way, as before.
        STATS["fallback: a short document"] += 1
        return None
    # The state laid out last, for the version the update makes (``check._advance``).
    doc.fields_laid = (seq, len(layout.pages))
    STATS["served"] += 1
    return layout


class following:
    """Around an update: where it changed the owner's state and then failed (its change
    taken back), the owner no longer follows the document (made again at the next check)."""

    def __init__(self, document: Any) -> None:
        self.doc = document.__dict__.get("_rust_doc")
        self.seq = self.doc.seq if self.doc is not None else None

    def __enter__(self) -> "following":
        return self

    def __exit__(self, kind: Any, *_: Any) -> None:
        doc = self.doc
        if kind is not None and doc is not None and doc.stale is None and doc.seq != self.seq:
            doc.stale = "a fields update that failed"
            STATS["stale: a fields update that failed"] += 1


class _Reading:
    """What :func:`docx_agent.rust.check.open_doc` reads of a call: the committed version's
    read copy (``document``) and its entry."""

    __slots__ = ("document", "entry")

    def __init__(self, document: Any, entry: Any) -> None:
        self.document, self.entry = document, entry


def ahead(call: Any) -> bool:
    """``word_fields`` update's work ahead of its change: whether the update will be read
    from the document's owner (nothing laid out ahead in Python then).  A document with no
    owner gets one here, with the lock let go, opened from the version committed (as a
    check of a range opens it): laying out a whole document for each pass costs more."""
    document = call.document
    if owner_of(document) is None and enabled() and call.may_unlock and \
            not document.__dict__.get("_rust_refused"):
        from .check import core, open_doc

        reader = call.entry.reader() if core() is not None and call.entry is not None else None
        if reader is not None:
            shim = _Reading(reader.document, reader)
            with call.unlocked():
                try:
                    open_doc(shim)
                except Exception:  # noqa: BLE001 -- laid out docx-agent's own way
                    pass
            if owner_of(document) is not None:
                STATS["opened"] += 1
    if why_not(document, document._all_fields()) is not None:
        return False
    # docx-agent's own update laid the version out ahead: the change's reflow says so.
    from ..tools._base import REFLOW_PAGES
    from .check import core

    doc = owner_of(document)
    version = document.history.version
    seq = doc.seqs.get(version) if doc.version == version else None
    if seq is None:
        return True
    try:
        text = core().field_pages(doc.key, seq, [], False)
    except Exception:  # noqa: BLE001
        text = None
    if text is None:
        return True
    pages = json.loads(text)["pages"]
    if pages <= REFLOW_PAGES:
        return False                # (laid out docx-agent's own way: :func:`layout_for`)
    note_laid(document, version, pages)
    return True


# -- what docx-agent's own update keeps of its layouts, for the reflow ----------------------

#: Versions whose layout a fields update read here, kept per document (docx-agent's own
#: update keeps that many layouts, :data:`docx_agent.layout._CACHE_SIZE`).
_LAID_KEPT = 8


def note_laid(document: Any, version: int, count: int) -> None:
    """Version ``version`` of ``document`` was laid out (``count`` pages) for a fields
    update: its next change's reflow says so, as when docx-agent's own update laid it out
    and kept the layout (:func:`docx_agent.tools._base.laid_before`)."""
    from ..tools._base import REFLOW_PAGES

    if count <= REFLOW_PAGES:
        return
    laid = document.__dict__.setdefault("_rust_fields_laid", {})
    laid.pop(version, None)
    laid[version] = count
    while len(laid) > _LAID_KEPT:
        laid.pop(next(iter(laid)))


def laid_count(document: Any, version: int | None = None) -> int | None:
    laid = document.__dict__.get("_rust_fields_laid")
    if not laid:
        return None
    return laid.get(document.history.version if version is None else version)
