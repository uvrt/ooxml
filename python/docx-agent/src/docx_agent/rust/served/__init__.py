"""The host's side of the tools the Rust session serves (:mod:`docx_agent.rust.edits`): what
their calls read from the host, and each call's arguments as the session takes them.

What a tool's edits read from the host (its blobs, its page engine, the values it wrote for
references) is declared by each Rust tool where it is defined (``ooxml_tools_word::Hosting``)
and exported with the registry (``ooxml_core.tool_hosting``): :func:`hosting`.  Nothing here
lists the tools: every Word tool is the session's.

A served call's arguments go to the session as given, its document left out (the session's
id goes in its place: :func:`own`).  A tool whose arguments need the host first -- ``$name``
refs resolved, a fallback where the host's data is meant -- has an adapter in the module of
its group here, registered with :func:`adapter`; the modules are imported below, a sorted
list.  An adapter of a tool the registry does not export is refused when the export is read.
A form an adapter cannot hand over is refused as not supported yet (:func:`not_supported`).
"""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass
from typing import Any, Callable

from ooxml_edit.tools import ToolError

from .. import reads

Adapter = Callable[[Any, dict[str, Any]], dict[str, Any]]

#: The adapters, by tool (:func:`adapter`).
ADAPTERS: dict[str, Adapter] = {}


def not_supported(form: str, way_out: str, *, field: str | None = None) -> ToolError:
    """The refusal of a form the session is not handed yet: code ``refused``, the form in
    ``details.not_supported``, ``way_out`` saying what to give instead."""
    return ToolError("refused", f"{form} is not supported yet; {way_out}", field=field,
                     details={"not_supported": form})


class Adapted(dict):
    """A served tool's arguments as the session takes them, with the values written by
    reference the adapter handed over itself (``written``, :func:`written_json`) in place of
    the call's own: host data the session cannot read, such as a field ``from`` names or a
    value's own Markdown; and the rows of the sources the call names, as the host resolved
    them (``sources``, :func:`sources_json`)."""

    def __init__(self, given: dict[str, Any], written: str | None, *, sources: str | None = None) -> None:
        super().__init__(given)
        self.written = written
        self.sources = sources


def sources_json(found: list[tuple[str, Any]]) -> str | None:
    """The rows of sources (``(value as the call gives it, Rows)`` each: ``call.source``'s, or
    its refusal of the value) as ``CallDocuments(sources=...)`` takes them; ``None`` without
    any."""
    if not found:
        return None
    out: list[dict[str, Any]] = []
    for given, rows in found:
        if isinstance(rows, Exception):      # the host's refusal (an unknown dataset)
            out.append({"given": given, "refused": {"message": getattr(rows, "message", str(rows)),
                                                    "options": list(getattr(rows, "valid_options", None) or [])}})
            continue
        dataset = rows.dataset
        out.append({
            "given": given, "address": rows.address, "columns": [str(c) for c in dataset.columns],
            "types": list(dataset.types) if dataset.types is not None else None,
            "rows": [[_value(v) for v in row] for row in dataset.rows],
            "sources": ([[list(s) for s in row] for row in dataset.sources]
                        if dataset.sources is not None else None),
            "doc": rows.doc, "name": rows.name, "version": rows.version, "stale": bool(rows.stale),
            "cache": dict(dataset.cache) if dataset.cache else None})
    return json.dumps(out)


def _value(value: Any) -> Any:
    """A source value as :func:`sources_json` writes it: JSON's own where it has one, a date, a
    datetime and a time as their parts, anything else by its kind (``sources.what``)."""
    import datetime as _dt
    import math

    from ooxml_edit.tools import sources

    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, int):
        return value if -(2 ** 63) <= value < 2 ** 64 else {"int": str(value)}
    if isinstance(value, float):
        return value if math.isfinite(value) else {"other": sources.what(value)}
    if isinstance(value, _dt.datetime):
        return {"datetime": [value.year, value.month, value.day, value.hour, value.minute, value.second,
                             value.microsecond]}
    if isinstance(value, _dt.date):
        return {"date": [value.year, value.month, value.day]}
    if isinstance(value, _dt.time):
        return {"time": [value.hour, value.minute, value.second, value.microsecond]}
    return {"other": sources.what(value)}


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
        return [reads.refs(call, item) for item in value]
    return reads.refs(call, value)


def sided(call: Any, value: Any) -> Any:
    """A place that may say its side (``before:X``, ``after:X``; docx-agent's ``_place``)
    with its ``$name`` refs resolved."""
    if isinstance(value, str):
        for side in ("after:", "before:"):
            if value.startswith(side):
                return side + reads.refs(call, value[len(side):])
    return reads.refs(call, value)


def blocks_at(call: Any, at: Any) -> Any:
    """A placing tool's ``at`` with its ``$name`` refs resolved: a ref naming several blocks
    (a span, as ``word_insert_markdown``'s ``ref`` names what it made) is after its last
    block, before its first, or replaced whole.  A ref naming a heading's range is refused
    as not supported yet."""
    if not isinstance(at, str) or "$" not in at:
        return at
    head, _, rest = at.partition(":")
    if head not in ("after", "before", "replace"):
        head, rest = "", at
    parts = [call.resolve_ref(part) if part.startswith("$") else part for part in rest.split("..")]
    if any(part.startswith("h:") for part in parts):
        raise not_supported("placing at a heading's range by reference",
                            "give the heading's first or last block's id", field="at")
    joined = "..".join(parts)
    if ".." in joined and "@" not in joined and head in ("after", "before"):
        first, _, last = joined.partition("..")
        joined = (first if head == "before" else last).strip() or joined
    return (head + ":" if head else "") + joined


@dataclass(frozen=True)
class Hosting:
    """What the tools' calls read from the host, by tool."""

    #: Every tool the registry exports.
    tools: frozenset[str]
    #: Those that read the session's blobs (an inserted picture's bytes).
    blobs: frozenset[str]
    #: Those whose edits read the document's pages (a field's result, a header's page number,
    #: the upgrade's reflow): the session lays the versions out with the document's page
    #: engine (lane P2c13).
    laid: frozenset[str]
    #: Those that keep the values written by reference on the paragraphs they went into
    #: (lane P2d2); another tool given one is refused as not supported yet.
    keeps_written: frozenset[str]

    @classmethod
    def of(cls, exported: dict[str, dict[str, bool]]) -> "Hosting":
        """From ``ooxml_core.tool_hosting``'s export; refuses an adapter of a tool it does not
        export."""
        tools = frozenset(exported)
        stray = sorted(set(ADAPTERS) - tools)
        if stray:
            raise ValueError(f"adapters of tools the Rust session does not export: {', '.join(stray)}")

        def having(fact: str) -> frozenset[str]:
            return frozenset(name for name in tools if exported[name][fact])

        return cls(tools, having("blobs"), having("layout"), having("keeps_written"))


NONE = Hosting(frozenset(), frozenset(), frozenset(), frozenset())
_HOSTING: list[Hosting] = []
_HOSTING_LOCK = threading.Lock()


def hosting() -> Hosting:
    """What the tools' calls read from the host, from the registry's export (read once);
    nothing where ``ooxml_core`` is not installed."""
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

hosting()  # an adapter of a tool the registry does not export is refused here, at import
