"""The object tools' adapters (:mod:`docx_agent.rust.served`): ``word_format_table``,
``word_drawings``, ``word_edit_table``, ``word_controls``."""

from __future__ import annotations

from typing import Any

from .. import reads
from . import Fallback, adapter, own, sided


#: keep_together on a table taller than a page (docx-agent's ``TALLER_THAN_PAGE``, round 25).
TALLER_THAN_PAGE = "taller than a page: Word starts it on a new page and breaks it where the page is full"


@adapter("word_format_table")
def _format_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    table = reads._refs(call, arguments.get("table"))
    if arguments.get("keep_together") and getattr(getattr(call.entry, "history", None), "grouped", False):
        # Round 28, W8: in a batch (the session's step group) the session leaves a table kept
        # together unmeasured; it is measured once the batch's last op ran, from one layout.
        call.defer(lambda result: _taller_after(call, result, table))
    return {**own(arguments), "table": table}


def _taller_after(call: Any, result: Any, table: str) -> None:
    """docx-agent's ``_later_taller``, served: the warning for a table kept together that is
    taller than a page, from the pages the session says the document draws it on now
    (``Session.block_pages``, numbered from 1, laid out by the document's page engine within
    the call's limit on laying out, with the call's writing settings), before the op's warning
    of a loose paragraph after it, else last."""
    import ooxml_core

    from ..edits import call_context
    from ..owner import options
    from ...pages import word_pages

    entry = call.entry
    table = result.renamed.get(table, table)
    engine, _why = word_pages(entry.held)
    laid = ooxml_core.CallLayout(engine) if engine is not None else None
    context = call_context(call, entry.held)
    mirror = entry.handle
    found, _not_laid = mirror.session.block_pages(mirror.doc, [table], options(context=context, layout=laid))
    kind, pages = (found[0][0], found[0][1]) if found else ("unknown", None)
    if kind == "outside" or (kind == "pages" and pages is not None and len(set(pages)) <= 1):
        return
    if kind == "pages":
        warning = f"{table} is {TALLER_THAN_PAGE}"
    else:
        warning = (f"{table} is kept together, but the document could not be laid out to it: whether it is "
                   "taller than a page is not known (if it is, Word starts it on a new page and breaks it where "
                   "the page is full)")
    if warning in result.warnings:
        return
    loose = next((k for k, w in enumerate(result.warnings) if f", between {table} and " in w), None)
    result.warnings.insert(len(result.warnings) if loose is None else loose, warning)


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
