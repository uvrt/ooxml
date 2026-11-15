"""docx-agent's ``check`` of some ranges served by the Rust core (lane F).

A document a check reads by ``range`` gets a Rust owner (:mod:`ooxml_core`) the first time
one does: the document is mirrored (:class:`~docx_agent.rust.mirror.Mirror`: docx2svg's model
of its body, each block's measuring input and its record, :mod:`.records`) and laid out in
Rust.  From then on each committed version is handed to the owner as a change set -- the
blocks whose bytes differ from the version before (the body summaries docx-agent's
``touched`` keeps), made again -- under the document's lock as the version is committed
(:func:`committed`); the owner lays out each version and keeps its snapshot.  A check of a
range then makes its arguments' checks, resolves its ranges, reads ``validate`` as before,
and asks the owner, the interpreter lock let go, for its facts on that version's snapshot
(:func:`ooxml_core.check`: one call); what it says back is the result, worded as before.

Everything the Rust facts do not cover runs as before, whole (:data:`STATS` counts why):
``DOCX_AGENT_RUST=0`` in the environment; ``ooxml-core`` not importable (said once in the
log); a request without a range, with ``pages``, ``copies``, ``min_size``, ``styles`` or a
deck's arguments, in a batch; a range of parts of blocks; a document the mirror does not
model (notes, drawings, frames, columns, several sections, hyphenation, merged or nested
cells, cells' vertical alignment, floating tables, other page number formats, SEQ,
SECTIONPAGES, STYLEREF; the export's ``Unsupported``); a change outside the body's blocks
(styles, numbering, settings, headers, footers: the mirror is made again at the next check);
a range whose keep would be tried (``if_kept``, ``if_released``), a table that may have a
``column_fit`` or reach past the margins.  The answer is the same either way: the
differential tests (``tests/test_rust_check.py``) compare them.
"""
from __future__ import annotations

import hashlib
import itertools
import json
import logging
import os
import re
import threading
import time
from collections import Counter, OrderedDict
from typing import Any

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
_P, _TBL = _W + "p", _W + "tbl"
_BLOCKS = frozenset({_P, _TBL, _W + "sdt", _W + "customXml"})

#: The environment variable that turns the Rust path off (``0``).
ENV = "DOCX_AGENT_RUST"

log = logging.getLogger("docx_agent.rust")

#: Why checks were served or not, counted (tests and measurements read it; never in results).
STATS: Counter = Counter()


def _stats_at_exit() -> None:
    """``DOCX_AGENT_RUST_STATS`` names a file: each process adds its :data:`STATS` to it, one
    JSON line, when it ends (a test run's or a measurement's count of what fell back); with
    the fields update's (:data:`.fields.STATS`) under ``"fields"``, where it ran."""
    import sys

    path = os.environ.get("DOCX_AGENT_RUST_STATS")
    stats: dict[str, Any] = dict(STATS)
    fields = sys.modules.get("docx_agent.rust.fields")
    if fields is not None and fields.STATS:
        stats["fields"] = dict(fields.STATS)
    if path and stats:
        try:
            with open(path, "a", encoding="utf-8") as out:
                out.write(json.dumps(stats) + "\n")
        except OSError:
            pass


if os.environ.get("DOCX_AGENT_RUST_STATS"):
    import atexit

    atexit.register(_stats_at_exit)

_core: Any = None
_core_failed = False
_core_lock = threading.Lock()
_keys = itertools.count(1)


def core() -> Any:
    """:mod:`ooxml_core`, or ``None``: turned off (``DOCX_AGENT_RUST=0``) or not importable
    (said once, in the log)."""
    global _core, _core_failed
    if os.environ.get(ENV, "1").strip() == "0":
        return None
    if _core is not None or _core_failed:
        return _core
    with _core_lock:
        if _core is None and not _core_failed:
            try:
                import ooxml_core

                if not hasattr(ooxml_core, "check"):
                    raise ImportError("ooxml_core has no check (ooxml-core 0.2.0 or later)")
                _core = ooxml_core
            except Exception as error:  # noqa: BLE001 -- checks run docx-agent's own way
                _core_failed = True
                log.warning("docx-agent: ooxml-core is not available (%s); checks lay documents out in Python",
                            error)
    return _core


class RustDoc:
    """One document's Rust owner and what follows it: the mirror, the version mirrored and
    the owner's sequence number of each version committed to it."""

    def __init__(self, key: str, mirror: Any, version: int, summary: Any, parts: dict[str, bytes]) -> None:
        self.key = key
        self.mirror = mirror
        self.version = version
        self.seq = 0
        #: Document version -> the owner's sequence number (the latest few).
        self.seqs: OrderedDict = OrderedDict({version: 0})
        self.summary = summary
        self.parts = parts
        #: Why the mirror no longer follows the document (made again at the next check).
        self.stale: str | None = None
        #: The styles, numbering and settings parts as the mirror has them (bytes).
        self.definitions: dict[str, bytes] = {}
        #: Style ids and list ids the body names that the document does not define.
        self.undefined: set[str] = set()
        self.opened: dict[str, Any] = {}
        self.commits: list[dict[str, Any]] = []

    def close(self) -> None:
        c = core()
        if c is not None:
            try:
                c.close_document(self.key)
            except Exception:  # noqa: BLE001
                pass


def _parts(document: Any) -> dict[str, bytes]:
    """A digest of every XML part but the body's (what a change outside the body is told by)."""
    package = document.package
    main = package.document_part()
    out = {}
    for part in package.part_names:
        if part == main or not (part.endswith(".xml") or part.endswith(".rels")):
            continue
        data = package.read(part)
        if data is not None:
            out[part] = hashlib.blake2b(data, digest_size=16).digest()
    return out


