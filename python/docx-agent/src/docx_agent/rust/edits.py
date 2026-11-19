"""docx-agent's editing tools answered by the Rust session that owns the document (phase P2;
lane P2c4: the session is the only owner, :mod:`.owner`; lane P2c6: side by side).

A tool with a Rust editing handler (:data:`SERVED`: ``word_set_text``, ``replace_text``,
``word_insert_text``, and each one ported after them) runs there, ahead of docx-agent's handler and its wrapper
(:func:`docx_agent.tools._base.wrap`): the call's arguments, its ``$name`` refs resolved, go to
``Session.edit`` with the call's caller and the document's tracking mode; the versions the
call's edits made are the call's step (:meth:`~ooxml_edit.tools.owned.OwnedEntry.served`,
numbered in the order the session made them), its reply the result.  The Python view of the
document is not read.

Such a call runs without the document's lock (:func:`serve_owned`, the format's ``serve``:
:mod:`ooxml_edit.tools.served`), beside the others the session serves -- the session prepares
each edit on a snapshot and commits it in a short section of its own.  Where its checks
compare layouts (Python lays them out) it runs under the lock (:func:`serve`).

The Python handler answers instead -- on the view, committed through the bridge, counted by
tool and reason (:func:`fallbacks`, ``edit_fallbacks{tool,reason}``), never silently -- where
the session does not answer as it yet: an op of a ``batch`` (one step), a call on a draft,
values written by reference, a ref name that would be folded, a refusal (the session's carries its own way out, tool rule 2,
which today's does not), a session given its own clock (the Rust session dates revisions by
the system's), and what the session says it leaves to the host (its ``unported``: styled text,
French spacing, regex, ...).  The Rust handler changes nothing in each of those.
"""

from __future__ import annotations

import json
import threading
from collections import Counter
from typing import Any, Callable

from ooxml_edit.tools import Result
from ooxml_edit.tools.session import REF
from ooxml_edit.tools.session import StepOwner
from ooxml_edit.tools.session import utc_now

from . import reads
from .owner import context_of

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


def _set_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {"target": reads._refs(call, arguments.get("target")), "text": arguments.get("text"),
            "paragraph_style": arguments.get("paragraph_style"),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


def _replace_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "replace": arguments.get("replace"),
            "expect": arguments.get("expect"), "regex": arguments.get("regex") or None,
            "slides": arguments.get("slides"), "range": reads._refs(call, arguments.get("range")),
            "stories": arguments.get("stories", "body")}


