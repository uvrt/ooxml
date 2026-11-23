"""The object tools' adapters (:mod:`docx_agent.rust.served`): ``word_format_table``,
``word_drawings``, ``word_edit_table``, ``word_controls``, ``edit_chart``."""

from __future__ import annotations

import json
from typing import Any

from .. import reads
from . import Adapted, adapter, not_supported, own, sided, sources_json, written_json


#: keep_together on a table taller than a page (docx-agent's ``TALLER_THAN_PAGE``, round 25).
TALLER_THAN_PAGE = "taller than a page: Word starts it on a new page and breaks it where the page is full"


@adapter("word_format_table")
def _format_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    table = reads.refs(call, arguments.get("table"))
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
    from ..engines import session_pages

    entry = call.entry
    table = result.renamed.get(table, table)
    laid = ooxml_core.CallLayout(session_pages(entry))
    context = call_context(call, entry)
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
        # The application's asset catalogue is the host's: its handler makes the asset a
        # blob of the session's and gives it as ``image``.
        raise not_supported("word_drawings asset", "give the picture as an image blob", field="asset")
    return {**own(arguments), "target": reads.refs(call, arguments.get("target")),
            "at": sided(call, arguments.get("at"))}


@adapter("word_edit_table")
def _edit_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    # The rows a call adds or writes into are looked at against the rows around them by the
    # session too (round 58, R126), and kept in its record (owner.SessionRows).
    given = {**own(arguments), "table": reads.refs(call, arguments.get("table"))}
    source = arguments.get("from") or None      # "": a strict decoder's placeholder for none
    facts = list(getattr(call, "facts", None) or ())
    from ...tools import field_text

    if isinstance(source, str) and not field_text.names_one_value(source):
        # The rows the source names, as the dispatcher resolves them (its refusals the
        # call's); an unknown dataset is the session's to word (what the document says of
        # the value: a drawing's id), with the datasets there are.
        found = _rows(call, source)
        return Adapted(given, written_json(facts, {}), sources=sources_json([(source, found)]))
    # Values by reference (rounds 45, 48, 49: R66, R89, R96): each cell's one value -- its own
    # from, else a top-level one naming one value where it gives no text -- resolved as the
    # handler reads it, a refusal handed over for the session to give where the handler does
    # (after the places, in the cells' order).
    from ooxml_edit.tools import ToolError

    refused: list[dict[str, Any]] = []
    for k, cell_from in _cell_froms(arguments, source):
        field = f"cells[{k}].from"
        try:
            text, record = call.fact(cell_from, field=field)
        except ToolError as error:
            field_text.with_rows(call, cell_from, field, error)
            refused.append({"given": cell_from, "field": field,
                            "refused": {"code": error.code, "message": error.message,
                                        "options": list(error.valid_options or [])}})
            continue
        facts.append(field_text.as_fact(cell_from, field, text, record))
    return Adapted(given, written_json(facts, {}), sources=json.dumps(refused) if refused else None)


def _cell_froms(arguments: dict[str, Any], source: Any) -> list[tuple[int, str]]:
    """The one value each ``set_cells`` entry names (docx-agent's ``_one_value_cells`` and
    ``_cell_value``): its own ``from``, else the top-level ``source`` (one value) where the
    entry gives no ``text`` either; none for an entry giving ``text`` and ``from`` (refused)."""
    cells = arguments.get("cells")
    if arguments.get("action") != "set_cells" or not isinstance(cells, list):
        return []
    out = []
    for k, cell in enumerate(cells):
        if not isinstance(cell, dict):
            continue
        own_from, text = cell.get("from") or None, cell.get("text") or None
        if own_from is None and text is None and isinstance(source, str):
            own_from = source
        if isinstance(own_from, str) and text is None:
            out.append((k, own_from))
    return out


@adapter("word_controls")
def _controls(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**own(arguments), **{k: reads.refs(call, arguments.get(k)) for k in ("control", "at")}}


def _rows(call: Any, source: str) -> Any:
    """``call.source`` of a ``from``; an unknown dataset as the host's refusal
    (:func:`~docx_agent.rust.served.sources_json` hands it over)."""
    from ooxml_edit.tools import ToolError

    try:
        return call.source(source, field="from")
    except ToolError as error:
        if error.code != "not_found" or not source.startswith("d:"):
            raise
        return error


@adapter("edit_chart")
def _chart(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    # (its data actions, the titles, the style actions and ``add``)
    return {**own(arguments), "target": reads.refs(call, arguments.get("target"))}