def _live(document: Any) -> Any:
    return document.__dict__.get("_live") or document


# -- the requests the Rust facts serve ----------------------------------------------------------


#: What a check includes that the Rust facts answer (and what the handler adds around them).
_INCLUDES = frozenset({"validate", "layout", "reflow", "fields", "numbers"})


def request_supported(arguments: Any, document: Any = None) -> str | None:
    """Why a check with these arguments runs docx-agent's own way (``None``: it may be
    served here)."""
    if not isinstance(arguments, dict):
        return "arguments"
    from ..scope import ranges_of

    if not ranges_of(arguments.get("range")):
        return "no range"
    for name in ("pages", "slides", "copies", "boxes", "spacing", "contrast", "styles", "min_size"):
        if arguments.get(name) not in (None, [], {}):
            return name
    if document is not None and getattr(document, "min_text_size", None) is not None:
        return "min_text_size"
    include = arguments.get("include")
    if include is not None and not set(include) <= _INCLUDES:
        return "include"
    return None


def will_serve(entry: Any, tool: Any, arguments: Any) -> bool:
    """Whether the check a call makes is answered here (lane O's owner then lays nothing out
    for it): the document not refused, the request supported."""
    if core() is None or getattr(tool, "name", None) != "check" or getattr(entry, "kind", None) != "docx":
        return False
    document = entry.document
    if document.__dict__.get("_rust_refused"):
        return False
    return request_supported(arguments, document) is None


# -- opening, and following each committed version -------------------------------------------


def open_doc(call: Any) -> RustDoc | None:
    """The document's :class:`RustDoc`, opened from the version the call reads where it has
    none (or a stale one); ``None`` where the mirror does not model the document (refused
    for the session, said in :data:`STATS`)."""
    copy = call.document
    live = _live(copy)
    found = live.__dict__.get("_rust_doc")
    if found is not None and found.stale is None:
        return found
    if live.__dict__.get("_rust_refused"):
        return None
    if core() is None:
        return None
    # One opening at a time per document: a check that comes meanwhile takes what it made.
    with live.__dict__.setdefault("_rust_opening", threading.Lock()):
        found = live.__dict__.get("_rust_doc")
        if found is not None and found.stale is None:
            return found
        if live.__dict__.get("_rust_refused"):
            return None
        return _open(call, copy, live, found)


def _open(call: Any, copy: Any, live: Any, found: Any) -> "RustDoc | None":
    c = core()
    from ..tools.touched import _summary
    from .exporter import Unsupported
    from .mirror import Mirror

    if getattr(call.entry, "live", None) is None:
        STATS["fallback: not a read copy"] += 1
        return None
    t0 = time.perf_counter()
    version = copy.history.version
    try:
        mirror = Mirror(copy)
        _check_keys(mirror)
        mirror.fstack_end = mirror.thread_fields(mirror.units, [])
        t1 = time.perf_counter()
        with _records(copy, mirror):
            text, stats = mirror.export_with(constants(copy, mirror))
        t2 = time.perf_counter()
        main = copy.package.document_part()
        summary = _summary(copy, copy.package.read(main))
        if summary.spans is None:
            raise Unsupported("the body's blocks are not read from its bytes")
        parts = _parts(copy)
    except Unsupported as why:
        STATS[f"refused: {str(why)[:60]}"] += 1
        live.__dict__["_rust_refused"] = str(why)
        if found is not None:
            found.close()
            live.__dict__.pop("_rust_doc", None)
        return None
    key = f"docx-agent-{os.getpid()}-{next(_keys)}"
    t3 = time.perf_counter()
    stat = c.open_document(key, "docx", text)
    t4 = time.perf_counter()
    doc = RustDoc(key, mirror, version, summary, parts)
    doc.definitions = {part: copy.package.read(part) or b"" for part in copy.package.part_names
                       if part.endswith(_DEFINITIONS) or part.endswith(".rels")
                       or (part != main and copy._story_of(part).startswith(("header", "footer")))}
    doc.sections = mirror.sections_sent
    doc.undefined = _undefined(copy, [u.element for u in mirror.units], doc.definitions)
    doc.opened = {"mirror_s": t1 - t0, "export_s": t2 - t1, "summary_s": t3 - t2, "owner_s": t4 - t3,
                  "owner": json.loads(stat), "items": stats.get("items"), "bytes": len(text)}
    entry = getattr(call.entry, "live", None) or call.entry
    if entry is call.entry:
        raise _NotAReader()
    if found is not None:
        found.close()
    live.__dict__["_rust_doc"] = doc
    # Versions committed while it was made: followed now where the document is free, as
    # each commit would have; else by the next commit (never a wait for an edit in flight).
    if live.history.version != version and entry.lock.acquire(blocking=False):
        try:
            committed(live)
        finally:
            entry.lock.release()
    STATS["opened"] += 1
    return doc if doc.stale is None else None


def _check_keys(mirror: Any) -> None:
    from .exporter import Unsupported

    keys = [u.key for u in mirror.units]
    if any(k is None for k in keys) or len(set(keys)) != len(keys):
        raise Unsupported("a block without a unique paraId")


class _NotAReader(Exception):
    pass


