"""The object tools' adapters (:mod:`docx_agent.rust.served`): ``word_format_table``,
``word_drawings``."""

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
