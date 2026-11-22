"""docx-agent's reading tools answered by the Rust session (phase P1, lane P1c): ``describe``,
``word_read``, ``word_inspect`` (``layout`` too), ``find_text``, ``word_compare_blocks`` and
``word_design_facts`` (lane P2c14) and, through them, the ops of ``read_batch``, each with the
interpreter lock let go while Rust reads; and (lanes P5a, P5b) the checks of a document the
session owns that read the document alone or its pages (:func:`serve_check`).

**The versions.**  Python owns the document (until phase P2).  A reading call reads a
committed version's read copy (ooxml-edit's ``DocumentFormat.read_copy``); the first one on
a document opens it in the process's Rust session from that version's bytes, and each later
version is handed over as the parts whose bytes differ from what the session has
(:meth:`Mirror.version`: an unchanged part is the very object, or the same digest), committed
there as one step: Rust parses only those, and reuses its reading of every story part whose
tree did not change.  Each version handed over is kept as an ``ooxml_core.Version`` (the
latest :data:`KEPT`), and a call reads the version its read copy is of, whatever was
committed since.

**What Python adds.**  The document's state Python holds goes into the answer as before:
``describe``'s ``name``, ``pages`` (the layout's, as :func:`docx_agent.tools.read._pages`
asks it), ``tracking`` and ``problems``, and its ``memory``; ``word_inspect``'s placements,
asked of the document's layout as :func:`docx_agent.tools.read._placed_by_worker` asks them
(a :class:`Placer` Rust calls back once per call, with the lock taken back).

**Falling back.**  Where the Rust answer is not yet the same, the call is answered by the
Python path, counted by tool and reason (:func:`fallbacks`, ``read_fallbacks{tool,reason}``)
and never silently: ``ooxml_core`` without a session; a document that is not a committed
version's read copy (a batch's reads of its own changes); limits below 1 (the host's own
limits are the Rust session's, ``CallContext(limits=ooxml_core.Limits(...))``); an address naming an id this session renamed or deleted (the ledger and
the aliases are Python's until P2); a regular expression Rust does not match as Python does;
A form Rust does not support yet is its refusal (``details.not_supported``).  A refusal is not a fallback (lane P2e1): Rust's is the call's answer, in
docx-agent's words with its own way out (:mod:`.refusals`).
"""

from __future__ import annotations

import hashlib
import json
import logging
import threading
import weakref
from collections import Counter, OrderedDict
from typing import Any, Callable

from ooxml_edit.tools import Result, ToolError

from .. import finalizers
from .refusals import refusal

log = logging.getLogger("docx_agent.rust")

#: The versions of a document kept for the calls reading them.
KEPT = 4

_SESSION: list[Any] = []
_SESSION_LOCK = threading.Lock()
_COUNTS_LOCK = threading.Lock()
_FALLBACKS: Counter = Counter()
_SERVED: Counter = Counter()


def _session() -> Any:
    """The process's Rust session, made when first needed; ``None`` where ``ooxml_core`` has
    none (said once, in the log)."""
    with _SESSION_LOCK:
        if not _SESSION:
            try:
                import ooxml_core

                _SESSION.append(ooxml_core.Session() if hasattr(ooxml_core, "Version") else None)
            except ImportError:
                _SESSION.append(None)
            if _SESSION[0] is None:
                log.warning("docx-agent: ooxml-core has no session with versions; reads run in Python")
        return _SESSION[0]


def use_session(session: Any) -> Any:
    """Answer reads in ``session`` (an ``ooxml_core.Session``) from now on -- ``None``: in
    none, every read by the Python path (counted, ``no session``) -- and give back the one
    used before (``None`` where none was made yet).  For an application that holds its own
    session, and for the differential tests."""
    with _SESSION_LOCK:
        before = _SESSION[0] if _SESSION else None
        _SESSION[:] = [session]
        return before


def available() -> bool:
    """Whether reads may be answered in Rust."""
    return _session() is not None


def fallbacks() -> dict[str, dict[str, int]]:
    """``read_fallbacks{tool,reason}``: the reading calls answered by the Python path, by
    tool and reason, since the process started."""
    with _COUNTS_LOCK:
        out: dict[str, dict[str, int]] = {}
        for (tool, reason), n in sorted(_FALLBACKS.items()):
            out.setdefault(tool, {})[reason] = n
        return out


