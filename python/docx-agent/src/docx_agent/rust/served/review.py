"""The review tools' adapters (:mod:`docx_agent.rust.served`): ``word_changes``,
``word_comments`` (``word_set_tracking`` takes its arguments as given)."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import adapter, own, refs_in


@adapter("word_changes")
def _changes(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "ids": refs_in(call, arguments.get("ids")),
            "within": reads.refs(call, arguments.get("within"))}


@adapter("word_comments")
def _comments(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {**own(arguments), "items": None if items is None else [
        {**item, **{k: reads.refs(call, item.get(k)) for k in ("target", "within", "comment") if k in item}}
        if isinstance(item, dict) else item for item in items]}
