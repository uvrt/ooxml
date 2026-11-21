"""docx-agent's editing tools answered by the Rust session that owns the document (phase P2;
lane P2c4: the session is the only owner, :mod:`.owner`; lane P2c6: side by side).

A tool the Rust session serves (declared by the Rust tool and exported with the registry:
:func:`docx_agent.rust.served.hosting`; its arguments as the session takes them:
:func:`docx_agent.rust.served.arguments_for`) runs there, ahead of docx-agent's handler and its wrapper
(:func:`docx_agent.tools._base.wrap`): the call's arguments, its ``$name`` refs resolved, go to
``Session.edit`` with the call's caller and the document's tracking mode; the versions the
call's edits made are the call's step (:meth:`~ooxml_edit.tools.owned.OwnedEntry.served`,
numbered in the order the session made them), its reply the result.  The Python view of the
document is not read.

An op of a ``batch`` is served the same way, under the batch's locks, into the batch's step
group of the session's (lane H1: ooxml-edit's ``OwnedEntry.op_step``): the batch is one step,
the ops' versions numbered with it.

Such a call runs without the document's lock (:func:`serve_owned`, the format's ``serve``:
:mod:`ooxml_edit.tools.served`), beside the others the session serves -- the session prepares
each edit on a snapshot and commits it in a short section of its own.  Where its checks
compare layouts (Python lays them out) it runs under the lock (:func:`serve`).

The Python handler answers instead -- on the view, committed through the bridge, counted by
tool and reason (:func:`fallbacks`, ``edit_fallbacks{tool,reason}``), never silently -- where
the session does not answer as it yet: values written by reference in a tool that does not
keep them yet (``Hosting.keeps_written``), a refusal (the session's carries its own way out, tool rule 2,
which today's does not), a session given its own clock (the Rust session dates revisions by
the system's), and what the session says it leaves to the host (its ``unported``: a find
within a block, a picture, regex, ...).  The Rust handler changes nothing in each of those.
"""

from __future__ import annotations

import json
import threading
import time
from collections import Counter
from typing import Any, Callable

from ooxml_edit.tools import Result, ToolError
from ooxml_edit.tools.session import StepOwner
from ooxml_edit.tools.session import folded_warning
from ooxml_edit.tools.session import utc_now

from . import served as _served
from .owner import context_of, documents, options
from .served import Adapted, Fallback, written_json

_COUNTS_LOCK = threading.Lock()
_FALLBACKS: Counter = Counter()
_SERVED: Counter = Counter()


def fallbacks() -> dict[str, dict[str, int]]:
    """``edit_fallbacks{tool,reason}``: the editing calls of a ported tool answered by the
    Python handler, by tool and reason, since the process started."""
    with _COUNTS_LOCK:
        out: dict[str, dict[str, int]] = {}
        for (tool, reason), n in sorted(_FALLBACKS.items()):
            out.setdefault(tool, {})[reason] = n
        return out


def served() -> dict[str, int]:
    """The editing calls answered by the Rust session, by tool, since the process started."""
    with _COUNTS_LOCK:
        return dict(_SERVED)


class _Blobs:
    """The session's blobs as ``CallOptions(blobs=...)`` asks them (lane P2c14): Rust calls
    back with the interpreter lock taken back, for the blob a call names."""

    def __init__(self, session: Any) -> None:
        self.session = session

    def blob(self, handle: str) -> tuple[str, str, bytes] | None:
        try:
            found = self.session.blob(handle)
        except Exception:  # noqa: BLE001 -- no such blob: the tool refuses, listing them
            return None
        return found.name, found.mime, bytes(found.data)

    def listed(self) -> list[tuple[str, str]]:
        return [(b["handle"], b["name"]) for b in self.session.describe()["blobs"]]


def _written(call: Any) -> str | None:
    """The values the dispatcher wrote into the call's arguments for references
    (``call.facts``), as ``Session.edit`` takes them; ``None`` without any."""
    return written_json(getattr(call, "facts", None) or (), {})


def _kept_refs(call: Any, arguments: dict[str, Any]) -> tuple[dict[str, Any], dict[str, str]]:
    """``arguments`` with each ref name they give (``ref``, ``items[].ref``) as the call keeps it
    (ooxml-edit's ``Call.ref_name``: a camelCase or hyphenated name folded; one that folds to
    none, or to another address's ref, refused before the edit, so the refusal changes
    nothing -- lane H2), and the names folded, ``{kept: given}``."""
    folded: dict[str, str] = {}

    def kept(holder: dict[str, Any]) -> dict[str, Any]:
        name = holder.get("ref")
        if not isinstance(name, str) or not name:
            return holder
        ref = call.ref_name(name)
        if ref == name:
            return holder
        folded[ref] = name
        return {**holder, "ref": ref}

    out = kept(arguments)
    items = out.get("items")
    if isinstance(items, list):
        out = {**out, "items": [kept(item) if isinstance(item, dict) else item for item in items]}
    return out, folded