def served() -> dict[str, int]:
    """The reading calls answered in Rust, by tool, since the process started."""
    with _COUNTS_LOCK:
        return dict(_SERVED)


def _fell_back(tool: str, reason: str) -> None:
    with _COUNTS_LOCK:
        _FALLBACKS[(tool, reason)] += 1


def _served(tool: str) -> None:
    with _COUNTS_LOCK:
        _SERVED[tool] += 1


# -- the document's versions in the Rust session ---------------------------------------------


def _close(session: Any, doc: str) -> None:
    try:
        session.close(doc)
    except Exception:  # noqa: BLE001 -- closed already
        pass


class Mirror:
    """One document in the Rust session: what of each part the session's latest version
    has, and the versions handed over, by the document's version number."""

    def __init__(self, session: Any, doc: str, parts: dict[str, Any]) -> None:
        self.session = session
        self.doc = doc
        #: Part -> what the session's latest version has of it (:func:`_identities`).
        self.parts = parts
        self.versions: OrderedDict[int, Any] = OrderedDict()
        #: Lane P2c10: the session's version -> the document's version number, for every
        #: version kept (:meth:`keep`): what the session's record of retired ids is numbered by.
        self.numbers: dict[int, int] = {}
        self.lock = threading.Lock()
        #: Lane P2c4: whether the session owns the document (:mod:`.owner`): its versions are
        #: kept here by the tool layer's numbers as the session makes them, never handed over.
        self.owned = False
        #: The session's version the owned document's Python view is (:mod:`.view`), the
        #: one it was opened as, and the view (a weak reference: the entry holds it).
        self.view_head: Any = None
        self.opened: Any = None
        self.live: Any = None
        #: The tracking mode the session edits the document under (:mod:`.edits`).
        self.tracking: dict[str, Any] | None = None
        #: The view's :func:`docx_agent.layout.laid_mark` when its last Python step was
        #: committed (:meth:`docx_agent.rust.owner.RustOwner.bridge`).
        self.laid: tuple | None = None

    def keep(self, version: Any, first: int | None = None) -> None:
        """Keep ``version`` (an ``ooxml_core.Version``) as the document's version
        ``version.state``; ``first``: the session's first version of the step, where it made
        several (a batch's ops), each numbered so too."""
        number = version.state
        self.versions[number] = version
        for seq in range(first if first is not None else version.number, version.number + 1):
            self.numbers[seq] = number
        self.versions.move_to_end(number)
        while len(self.versions) > KEPT:
            self.versions.popitem(last=False)

    def version(self, copy: Any) -> Any:
        """The ``ooxml_core.Version`` of ``copy``'s version, handed over where it is new."""
        number = copy.history.version
        with self.lock:
            found = self.versions.get(number)
            if found is not None:
                self.versions.move_to_end(number)
                return found
            if self.owned:
                # Committed in the session as it was made (lane P2c3), and let go since.
                raise _Fallback("version let go")
            now = _identities(copy.package)
            changed: dict[str, bytes | None] = {}
            for part, (identity, data) in now.items():
                if self.parts.get(part) != identity:
                    changed[part] = data() if callable(data) else data
            for part in self.parts.keys() - now.keys():
                changed[part] = None
            made = self.session.commit(self.doc, changed)
            self.parts = {part: identity for part, (identity, _) in now.items()}
            self.versions[number] = made
            while len(self.versions) > KEPT:
                self.versions.popitem(last=False)
            return made


def _identities(package: Any) -> dict[str, tuple[Any, Any]]:
    """Each part of ``package``: what tells it from another version's -- a part as read,
    its compressed bytes' digest (the very bytes every version that did not change it
    shares); an edited one, its bytes' digest -- and its bytes (or what reads them).  The
    digests are taken with the interpreter lock let go (``hashlib``, for long bytes)."""
    out: dict[str, tuple[Any, Any]] = {}
    dirty = package.dirty_parts
    for part in list(package._parts):
        if part in dirty:
            data = package.read(part)
        else:
            stored = package._parts.stored(part)
            if stored is None:
                continue
            if not isinstance(stored, (bytes, bytearray, memoryview)):
                out[part] = (("zip", stored.size, _digest(stored.compressed)), stored.read)
                continue
            data = bytes(stored)
        out[part] = (("bytes", len(data), _digest(data)), data)
    return out