class _records:
    """Records made with the mirror's model for the document's (``effective.model``), put
    back after."""

    def __init__(self, document: Any, mirror: Any) -> None:
        self.document, self.mirror = document, mirror

    def __enter__(self) -> None:
        from ..edit.numbering import reading
        from .mirror import MirrorModel
        from .records import Context

        self.saved = self.document.__dict__.get("_effective")
        self.document._effective = (self.document._state, MirrorModel(self.document, self.mirror))
        self.mirror.ctx = Context(self.document)
        # the numbering part's lists and definitions listed once for the whole reading
        self.reading = reading(self.document)
        self.reading.__enter__()

    def __exit__(self, *exc: Any) -> None:
        self.reading.__exit__(*exc)
        self.mirror.ctx = None
        self.document._effective = self.saved


def constants(document: Any, mirror: Any) -> dict[str, Any]:
    """What the facts read of the document as a whole: each section's body height
    (``_section_bodies``), the default language tag and its source, the languages in the
    order the facts rank them, and which page fields the headers and footers show."""
    from ..edit import typography
    from ..edit.fields import scan
    from ..edit.text_language import LANGUAGES
    from ..layout import _section_bodies
    from ..oxml.xml import make

    cascade = typography.Cascade(document)
    found = cascade.of_with_source(make("w:p"), None)
    stories = set()
    main = document.package.document_part()
    for part in document._parts():
        if part == main or document._index(part).story not in ("header", "footer") and \
                not document._story_of(part).startswith(("header", "footer")):
            continue
        for item in scan(part, document.package.tree(part)):
            if item.keyword in ("PAGE", "NUMPAGES"):
                stories.add(item.keyword)
    story_fields = sorted(stories)
    from .records import fields_of

    # Every story's fields (the body's are its blocks': ``{"body": true}`` holds its place)
    # and the bookmarks besides the body's, for the fields facts.
    field_parts, bookmarks = [], []
    for part in document._parts():
        if part == main:
            field_parts.append({"body": True, "story": document._story_of(part)})
            continue
        root = document.package.tree(part)
        fields, names = fields_of(root, [])
        field_parts.append({"body": False, "story": document._story_of(part), "fields": fields})
        bookmarks += names
    h = hashlib.blake2b(digest_size=8)
    for rid in sorted(getattr(mirror.d2, "stories", None) or {}):
        h.update(rid.encode() + _drawn_story(mirror.d2.stories[rid]))
    return {"bodies": _section_bodies(document), "lang_tag": found[0], "lang_source": found[3],
            "languages": list(LANGUAGES),
            # Every section and kind alike: what any header or footer shows.
            "stories": [[story_fields] * 3 for _ in mirror.d2.sections],
            # What the headers and footers draw, as written (a page's digest holds it).
            "epoch": h.hexdigest(), "field_parts": field_parts, "bookmarks": bookmarks}


def _drawn_story(blocks: Any) -> bytes:
    """What a header or footer draws, as docx2svg reads it, but what the layout computes:
    the text its page number and page count fields cache (drawn as laid out)."""
    import dataclasses

    def clear(value: Any) -> Any:
        if isinstance(value, (list, tuple)):
            return type(value)(clear(v) for v in value)
        if dataclasses.is_dataclass(value) and not isinstance(value, type):
            if getattr(value, "keyword", None) in ("PAGE", "NUMPAGES") and hasattr(value, "cached"):
                return dataclasses.replace(value, cached=())     # (a field: its cached result)
            changes = {}
            field = getattr(value, "field", None)
            if field is not None and getattr(field, "keyword", None) in ("PAGE", "NUMPAGES") and \
                    hasattr(value, "text"):
                changes["text"] = ""
            for f in dataclasses.fields(value):
                inner = getattr(value, f.name)
                if isinstance(inner, (list, tuple)) or dataclasses.is_dataclass(inner):
                    changes[f.name] = clear(inner)
            return dataclasses.replace(value, **changes) if changes else value
        return value

    return repr(clear(blocks)).encode()


def committed(live: Any) -> None:
    """The format's commit of a version (under the document's lock,
    :func:`docx_agent.layout.read_copy`): the version handed to the document's owner as a
    change set.  Anything the mirror cannot follow makes it stale (made again at the next
    check)."""
    doc = live.__dict__.get("_rust_doc")
    if doc is None or doc.stale is not None:
        return
    c = core()
    if c is None:
        return
    version = live.history.version
    if version == doc.version:
        return
    t0 = time.perf_counter()
    try:
        _advance(c, live, doc, version)
    except Exception as why:  # noqa: BLE001 -- the next check makes the mirror again
        doc.stale = f"{type(why).__name__}: {why}"[:200]
        STATS[f"stale: {doc.stale[:70]}"] += 1
        log.debug("docx-agent: the Rust owner of a document no longer follows it: %s", doc.stale)
        return
    doc.commits.append({"version": version, "seq": doc.seq, "s": time.perf_counter() - t0})
    del doc.commits[:-64]


def sync_state(live: Any, doc: RustDoc) -> int | None:
    """0.1.117: the live document's state now -- within an edit, a fields update between
    its passes (:mod:`.fields`) -- handed to its owner where it changed since the state the
    owner has; the owner's sequence number of it (``None``: the owner no longer follows).
    It is no version of the document's: checks read the versions committed (``seqs``), and
    the edit's commit takes the change from this state on.  Anything the mirror cannot
    follow makes the owner stale (made again at the next check) and is raised."""
    c = core()
    if c is None or doc.stale is not None:
        return None
    try:
        _advance(c, live, doc, None)
    except Exception as why:
        doc.stale = f"{type(why).__name__}: {why}"[:200]
        STATS[f"stale: {doc.stale[:70]}"] += 1
        raise
    return doc.seq


