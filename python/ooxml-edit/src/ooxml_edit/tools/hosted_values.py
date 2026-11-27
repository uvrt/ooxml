"""The values channel (Phase 3, design section 2; Phase 4 deletes it): what a call the Rust host
runs reads of the documents the Python host holds, resolved here first and handed to the host as
values -- nothing calls back from Rust into Python.

For each such document the call may read, :func:`held_values` gives the host:

- the rows each address the call names gives there, or the format's refusal of it: every value
  of a declared source (``Tool.sources``, ``from``) and every reference in a declared text
  (``Tool.facts``, ``{{t:Notes/#2/Body}}``) that qualifies with the document, with the rows a
  reference reads (its source, and its source with the row it selects, as
  :meth:`~.dispatch.Call.fact` reads them);
- the tables it lists (its format's ``value_sources``), for every call that reads sources,
  datasets or references: what a refusal offers, and names (a table of another document);
- every listed table's rows too, where a check of numbers reads every document's values
  (``Tool.value_reads``), or where one of the call's addresses was refused (a refusal offers
  only sources that resolve).

The host reads nothing else of such a document: anything a call asks that was not resolved here
is its internal error, naming the stand-in (``ooxml_session::elsewhere``).

Must not: run the call, or change a document.
"""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, Any, Iterable, Mapping

from . import textfacts
from .datasets import DATASET_PREFIX
from .hosted_data import dataset_json
from .results import ToolError
from .sources import qualify, resolve as resolve_rows

if TYPE_CHECKING:
    from .dispatch import Toolbox
    from .hosted import HostedSession


class HeldValues:
    """What one call reads of the documents the Python host holds: each document's addresses
    resolved, and whether its every listed table is (:func:`held_values`).  :meth:`add` takes
    what the call asked that was not resolved (the host notes it: ``missed``), for the call
    to be made again with it."""

    def __init__(self, toolbox: "Toolbox", session: "HostedSession", wanted: dict[str, list[str]],
                 every: set[str], listing: bool) -> None:
        self.toolbox = toolbox
        self.session = session
        self.wanted = wanted
        self.every = every
        self.listing = listing
        self._made: dict[str, dict[str, Any]] = {}

    def json(self) -> str | None:
        """What ``HostSession.call(held=...)`` takes (JSON text); ``None`` for nothing."""
        out = []
        for doc, addresses in self.wanted.items():
            if doc not in self.session._held or (not addresses and doc not in self.every and not self.listing):
                continue
            made = self._made.get(doc)
            if made is None or made["_asked"] != (tuple(addresses), doc in self.every):
                made = self._made[doc] = _document(self.toolbox, self.session, doc, addresses, doc in self.every)
                made["_asked"] = (tuple(addresses), doc in self.every)
            out.append({key: value for key, value in made.items() if key != "_asked"})
        return json.dumps(out) if out else None

    def add(self, missed: Iterable[Mapping[str, str]]) -> bool:
        """The addresses a call asked that were not resolved (``[{"doc", "address"}]``)
        resolved too: whether any was new."""
        added = False
        for item in missed:
            doc, address = item.get("doc"), item.get("address")
            if doc in self.session._held and isinstance(address, str):
                listed = self.wanted.setdefault(doc, [])
                if address not in listed:
                    listed.append(address)
                    added = True
        return added


def held_values(toolbox: "Toolbox", session: "HostedSession", name: Any, arguments: Any) -> HeldValues | None:
    """What the call ``name`` with ``arguments`` reads of the documents ``session``'s Python host
    holds (:class:`HeldValues`); ``None`` where it holds none."""
    if not session._held or not isinstance(name, str):
        return None
    parsed = _parsed(arguments)
    wanted: dict[str, list[str]] = {doc: [] for doc in session._held}
    every: set[str] = set()
    listing = False
    for tool, given in _calls(toolbox, name, parsed) if parsed is not None else ():
        listing = listing or bool(tool.sources or tool.facts or tool.datasets)
        if tool.value_reads is not None and tool.value_reads(given):
            every.update(session._held)
        for address in _addresses(toolbox, session, tool, given):
            try:
                doc, found = qualify(address, session.documents, session.formats)
            except ToolError:
                continue                        # (the host refuses it, in its words)
            if doc in wanted and found not in wanted[doc]:
                wanted[doc].append(found)
    return HeldValues(toolbox, session, wanted, every, listing)


