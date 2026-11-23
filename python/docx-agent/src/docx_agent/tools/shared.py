"""The Word handlers of the shared tools (``ooxml_edit.tools.shared``): one definition each
for every format, dispatched by the document's kind.

``open_document``, ``close_document``, ``undo``, ``batch``, ``read_blob`` and
``read_batch`` are the session's own (``shared.SESSION_TOOLS``, ``shared.READ_BATCH``);
this module adds docx's ``new_document`` (the host's: a new blank package from the Rust core,
opened in the session), and the session's ``save_document``, ``describe`` (its handler is in
:mod:`.read`), ``find_text``, ``replace_text``, ``render``, ``check``, ``edit_chart`` (its
``read`` action too) and ``set_properties``.  The shared definitions are the Rust session's
specs of these tools byte for byte (``ooxml_core.tool_specs``; a test holds them so); they
are kept as ooxml-edit's so that a toolbox with another library's tools merges them into
one tool."""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from ooxml_edit.tools import Result, ToolError, shared

from ..rust import reads
from . import handlers
from .read import describe

KIND = "docx"
PAGE_SIZES = ("A4", "Letter", "Legal", "A5", "A3")
#: The text's language of a new document made without a locale.
FALLBACK_LANGUAGE = "en-US"


def _decks_only(name: str, use: str) -> ToolError:
    return ToolError("invalid_arguments", f"{name} is for decks; for a document use {use}", field=name)


# -- new_document -------------------------------------------------------------------------------


def new_document(call: Any, kind: str, template_blob: str | None = None, size: str | None = None,
                 title: str | None = None, author: str | None = None, name: str | None = None) -> Result:
    """A new blank document's package made by the Rust core (``ooxml_core.new_document``),
    opened by the session alone."""
    if size is not None and size not in PAGE_SIZES:
        raise ToolError("invalid_arguments", f"a document's size is a page size, not {size!r}", field="size",
                        valid_options=list(PAGE_SIZES))
    if template_blob:
        raise ToolError("refused", "a new document from a template is not supported yet: open the template's "
                        "blob with open_document to edit the template itself, or make a blank document",
                        field="template_blob", details={"not_supported": "new_document template_blob"},
                        try_call={"tool": "new_document", "arguments": {
                            "kind": kind, **{k: v for k, v in (("size", size), ("title", title),
                                                               ("author", author), ("name", name)) if v}}})
    doc_id = call.session.open(_blank_bytes(call, size, title, author), name or "new.docx", kind=KIND,
                               source="new")
    entry = call.session.entry(doc_id)
    from .format import summary

    return Result(summary=f"Made {entry.name} as {doc_id}", created=[doc_id],
                  data={**entry.describe(), "summary": summary(entry.subject)})


def _stamp(value: Any) -> str:
    """The session clock's moment (a datetime, naive is UTC, or ISO 8601 text) in W3CDTF,
    UTC to the second, as Word writes ``dcterms:created``."""
    if isinstance(value, str):
        text = value.strip()
        value = datetime.fromisoformat(text[:-1] + "+00:00" if text.endswith("Z") else text)
    if not isinstance(value, datetime):
        value = datetime.now(timezone.utc)
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _blank_bytes(call: Any, size: str | None, title: str | None, author: str | None) -> bytes:
    """A new blank document's bytes made by the Rust core: the page, the call's locale's
    language and separators (en-US's without one), the properties and the call's clock."""
    import ooxml_core
    from ooxml_common.locales import EN_US

    locale = call.locale
    separators = locale if locale is not None else EN_US
    data = ooxml_core.new_document(
        page=(size or "A4", False),
        language=locale.language if locale is not None else FALLBACK_LANGUAGE,
        separators=(separators.decimal_separator, separators.list_separator),
        properties=(title, author, _stamp(call.now())))
    return bytes(data)


# -- save_document ------------------------------------------------------------------------------


def save_document(call: Any, doc: str, name: str, format: str, clean: bool = False) -> Result:
    """The file written by the Rust session (its bytes come back beside the reply); the output
    and its id state (``call.output``) are the host's, the coverage the session's."""
    files: list[tuple[str, str, bytes]] = []

    def answer(envelope: dict[str, Any]) -> Result:
        (_, given_format, data), = files
        if given_format == "markdown":
            return Result(summary=envelope.get("summary", ""), data=call.output(name, "markdown", data))
        result = Result(summary=envelope.get("summary", ""), data=call.output(name, format, data, state=True))
        replied = envelope.get("data") or {}
        if replied.get("cleaned") is not None:
            result.data["cleaned"] = replied["cleaned"]
        # How much of the document the layout could lay out: a check that passed on a complete
        # layout is told from one that could not see everything.  TODO(lane CUT-B): the
        # session's reply carries it; until then a save says none.
        if "coverage" in replied:
            result.data["coverage"] = replied["coverage"]
        return result

    return reads.serve(call, "save_document", {"name": name, "format": format, "clean": clean or None},
                       answer=answer, files=files)


# -- find_text, render, check -------------------------------------------------------------------


def find_text(call: Any, doc: str, text: str, regex: bool = False, slides: list | None = None,
              range: str | None = None, stories: str = "body", cursor: str | None = None) -> Result:
    if slides is not None:
        raise _decks_only("slides", "range or stories")
    return reads.serve(call, "find_text", {"text": text, "regex": regex or None, "range": reads.refs(call, range),
                                           "stories": stories, "cursor": cursor})


def render(call: Any, doc: str, **arguments: Any) -> Result:
    """Laid out by the document's page engine, its pages drawn in Rust
    (:func:`docx_agent.rust.reads.serve_render`)."""
    return reads.serve_render(call, arguments)


def check(call: Any, doc: str, **arguments: Any) -> Result:
    """The document's facts, the session's (:func:`docx_agent.rust.reads.serve_check`): it
    keeps what the check saw for the next one's ``since_last_check``."""
    return reads.serve_check(call, arguments)


#: The shared tools' Word handlers, by name.
HANDLERS = {
    "new_document": new_document, "save_document": save_document, "describe": describe,
    "find_text": find_text, "replace_text": handlers.edit, "render": render, "check": check,
    "edit_chart": handlers.edit, "set_properties": handlers.edit,
}

#: The shared tools with their Word handlers, in the order the toolbox lists them.  The
#: session's own tools follow (``read_batch``: many reads in one call, a reader's tool, in
#: the core).
TOOLS = [shared.handler(name, kind=KIND)(handler) for name, handler in HANDLERS.items()] \
    + [*shared.SESSION_TOOLS, shared.READ_BATCH]