def _advance(c: Any, live: Any, doc: RustDoc, version: int | None) -> None:
    from ..tools.touched import _rescan, _scan
    from .exporter import Unsupported

    package = live.package
    main = package.document_part()
    changed = {}
    for part in set(package.dirty_parts) | set(getattr(package, "_raw_changes", {}) or ()):
        if part == main or not (part.endswith(".xml") or part.endswith(".rels")):
            continue
        data = package.read(part) or b""
        if hashlib.blake2b(data, digest_size=16).digest() != doc.parts.get(part):
            changed[part] = data
    held = live.__dict__.get("_body_summary")
    if held is not None and held[0] == version:
        data = held[1].data
    else:
        data = package.read(main)
    old = doc.summary
    new = _rescan(data, old) if old.data != data else old
    if new is None:
        new = _scan(data)
    if new is None or new.spans is None:
        raise Unsupported("the body's blocks are not read from its bytes")
    if version is None and new is old and not changed:
        return                  # (an edit's state, :func:`sync_state`: the owner's already)
    laid = doc.__dict__.pop("fields_laid", None) if version is not None else None
    if laid is not None and new is old and not changed and laid[0] == doc.seq:
        # The version a fields update read the layout of last (:mod:`.fields`).
        from .fields import note_laid

        note_laid(live, version, laid[1])
    if new is old:
        before, after = len(old.spans), 0
    elif new.kept is not None and new.kept[2] is old:
        before, after = new.kept[0], new.kept[1]
    else:
        before = 0
        limit = min(len(old.spans), len(new.spans))
        while before < limit and old.data[slice(*old.spans[before])] == data[slice(*new.spans[before])]:
            before += 1
        after = 0
        while after < limit - before and old.data[slice(*old.spans[-1 - after])] == \
                data[slice(*new.spans[-1 - after])]:
            after += 1
    lo, hi_old, hi_new = before, len(old.spans) - after, len(new.spans) - after
    if new is not old:
        # Lane G: the state it was read from is not kept by it (each version's summary held
        # the one before, so the owner held every version's bytes).
        new.kept = None
    stories = _definitions(doc, changed) if changed else False
    body = package.tree(main).find(_W + "body")
    blocks = [child for child in body if child.tag in _BLOCKS]
    if len(blocks) != len(new.spans):
        raise Unsupported("the body's blocks and its bytes' differ")
    elements = blocks[lo:hi_new]
    mirror = doc.mirror
    # Within the region, the blocks byte for byte as they were are kept, not made again (a
    # batch's two edits far apart change two blocks, not every one between them).
    same: list = [None] * len(elements)
    if hi_old > lo and hi_new > lo:
        import difflib

        def digests(data: bytes, spans: list) -> list[bytes]:
            return [hashlib.blake2b(data[a:b], digest_size=16).digest() for a, b in spans]

        before = digests(old.data, old.spans[lo:hi_old])
        after = digests(data, new.spans[lo:hi_new])
        matcher = difflib.SequenceMatcher(None, before, after, autojunk=False)
        for tag, i1, i2, j1, j2 in matcher.get_opcodes():
            if tag == "equal":
                for k in range(i2 - i1):
                    same[j1 + k] = mirror.units[lo + i1 + k]
    doc.undefined |= _undefined(live, [e for e, kept in zip(elements, same) if kept is None], doc.definitions)
    doc.seq += 1
    with _records(live, mirror):
        extra = {"constants": constants(live, mirror)} if stories else {}
        if stories and mirror.sections_json() != doc.sections:
            extra["sections"] = doc.sections = mirror.sections_json()
        # every unit on the document's own elements now (a neighbour made again reads them)
        for unit, element in zip(mirror.units[:lo], blocks[:lo]):
            unit.element = element
        for unit, element in zip(mirror.units[hi_old:], blocks[hi_new:]):
            unit.element = element
        text, stats = mirror.splice(lo, hi_old, elements, doc.seq, extra, same)
    _check_keys(mirror)
    c.commit(doc.key, text)
    doc.summary = new
    if version is None:
        return                  # (an edit's state, :func:`sync_state`: no version of its own)
    doc.version = version
    doc.seqs[version] = doc.seq
    while len(doc.seqs) > 64:
        doc.seqs.popitem(last=False)


#: The parts a change to which the mirror follows when it only adds (styles, lists) or reads
#: the same (settings): those whose names end so.
_DEFINITIONS = ("word/styles.xml", "word/numbering.xml", "word/settings.xml")
#: Parts whose changes draw nothing the facts read.
_IGNORED = ("[Content_Types].xml",)


def _undefined(document: Any, elements: list, definitions: dict[str, bytes]) -> set[str]:
    """The style ids and list ids ``elements`` name that ``definitions`` do not define."""
    from lxml import etree

    named: set[str] = set()
    for element in elements:
        for node in element.iter(_W + "pStyle", _W + "rStyle", _W + "tblStyle", _W + "numId"):
            value = node.get(_W + "val")
            if value is not None:
                named.add(("n:" if node.tag == _W + "numId" else "s:") + value)
    defined: set[str] = set()
    for part, data in definitions.items():
        if part.endswith("styles.xml") and data:
            for node in etree.fromstring(data).iter(_W + "style"):
                defined.add("s:" + (node.get(_W + "styleId") or ""))
        elif part.endswith("numbering.xml") and data:
            for node in etree.fromstring(data).iter(_W + "num"):
                defined.add("n:" + (node.get(_W + "numId") or ""))
    return named - defined


def _children(data: bytes, tag: str, key: str) -> tuple[dict[str, bytes], bytes]:
    """A definitions part's children of ``tag`` by their ``key`` attribute (as bytes), and
    the rest of the part's children's bytes."""
    from lxml import etree

    if not data:
        return {}, b""
    root = etree.fromstring(data)
    found: dict[str, bytes] = {}
    rest = []
    for child in root:
        if child.tag == _W + tag:
            found[child.get(_W + key) or ""] = etree.tostring(child)
        elif not isinstance(child.tag, str) or child.tag in (_W + "latentStyles", _W + "numIdMacAtCleanup"):
            continue
        else:
            rest.append(etree.tostring(child))
    return found, b"".join(rest)