def owned_entry(call: Any) -> Any:
    """The call's document's entry when the session owns it and serves the call's tool."""
    entry = call.entry
    if call.tool.name not in _served.hosting().served or entry is None or getattr(entry, "owner", None) is None:
        return None
    return entry


def _why_not(call: Any, entry: Any, arguments: dict[str, Any]) -> str | None:
    # (No draft: ooxml-edit prepares no call of a document the session owns on a draft, and
    # ``entry`` -- an owner's -- is never a reader's copy; lane H2.)
    if getattr(call, "facts", None) and call.tool.name not in _served.hosting().keeps_written:
        return "values by reference"
    if getattr(call.session, "clock", utc_now) is not utc_now:
        # Revisions are dated by the session's clock: the Rust session's is the system's.
        return "the session's own clock"
    return None


def will_serve(call: Any, arguments: dict[str, Any]) -> bool:
    """Whether the session is asked first for the changing call (lane P2c13): then its work
    ahead (a layout in Python, the lock let go) is not done -- the session lays out in Rust;
    where it then leaves the call to the Python handler, that lays out under the lock."""
    entry = owned_entry(call)
    if entry is None or not call.changing or call.owner_declined:
        return False
    if _why_not(call, entry, arguments) is not None:
        return False
    return call.tool.name not in _served.hosting().laid or _pages(entry) is not None


def _pages(entry: Any) -> Any:
    """The document's page engine (:func:`docx_agent.pages.word_pages`), or ``None``."""
    from ..pages import word_pages

    return word_pages(entry.held)[0]


def _layout(call: Any, entry: Any, measured: bool) -> Any:
    """The page engine a call of a tool whose edits read the pages
    (:attr:`~docx_agent.rust.served.Hosting.laid`) edits with; for the others, where the
    call's checks take the reflow the session measures (``measured``: served without the
    lock, lane P5m), the document's engine where this process has one already; else
    ``None``."""
    from ..pages import word_pages

    if call.tool.name not in _served.hosting().laid:
        return word_pages(entry.held, make=False)[0] if measured else None
    found, why = word_pages(entry.held)
    if found is None:
        raise Fallback(f"the layout: {why}")
    return found


def _tracking(call: Any, entry: Any, context: Any, counted: bool) -> None:
    """The session's tracking mode of the document made the call's (``entry.tracking``); a
    version that made (the settings written) is a step of its own, ``word_set_tracking``'s."""
    mirror = entry.handle
    with mirror.lock:
        wanted = entry.tracking or None
        if mirror.tracking == wanted:
            return
        arguments: dict[str, Any] = {"doc": mirror.doc, "on": bool(wanted)}
        if wanted and wanted.get("author"):
            arguments["author"] = wanted["author"]
        reply, _app, made = mirror.session.edit("word_set_tracking", json.dumps(arguments), options(context=context))
        error = json.loads(reply).get("error")
        if made is None and error is not None:
            # (Never so: the host keeps only a mode word_set_tracking took, lane H2.)  The
            # session's refusal is the call's: nothing is changed.
            raise ToolError(error["code"], error["message"], field=error.get("field"),
                            valid_options=error.get("valid_options"), details=error.get("details"))
        mirror.tracking = wanted
    if made is not None:
        first, head, _touched, _validation, _reflow = made
        entry.served(first, head, StepOwner(dict(call.caller), "word_set_tracking"), counted=counted)


def _set_mode(entry: Any, given: dict[str, Any], context: Any, names: dict[str, str]) -> tuple[str, str, Any]:
    """``word_set_tracking`` served: the session's tracking mode of the document and the
    host's (``entry.tracking``, which later calls are made under and :func:`_tracking` keeps
    the session to) set together, under the mirror's lock, where the session sets it."""
    mirror = entry.handle
    with mirror.lock:
        reply, app, made = mirror.session.edit("word_set_tracking", json.dumps(given), options(
            context=context, documents=documents(names=names)))
        if json.loads(reply).get("error") is None and not json.loads(app).get("unported"):
            wanted = {"on": True, "author": given.get("author")} if given.get("on") else None
            entry.tracking = wanted
            mirror.tracking = wanted
    return reply, app, made