def _digest(data: Any) -> bytes:
    return hashlib.blake2b(data, digest_size=20).digest()


_MIRRORS_LOCK = threading.Lock()


def _mirror(copy: Any) -> "Mirror | None":
    """The Rust session's copy of the document ``copy`` is a version of, opened from
    ``copy``'s bytes the first time; ``None`` where ``copy`` is not a read copy."""
    live = copy.__dict__.get("_live")
    if live is None:
        return None
    return mirror_of(live, copy)


def mirror_of(live: Any, copy: Any = None) -> "Mirror | None":
    """The Rust session's copy of the document ``live``, opened the first time from
    ``copy``'s bytes (a read copy of it; ``live``'s own where none is given); ``None`` where
    there is no session."""
    finalizers.drain()
    session = _session()
    if session is None:
        return None
    found = live.__dict__.get("_rust_reads")
    if found is not None and found.session is session:
        return found
    with _MIRRORS_LOCK:
        found = live.__dict__.get("_rust_reads")
        if found is not None and found.session is session:
            return found
        source = live if copy is None else copy
        data = source.to_bytes()
        doc = session.open(data, kind="docx", name="document.docx", ids=_earlier_ids(source))
        parts = {part: identity for part, (identity, _) in _identities(source.package).items()}
        made = Mirror(session, doc, parts)
        made.versions[source.history.version] = session.head(doc)
        # L4b: the finalizer only hands the close over (docx_agent.finalizers).
        weakref.finalize(live, finalizers.later, _close, session, doc)
        live.__dict__["_rust_reads"] = made
        return made


def _earlier_ids(document: Any) -> "tuple[list[int], list[tuple[str, int]], int, list[int]] | None":
    """The ids ``document`` had before it was saved, where its id state was opened with it
    (round 38: its ledger's pool, restored): every value, each seed's cursor, the highest
    drawing number and the drawing numbers retired (``drawings_gone``) -- for the session
    never to give a retired id again, nor keep a retired drawing number in a copy.  ``None``
    where its pool is not read yet (the parts' ids are all it has)."""
    ledger = getattr(document, "_ledger", None)
    pool = getattr(ledger, "pool", None)
    if pool is None:
        return None
    return (list(pool), list(getattr(pool, "cursors", {}).items()), ledger.highest_drawing,
            sorted(ledger.drawings_gone))


# -- serving a call ---------------------------------------------------------------------------


class _Fallback(Exception):
    """The call is answered by the Python path, for this reason."""


def _named(call: Any) -> Any:
    """The document whose ids an address of the call names: the call's; for one the session
    owns (or a read of a version of it), its view where that is current at the call's
    version, else none -- the session follows its renames and refuses a deleted id itself
    (worded as the view words it)."""
    entry = call.entry
    owned = _owned(entry)
    if owned is None:
        return call.document
    if owned.view_current and owned.version == entry.version:
        # (The view as held: reading it through the entry would open the step of a call
        # changing it meanwhile, in this call's thread.)
        return owned.held
    return None


def _owned(entry: Any) -> Any:
    """The entry the session owns that ``entry`` is, or reads a committed version of."""
    for found in (entry, getattr(entry, "live", None)):
        if isinstance(getattr(found, "handle", None), Mirror):
            return found
    return None


def _renamed_or_deleted(document: Any, *texts: Any) -> bool:
    """Whether an address names an id this session renamed (an alias) or deleted (the
    ledger): Python's answer follows the alias or says who deleted it."""
    from ..edit.document import _GONE_BLOCK

    if document is None:
        return False

    aliases = getattr(document, "_aliases", None) or {}
    for text in texts:
        if not isinstance(text, str):
            continue
        if document._deleted(text) is not None:
            return True
        if aliases:
            for match in _GONE_BLOCK.finditer(text):
                named = match.group(0).lstrip("/")
                if named in aliases or named.split("/")[-1] in aliases:
                    return True
    return False


def _refs(call: Any, value: Any) -> Any:
    """``value`` with its ``$name`` refs resolved (a span's ends too), as the tools resolve
    them; a heading's range is Rust's to expand."""
    if isinstance(value, str) and ".." in value and "$" in value:
        first, _, last = value.partition("..")
        return "..".join(call.resolve_ref(end.strip()) if end.strip().startswith("$") else end
                         for end in (first, last))
    if isinstance(value, str) and value.startswith("$"):
        return call.resolve_ref(value)
    return value


