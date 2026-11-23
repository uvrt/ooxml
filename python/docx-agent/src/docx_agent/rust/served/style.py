"""The style tools' adapters (:mod:`docx_agent.rust.served`): ``word_format``, ``word_lists``,
``word_follow_sample`` (``word_styles`` and ``word_template`` take their arguments as given)."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import adapter, own, refs_in


@adapter("word_format")
def _format(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    find = arguments.get("find")
    finds = find if isinstance(find, list) else None if find is None else [find]
    return {**own(arguments), "targets": refs_in(call, arguments.get("targets")),
            "like": reads.refs(call, arguments.get("like")),
            "find": None if finds is None else [
                {**item, "within": reads.refs(call, item.get("within"))} if isinstance(item, dict) else item
                for item in finds]}


@adapter("word_lists")
def _lists(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), "targets": refs_in(call, arguments.get("targets")),
            "join": reads.refs(call, arguments.get("join"))}


@adapter("word_follow_sample")
def _follow_sample(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    sample = arguments.get("sample")
    if isinstance(sample, dict):
        # (a sample in another document: its id the host's, as the session's names take it)
        sample = {**sample, "block": reads.refs(call, sample.get("block"))}
    return {**own(arguments), "sample": sample, "copies": refs_in(call, arguments.get("copies"))}