def _names(call: Any, entry: Any, arguments: dict[str, Any]) -> dict[str, str]:
    """The host's names for the documents a call names, each the session's id of it
    (``Session.edit``'s ``names``; lane P2c17): the arguments name documents as the host
    does, and so does the reply (an undo or way-out call, a summary).  The Python handler
    answers where another document named is not the session's as it is.  Another document
    held as a reader of a committed version of one the session owns (``live``: the owner's
    entry) is that document: the session reads its own committed head of it (lane H2)."""
    mirror = entry.handle
    names = {entry.doc_id: mirror.doc}
    for doc in call.tool.doc_ids(arguments):
        if doc in names:
            continue
        other = call.entries.get(doc)
        if other is not None and getattr(other, "owner", None) is None:
            other = getattr(other, "live", None)
        if other is None or getattr(other, "owner", None) is None:
            raise Fallback("a document the session does not own")
        if other.handle.session is not mirror.session:
            raise Fallback("a document in another session")
        names[doc] = other.handle.doc
    return names


def _layout_before(call: Any, entry: Any) -> Any:
    """What the call's reflow is measured from, as docx-agent's wrapper takes it -- only where
    the view is current (the version before the edit; ``entry.held``: read as it is, no step
    of the view's opened for it); ``None`` where there is none."""
    from ..tools.laid import laid_before, seen_before, state_bytes
    from .reads import checked_before

    if not entry.view_current:
        return None
    document = entry.held
    return seen_before(document, state_bytes(call)) or checked_before(call) or laid_before(document)


def _hand_engine(call: Any, entry: Any, before: Any, changed: bool) -> None:
    """The toolbox's engine on the view as it is (``entry.held``), as docx-agent's wrapper
    engages it for a Python edit: where the call's checks compare layouts (a layout before of
    at most ``REFLOW_PAGES`` pages), handed to them -- they make the view current, after the
    step; elsewhere, where the call ``changed`` the document, for the relayout on commit its
    checks start (lane P2c13), which lets it go.  Under the document's lock."""
    from ..tools._base import engage_document, hand_to_checks

    if _compares(before):
        engage_document(call, entry.held)
        hand_to_checks(entry.held)
    elif changed:
        engage_document(call, entry.held)


def _engine(call: Any, entry: Any) -> Any:
    """The engine a call served without the lock lays its version out with on commit (lane
    P2c13): the toolbox's, as :func:`_hand_engine` engages it, not put on the view (the
    document's lock is not held)."""
    from ..tools._base import Engine

    return Engine.of(call.toolbox.pool, call.limits.layout_timeout, entry.held)


def _compares(before: Any) -> bool:
    """Whether a call's checks compare layouts, from ``before`` (Python lays them out)."""
    from ..tools._base import REFLOW_PAGES

    return before is not None and before.count <= REFLOW_PAGES


def _timing(call: Any, arguments: dict[str, Any]) -> Any:
    """The ``Timing`` of a call whose result says where its time went (round 45, R67:
    ``word_fields`` update), or ``None``."""
    from ..tools._base import Timing

    if call.tool.name == "word_fields" and arguments.get("action") == "update":
        return Timing()
    return None


def _edit(call: Any, entry: Any, arguments: dict[str, Any], counted: bool) -> tuple[Result, Any, list[Any]]:
    """The call's edit made by the session: its result, what it made (``Session.edit``'s
    ``(first, head, touched, validation)``, ``None`` where it changed nothing), and the
    markers its checks get (its ``Timing``, :func:`_timing`).  ``counted``: as for
    :meth:`~ooxml_edit.tools.owned.OwnedEntry.served`."""
    mirror = entry.handle
    tool = call.tool.name
    context = context_of(call.caller)
    arguments, folded = _kept_refs(call, arguments)
    entry.flush_view()        # (in a batch: the Python ops' changes before it, the session's)
    _tracking(call, entry, context, counted)
    adapted = _served.arguments_for(call, tool, arguments)
    written = adapted.written if isinstance(adapted, Adapted) else _written(call)
    given = {**{k: v for k, v in adapted.items() if v is not None}, "doc": entry.doc_id}
    names = _names(call, entry, arguments)
    layout = _layout(call, entry, measured=counted)
    timing = _timing(call, arguments)
    started = time.perf_counter()
    blobs = _Blobs(call.session) if tool in _served.hosting().blobs else None
    if tool == "word_set_tracking":
        reply, app, made = _set_mode(entry, given, context, names)
    else:
        reply, app, made = mirror.session.edit(tool, json.dumps(given), options(
            context=context, layout=layout, blobs=blobs, written=written, documents=documents(names=names)))
    if timing is not None:
        timing.add("update", time.perf_counter() - started)
    envelope = json.loads(reply)
    error = envelope.get("error")
    unported = json.loads(app).get("unported") or []
    if made is None and error is not None:
        raise Fallback(f"refusal: {error.get('code')}")
    if made is None and unported:
        raise Fallback(unported[0])
    result = _result(envelope)
    refs = envelope.get("refs") or {}
    for name, address in refs.items():
        result.refs[call.define_ref(name, address)] = address   # the call's refs, as docx-agent's remember
    # (first, as the call's own warnings come before its handler's)
    result.warnings[:0] = [folded_warning(given, ref) for ref, given in folded.items() if ref in refs]
    if timing is None:
        return result, made, []
    if isinstance(result.data, dict):
        result.data["timing"] = timing.parts      # (the checks add theirs)
    return result, made, [timing]


