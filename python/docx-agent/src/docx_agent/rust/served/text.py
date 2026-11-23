"""The text tools' adapters (:mod:`docx_agent.rust.served`): ``word_set_text``,
``replace_text``, ``word_insert_text``, ``word_delete``, ``word_insert_markdown``."""

from __future__ import annotations

from typing import Any

from .. import reads
from ooxml_edit.tools import ToolError

from . import Adapted, adapter, blocks_at, own, written_json


@adapter("word_set_text")
def _set_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {"target": reads.refs(call, arguments.get("target")), "text": arguments.get("text"),
            "paragraph_style": arguments.get("paragraph_style"),
            "items": None if items is None else [
                {**item, "target": reads.refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


@adapter("replace_text")
def _replace_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "replace": arguments.get("replace"),
            "expect": arguments.get("expect"), "regex": arguments.get("regex") or None,
            "slides": arguments.get("slides"), "range": reads.refs(call, arguments.get("range")),
            "stories": arguments.get("stories", "body")}


@adapter("word_insert_text")
def _insert_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {**{k: arguments.get(k) for k in ("find", "where", "at", "text", "ref")},
            "target": reads.refs(call, arguments.get("target")),
            "within": reads.refs(call, arguments.get("within")),
            "items": None if items is None else [
                {**item, "target": reads.refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


@adapter("word_delete")
def _delete(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "within": reads.refs(call, arguments.get("within")),
            "target": reads.refs(call, arguments.get("target")),
            "collapse_space": arguments.get("collapse_space"), "force": arguments.get("force")}


@adapter("word_insert_markdown")
def _insert_markdown(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    given = own(arguments)
    blob = given.get("blob")
    if isinstance(blob, str) and given.get("markdown") is None and given.get("from") is None:
        # The blob is the host's: its text goes as the Markdown, read as the tool reads it.
        try:
            given["markdown"] = call.blob(blob).data.decode("utf-8")
        except UnicodeDecodeError:
            raise ToolError("invalid_arguments", f"{blob} is not UTF-8 text: Markdown is given as text",
                            field="blob") from None
        del given["blob"]
    given["at"] = blocks_at(call, given.get("at"))
    source = given.get("from")
    if isinstance(source, str):
        return _field_given(call, given, source)
    facts = call.facts_at("markdown") if getattr(call, "facts", None) else []
    values = zip(facts, _values_markdown(call, facts))
    own_markdown: dict[int, str | None] = {id(fact): markdown for fact, markdown in values if markdown is not None}
    return Adapted(given, written_json(call.facts, own_markdown)) if own_markdown else given


def _values_markdown(call: Any, facts: list[Any]) -> list[str | None]:
    """Each value's own Markdown, read again from its source (``field_text.reference_markdown``:
    a workbook cell's emphasis and list lines), ``None`` where it is its text."""
    from ...tools import field_text

    return [field_text.reference_markdown(call, fact) for fact in facts]


def _field_given(call: Any, given: dict[str, Any], source: str) -> dict[str, Any]:
    """``word_insert_markdown`` from one field of one row (``field_text.field_from``, the host's
    data): the field's Markdown as the call's, the value handed over as written into
    ``from`` (the session keeps it on every paragraph made).  Rows as a table stay ``from``
    (the session leaves them to the host)."""
    from ...tools.field_text import field_from

    field = field_from(call, source, given.get("from_columns"), given.get("header"), given.get("date_format"))
    if field is None:
        return given
    given["markdown"] = field.text
    del given["from"]
    return Adapted(given, written_json([field], {}))