def _relationships(data: bytes) -> dict[str, tuple]:
    from lxml import etree

    out = {}
    for node in etree.fromstring(data):
        out[node.get("Id")] = (node.get("Type"), node.get("Target"), node.get("TargetMode"))
    return out


def _definitions(doc: RustDoc, changed: dict[str, bytes]) -> bool:
    """Follow a change to the parts besides the body: styles or lists only added (the
    existing ones as they were, none of the new named by a block before), the settings
    reading the same, the content types; anything else makes the mirror stale."""
    from docx2svg.parse.document import settings_digest
    from docx2svg.parse.styles import parse_numbering, parse_styles

    from .exporter import Unsupported

    d2 = doc.mirror.d2
    added: set[str] = set()
    stories = False
    for part, data in changed.items():
        old = doc.definitions.get(part)
        if part.endswith(_IGNORED) or part.startswith(("customXml/", "docProps/")):
            pass                        # nothing laid out reads them
        elif part.endswith("word/settings.xml") and old is not None:
            if settings_digest(old) != settings_digest(data):
                raise Unsupported("the settings read otherwise")
        elif part.endswith("word/styles.xml") or part.endswith("word/numbering.xml"):
            styles = part.endswith("styles.xml")
            groups = [("style", "styleId", "s:")] if styles else [("abstractNum", "abstractNumId", "a:"),
                                                                  ("num", "numId", "n:")]
            for tag, key, prefix in groups:
                before, rest_before = _children(old or b"", tag, key)
                after, rest_after = _children(data, tag, key)
                if any(after.get(k) != v for k, v in before.items()) or (old and rest_before != rest_after):
                    raise Unsupported(f"{part}: a definition changed")
                added |= {prefix + k for k in after if k not in before}
            if styles:
                d2.styles = parse_styles(data)
            else:
                numbering = parse_numbering(data, d2.styles)
                if b"numPicBullet" in data:
                    raise Unsupported("picture bullets")
                d2.numbering, d2.numbering_lists, d2.numbering_restarts = (numbering.levels, numbering.lists,
                                                                          numbering.restarts)
            doc.definitions[part] = data
        elif part.endswith(".rels") and old is not None:
            if not _relationships(old).items() <= _relationships(data).items():
                raise Unsupported(f"{part}: a relationship changed")
            doc.definitions[part] = data
        elif old is not None and doc.mirror.story_of_part(part) is not None:
            doc.mirror.restory(part, data)
            stories = True
            doc.definitions[part] = data
        else:
            raise Unsupported(f"{part} changed")
        doc.parts[part] = hashlib.blake2b(data, digest_size=16).digest()
    if added & doc.undefined:
        raise Unsupported("a style or list a block named before is defined now")
    d2.__dict__.pop("_resolved", None)
    return stories


# -- the check -----------------------------------------------------------------------------------


#: The fields facts' note (``shared._field_facts``).
FIELDS_NOTE = "results are caches: after moving headings or text, word_fields update"


class Fallback(Exception):
    """This check runs docx-agent's own way (why: :data:`STATS`)."""


def serve(call: Any, arguments: dict[str, Any], timing: Any) -> Any:
    """``check``'s result for these arguments from the document's Rust owner, or ``None``
    (the handler runs as before)."""
    c = core()
    if c is None:
        return None
    why = request_supported(arguments, call.document)
    if why is not None or not call.may_unlock or call.in_batch:
        STATS[f"fallback: {why or 'in a batch'}"] += 1
        return None
    try:
        return _serve(c, call, arguments, timing)
    except Fallback as why:
        STATS[f"fallback: {why}"] += 1
        return None
    except Exception as error:  # noqa: BLE001 -- docx-agent's own way says what is wrong
        # (an argument refused here is refused there the same way)
        STATS[f"error: {type(error).__name__}"] += 1
        log.debug("docx-agent: a check served by the Rust owner failed", exc_info=True)
        return None