def _count(tool: str, why: str | None) -> None:
    with _COUNTS_LOCK:
        if why is None:
            _SERVED[tool] += 1
        else:
            _FALLBACKS[(tool, why)] += 1


class Made:
    """A check's marker (:attr:`~ooxml_edit.tools.served.Served.touched`): the version a
    call the session served made, and its validation (``(new, fixed, baseline)``, as
    ``Session.validation`` gives it), which its checks report; the ``engine`` its relayout on
    commit lays the version out with (lane P2c13).  Lane P5m: the ``reflow`` its checks
    report, as the session measured it after the call's edits (JSON text), where it did."""

    def __init__(self, head: Any, validation: tuple[list[str], list[str], int], engine: Any = None,
                 reflow: str | None = None) -> None:
        self.head = head
        self.engine = engine
        new, fixed, baseline = validation
        self.validation = {"new": list(new), "fixed": list(fixed), "baseline": baseline}
        self.reflow = json.loads(reflow) if reflow is not None else None


def serve_owned(call: Any, arguments: dict[str, Any]) -> Any:
    """The format's ``serve`` (lane P2c6): the call's edit made by the session beside the
    others it serves, without the document's lock (:mod:`ooxml_edit.tools.served`).  ``None``
    where the call's checks compare layouts (Python lays them out, under the lock: the
    session edits there); a ``Declined`` where the Python handler answers (counted)."""
    from ooxml_edit.tools.served import Declined, Served

    entry = owned_entry(call)
    if entry is None:
        return None
    tool = call.tool.name
    why = _why_not(call, entry, arguments)
    current = entry.view_current
    if why is None:
        before = _layout_before(call, entry)
        if _compares(before):
            return None
        try:
            result, made, markers = _edit(call, entry, arguments, counted=True)
        except Fallback as fallback:
            why = str(fallback)
    if why is not None:
        _count(tool, why)
        return Declined(why)
    _count(tool, None)
    touched: list[Any] = [*([_before(before)] if current else []), *markers]
    if made is None:
        return Served(result, None, None, touched)
    first, head, step, validation, reflow = made
    return Served(result, first, head, [*touched, Made(head, validation, _engine(call, entry), reflow)],
                  None if step is None else json.loads(step))


def _before(layout: Any) -> Any:
    from ..tools._base import Before

    return Before(layout)


def serve(call: Any, arguments: dict[str, Any], python: Callable[[], Any]) -> Any:
    """Under the document's lock: the call's edit made by the session, or ``python()``'s where
    the session does not answer as it yet (counted, with why).  Only for a call
    :func:`owned_entry` names.  A call the session was tried for without the lock
    (:attr:`~ooxml_edit.tools.dispatch.Call.owner_declined`) goes to ``python()`` at once."""
    entry = owned_entry(call)
    tool = call.tool.name
    if call.owner_declined:
        return python()
    try:
        why = _why_not(call, entry, arguments)
        if why is not None:
            raise Fallback(why)
        before = _layout_before(call, entry)
        if entry.view_current:
            call.touch(_before(before))
        with entry.serving():
            result, made, markers = _edit(call, entry, arguments, counted=False)
            for marker in markers:
                call.touch(marker)
            if made is not None:
                entry.served(made[0], made[1])
        _hand_engine(call, entry, before, made is not None)
    except Fallback as fallback:
        _count(tool, str(fallback))
        return python()
    _count(tool, None)
    return result


def _result(envelope: dict[str, Any]) -> Result:
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"))
    for key in ("changed", "created", "removed"):
        getattr(result, key).extend(envelope.get(key) or [])
    result.renamed.update(envelope.get("renamed") or {})
    result.warnings.extend(envelope.get("warnings") or [])
    return result
