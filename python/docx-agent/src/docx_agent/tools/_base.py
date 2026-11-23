"""What every Word tool shares: the edit checks and their timing, problems' keys, and the
application's block lookups.

* :func:`checks` is the docx format's ``checks`` hook: after a changing call -- or once, at
  the end of a ``batch`` -- the validation delta against the document as opened and the
  pages the change reflowed, both the Rust session's.
* :class:`Timing`: where a long call's time went, part by part.
* :func:`block_of` and :func:`blocks_between`: the top-level body blocks an id is in, or a
  span covers, in an open document, ``$name`` refs resolved -- the session's reading.
"""

from __future__ import annotations

import time
from contextlib import contextmanager
from typing import Any, Iterator, Sequence

from ooxml_edit.tools import ToolError

from ..rust import reads
from ..rust.edits import Made
from ..rust.owner import owned_validation


class Timing:
    """Where a long call's time went, part by part -- its result's ``data.timing``, seconds
    of wall-clock time each, rounded to milliseconds.  The call times its parts
    (:meth:`part`) and touches the call with it (``call.touch(timing)``); the checks after the
    call add theirs (``validate``).  The time a call waited for the document's lock is in none
    of them."""

    def __init__(self) -> None:
        #: The parts so far, in order: what the result's ``timing`` is (the same dict).
        self.parts: dict[str, float] = {}

    @contextmanager
    def part(self, name: str) -> Iterator[None]:
        started = time.perf_counter()
        try:
            yield
        finally:
            self.add(name, time.perf_counter() - started)

    def add(self, name: str, seconds: float) -> None:
        self.parts[name] = round(self.parts.get(name, 0.0) + seconds, 3)


def checks(entry: Any, touched: Sequence[Any]) -> dict[str, Any]:
    """The docx format's ``checks``: after a changing call -- or once, at the end of a
    ``batch`` -- the validation delta against the document as opened, and the reflow the
    session measured after the call's edits (``None`` where it measured none: no engine laid
    the document out before), both the session's (:class:`~docx_agent.rust.edits.Made`; the
    head's validation where the call made no version)."""
    made = [t for t in touched if isinstance(t, Made)]
    timed = next((t for t in touched if isinstance(t, Timing)), None) or Timing()
    with timed.part("validate"):
        validated = made[-1].validation if made else owned_validation(entry)
    reflow = next((m.reflow for m in reversed(made) if m.reflow is not None), None)
    return {"validate": validated, "reflow": reflow}


def problem_key(problem: Any) -> Any:
    """What tells two validation problems apart (the save gate compares them by it)."""
    return (getattr(problem, "code", None), getattr(problem, "part", None), str(problem))


def block_of(where: Any, *args: str) -> str | None:
    """The top-level body block holding an id, a ``$name`` ref resolved first (refs live in
    the session, not the document).  In a handler, ``block_of(call, identifier)``; in the
    application, outside a call, ``block_of(session, doc, identifier)`` -- the open document
    ``doc`` (``"d1"``) at its head, its refs as :meth:`Session.resolve_ref
    <ooxml_edit.tools.Session.resolve_ref>` resolves them::

        block_of(session, "d1", "$intro")     # -> "t:5B1F07A2"
    """
    version, (identifier,) = _addressed(where, args, 1, "block_of")
    found = version.block_of(identifier)
    return str(found) if found is not None else None


def blocks_between(where: Any, *args: str) -> list[str]:
    """The top-level body blocks from one id's block through another's, either end a
    ``$name`` ref: ``blocks_between(call, first, last)`` in a handler,
    ``blocks_between(session, doc, first, last)`` outside a call (see :func:`block_of`)."""
    version, (first, last) = _addressed(where, args, 2, "blocks_between")
    return [str(found) for found in version.blocks_between(first, last)]


def _addressed(where: Any, args: Sequence[str], count: int, name: str) -> tuple[Any, list[str]]:
    """The session's version and the ``count`` ids of a call's ``(call, *ids)`` or the
    application's ``(session, doc, *ids)``, each ``$name`` resolved."""
    in_call = hasattr(where, "entry") and not callable(getattr(where, "entry"))
    if in_call and len(args) == count:
        return reads.version_read(where.entry)[1], [reads.refs(where, value) for value in args]
    if in_call or len(args) != count + 1:
        ends = "identifier" if count == 1 else "first, last"
        raise TypeError(f"{name} takes (call, {ends}) in a handler or (session, doc, {ends}) outside one")
    from .format import KIND                  # format imports this module

    doc, *values = args
    entry = where.entry(doc)                  # not_found names the open documents
    if entry.kind != KIND:
        raise ToolError("invalid_arguments", f"{doc} is a {entry.kind} document; {name} reads a Word "
                        "document's blocks", field="doc")
    return reads.version_read(entry)[1], [where.resolve_ref(doc, value) for value in values]
