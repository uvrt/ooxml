"""A refusal of the Rust session, answered as the call's reply (lane P2e1): a refusal is
not a fallback.

The Rust tools word their refusals as docx-agent's handlers do (code, message and field; the
differentials hold them to it) and carry their way out besides (``valid_options``,
``try_call``: tool rule 2).  A served call the session refuses is answered with that
refusal (:func:`refusal`), never handed to the Python handler for its wording.

One refusal the session leaves to the host to word: an id a change of this session (or one
made before the document was saved and opened again) removed.  The session keeps the record
-- each retired id with the version that removed it, its caller and tool
(``Session.retired``) -- and refuses the id as any other it lacks; the host, which numbers the
versions and says who made them, words it as docx-agent's view does (round 37: which change
removed it, and the way on), from that record alone.

Must not: read the Python view of the document, or word any other refusal.
"""

from __future__ import annotations

import re
from collections.abc import Callable, Iterator
from typing import Any

from ooxml_edit.tools import ToolError

#: A block id inside any address (``p:X``, ``header1/p:X``, ``t:X/c1,2``, ``p:X@3:9``,
#: ``h:p:X``, ``s:X``, ``tr:X``, a span's ends): the eight hex digits of its paraId (as
#: docx-agent's view reads an address it lacks).
_BLOCK = re.compile(r"(?:^|(?<=[/:.]))(?:[A-Za-z0-9_]+/)?(?:p|t|tr|s):(?P<hex>[0-9A-Fa-f]{8})(?![0-9A-Fa-f])")
#: A drawing's id, ``d:<number>`` (a group's member: its group).
_DRAWING = re.compile(r"d:(\d+)(?![\d#])")

#: The read a refusal of a removed id points to, as the tool layer names it.
READ_HINT = "word_read"


def refusal(error: dict[str, Any], retired: Callable[[], Any] | None = None,
            arguments: Any = None, describe: Callable[[int], str | None] | None = None) -> ToolError:
    """The session's refusal ``error`` (its envelope's ``error``) as the call's
    :class:`ToolError`.  A ``not_found`` where an address of ``arguments`` names an id the
    document retired (``retired()``: :func:`~.owner.retired_ids`) says which change removed
    it (:func:`deleted`); ``describe`` says who made a version, as the tool layer records it."""
    if error.get("code") == "not_found" and retired is not None:
        found = deleted(retired(), _strings(arguments), describe)
        if found is not None:
            message, options = found
            return ToolError("not_found", message, valid_options=options, field=error.get("field"))
    return ToolError(error["code"], error["message"], field=error.get("field"),
                     valid_options=error.get("valid_options"), details=error.get("details"),
                     try_call=error.get("try_call"))


def deleted(retired: Any, identifiers: Iterator[str],
            describe: Callable[[int], str | None] | None = None) -> tuple[str, list[str]] | None:
    """What naming a removed id says, and its options, for the first of ``identifiers`` that
    names one ``retired`` holds (``RetiredIds``); ``None`` where none does."""
    for identifier in identifiers:
        found = _DRAWING.match(identifier)
        if found is not None and int(found.group(1)) in retired.drawings_gone:
            version = retired.drawings_gone[int(found.group(1))]
            return _message(retired, found.group(0), version, identifier=None, describe=describe), []
        for match in _BLOCK.finditer(identifier):
            value = int(match.group("hex"), 16)
            if value in retired.gone:
                named = match.group(0).lstrip("/")
                return _message(retired, named, retired.gone[value], identifier, describe), []
    return None


def _message(retired: Any, named: str, version: int, identifier: str | None,
             describe: Callable[[int], str | None] | None) -> str:
    """docx-agent's words for a removed id (``edit.document._deleted_message``, round 37):
    which change removed it, and how to read again; a change made before the document was
    saved and opened again says so (round 38)."""
    if version in retired.earlier:
        who = retired.earlier[version]
    else:
        who = (describe(version) if describe is not None else None) or retired.said.get(version)
    by = f" by {who}, the change that made version {version}" if who else f" in the change that made version {version}"
    if version in retired.earlier:
        by += ", before the document was saved and opened again"
    head = f"{identifier}: " if identifier and identifier != named else ""
    return (f"{head}{named} was deleted{by}. An id is never given to another block: read that part of "
            f"the document again ({READ_HINT}) for the ids there now")


def describe_of(entry: Any) -> Callable[[int], str | None]:
    """Who made a version of ``entry``'s document, as the tool layer records it
    (``DocumentEntry.owners``: the tool and its caller's context), as docx-agent's
    ``describe_step`` words it."""
    def describe(version: int) -> str | None:
        owner = entry.owners.get(version)
        if owner is None:
            return None
        context = ", ".join(f"{key}={value}" for key, value in owner.caller.items())
        return f"{owner.tool} ({context})" if context else owner.tool

    return describe


def _strings(value: Any) -> Iterator[str]:
    """The strings of a call's arguments, in order (an address may be at any depth)."""
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from _strings(item)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from _strings(item)
