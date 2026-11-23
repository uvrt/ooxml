"""Reading: ``describe`` (the shared tool's Word handler, made a tool in :mod:`.shared`),
``word_read`` (Markdown with ids, paged) and ``word_inspect`` (a block's exact formatting),
each the Rust session's read of the version the call reads (:mod:`docx_agent.rust.reads`),
with what the host adds: its refs resolved, and ``describe``'s name, tracking, problems and
memory."""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import Result

from ..rust import reads


def _memory(call: Any) -> dict[str, Any]:
    """What this process holds, in MB -- ``document``, what the session keeps of the document
    (its versions, its history and its layouts: ``Session.document_memory``), and the
    process's own (:func:`ooxml_edit.tools.memory.process_memory`) -- so a run sees its own
    growth, call by call: the application's, never the model's."""
    from ooxml_edit.tools.memory import process_memory

    mirror = reads.owned(call.entry).handle
    kept = mirror.session.document_memory(mirror.doc)
    return {"document": round(kept / 2 ** 20, 1), **process_memory()}


def describe(call: Any, doc: str) -> Result:
    """The shared ``describe``'s Word handler: the document at a glance, read by the session
    -- its pages laid out by the document's page engine -- with what the host holds of the
    document added (:func:`_described`)."""
    return reads.serve(call, "describe", {}, layout=reads.session_layout(call),
                       answer=lambda envelope: _described(call, envelope))


def _described(call: Any, envelope: dict[str, Any]) -> Result:
    """``describe``'s answer: the session's data (its ``pages`` too: the count, or the
    layout's refusal as ``<code>: <message>``) with what the host holds (``name``,
    ``tracking``, ``problems``) in their places, and ``memory``."""
    entry = call.entry
    data: dict[str, Any] = {"name": entry.name}
    for key, value in envelope["data"].items():
        if key == "name":
            continue
        if key == "tracking":
            value = ({"author": entry.tracking["author"]} if entry.tracking and entry.tracking.get("on")
                     else None)
        data[key] = value
        if key == "controls":
            data["problems"] = [str(p) for p in entry.baseline_problems][:20]
    result = Result(summary=f"Described {entry.name}", data=data)
    result.memory = _memory(call)
    return result


def word_read(call: Any, doc: str, range: str | None = None, view: str = "final",
              stories: str = "body", cursor: str | None = None) -> Result:
    return reads.serve(call, "word_read", {"range": reads.refs(call, range), "view": view, "stories": stories,
                                           "cursor": cursor})


def word_inspect(call: Any, doc: str, target: str, layout: bool = False) -> Result:
    """With ``layout``, placed by the session's layout of the version, laid out by the
    session's page engine for the document."""
    return reads.serve(call, "word_inspect", {"target": reads.refs(call, target), "layout": layout or None},
                       layout=reads.session_layout(call) if layout else None)


HANDLERS = {"word_read": word_read, "word_inspect": word_inspect}
