"""The structure tools' adapters (:mod:`docx_agent.rust.served`): ``word_fields``,
``word_headers_footers``, ``word_move``, ``word_sections``, ``word_notes``, ``word_links``."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import adapter, own, refs_in, sided


@adapter("word_fields")
def _fields(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("after", "before", "at")},
            "ids": refs_in(call, arguments.get("ids"))}


@adapter("word_headers_footers")
def _headers_footers(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "section": reads._refs(call, arguments.get("section"))}


@adapter("word_move")
def _move(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "after": sided(call, arguments.get("after")),
            **{k: reads._refs(call, arguments.get(k)) for k in ("section_heading", "blocks", "before",
                                                                 "after_section")}}


@adapter("word_sections")
def _sections(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "after": sided(call, arguments.get("after")),
            **{k: reads._refs(call, arguments.get(k)) for k in ("before", "section")}}


@adapter("word_notes")
def _notes(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("at", "note")}}


@adapter("word_links")
def _links(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "target": reads._refs(call, arguments.get("target"))}