def _serve(c: Any, call: Any, arguments: dict[str, Any], timing: Any) -> Any:
    from ooxml_edit.tools import Result, ToolError

    from ..scope import MAX_RANGES, Scope, ranges_of, read_scope
    from ..tools import keep_trials
    from ..tools.shared import KEEP_TRIALS, _original_lost, _field_facts, resolve

    document, entry = call.document, call.entry
    include = arguments.get("include")
    wanted = list(include or ["validate", "reflow", "fields"])
    wanted = [k for k in wanted if k != "numbers"]
    if not wanted:
        raise Fallback("numbers alone")
    with timing.part("resolve"):
        ranges = ranges_of(resolve(call, arguments.get("range")))
        if len(ranges) > MAX_RANGES:
            raise ToolError("limit", f"{len(ranges)} ranges: at most {MAX_RANGES} per check; a span p:A..t:B "
                            "covers many blocks", field="range")
        scope = _cached_scope(call, ranges)
        spans = _spans(ranges)
        if spans is None:
            if scope is None:
                scope = Scope(document, ranges)
            spec = []
            for found in scope.ranges:
                blocks = _whole_blocks(document, found)
                if blocks is None:
                    raise Fallback("a range of parts of blocks")
                spec.append({"address": found.address, "paragraphs": list(found.paragraphs), "blocks": blocks})
        else:
            # Blocks and spans of blocks: resolved by the owner, from its own list of blocks.
            spec = [{"address": address, "span": [first, last]} for address, first, last in spans]
    doc = open_doc(call)
    if doc is None:
        raise Fallback("document not modelled")
    version = document.history.version
    seq = doc.seqs.get(version) if doc.version >= 0 else None
    live = _live(document)
    if seq is None or doc.stale is not None:
        raise Fallback("version not followed")
    if "layout" not in wanted:
        wanted.append("layout")
    keeps = KEEP_TRIALS or "keeps" in wanted
    report: dict[str, Any] = {}
    seen = _seen_before(live, doc, seq)
    since = "reflow" in wanted and seen is not None and seen[2] != version
    if "reflow" in wanted and seen is None and live.__dict__.get("_tool_seen_layout") is not None:
        raise Fallback("a layout seen before the owner")
    aliases = dict(document._aliases) if since else {}
    from .. import layout_facts
    from ..tools import scoped

    request = {"ranges": spec, "derive": scope is None,
               "subjects": sorted(scope.subjects()) if scope is not None else [],
               "scoped": sorted(scope.paragraphs) if scope is not None else [],
               "keeps": bool(keeps and keep_trials.asked()), "layout": True, "since": since, "aliases": aliases,
               "fields": "fields" in wanted,
               # the caps as docx-agent's modules hold them now
               "caps": [layout_facts.CAP, scoped.EXTENT_PAGES, scoped.BREAKS_LISTED,
                        _trial_cap(keep_trials.asked())]}
    started = time.perf_counter()
    with call.unlocked():
        text, snap = c.check(doc.key, seq, json.dumps(request), seen[3] if since else None)
    elapsed = time.perf_counter() - started
    if text is None:
        raise Fallback("version's snapshot not kept")
    answer = json.loads(text)
    # the owner's layout of the version (the wait for its snapshot), then the facts read there
    waited = min(elapsed, (answer.get("ms") or {}).get("wait", 0.0) / 1000)
    timing.add("layout", waited)
    timing.add("facts", elapsed - waited)
    if answer.get("fallback"):
        raise Fallback(answer["fallback"])
    if scope is None:
        scope = RustScope(document, answer["resolved"], ranges)
        _keep_scope(call, ranges, scope)
    call.read_scope = read_scope(scope, layout=True)
    if "validate" in wanted:
        with timing.part("validate"):
            _validate_in(document, entry, scope, report, cache=not call.in_batch)
    started = time.perf_counter()
    report["range"] = answer["range"]
    on = answer["on"]
    if "reflow" in wanted:
        count = answer["pages"]
        coverage, warnings = _coverage(doc.mirror, count, answer["blocks"])
        reflow: dict[str, Any] = {"pages": count, "complete": True, "coverage": coverage}
        found = answer.get("since")
        if found is not None:
            found["changed"] = [p for p in found["changed"] if p in on]
            reflow["since_last_check"] = found
        if warnings:
            reflow["layout_warnings"] = warnings
        report["reflow"] = reflow
        _seen_add(live, doc, seq, version, snap, "rust")
    layout = answer["layout"]
    layout = _original_lost(document, scope, layout)
    from ..tools.objects import ragged_made_by

    ragged_made_by(document, layout)
    report["layout"] = layout
    timing.add("facts", time.perf_counter() - started)
    if "fields" in wanted:
        with timing.part("fields"):
            fields = answer.get("fields")
            report["fields"] = {**fields, "note": FIELDS_NOTE} if fields is not None else _field_facts(document)
    summary = "Checked " + ", ".join(wanted) + f" in {len(scope.ranges)} range(s)"
    report["timing"] = timing.parts
    STATS["served"] += 1
    # (the order the handler writes them in)
    ordered = {k: report[k] for k in ("validate", "range", "reflow", "layout", "fields", "timing") if k in report}
    return Result(summary=summary, data=ordered)


def _validate_in(document: Any, entry: Any, scope: Any, report: dict[str, Any], *, cache: bool) -> None:
    """``shared._validate``: with no problems in the document, none in the scope and none
    elsewhere, read without placing them (a scope made here reads its roots only to place
    some)."""
    from ..tools.format import FORMAT
    from ..tools.shared import _validate, problems_now, validate_delta

    found = problems_now(entry, FORMAT, cache=cache)
    if found:
        _validate(document, entry, scope, report, cache=cache)
        return
    delta = validate_delta(entry, found)
    report["validate"] = {"new": [], "fixed": delta["fixed"], "baseline": delta["baseline"], "problems": [],
                          "elsewhere": 0}


#: A block's id as the Word tools write it, and a span of two.
_SPAN = re.compile(r"^([pt]:[0-9A-F]{8})(?:\.\.([pt]:[0-9A-F]{8}))?$")


def _spans(ranges: list[str]) -> "list[tuple[str, str, str]] | None":
    """``[(address, first, last)]`` where every range is a top-level block or a span of
    them by their ids (the owner resolves those), else ``None``."""
    out = []
    for address in ranges:
        found = _SPAN.match(address)
        if found is None:
            return None
        out.append((address, found.group(1), found.group(2) or found.group(1)))
    return out


def _cached_scope(call: Any, ranges: list[str]) -> Any:
    found = call.__dict__.get("_rust_scope")
    return found[1] if found is not None and found[0] == tuple(ranges) else None


def _keep_scope(call: Any, ranges: list[str], scope: Any) -> None:
    """The scope a check's numbers resolved, for its handler (one resolution per call)."""
    call.__dict__["_rust_scope"] = (tuple(ranges), scope)


