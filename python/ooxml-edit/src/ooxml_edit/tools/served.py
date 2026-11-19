"""Changing calls a document's owner serves itself, side by side (lane P2c6).

A document an owner holds (:class:`~.owned.OwnedEntry`) whose format edits with the owner
(:attr:`~.session.DocumentFormat.serve`: the tools a library's native session edits with)
runs such a call without the document's lock: the call holds the document's
:class:`~.gate.Gate` shared, beside the others the owner serves, while the owner makes its
edit (prepared on a snapshot, committed in a short section of the owner's own) and its step
is numbered in the order the owner made the steps (:meth:`~.owned.OwnedEntry.served`, which
also counts what the dispatcher counts per change).  Its checks run after, with nothing
held, on the version the call made.

What the owner does not serve so -- a call it refuses or leaves to the format's Python
handler, a call with an idempotence key, slow reading ahead, more than one document, a draft
-- runs as any other call does, under the document's lock (which holds the gate alone).

Must not: take the document's lock (or read the view, which takes it) while the gate is held,
or count a step but through the entry.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Callable, Mapping, Sequence

from .owned import OwnedEntry
from .results import Result
from .session import DocumentEntry, Session, StepOwner

if TYPE_CHECKING:
    from .dispatch import Toolbox, _Turn
    from .registry import Tool


@dataclass
class Served:
    """What a format's ``serve`` hands back for a call the owner served: the result, and the
    owner's versions the call made -- ``first``'s number to ``head``."""

    result: Result
    first: int
    head: Any
    #: Markers for the call's checks (the format's ``checks`` get them with what it touched).
    touched: list[Any] = field(default_factory=list)


class Declined:
    """What a format's ``serve`` hands back for a call the owner tried and left to the
    format's Python handler (it changed nothing): the call runs under the document's lock,
    its handler told not to try the owner again (:attr:`~.dispatch.Call.owner_declined`)."""

    def __init__(self, reason: str) -> None:
        self.reason = reason

    def __repr__(self) -> str:
        return f"Declined({self.reason!r})"


def serve_unlocked(toolbox: "Toolbox", session: Session, tool: "Tool", arguments: dict[str, Any],
                   doc_ids: Sequence[str], roles: Sequence[str], caller: Mapping[str, Any],
                   turn: "_Turn | None") -> tuple[Result, dict[str, DocumentEntry], DocumentEntry] | Declined | None:
    """The call, served by its document's owner without the document's lock (the module's
    notes); else it runs as any other call (nothing was changed): :class:`Declined` where the
    owner was tried, ``None`` where it was not."""
    from .dispatch import Call, _context

    found = _served_entry(session, tool, arguments, doc_ids)
    if found is None:
        return None
    entry, serve = found
    entries: dict[str, DocumentEntry] = {entry.doc_id: entry}
    context = _context(roles, caller)
    read: list[str] = []
    with entry.gate.shared():
        version = entry.version
        given = toolbox._with_refs(tool, dict(arguments), entries, entry, context, read)
        call = Call(toolbox, session, tool, given, entries, entry, context)
        given = toolbox._with_facts(tool, given, call)
        with entry.serving():
            served = serve(call, given)
            if not isinstance(served, Served):
                return served
            if served.first is None:            # (it changed nothing: no step)
                number, before = entry.version, None
            else:
                number, before = entry.served(served.first, served.head,
                                              StepOwner(dict(caller), tool.name), counted=True)
    result = served.result
    result.warnings[:0] = tool.ignored_for(entry.kind, given) + read
    result.version = number
    if before is not None:
        made = entry.owner.touched(entry.handle, entry.history.head(before), served.head)
        result.touched = [{"doc": entry.doc_id, **item} for item in made]
    context.touched.setdefault(entry.doc_id, []).extend(served.touched)
    result.checks = {**toolbox._checks(session, entry, context), **result.checks}
    if turn is not None:
        turn.made(entries, {entry.doc_id: version})
    return result, entries, entry


def _served_entry(session: Session, tool: "Tool", arguments: Mapping[str, Any],
                  doc_ids: Sequence[str]) -> tuple[OwnedEntry, Callable[..., Any]] | None:
    """The call's document when its owner may serve the call without the lock, and its
    format's ``serve``."""
    if tool.composite or len(doc_ids) != 1 or not tool.changes(arguments):
        return None
    entry = session.documents.get(doc_ids[0])
    if not isinstance(entry, OwnedEntry) or entry.owner is None:
        return None
    fmt = session.formats.get(entry.kind)
    serve = fmt.serve if fmt is not None else None
    if serve is None or entry.live is not None or entry.pinned is not None:
        return None
    if tool.ahead_for(entry.kind) is not None or (tool.retry_key and isinstance(arguments.get("key"), str)):
        return None
    if tool.facts and "{{" in json.dumps(arguments, default=str):
        return None     # (values written by reference read other documents: under the lock)
    return entry, serve


__all__ = ["Declined", "Served", "serve_unlocked"]