def _insert_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {**{k: arguments.get(k) for k in ("find", "where", "at", "text", "ref")},
            "target": reads._refs(call, arguments.get("target")),
            "within": reads._refs(call, arguments.get("within")),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


#: The tools the session edits with, and each one's arguments as the session takes them.
SERVED: dict[str, Callable[[Any, dict[str, Any]], dict[str, Any]]] = {
    "word_set_text": _set_text,
    "replace_text": _replace_text,
    "word_insert_text": _insert_text,
}


def _ref_names(arguments: dict[str, Any]) -> list[str]:
    """The ref names a call gives (``ref``, ``items[].ref``)."""
    items = arguments.get("items") if isinstance(arguments.get("items"), list) else []
    names = [arguments.get("ref"), *(item.get("ref") for item in items if isinstance(item, dict))]
    return [name for name in names if isinstance(name, str) and name]


class _Fallback(Exception):
    """The call is answered by the Python handler, for this reason."""


def owned_entry(call: Any) -> Any:
    """The call's document's entry when the session owns it and serves the call's tool."""
    entry = call.entry
    if call.tool.name not in SERVED or entry is None or getattr(entry, "owner", None) is None:
        return None
    return entry


def _why_not(call: Any, entry: Any, arguments: dict[str, Any]) -> str | None:
    if entry.live is not None or entry.pinned is not None:
        return "a draft"
    if any(not REF.match(name) for name in _ref_names(arguments)):
        # folded with a warning, or refused, by the tool layer as the Python handler names it
        return "a ref name to fold"
    if call.in_batch:
        return "batch op"
    if getattr(call, "facts", None):
        return "values by reference"
    if getattr(call.session, "clock", utc_now) is not utc_now:
        # Revisions are dated by the session's clock: the Rust session's is the system's.
        return "the session's own clock"
    return None


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
        reply, _app, made = mirror.session.edit("word_set_tracking", json.dumps(arguments), context=context)
        if made is None and json.loads(reply).get("error") is not None:
            raise _Fallback("tracking")
        mirror.tracking = wanted
    if made is not None:
        first, head, _touched, _validation = made
        entry.served(first, head, StepOwner(dict(call.caller), "word_set_tracking"), counted=counted)


def _layout_before(call: Any, entry: Any) -> Any:
    """What the call's reflow is measured from, as docx-agent's wrapper takes it -- only where
    the view is current (the version before the edit; ``entry.held``: read as it is, no step
    of the view's opened for it); ``None`` where there is none."""
    from ..tools.laid import laid_before, seen_before, state_bytes

    if not entry.view_current:
        return None
    document = entry.held
    return seen_before(document, state_bytes(call)) or laid_before(document)


def _hand_engine(call: Any, entry: Any, before: Any) -> None:
    """Where the call's checks compare layouts (a layout before of at most ``REFLOW_PAGES``
    pages), the toolbox's engine for them, as docx-agent's wrapper hands it after a Python
    edit: the version the session made is laid out where the document's layouts are held.
    Put on the view as it is (``entry.held``): the checks make it current, after the step."""
    from ..tools._base import engage_document, hand_to_checks

    if not _compares(before):
        return
    engage_document(call, entry.held)
    hand_to_checks(entry.held)


def _compares(before: Any) -> bool:
    """Whether a call's checks compare layouts, from ``before`` (Python lays them out)."""
    from ..tools._base import REFLOW_PAGES

    return before is not None and before.count <= REFLOW_PAGES


def _edit(call: Any, entry: Any, arguments: dict[str, Any], counted: bool) -> tuple[Result, Any]:
    """The call's edit made by the session: its result, and what it made (``Session.edit``'s
    ``(first, head, touched, validation)``, ``None`` where it changed nothing).  ``counted``: as for
    :meth:`~ooxml_edit.tools.owned.OwnedEntry.served`."""
    mirror = entry.handle
    tool = call.tool.name
    context = context_of(call.caller)
    _tracking(call, entry, context, counted)
    given = {"doc": mirror.doc, **{k: v for k, v in SERVED[tool](call, arguments).items()
                                   if v is not None}}
    reply, app, made = mirror.session.edit(tool, json.dumps(given), context=context)
    envelope = json.loads(reply)
    error = envelope.get("error")
    unported = json.loads(app).get("unported") or []
    if made is None and error is not None:
        raise _Fallback(f"refusal: {error.get('code')}")
    if made is None and unported:
        raise _Fallback(unported[0])
    result = _result(envelope)
    for name, address in (envelope.get("refs") or {}).items():
        call.define_ref(name, address)        # the call's refs, as docx-agent's remember
        result.refs[name] = address
    return result, made


def _count(tool: str, why: str | None) -> None:
    with _COUNTS_LOCK:
        if why is None:
            _SERVED[tool] += 1
        else:
            _FALLBACKS[(tool, why)] += 1


class Made:
    """A check's marker (:attr:`~ooxml_edit.tools.served.Served.touched`): the version a
    call the session served made, and its validation (``(new, fixed, baseline)``, as
    ``Session.validation`` gives it), which its checks report."""

    def __init__(self, head: Any, validation: tuple[list[str], list[str], int]) -> None:
        self.head = head
        new, fixed, baseline = validation
        self.validation = {"new": list(new), "fixed": list(fixed), "baseline": baseline}


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
            result, made = _edit(call, entry, arguments, counted=True)
        except _Fallback as fallback:
            why = str(fallback)
    if why is not None:
        _count(tool, why)
        return Declined(why)
    _count(tool, None)
    touched: list[Any] = [_before(before)] if current else []
    if made is None:
        return Served(result, None, None, touched)
    first, head, step, validation = made
    return Served(result, first, head, [*touched, Made(head, validation)],
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
            raise _Fallback(why)
        before = _layout_before(call, entry)
        if entry.view_current:
            call.touch(_before(before))
        with entry.serving():
            result, made = _edit(call, entry, arguments, counted=False)
            if made is not None:
                entry.served(made[0], made[1])
        _hand_engine(call, entry, before)
    except _Fallback as fallback:
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