def _rust_scope_class() -> Any:
    from ..scope import Range, Scope

    class _RustScope(Scope):
        """:class:`docx_agent.scope.Scope` of some blocks and spans of blocks, as the
        document's owner resolved them (its own list of the body's blocks): the same
        paragraphs, blocks and tables; ``roots`` (the elements validate's problems are looked
        for in) from docx-agent's own reading, made when asked (only where there are
        problems to place)."""

        def __init__(self, document: Any, resolved: dict[str, Any], addresses: list[str]) -> None:
            self.document = document
            self.addresses = list(addresses)
            self.ranges = [Range(r["address"], list(r["paragraphs"]), list(r["blocks"]), [])
                           for r in resolved["ranges"]]
            self.paragraphs = {p for r in self.ranges for p in r.paragraphs}
            self.tables = set(resolved["tables"])
            self.drawings = set()
            self._roots = None

        @property
        def roots(self) -> list:
            if self._roots is None:
                self._roots = Scope(self.document, self.addresses).roots
            return self._roots

    return _RustScope


def RustScope(document: Any, resolved: dict[str, Any], addresses: list[str]) -> Any:  # noqa: N802
    return _rust_scope_class()(document, resolved, addresses)


def number_texts(call: Any, scope_arguments: Any) -> Any:
    """``textfacts.number_texts``' texts of a range, from the document's owner (each
    paragraph's text kept with its block), or ``None`` (read its own way)."""
    c = core()
    arguments = getattr(call, "arguments", None)
    if c is None or not call.may_unlock or call.in_batch or request_supported(arguments, call.document) or \
            scope_arguments.get("pages") is not None or scope_arguments.get("slides") is not None:
        return None
    from ..scope import ranges_of
    from ..tools._base import resolve

    ranges = ranges_of(resolve(call, scope_arguments.get("range")))
    spans = _spans(ranges) if ranges else None
    if not spans:
        return None
    try:
        doc = open_doc(call)
    except Exception:  # noqa: BLE001
        return None
    if doc is None or doc.stale is not None:
        return None
    seq = doc.seqs.get(call.document.history.version)
    if seq is None:
        return None
    request = {"ranges": [{"address": a, "span": [f, l]} for a, f, l in spans], "derive": True, "numbers": True}
    try:
        with call.unlocked():
            text, _ = c.check(doc.key, seq, json.dumps(request), None)
    except Exception:  # noqa: BLE001
        return None
    if text is None:
        return None
    answer = json.loads(text)
    if answer.get("fallback"):
        return None
    _keep_scope(call, ranges, RustScope(call.document, answer["resolved"], ranges))
    return answer["texts"]


def _in_layout_order(mirror: Any, substitutions: dict) -> list:
    """The faces substituted in the order docx2svg's layout of the document meets them: the
    body's in the order its blocks first name them, then the rest (the headers' and
    footers') as the mirror met them."""
    left = dict(substitutions)
    out = []
    for unit in mirror.units:
        for face in unit.faces:
            found = left.pop(face, None)
            if found is not None:
                out.append(found)
        if not left:
            break
    return out + list(left.values())


def _coverage(mirror: Any, pages: int, blocks: int) -> tuple[dict[str, Any], list[str]]:
    """``reflow``'s ``coverage`` and ``layout_warnings`` of a complete layout
    (``layout.coverage_facts``, docx2svg's ``coverage_of`` and the conversion's warnings):
    the faces substituted as the mirror measured them, the parser's warnings."""
    import docx2svg

    from ..tools._base import short

    fonts = mirror.fonts
    if getattr(fonts, "missing", None):
        raise Fallback("a face missing")
    # The exporter asks every face for every character it sends; a fallback counts where the
    # face draws the character (docx2svg's layout records only those), or for a character
    # the body does not hold (a header's).  One such is said as docx2svg says it; several
    # would be said in the order the layout met them, which is not known here.
    chars, drawn = mirror.ex.chars, mirror.ex.drawn
    real = [(family, char, used) for (family, char), used in
            (getattr(fonts, "glyph_fallbacks", None) or {}).items()
            if char not in chars or (family, char) in drawn]
    if len(real) > 1 or any(char not in chars for _, char, _ in real):
        raise Fallback("glyphs measured with other faces")
    from ..layout import coverage_facts

    # (docx2svg's StopInfo of such a fallback: its text is the reason, no message)
    approximations = [{"page": None, "path": None, "message": None,
                       "reason": f"{family!r} has no glyph for {char!r}: measured with "
                                 f"{'its missing glyph' if used == '.notdef' else repr(used)}"}
                      for family, char, used in real]
    substitutions = _in_layout_order(mirror, getattr(fonts, "substitutions", None) or {})
    coverage = coverage_facts({"complete": True, "pages": pages, "blocks_laid_out": blocks, "blocks": blocks,
                               "estimate_source": "layout", "stop": None, "story_stops": [],
                               "substituted_fonts": [s.as_dict() for s in substitutions],
                               "missing_fonts": [], "approximations": approximations})
    warnings = [docx2svg.Warning(code, docx2svg._PARSER_WARNINGS.get(code, "a body element is not read"))
                for code in mirror.d2.warnings]
    warnings += [docx2svg.Warning("font-substituted", s.message()) for s in substitutions]
    for family, char, used in real:
        source = "its missing glyph (.notdef)" if used == ".notdef" else repr(used)
        warnings.append(docx2svg.Warning(
            "font-glyph-fallback",
            f"{family!r} has no glyph for {char!r} (U+{ord(char):04X}): measured with {source}, as an "
            "approximation (Word's own font fallback is not measured)"))
    return coverage, [short(str(w), 160) for w in warnings[:10]]


