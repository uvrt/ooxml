"""The application's block lookups in an open Word document: :func:`block_of` and
:func:`blocks_between`, the top-level body blocks an id is in, or a span covers, ``$name`` refs
resolved -- read from the version the Rust host holds."""

from __future__ import annotations

from typing import Any, Sequence

from ooxml_edit.tools import ToolError

from .specs import KIND


def block_of(session: Any, *args: str) -> str | None:
    """``block_of(session, doc, identifier)``: the top-level body block holding an id in the
    open document ``doc`` (``"d1"``) at its head, a ``$name`` ref resolved first, as :meth:`Session.resolve_ref
    <ooxml_edit.tools.Session.resolve_ref>` resolves it (refs live in the session, not the
    document)::

        block_of(session, "d1", "$intro")     # -> "t:5B1F07A2"
    """
    version, (identifier,) = _addressed(session, args, 1, "block_of")
    found = version.block_of(identifier)
    return str(found) if found is not None else None


def blocks_between(session: Any, *args: str) -> list[str]:
    """The top-level body blocks from one id's block through another's, either end a
    ``$name`` ref: ``blocks_between(session, doc, first, last)`` (see :func:`block_of`)."""
    version, (first, last) = _addressed(session, args, 2, "blocks_between")
    return [str(found) for found in version.blocks_between(first, last)]


def _addressed(session: Any, args: Sequence[str], count: int, name: str) -> tuple[Any, list[str]]:
    """The host's version of the document ``args[0]`` names and the ``count`` ids after it,
    each ``$name`` resolved."""
    if len(args) != count + 1:
        ends = "identifier" if count == 1 else "first, last"
        raise TypeError(f"{name} takes (session, doc, {ends})")
    doc, *values = args
    host = getattr(session, "rust", None)
    if host is None:
        raise TypeError(f"{name} reads a Word document of a session on the Rust host: "
                        f"{name}(session, doc, ...) with a session of a toolbox holding docx-agent's FORMAT")
    kind = session.kind_of(doc)               # not_found names the open documents
    if kind != KIND:
        raise ToolError("invalid_arguments", f"{doc} is a {kind} document; {name} reads a Word "
                        "document's blocks", field="doc")
    return host.version(doc), [session.resolve_ref(doc, value) for value in values]