def _owned_version(entry: Any) -> tuple["Mirror | None", Any]:
    """Lane P2c4: the session's version a call reads of a document the session owns (the
    entry's version now), found without the Python view; ``(None, None)`` otherwise."""
    owned = _owned(entry)
    mirror = getattr(owned, "handle", None)
    if not isinstance(mirror, Mirror):
        return None, None
    if owned is entry and getattr(entry.history, "grouped", False):
        # Lane H1: a call of a batch reads what the batch's ops made so far.
        entry.flush_view()
        return mirror, entry.history.reading_head()
    with mirror.lock:
        found = mirror.versions.get(entry.version)
    if found is None:
        raise _Fallback("version let go")
    return mirror, found


def _version_read(entry: Any) -> tuple["Mirror", Any]:
    """The Rust session's copy of the document ``entry`` is, and the version of it a call
    reads (``entry.document``: the version the call was given)."""
    mirror, version = _owned_version(entry)
    if mirror is None:
        copy = entry.document
        mirror = _mirror(copy)
        if mirror is None:
            raise _Fallback("not a committed version")
        version = mirror.version(copy)
    return mirror, version


def _with_others(call: Any, arguments: dict[str, Any], others: tuple[str, ...],
                 session: Any) -> tuple[dict[str, str], list[Any]]:
    """The host's names of the other documents the call names (at a path of ``others``,
    ``reference.doc``: its tool's ``documents`` after the first), each the session's id of
    it (``Session.call``'s ``names``), and the versions of them the call reads (lane P2c14;
    #103's ``Call::others``).  The call's own document named again is the tool's."""
    versions: list[Any] = []
    names: dict[str, str] = {}
    for given in (found for path in others for found in _at_path(arguments, path.split("."))):
        if not isinstance(given, str) or given == getattr(call.entry, "doc_id", None) or given in names:
            continue
        entry = call.entries.get(given)
        if entry is None:
            raise _Fallback("a document not open")
        mirror, version = _version_read(entry)
        if mirror.session is not session:
            raise _Fallback("a document in another session")
        names[given] = mirror.doc
        versions.append(version)
    return names, versions


def _at_path(value: Any, path: list[str]) -> list[Any]:
    """The values at ``path`` in ``value`` (``reference.doc``; a part ending ``[]`` is each
    item of a list: ``copies[].reference.doc``)."""
    if not path:
        return [value]
    head, rest = path[0], path[1:]
    if head.endswith("[]"):
        items = value.get(head[:-2]) if isinstance(value, dict) else None
        return [found for item in items or () for found in _at_path(item, rest)] \
            if isinstance(items, list) else []
    return _at_path(value.get(head), rest) if isinstance(value, dict) else []