def _trial_cap(asked: dict[str, float]) -> int:
    """The keeps a check tries at most: ``trials``, the first alone where no seconds are
    given for them (``keep_trials``: none is started past them; here each takes no time)."""
    trials = int(asked["trials"])
    return min(trials, 1) if asked["seconds"] <= 0 else trials


def _whole_blocks(document: Any, found: Any) -> list[str] | None:
    """The top-level blocks of a range of whole blocks (``scoped.range_keeps``' reading), or
    ``None`` for a range of parts of blocks."""
    from ..edit.copies import blocks_of

    try:
        blocks = blocks_of(document, found.address)
    except (KeyError, ValueError):
        return None
    held = {id(element) for element in found.elements}
    for _, element in blocks:
        if element.tag not in (_P, _TBL):
            return None
        inside = [element] if element.tag == _P else list(element.iter(_P))
        if not all(id(paragraph) in held for paragraph in inside):
            return None
    if len(held) != sum(1 if e.tag == _P else len(list(e.iter(_P))) for _, e in blocks):
        return None
    return [identifier for identifier, _ in blocks]


def pages_of(call: Any, ids: list[str]) -> Any:
    """The pages of ``ids`` from the document's Rust owner (a check of numbers), or ``None``
    where it does not follow this version (the caller lays out its own way)."""
    c = core()
    if c is None:
        return None
    live = _live(call.document)
    doc = live.__dict__.get("_rust_doc")
    if (doc is None or doc.stale is not None) and call.may_unlock and not call.in_batch \
            and request_supported(getattr(call, "arguments", None), call.document) is None:
        try:
            doc = open_doc(call)        # the check it is part of is served here: opened first
        except Exception:  # noqa: BLE001 -- laid out docx-agent's own way
            doc = None
    if doc is None or doc.stale is not None:
        return None
    seq = doc.seqs.get(call.document.history.version)
    if seq is None:
        return None
    if any("/" in i for i in ids):
        return None
    keys = sorted(set(ids))
    if not keys:
        from ..tools.textfacts import _PagesOf

        return _PagesOf({})
    try:
        with call.unlocked() if call.may_unlock else _held():
            text, _ = c.check(doc.key, seq, json.dumps({"pages_of": keys}), None)
    except Exception:  # noqa: BLE001 -- laid out docx-agent's own way
        return None
    if text is None:
        return None
    from ..tools.textfacts import _PagesOf

    pages = json.loads(text).get("pages_of") or []
    return _PagesOf({k: p for k, p in zip(keys, pages) if p})


class _held:
    def __enter__(self) -> None:
        return None

    def __exit__(self, *exc: Any) -> None:
        return None


#: Checks seen (``since_last_check``) kept per document, the latest.
_SEEN_KEPT = 64
_seen_lock = threading.Lock()
_seen_order = itertools.count(1)


def _seen_before(live: Any, doc: RustDoc, seq: int) -> Any:
    """The check seen last before one of the owner's version ``seq``, as the checks of a
    document follow each other one at a time in the order of the versions they read (a
    check of an older version that ends after one of a newer is before it):
    ``(owner key, order, version, snapshot, by)``, or ``None``.  One seen by an owner made
    before this one is before all of this one's."""
    with _seen_lock:
        log = list(live.__dict__.get("_rust_seen") or ())
    found = [e for e in log if e[0][0] != doc.key or e[0][1] <= seq]
    if not found:
        return None
    best = max(found, key=lambda e: (e[0][0] == doc.key, e[0][1] if e[0][0] == doc.key else 0, e[1]))
    return (best[0], best[1], best[2], best[3], best[4])


def _seen_add(live: Any, doc: RustDoc, seq: int, version: int, snap: Any, by: str) -> None:
    with _seen_lock:
        log = live.__dict__.get("_rust_seen")
        if not isinstance(log, list):
            log = live.__dict__["_rust_seen"] = []
        log.append(((doc.key, seq), next(_seen_order), version, snap, by))
        del log[:-_SEEN_KEPT]


def since_for_python(call: Any, version: int) -> Any:
    """For docx-agent's own check of a document the Rust owner follows: ``(True, reflow)``
    -- ``since_last_check`` against the version a check served here saw last (its own
    layout's summary is then of an older one) -- or ``(False, None)``: as before."""
    live = _live(call.document)
    doc = live.__dict__.get("_rust_doc")
    seq = doc.seqs.get(version) if doc is not None else None
    seen = _seen_before(live, doc, seq) if seq is not None else None
    if seen is None or seen[4] != "rust":
        return False, None
    if seen[2] == version:
        return True, None           # the version that check saw: nothing since
    snap = _snapshot(call, doc, version)
    if snap is None:
        return False, None
    c = core()
    request = {"since": True, "aliases": dict(call.document._aliases)}
    text, _ = c.check(doc.key, seq, json.dumps(request), seen[3])
    return True, json.loads(text).get("since")


def seen_by_python(call: Any, version: int) -> None:
    """docx-agent's own check saw ``version``: the snapshot of it, for the next check served
    here (``since_last_check``)."""
    live = _live(call.document)
    doc = live.__dict__.get("_rust_doc")
    if doc is None:
        return
    snap = _snapshot(call, doc, version)
    if snap is not None:
        _seen_add(live, doc, doc.seqs[version], version, snap, "python")


def _snapshot(call: Any, doc: RustDoc, version: int) -> Any:
    c = core()
    seq = doc.seqs.get(version)
    if c is None or doc.stale is not None or seq is None:
        return None
    try:
        _, snap = c.check(doc.key, seq, "{}", None)
    except Exception:  # noqa: BLE001
        return None
    return snap