def _parsed(arguments: Any) -> Mapping[str, Any] | None:
    if isinstance(arguments, str):
        try:
            arguments = json.loads(arguments)
        except ValueError:
            return None
    return arguments if isinstance(arguments, Mapping) else None


def _calls(toolbox: "Toolbox", name: str, arguments: Mapping[str, Any]) -> Iterable[tuple[Any, Mapping[str, Any]]]:
    """The tool of the call and its arguments; a batch's: each op's."""
    tool = _tool(toolbox, name)
    if tool is None:
        return []
    if not tool.composite:
        return [(tool, arguments)]
    out = []
    for op in arguments.get("ops") or ():
        if not isinstance(op, Mapping) or not isinstance(op.get("arguments"), Mapping):
            continue
        found = _tool(toolbox, op.get("tool"))
        if found is not None and not found.composite:
            out.append((found, op["arguments"]))
    return out


def _tool(toolbox: "Toolbox", name: Any) -> Any:
    if not isinstance(name, str):
        return None
    try:
        return toolbox.tools[toolbox.tool_named(name)]
    except (ToolError, KeyError):
        return None


def _addresses(toolbox: "Toolbox", session: "HostedSession", tool: Any, arguments: Mapping[str, Any]) -> list[str]:
    """Every address the call's declared sources and texts name, with the rows a reference
    reads (:func:`_read_by`); datasets left out (the host holds them)."""
    from .dispatch import _walk

    found: list[str] = []

    def source(value: Any, _where: str) -> Any:
        if isinstance(value, str) and value and not value.startswith(DATASET_PREFIX):
            found.extend(_read_by(value, source=True))
        return value

    for path in tool.sources:
        _walk(arguments, path.split("."), "", source)
    if tool.facts:
        known = textfacts.known_prefixes(session.formats)

        def text(value: str, _where: str) -> str:
            if "{{" in value:
                for _match, address in textfacts.references(value, known):
                    if not address.startswith(DATASET_PREFIX):
                        found.extend(_read_by(address, source=False))
            return value

        for path in tool.facts:
            textfacts.walk_texts(arguments, path.split("."), "", text)
    return found


def _read_by(address: str, *, source: bool) -> list[str]:
    """The addresses whose rows a source value (``source``) or a reference ``address`` reads,
    as the host reads them: a source value as rows, and as one value -- its source, or its
    source with the row it selects (:meth:`~.dispatch.Call._fact`)."""
    out = [textfacts.strip_format(address)] if source else []
    base, segments, _format = textfacts.split(address)
    if len(segments) >= 2:
        segment, quoted = segments[0]
        numbered = segment.startswith("#") and segment[1:].isdigit() and not segment[1:].startswith("0")
        raw = textfacts.quote(segment) if quoted or not numbered else segment
        out.append(f"{base}/{raw}")
    else:
        out.append(base)
    return out


def _document(toolbox: "Toolbox", session: "HostedSession", doc: str, addresses: list[str],
              every: bool) -> dict[str, Any]:
    """One held document's values: the rows of ``addresses`` (and, with ``every`` or a refusal
    among them, of every table it lists), and the tables it lists."""
    entry = session._held[doc]
    fmt = session.formats[entry.kind]
    with entry.lock:
        listed = getattr(fmt, "value_sources", None)
        tables = [str(t) for t in listed(entry.document)] if listed is not None else None
        answers = {address: _answer(toolbox, fmt, entry, address) for address in addresses}
        if tables is not None and (every or any("refused" in a for a in answers.values())):
            for table in tables:
                if table not in answers:
                    answers[table] = _answer(toolbox, fmt, entry, table)
    return {"doc": doc, "answers": list(answers.values()), "value_sources": tables}


def _answer(toolbox: "Toolbox", fmt: Any, entry: Any, address: str) -> dict[str, Any]:
    """The rows ``address`` gives in the document, or the format's refusal of it."""
    try:
        return {"address": address, "rows": dataset_json(resolve_rows(fmt, entry.document, address))}
    except ToolError as error:
        refused = error
    except Exception as exc:  # noqa: BLE001 -- the format's error, as the dispatcher maps it
        refused = toolbox.map_exception(exc)
    return {"address": address, "refused": {"code": refused.code, "message": refused.message,
                                            "options": list(refused.valid_options or []),
                                            "details": dict(refused.details or {})}}


__all__ = ["held_values"]
