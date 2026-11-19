"""docx-agent's editing tools answered by the Rust session that owns the document (phase P2;
lane P2c4: the session is the only owner, :mod:`.owner`).

A tool with a Rust editing handler (:data:`SERVED`: ``word_set_text``, ``replace_text``, and
each one ported after them) runs there, ahead of docx-agent's handler and its wrapper
(:func:`docx_agent.tools._base.wrap`): the call's arguments, its ``$name`` refs resolved, go to
``Session.call`` with the call's caller and the document's tracking mode; the session's
version is the call's step (:meth:`~ooxml_edit.tools.owned.OwnedEntry.served`), its reply the
result, its ``touched`` the result's.  The Python view of the document is not read.

The Python handler answers instead -- on the view, committed through the bridge, counted by
tool and reason (:func:`fallbacks`, ``edit_fallbacks{tool,reason}``), never silently -- where
the session does not answer as it yet: an op of a ``batch`` (one step), a call on a draft,
values written by reference, a refusal (the session's carries its own way out, tool rule 2,
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


#: The tools the session edits with, and each one's arguments as the session takes them.
SERVED: dict[str, Callable[[Any, dict[str, Any]], dict[str, Any]]] = {
    "word_set_text": _set_text,
    "replace_text": _replace_text,
}


class _Fallback(Exception):
    """The call is answered by the Python handler, for this reason."""


def owned_entry(call: Any) -> Any:
    """The call's document's entry when the session owns it and serves the call's tool."""
    entry = call.entry
    if call.tool.name not in SERVED or entry is None or getattr(entry, "owner", None) is None:
        return None
    return entry


def _why_not(call: Any, entry: Any) -> str | None:
    if entry.live is not None or entry.pinned is not None:
        return "a draft"
    if call.in_batch:
        return "batch op"
    if getattr(call, "facts", None):
        return "values by reference"
    if getattr(call.session, "clock", utc_now) is not utc_now:
        # Revisions are dated by the session's clock: the Rust session's is the system's.
        return "the session's own clock"
    return None


def _tracking(mirror: "reads.Mirror", entry: Any, context: Any) -> None:
    """The session's tracking mode of the document made the call's (``entry.tracking``)."""
    wanted = entry.tracking or None
    if mirror.tracking == wanted:
        return
    arguments: dict[str, Any] = {"doc": mirror.doc, "on": bool(wanted)}
    if wanted and wanted.get("author"):
        arguments["author"] = wanted["author"]
    reply, _images, _app = mirror.session.call("word_set_tracking", json.dumps(arguments), context=context)
    if json.loads(reply).get("error") is not None:
        raise _Fallback("tracking")
    mirror.tracking = wanted


def _layout_before(call: Any, entry: Any) -> Any:
    """What the call's reflow is measured from, as docx-agent's wrapper takes it -- only where
    the view is current (the version before the edit; ``entry.held``: read as it is, no step
    of the view's opened for it); ``None`` where there is none."""
    from ..tools._base import Before
    from ..tools.laid import laid_before, seen_before, state_bytes

    if not entry.view_current:
        return None
    document = entry.held
    before = seen_before(document, state_bytes(call)) or laid_before(document)
    call.touch(Before(before))
    return before


def _hand_engine(call: Any, entry: Any, before: Any) -> None:
    """Where the call's checks compare layouts (a layout before of at most ``REFLOW_PAGES``
    pages), the toolbox's engine for them, as docx-agent's wrapper hands it after a Python
    edit: the version the session made is laid out where the document's layouts are held.
    Put on the view as it is (``entry.held``): the checks make it current, after the step."""
    from ..tools._base import REFLOW_PAGES, engage_document, hand_to_checks

    if before is None or before.count > REFLOW_PAGES:
        return
    engage_document(call, entry.held)
    hand_to_checks(entry.held)


def _run(call: Any, entry: Any, arguments: dict[str, Any]) -> Result:
    mirror = entry.handle
    tool = call.tool.name
    context = context_of(call.caller)
    with mirror.lock:
        _tracking(mirror, entry, context)
        given = {"doc": mirror.doc, **{k: v for k, v in SERVED[tool](call, arguments).items()
                                       if v is not None}}
        reply, _images, app = mirror.session.call(tool, json.dumps(given), context=context)
        envelope = json.loads(reply)
        error = envelope.get("error")
        if error is not None:
            raise _Fallback(f"refusal: {error.get('code')}")
        application = json.loads(app)
        unported = application.get("unported") or []
        if unported:
            raise _Fallback(unported[0])
        head = mirror.session.head(mirror.doc)
    entry.served(head)
    return _result(envelope)


def serve(call: Any, arguments: dict[str, Any], python: Callable[[], Any]) -> Any:
    """The call's edit made by the Rust session, or ``python()``'s where the session does not
    answer as it yet (counted, with why).  Only for a call :func:`owned_entry` names."""
    entry = owned_entry(call)
    tool = call.tool.name
    try:
        why = _why_not(call, entry)
        if why is not None:
            raise _Fallback(why)
        before = _layout_before(call, entry)
        result = _run(call, entry, arguments)
        _hand_engine(call, entry, before)
    except _Fallback as why:
        with _COUNTS_LOCK:
            _FALLBACKS[(tool, str(why))] += 1
        return python()
    with _COUNTS_LOCK:
        _SERVED[tool] += 1
    return result


def _result(envelope: dict[str, Any]) -> Result:
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"))
    for key in ("changed", "created", "removed"):
        getattr(result, key).extend(envelope.get(key) or [])
    result.renamed.update(envelope.get("renamed") or {})
    result.warnings.extend(envelope.get("warnings") or [])
    return result
