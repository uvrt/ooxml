"""The host's side of the editing tools the Rust session serves (:mod:`docx_agent.rust.edits`):
which tools those are, and each call's arguments as the session takes them.

Which tools the session serves, and what their edits read from the host (its blobs, its page
engine, the values it wrote for references), is declared by each Rust tool where it is
defined (``ooxml_tools_word::Hosting``) and exported with the registry
(``ooxml_core.tool_hosting``): :func:`hosting`.  Nothing here lists the tools.

A served call's arguments go to the session as given, its document left out (the session's
id goes in its place: :func:`own`).  A tool whose arguments need the host first -- ``$name``
refs resolved, a fallback where the host's data is meant -- has an adapter in the module of
its group here, registered with :func:`adapter`; the modules are imported below, a sorted
list.  An adapter of a tool the session does not serve is refused when the export is read.
"""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass
from typing import Any, Callable

from .. import reads

Adapter = Callable[[Any, dict[str, Any]], dict[str, Any]]

#: The adapters, by tool (:func:`adapter`).
ADAPTERS: dict[str, Adapter] = {}


class Fallback(Exception):
    """The call is answered by the Python handler, for this reason."""


class Adapted(dict):
    """A served tool's arguments as the session takes them, with the values written by
    reference the adapter handed over itself (``written``, :func:`written_json`) in place of
    the call's own: host data the session cannot read, such as a field ``from`` names or a
    value's own Markdown."""

    def __init__(self, given: dict[str, Any], written: str | None) -> None:
        super().__init__(given)
        self.written = written


def written_json(facts: Any, markdown: dict[int, str | None]) -> str | None:
    """``facts`` (values written by reference) as ``Session.edit`` takes them, each with its
    own Markdown where ``markdown`` (by the fact's ``id``) has one; ``None`` without any."""
    if not facts:
        return None
    out = []
    for f in facts:
        item = {"field": f.field, "start": f.start, "end": f.end, "text": f.text, "record": f.record,
                "brief": f.brief()}
        if markdown.get(id(f)) is not None:
            item["markdown"] = markdown[id(f)]
        out.append(item)
    return json.dumps(out)


def adapter(tool: str) -> Callable[[Adapter], Adapter]:
    """Registers the decorated function as ``tool``'s adapter: the call's arguments as the
    session takes them."""

    def register(function: Adapter) -> Adapter:
        if tool in ADAPTERS:
            raise ValueError(f"{tool} has two adapters: {ADAPTERS[tool].__qualname__}, {function.__qualname__}")
        ADAPTERS[tool] = function
        return function

    return register


def own(arguments: dict[str, Any]) -> dict[str, Any]:
    """The call's arguments but its document (the session's id goes in its place)."""
    return {k: v for k, v in arguments.items() if k != "doc"}


def arguments_for(call: Any, tool: str, arguments: dict[str, Any]) -> dict[str, Any]:
    """The call's arguments as the session takes them: its adapter's, or :func:`own`."""
    found = ADAPTERS.get(tool)
    return own(arguments) if found is None else found(call, arguments)


def refs_in(call: Any, value: Any) -> Any:
    """A list of addresses (or one) with its ``$name`` refs resolved."""
    if isinstance(value, list):
        return [reads._refs(call, item) for item in value]
    return reads._refs(call, value)


def sided(call: Any, value: Any) -> Any:
    """A place that may say its side (``before:X``, ``after:X``; docx-agent's ``_place``)
    with its ``$name`` refs resolved."""
    if isinstance(value, str):
        for side in ("after:", "before:"):
            if value.startswith(side):
                return side + reads._refs(call, value[len(side):])
    return reads._refs(call, value)


def blocks_at(call: Any, at: Any) -> Any:
    """A placing tool's ``at`` with its ``$name`` refs resolved as the tool resolves them
    (``text._at``: a span's last block after it, its first before it); a ref naming a
    heading's range is the Python view's to expand."""
    if not isinstance(at, str) or "$" not in at:
        return at
    head, _, rest = at.partition(":")
    if head not in ("after", "before", "replace"):
        rest = at
    if any("h:" in call.resolve_ref(part) for part in rest.split("..") if part.startswith("$")):
        raise Fallback("a heading's range by reference")
    from ...tools.text import _at

    return _at(call, at)


@dataclass(frozen=True)
class Hosting:
    """The editing tools the session serves, and what their edits read from the host."""

    #: The tools the session edits with.
    served: frozenset[str]
    #: Those that read the session's blobs (an inserted picture's bytes).
    blobs: frozenset[str]
    #: Those whose edits read the document's pages (a field's result, a header's page number,
    #: the upgrade's reflow): the session lays the versions out with the document's page
    #: engine (lane P2c13).
    laid: frozenset[str]
    #: Those that keep the values written by reference on the paragraphs they went into
    #: (lane P2d2); another served tool given one is answered by the Python handler.
    keeps_written: frozenset[str]

    @classmethod
    def of(cls, exported: dict[str, dict[str, bool]]) -> "Hosting":
        """From ``ooxml_core.tool_hosting``'s export; refuses an adapter of a tool not served."""
        served = frozenset(name for name, facts in exported.items() if facts["served"])
        stray = sorted(set(ADAPTERS) - served)
        if stray:
            raise ValueError(f"adapters of tools the Rust session does not serve: {', '.join(stray)}")

        def having(fact: str) -> frozenset[str]:
            return frozenset(name for name in served if exported[name][fact])

        return cls(served, having("blobs"), having("layout"), having("keeps_written"))


NONE = Hosting(frozenset(), frozenset(), frozenset(), frozenset())
_HOSTING: list[Hosting] = []
_HOSTING_LOCK = threading.Lock()


def hosting() -> Hosting:
    """The tools the session serves, from the registry's export (read once); none where
    ``ooxml_core`` is not installed."""
    with _HOSTING_LOCK:
        if not _HOSTING:
            try:
                import ooxml_core
            except ImportError:
                _HOSTING.append(NONE)
            else:
                _HOSTING.append(Hosting.of(json.loads(ooxml_core.tool_hosting("docx"))))
        return _HOSTING[0]


from . import objects, review, structure, style, text  # noqa: E402,F401 -- the adapters register on import

hosting()  # an adapter of a tool the session does not serve is refused here, at import
