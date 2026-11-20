"""docx-agent's reading tools answered by the Rust session (phase P1, lane P1c): ``describe``,
``word_read``, ``word_inspect`` (``layout`` too), ``find_text`` and, through them, the ops of
``read_batch``, each with the interpreter lock let go while Rust reads; and (lane P5a) the
checks of a document the session owns that read the document alone (:func:`serve_check`).

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
version's read copy (a batch's reads of its own changes); limits other than the defaults the
Rust paging uses; an address naming an id this session renamed or deleted (the ledger and
the aliases are Python's until P2); a regular expression Rust does not match as Python does;
and what Rust says it does not give as Python does yet (its ``unported``: Accept All's cell
ends).  A refusal goes to the Python
path too: Rust's carries its own way out (``valid_options``, ``field``, ``try_call``, tool
rule 2), which today's does not, so the reply stays today's.
"""

from __future__ import annotations

import hashlib
import json
import logging
import threading
import weakref
from collections import Counter, OrderedDict
from typing import Any, Callable

from ooxml_edit.tools import Result

from .. import finalizers

log = logging.getLogger("docx_agent.rust")

#: The versions of a document kept for the calls reading them.
KEPT = 4

#: The defaults the Rust paging matches (``Limits().max_result_chars``,
#: ``Limits().max_list_items``): other limits are answered by the Python path.
_RESULT_CHARS = 24_000
_LIST_ITEMS = 50

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

    def keep(self, number: int, version: Any) -> None:
        """Keep ``version`` (an ``ooxml_core.Version``) as the document's version ``number``."""
        self.versions[number] = version
        self.numbers[version.number] = number
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


def _earlier_ids(document: Any) -> "tuple[list[int], list[tuple[str, int]], int] | None":
    """The ids ``document`` had before it was saved, where its id state was opened with it
    (round 38: its ledger's pool, restored): every value, each seed's cursor and the highest
    drawing number -- for the session never to give a retired id again.  ``None`` where its
    pool is not read yet (the parts' ids are all it has)."""
    ledger = getattr(document, "_ledger", None)
    pool = getattr(ledger, "pool", None)
    if pool is None:
        return None
    return list(pool), list(getattr(pool, "cursors", {}).items()), ledger.highest_drawing


# -- serving a call ---------------------------------------------------------------------------


class _Fallback(Exception):
    """The call is answered by the Python path, for this reason."""


def _named(call: Any) -> Any:
    """The document whose ids an address of the call names: the call's; for one the session
    owns (or a read of a version of it), its view where that is current at the call's
    version, else none -- the session follows its renames and refuses a deleted id itself,
    and a refusal is answered by the Python path."""
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


def _owned_version(call: Any) -> tuple["Mirror | None", Any]:
    """Lane P2c4: the session's version a call on a document the session owns reads (the
    entry's version now), found without the Python view; ``(None, None)`` otherwise."""
    entry = call.entry
    mirror = getattr(_owned(entry), "handle", None)
    if not isinstance(mirror, Mirror):
        return None, None
    with mirror.lock:
        found = mirror.versions.get(entry.version)
    if found is None:
        raise _Fallback("version let go")
    return mirror, found


def _call(call: Any, tool: str, arguments: dict[str, Any], *, placer: "Placer | None" = None) -> dict[str, Any]:
    """The Rust envelope of ``tool`` with ``arguments`` on the version ``call`` reads."""
    if _session() is None:
        raise _Fallback("no session")
    mirror, version = _owned_version(call)
    if mirror is None:
        copy = call.document
        mirror = _mirror(copy)
        if mirror is None:
            raise _Fallback("not a committed version")
        version = mirror.version(copy)
    arguments = {"doc": mirror.doc, **{k: v for k, v in arguments.items() if v is not None}}
    reply, _images, app = mirror.session.call(tool, json.dumps(arguments), at=[version],
                                              placer=placer.place if placer is not None else None)
    if placer is not None and placer.error is not None:
        raise placer.error
    envelope = json.loads(reply)
    error = envelope.get("error")
    if error is not None:
        # A refusal Rust words as Python does but with its own way out (valid_options,
        # field, try_call: tool rule 2), which today's answer does not carry: answered by
        # the Python path, so the reply is today's.
        raise _Fallback("regex" if error.get("field") == "regex" else f"refusal: {error.get('code')}")
    unported = json.loads(app).get("unported") or []
    if unported:
        raise _Fallback(unported[0])
    return envelope


def serve(call: Any, tool: str, arguments: dict[str, Any],
          python: Callable[[], Result], *, addresses: tuple = (),
          answer: Callable[[dict[str, Any]], Result] | None = None,
          placer: "Placer | None" = None, why_not: str | None = None) -> Result:
    """``tool``'s answer from the Rust session (``answer`` makes the :class:`Result` of its
    envelope), or ``python()``'s where Rust's is not the same yet (counted, with why)."""
    try:
        if why_not is not None:
            raise _Fallback(why_not)
        if call.limits.max_result_chars != _RESULT_CHARS or call.limits.max_list_items != _LIST_ITEMS:
            raise _Fallback("limits")
        if _renamed_or_deleted(_named(call), *addresses):
            raise _Fallback("renamed or deleted id")
        envelope = _call(call, tool, arguments, placer=placer)
        result = (answer or _result)(envelope)
    except _Fallback as why:
        _fell_back(tool, str(why))
        return python()
    _served(tool)
    return result


def serve_check(call: Any, arguments: dict[str, Any]) -> Result | None:
    """Lane P5a: ``check`` answered by the Rust session where it answers it whole -- the
    facts read from the document alone (``validate``, ``fields``) of a document it owns,
    outside a batch -- else ``None`` (counted, with why), and the handler goes on.  What
    needs the layout, copies, a scale and the application's ``min_text_size`` (which asks
    for layout facts) are the handler's still (the session says which: ``unported``)."""
    owned = _owned(call.entry)
    why = None
    if owned is None:
        why = "not owned"
    elif call.in_batch:
        why = "in a batch"
    elif getattr(owned.held, "min_text_size", None) is not None:
        why = "min_text_size"
    return serve(call, "check", arguments, lambda: None, why_not=why)


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
