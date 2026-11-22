"""The object tools' adapters (:mod:`docx_agent.rust.served`): ``word_format_table``,
``word_drawings``, ``word_edit_table``, ``word_controls``."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import Adapted, Fallback, adapter, own, sided, sources_json, written_json


@adapter("word_format_table")
def _format_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "table": reads._refs(call, arguments.get("table"))}


@adapter("word_drawings")
def _drawings(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    if arguments.get("asset") is not None:
        # The application's asset catalogue is the host's (ooxml_edit.tools.assets).
        raise Fallback("a catalogue asset")
    return {**own(arguments), "target": reads._refs(call, arguments.get("target")),
            "at": sided(call, arguments.get("at"))}


@adapter("word_edit_table")
def _edit_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    # The rows a call adds or writes into are looked at against the rows around them by the
    # session too (round 58, R126), and kept in its record (owner.SessionRows).
    given = {**own(arguments), "table": reads._refs(call, arguments.get("table"))}
    source = arguments.get("from") or None      # "": a strict decoder's placeholder for none
    from ...tools import field_text

    if not isinstance(source, str) or field_text.names_one_value(source):
        return given
    # The rows the source names, as the dispatcher resolves them (its refusals the call's); an
    # unknown dataset is the session's to word (what the document says of the value: a
    # drawing's id), with the datasets there are.
    found = _rows(call, source)
    return Adapted(given, written_json(getattr(call, "facts", None) or (), {}),
                   sources=sources_json([(source, found)]))


@adapter("word_controls")
def _controls(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("control", "at")}}


def _rows(call: Any, source: str) -> Any:
    """``call.source`` of a ``from``; an unknown dataset as the host's refusal
    (:func:`~docx_agent.rust.served.sources_json` hands it over)."""
    from ooxml_edit.tools import ToolError

    try:
        return call.source(source, field="from")
    except ToolError as error:
        if error.code != "not_found" or not source.startswith("d:"):
            raise
        return error
