"""The text tools' adapters (:mod:`docx_agent.rust.served`): ``word_set_text``,
``replace_text``, ``word_insert_text``, ``word_delete``, ``word_insert_markdown``."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import Fallback, adapter, own


@adapter("word_set_text")
def _set_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {"target": reads._refs(call, arguments.get("target")), "text": arguments.get("text"),
            "paragraph_style": arguments.get("paragraph_style"),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


@adapter("replace_text")
def _replace_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "replace": arguments.get("replace"),
            "expect": arguments.get("expect"), "regex": arguments.get("regex") or None,
            "slides": arguments.get("slides"), "range": reads._refs(call, arguments.get("range")),
            "stories": arguments.get("stories", "body")}


@adapter("word_insert_text")
def _insert_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {**{k: arguments.get(k) for k in ("find", "where", "at", "text", "ref")},
            "target": reads._refs(call, arguments.get("target")),
            "within": reads._refs(call, arguments.get("within")),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


@adapter("word_delete")
def _delete(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "within": reads._refs(call, arguments.get("within")),
            "target": reads._refs(call, arguments.get("target")),
            "collapse_space": arguments.get("collapse_space"), "force": arguments.get("force")}


@adapter("word_insert_markdown")
def _insert_markdown(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    if getattr(call.entry.held, "min_text_size", None) is not None:
        # The text a call writes below the application's minimums is measured in Python
        # (``note_small_text``); the session has no minimums.
        raise Fallback("a minimum text size")
    given = own(arguments)
    blob = given.get("blob")
    if isinstance(blob, str) and given.get("markdown") is None and given.get("from") is None:
        # The blob is the host's: its text goes as the Markdown, read as the tool reads it.
        try:
            given["markdown"] = call.blob(blob).data.decode("utf-8")
        except UnicodeDecodeError:
            raise Fallback("a blob that is not UTF-8") from None
        del given["blob"]
    given["at"] = _blocks_at(call, given.get("at"))
    return given


def _blocks_at(call: Any, at: Any) -> Any:
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
