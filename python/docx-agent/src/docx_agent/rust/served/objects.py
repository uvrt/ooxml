"""The object tools' adapters (:mod:`docx_agent.rust.served`): ``word_format_table``,
``word_drawings``, ``word_edit_table``, ``word_controls``, ``edit_chart``."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import Fallback, adapter, own, sided


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
    return {**own(arguments), "table": reads._refs(call, arguments.get("table"))}


@adapter("word_controls")
def _controls(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("control", "at")}}


@adapter("edit_chart")
def _chart(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    # (its data actions, the titles and five style actions; the rest answer ``unported``)
    return {**own(arguments), "target": reads._refs(call, arguments.get("target"))}