def _call(call: Any, tool: str, arguments: dict[str, Any], *, placer: "Placer | None" = None,
          others: tuple[str, ...] = (), layout: Any = None, raster: Any = None,
          images: "list[tuple[str, int, int]] | None" = None,
          context: "Callable[[], dict[str, Any]] | None" = None,
          files: "list[tuple[str, str, bytes]] | None" = None) -> dict[str, Any]:
    """The Rust envelope of ``tool`` with ``arguments`` on the version ``call`` reads (and
    of each other document named at a path of ``others``, :func:`_with_others`); ``layout``:
    the document's page engine (an ``ooxml_core.WordPages``), for a call that reads the
    pages (``check``'s ``reflow``, ``render``), laying out within
    ``call.limits.layout_timeout`` (lane P5c: past it, ``reflow.error`` says so, as the
    Python path's does); ``raster``: the faces ``render`` draws pages with (an
    ``ooxml_core.PageRaster``, :func:`host_raster`) and ``images`` the session's and the
    round's image budgets as they stand (:func:`image_budgets`). Run with the
    ``ooxml_core.CallContext`` of the host's limits and the keywords ``context`` gives where
    given -- asked only once the session serves the call, so a call answered in Python never
    reads them; the files it
    gives the application appended to ``files`` as ``(name, format, bytes)``."""
    if _session() is None:
        raise _Fallback("no session")
    mirror, version = _version_read(call.entry)
    names, pinned = _with_others(call, arguments, others, mirror.session)
    # the documents named as the host names them: the reply says those names (lane P2c17)
    names[call.entry.doc_id] = mirror.doc
    arguments = {**{k: v for k, v in arguments.items() if v is not None}, "doc": call.entry.doc_id}
    import ooxml_core

    from .owner import context_of

    # the host's limits on the reply: the Rust session pages as the Python path would (lane
    # H3); a call that lays the document out does so within the host's layout_timeout; what
    # ``context`` adds, asked only now that the session serves the call
    made = context_of(None, call.limits, call.limits.layout_timeout if layout is not None else None,
                      **(context() if context is not None else {}))
    laid = None
    if layout is not None or raster is not None or placer is not None:
        laid = ooxml_core.CallLayout(layout, raster=raster, image_budgets=images,
                                     placer=placer.place if placer is not None else None)
    reply, given_files, app = mirror.session.call(tool, json.dumps(arguments), ooxml_core.CallOptions(
        context=made, documents=ooxml_core.CallDocuments(at=[version, *pinned], names=names), layout=laid))
    if placer is not None and placer.error is not None:
        raise placer.error
    envelope = json.loads(reply)
    error = envelope.get("error")
    if error is not None:
        if error.get("field") == "regex":
            raise _Fallback("regex")        # (a pattern Rust does not read as Python does)
        # A refusal is the call's answer, in docx-agent's words with Rust's way out (lane P2e1).
        raise refusal(error)
    app = json.loads(app)
    if files is not None:
        files.extend((given["name"], given["format"], data)
                     for given, data in zip(app.get("files") or [], given_files))
    return envelope


def serve(call: Any, tool: str, arguments: dict[str, Any],
          python: Callable[[], Result], *, addresses: tuple = (),
          answer: Callable[[dict[str, Any]], Result] | None = None,
          placer: "Placer | None" = None, why_not: str | None = None,
          others: tuple[str, ...] = (), layout: Any = None, raster: Any = None,
          images: "list[tuple[str, int, int]] | None" = None,
          context: "Callable[[], dict[str, Any]] | None" = None,
          files: "list[tuple[str, str, bytes]] | None" = None) -> Result:
    """``tool``'s answer from the Rust session (``answer`` makes the :class:`Result` of its
    envelope), or ``python()``'s where Rust's is not the same yet (counted, with why).
    ``others``: the paths of ``arguments`` naming another open document; ``layout``,
    ``raster``, ``images``, ``context``, ``files``: as :func:`_call` takes them."""
    try:
        if why_not is not None:
            raise _Fallback(why_not)
        if call.limits.max_result_chars < 1 or call.limits.max_list_items < 1:
            raise _Fallback("limits")
        if _renamed_or_deleted(_named(call), *addresses):
            raise _Fallback("renamed or deleted id")
        envelope = _call(call, tool, arguments, placer=placer, others=others, layout=layout,
                         raster=raster, images=images, context=context, files=files)
        result = (answer or _result)(envelope)
    except _Fallback as why:
        _fell_back(tool, str(why))
        return python()
    except ToolError:
        _served(tool)                   # the session's refusal is the answer
        raise
    _served(tool)
    return result


def serve_check(call: Any, arguments: dict[str, Any]) -> Result | None:
    """Lane P5a: ``check`` answered by the Rust session where it answers it whole -- the
    facts read from the document alone (``validate``, ``fields``) of a document it owns,
    outside a batch -- else ``None`` (counted, with why), and the handler goes on.  What
    needs the layout's facts, copies, a scale and the application's ``min_text_size``
    (which asks for layout facts) are refused as not supported yet by the session.

    Lane P5b: ``reflow`` (asked, or by default) is answered there too, laid out by the
    document's page engine (:func:`docx_agent.pages.word_pages`), handed to the call; the
    session keeps what the check saw for the next one's ``since_last_check``
    (:func:`last_check`)."""
    owned = _owned(call.entry)
    why = None
    layout = None
    if owned is None:
        why = "not owned"
    elif call.in_batch:
        why = "in a batch"
    else:
        why, arguments = _session_check(call, owned.held, arguments)
    if why is None and _laid_check(arguments):
        from ..pages import word_pages

        layout, _ = word_pages(owned.held)     # none: the session refuses it (not supported yet)
    served = serve(call, "check", arguments, lambda: None, why_not=why, layout=layout,
                   others=("copies[].reference.doc",))
    return served


