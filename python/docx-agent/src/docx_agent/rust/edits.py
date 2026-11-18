"""docx-agent's editing tools answered by the Rust session that owns the document (phase P2,
lane P2c3; :mod:`.owner` says who owns what).

A tool with a Rust editing handler (``word_set_text``, ``replace_text``, and each one ported
after them) runs there: the call's arguments, its ``$name`` refs resolved, go to
``Session.call`` with the call's caller and the document's tracking mode; the parts the edit
changed are written into the working copy as the call's step (``Document.take_parts``), and
the reply is the session's.  The Python handler answers instead -- counted by tool and reason
(:func:`fallbacks`, ``edit_fallbacks{tool,reason}``), never silently, and committed in the
session through the bridge as every unported tool is -- where the session does not answer as
it yet: an op of a ``batch`` (one step of the working copy's, so one bridged step), a call
prepared on a draft, an address naming an id the working copy renamed or deleted, values
written by reference, a refusal (the session's carries its own way out, tool rule 2, which
today's does not), and what the session says it leaves to the host (its ``unported``:
styled text, French spacing, regex, ...).  The Rust handler changes nothing in each of those.
"""

from __future__ import annotations

import json
import threading
from collections import Counter
from typing import Any, Callable

from ooxml_edit.tools import Result
from ooxml_edit.tools.session import utc_now

from . import reads
from .owner import context_of, owned

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


class _Fallback(Exception):
    """The call is answered by the Python handler, for this reason."""


def _why_not(call: Any) -> str | None:
    entry = call.entry
    if entry is None or entry.live is not None or entry.pinned is not None or call.document is not entry.document:
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


def _run(call: Any, tool: str, arguments: dict[str, Any]) -> Result:
    document = call.document
    mirror = owned(document)
    if mirror is None:
        raise _Fallback("no session")
    with mirror.lock:
        if mirror.synced != document.history.version:
            raise _Fallback("not at the session's head")
        context = context_of(call.caller)
        _tracking(mirror, call.entry, context)
        before = mirror.session.head(mirror.doc)
        given = {"doc": mirror.doc, **{k: v for k, v in arguments.items() if v is not None}}
        reply, _images, app = mirror.session.call(tool, json.dumps(given), context=context)
        envelope = json.loads(reply)
        error = envelope.get("error")
        if error is not None:
            raise _Fallback(f"refusal: {error.get('code')}")
        unported = json.loads(app).get("unported") or []
        if unported:
            raise _Fallback(unported[0])
        head = mirror.session.head(mirror.doc)
        if head.number != before.number:
            parts = dict(head.changed_since(before))
            document.take_parts(parts, dict(envelope.get("renamed") or {}))
            # The session has these parts as written: the working copy's commit of the call
            # hands over nothing more (unless something else changed in its step).
            for part, data in parts.items():
                if data is None:
                    mirror.parts.pop(part, None)
                else:
                    mirror.parts[part] = ("bytes", len(data), reads._digest(data))
    return _result(envelope)


def serve(call: Any, tool: str, arguments: dict[str, Any], python: Callable[[], Result],
          *, addresses: tuple = ()) -> Result:
    """``tool``'s edit made by the Rust session, or ``python()``'s where the session does not
    answer as it yet (counted, with why)."""
    try:
        why = _why_not(call)
        if why is not None:
            raise _Fallback(why)
        if reads._renamed_or_deleted(call.document, *addresses):
            raise _Fallback("renamed or deleted id")
        result = _run(call, tool, arguments)
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