def _session_check(call: Any, held: Any, arguments: dict[str, Any]) -> tuple[str | None, dict[str, Any]]:
    """Lane P5k: why the session cannot answer this check of ``held`` (``None``: it may), and
    the arguments it is asked with: the application's ``min_text_size`` (one number) as its
    ``min_size`` where the check gives none.  What this session's table edits made here
    (rows written, a ragged table's call) the session does not hold yet: such a document's
    layout facts are the handler's."""
    from ..tools._base import resolve
    from ..tools.objects import RAGGED_BY, ROWS_WRITTEN

    if arguments.get("range") is not None:
        # a ``$name`` ref replaced, as the handler does (the session takes addresses)
        arguments = {**arguments, "range": resolve(call, arguments["range"])}
    minimum = getattr(held, "min_text_size", None)
    if minimum is not None and arguments.get("min_size") is None:
        if isinstance(minimum, bool) or not isinstance(minimum, (int, float)):
            return "min_text_size by kind", arguments
        arguments = {**arguments, "min_size": minimum}
    live = call.document.__dict__.get("_live", call.document)
    if _laid_check(arguments) and (live.__dict__.get(ROWS_WRITTEN) or live.__dict__.get(RAGGED_BY)):
        return "rows written here", arguments
    return None, arguments


def _laid_check(arguments: dict[str, Any]) -> bool:
    """Whether a check with these arguments reads the layout: ``reflow`` or ``layout``
    asked (or the default), or what the layout's facts measure (``range``, ``min_size``,
    ``styles``)."""
    include = arguments.get("include")
    if include is None or "reflow" in include or "layout" in include:
        return True
    from ..scope import ranges_of

    return bool(ranges_of(arguments.get("range"))) or any(
        arguments.get(name) is not None for name in ("min_size", "styles"))


#: The faces pages are drawn with in Rust, by the font folders they were made with
#: (``ooxml_core.PageRaster``: the system's fonts and the folders, read once).
_RASTERS: dict[tuple[str, ...], Any] = {}
_RASTERS_LOCK = threading.Lock()


def host_raster(font_dirs: "list[str] | tuple[str, ...]") -> Any:
    """Lane RD1: the ``ooxml_core.PageRaster`` ``render`` draws a document's pages from --
    the system's fonts and ``font_dirs`` (the document's, as :func:`docx2svg.user_font_dirs`
    reads them), read once per set of folders; the session makes each document's raster
    from it, handed the faces its pages draw."""
    import ooxml_core
    from ooxml_common.fonts.office import user_font_dirs

    dirs = tuple(str(path) for path in user_font_dirs(list(font_dirs) or None))
    with _RASTERS_LOCK:
        found = _RASTERS.get(dirs)
    if found is None:
        made = ooxml_core.PageRaster([], list(dirs))
        with _RASTERS_LOCK:
            found = _RASTERS.setdefault(dirs, made)
    return found


def image_budgets(call: Any) -> list[tuple[str, int, int]]:
    """The session's and the round's image budgets as they stand for ``call``, each
    ``(name, limit, used)`` where the host's limits set one: what the Rust session refuses
    images past, worded as :meth:`ooxml_edit.tools.session.Session.check_images` words it
    (the call's own limit, ``max_images_per_call``, goes with the context's limits)."""
    limits, session = call.limits, call.session
    budgets = []
    if limits.image_budget is not None:
        budgets.append(("session", limits.image_budget, session.images_used))
    if limits.image_budget_per_round is not None:
        budgets.append(("round", limits.image_budget_per_round, session.images_this_round))
    return budgets


def serve_render(call: Any, arguments: dict[str, Any]) -> Result | None:
    """Lane RD1: ``render`` answered by the Rust session -- laid out by the document's page
    engine (:func:`docx_agent.pages.word_pages`), its pages drawn in Rust and kept there --
    for a document it owns, outside a batch; else ``None`` (counted, with why), and the
    handler goes on.  Each image comes back as a file (``page 3``, ``sheet of pages 1-12``),
    made an image of the reply here (``call.image``)."""
    from . import served as _served

    owned = _owned(call.entry)
    why = None
    layout = raster = None
    hosting = _served.hosting()
    if "render" not in hosting.served:
        why = "not served"
    elif owned is None:
        why = "not owned"
    elif call.in_batch:
        why = "in a batch"
    else:
        from ..layout import font_dirs_of
        from ..pages import word_pages

        if "render" in hosting.laid:
            layout, _ = word_pages(owned.held)     # none: the session refuses it (not supported yet)
        raster = host_raster(font_dirs_of(call.document))
    files: list[tuple[str, str, bytes]] = []
    served = serve(call, "render", arguments, lambda: None, why_not=why, layout=layout, raster=raster,
                   images=image_budgets(call) if why is None else None, files=files)
    if served is None:
        return None
    from ooxml_edit.tools.render import png_size

    for name, _format, data in files:
        width, height = png_size(data)
        call.image(data, width, height, label=name)
    return served


def last_check(call: Any) -> tuple[bool, Any, int | None]:
    """Lane P5b: what the last check of the document ``call`` reads saw, kept by the session
    where it owns the document (its own checks keep it there): ``(True, seen, number)`` --
    ``seen`` ``(version, summary)``, the session's number of the version seen (``None``: one
    a check here kept without one) and its ``LayoutSummary`` (Rust's), or ``None`` where no
    check saw one yet; ``number`` the session's number of the version ``call`` reads (``None``
    where it was let go) -- or ``(False, None, None)`` for a document the session does not
    own (the check's own record holds)."""
    mirror, version = _owned_now(call)
    if mirror is None:
        return False, None, None
    return True, mirror.session.last_check(mirror.doc), None if version is None else version.number


def checked_before(call: Any) -> Any:
    """Lane P5b: what a changing call's reflow is measured from where no layout of the
    document's current version was made here: the summary its last check saw, kept by the
    session, where that check was of this very version (a ``LayoutSummary`` held in Rust);
    else ``None``.  Lane P5r: the last check that laid it out, with ``reflow`` or not (a
    check of the layout alone, of ranges: the session keeps its pages too)."""
    mirror, version = _owned_now(call)
    if mirror is None or version is None:
        return None
    kept = mirror.session.last_laid(mirror.doc)
    if kept is None or kept[0] is None or kept[0] != version.number:
        return None
    from ..layout import LayoutSummary

    return LayoutSummary.held(kept[1])


def keep_last_check(call: Any, summary: Any) -> None:
    """Lane P5b: a check here saw the version ``call`` reads, ``summary`` its
    ``LayoutSummary`` held in Rust (``None``: one made in Python, which the session cannot
    compare -- it forgets what it kept): kept by the session for the next check, served or
    not.  Nothing for a document the session does not own."""
    mirror, version = _owned_now(call)
    if mirror is not None:
        mirror.session.set_last_check(mirror.doc, summary, at=version if summary is not None else None)


def _owned_now(call: Any) -> tuple["Mirror | None", Any]:
    """The session's mirror of the document ``call`` reads where it owns it (else ``None``),
    and the version read there (``None`` where it was let go)."""
    mirror = getattr(_owned(call.entry), "handle", None)
    if not isinstance(mirror, Mirror):
        return None, None
    with mirror.lock:
        return mirror, mirror.versions.get(call.entry.version)


def _result(envelope: dict[str, Any]) -> Result:
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"),
                    next_cursor=envelope.get("next_cursor"))
    result.total = envelope.get("total")
    result.warnings.extend(envelope.get("warnings") or [])
    return result


class Placer:
    """Where ``word_inspect``'s blocks are placed, asked of the document's layout as the
    Python path asks it (:func:`docx_agent.tools.read._placed_by_worker`, else the read
    copy's own layout), and written as :mod:`docx_agent.state` writes it.  Rust calls
    :meth:`place` with the interpreter lock taken back; an error there is kept and raised
    once the call is back (:attr:`error`)."""

    def __init__(self, call: Any) -> None:
        self.call = call
        self.error: BaseException | None = None

    def place(self, asked_json: str) -> str | None:
        try:
            from ..state import placement_keys
            from ..layout import placements_of
            from ..tools.read import _placed_by_worker

            asked = json.loads(asked_json)
            found = _placed_by_worker(self.call)(asked)
            if found is None:
                answers = placements_of(self.call.document.layout(), asked)
            else:
                answers = found[0]
            return json.dumps([placement_keys(where) for where in answers])
        except BaseException as error:  # noqa: BLE001 -- raised again once the call is back
            self.error = error
            return None
