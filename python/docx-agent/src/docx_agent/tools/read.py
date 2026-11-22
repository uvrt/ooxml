"""Reading: the document at a glance (``word_describe``, the shared ``describe``'s Word
handler, made a tool in :mod:`.shared`), ``word_read`` (Markdown with ids,
paged) and ``word_inspect`` (a block's exact formatting).

Lane P1c: each is answered by the Rust session where its answer is the same
(:mod:`docx_agent.rust.reads`, the interpreter lock let go while Rust reads), else by the
Python path here (``_python_*``), counted with why."""

from __future__ import annotations

from collections import Counter
from typing import Any

from ooxml_edit.tools import Result, ToolError, boolean, page_text, string

from ..markdown.read import _TYPED_NUMBER
from .laid import held_facts
from ._base import resolve, short, tool_names, word_tool

DOC = string("Document id.")
_AUTHOR = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}author"
CURSOR = string("next_cursor of the previous page.", optional=True)

#: Leave room in a page for the envelope around the Markdown.
_PAGE_MARGIN = 1500


def heading_tree(document: Any) -> list[dict[str, Any]]:
    """The body's headings in order (LW6's tree, composed): id, level, text, how it is
    numbered (``list`` or typed ``text``), and ``blocks``: how many blocks it owns, itself
    and everything up to the next heading at its level or above -- all ``h:<id>`` names
    (round 36, R32).  In a document of several sections, ``section`` is the one the
    heading is in (``s:2``).  Which paragraphs are headings: :mod:`docx_agent.edit.outline`."""
    from ..edit.outline import headings

    found = headings(document)
    several = len(document.sections()) > 1
    out = []
    for heading in found:
        item: dict[str, Any] = {"id": heading.id, "level": heading.level, "text": short(heading.text, 100),
                                "blocks": heading.blocks}
        if document.paragraph(heading.id).list is not None:
            item["numbered"] = "list"
        elif _TYPED_NUMBER.match(heading.text):
            item["numbered"] = "text"
        if several:
            item["section"] = f"s:{heading.section}"
        out.append(item)
    return out


def _pages(call: Any) -> Any:
    """The page count, or ``at least N (layout stopped)``.  Round 58: asked of the worker
    holding the document's layout (:func:`docx_agent.layout.check_facts`, a check's bare
    facts, the call's lock let go while it lays out) -- the layout never comes here for a
    number: a long document's whole layout read into this process for each version kept
    hundreds of megabytes per version here.  Laid out here only where the document has no
    such worker (a converter of the old protocol, a layout kept as its pickle) -- in a batch
    too since round 59 (:func:`~.laid.held_facts`)."""
    from ..layout import BARE_REQUEST

    try:
        made = held_facts(call, dict(BARE_REQUEST))
        if made is not None:
            reflow = made[0]["reflow"]
            return reflow["pages"] if reflow["complete"] else f"at least {reflow['pages']} (layout stopped)"
        layout = call.document.layout()
    except ToolError as error:
        return f"{error.code}: {error.message}"
    known = layout.pages_known
    return known if known is not None else f"at least {layout.page_count} (layout stopped)"


def _memory(call: Any) -> dict[str, Any]:
    """Round 58: what this process holds, in MB -- ``document``, the document's undo history
    (each distinct piece of its states once) and the layouts of it kept in this process
    (:func:`docx_agent.layout.layout_cost`, an estimate: about a megabyte a page), and the
    process's own (:func:`ooxml_edit.tools.memory.process_memory`: ``rss`` and ``pss`` where
    the system says, else ``peak_rss``) -- so a run sees its own growth, call by call."""
    from ooxml_edit.tools.memory import process_memory

    from ..layout import layout_cost

    live = call.session.documents.get(call.entry.doc_id) if call.entry is not None else None
    document = live.document if live is not None else call.document
    kept = document.history.kept_bytes()
    kept += sum(layout_cost(conversion.raw) for conversion in list(getattr(document, "_layouts", {}).values()))
    return {"document": round(kept / 2 ** 20, 1), **process_memory()}


def _sections(document: Any) -> list[dict[str, Any]]:
    out = []
    for section in document.sections():
        width, height = section.page_width, section.page_height
        item = {"id": section.id, "start": section.start, "orientation": section.orientation,
                "page": [width, height], "margins": {k: section.margins[k] for k in
                                                     ("top", "right", "bottom", "left")},
                "columns": section.columns.get("count", 1)}
        stories = section.stories()
        for which in ("header", "footer"):
            shown = {kind: value["story"] for kind, value in stories[which].items() if value["story"]}
            if shown:
                item[which + "s"] = shown
        out.append(item)
    return out


def _styles_in_use(document: Any) -> list[dict[str, Any]]:
    """The paragraph styles applied, by name, most used first.  Round 58: counted by the
    style each paragraph declares, then each one's name found once -- a paragraph's
    ``style_name`` reads every style in the document, which made describe of a long
    document read the styles part once per paragraph."""
    declared: Counter = Counter()
    for story in document.stories:
        for paragraph in story.paragraphs:
            declared[paragraph.style] += 1
    styles = document.styles
    default = styles.default("paragraph")
    counts: Counter = Counter()
    for style_id, n in declared.items():
        found = (styles.find(style_id, "paragraph") if style_id else None) or default
        counts[found.name if found else None] += n
    return [{"name": name, "paragraphs": n} for name, n in counts.most_common(40) if name]


def word_describe(call: Any, doc: str) -> Result:
    """The shared ``describe``'s Word handler: the document at a glance -- read in Rust,
    with what Python holds of the document added (:func:`_described`)."""
    from ..rust import reads

    return reads.serve(call, "describe", {}, lambda: _python_describe(call, doc),
                       answer=lambda envelope: _described(call, envelope))


def _described(call: Any, envelope: dict) -> Result:
    """``describe``'s answer from the Rust session's: its data with what the session holds
    (``name``, ``pages``, ``tracking``, ``problems``) in today's places, and ``memory``."""
    entry = call.entry
    rust = envelope["data"]
    data: dict[str, Any] = {"name": entry.name, "pages": _pages(call)}
    for key, value in rust.items():
        if key == "name":
            continue
        if key == "tracking":
            value = ({"author": entry.tracking["author"]} if entry.tracking and entry.tracking.get("on")
                     else None)
        data[key] = value
        if key == "controls":
            data["problems"] = [str(p) for p in entry.baseline_problems][:20]
    result = Result(summary=f"Described {entry.name}", data=data)
    result.memory = _memory(call)        # round 58: the application's, never the model's
    return result


def _python_describe(call: Any, doc: str) -> Result:
    """``describe`` read by docx-agent's own model (the Rust session's fallback)."""
    document = call.document
    entry = call.entry
    # Round 59 (R135): the comments' records found once -- each comment's ``author``,
    # ``parent`` and ``done`` found its record again among them all, three times per
    # comment: a long report's describe spent seconds there, more with each comment.
    comments = document._comment_records()
    threads = [c for c in comments if c["parent"] is None]
    # Round 58: the records' authors read from the records found once (a revision's
    # ``author`` finds its record again among them all: describe of a document with
    # thousands of revisions read them all once per revision).
    revisions = [element for _, _, element in document._revision_elements()]
    fields = Counter(f.keyword for f in document.fields())
    data: dict[str, Any] = {
        "name": entry.name, "pages": _pages(call), "compatibility_mode": document.compatibility_mode,
        "properties": {k: v for k, v in document.properties.items() if k in ("title", "author", "subject")},
        "tracking": ({"author": entry.tracking["author"]} if entry.tracking and entry.tracking.get("on")
                     else None),
        "headings": heading_tree(document)[:80],
        "sections": _sections(document),
        "styles_in_use": _styles_in_use(document),
        "comments": {"threads": len(threads), "open": sum(1 for c in threads if not c["done"]),
                     "by_author": dict(Counter(c["author"] for c in comments))},
        "revisions": {"records": len(revisions), "changes": len(document.changes()),
                      "by_author": dict(Counter(element.get(_AUTHOR) for element in revisions))},
        "fields": dict(fields),
        # Round 54 (lanes M, R): rows and columns as word_edit_table counts them (accepted).
        "tables": [{"id": t.id, **_accepted_size(document, t),
                    "first_row": short(" | ".join(_cell_text(t, 0, k) for k in range(t.column_count)), 80),
                    **{key: short(value, 80) for key, value in t.alt_text.items()},
                    **_kept_together(document, t)}
                   for t in document.tables()[:30]],
        "drawings": [{"id": d.id, "kind": drawing_kind(d), **({"alt_text": short(d.alt_text, 60)} if d.alt_text else {})}
                     for d in _drawings(document)][:30],
        "notes": dict(Counter(n.kind for n in document.notes())),
        "controls": len(document.content_controls()),
        "problems": [str(p) for p in entry.baseline_problems][:20],
    }
    # Each content control a tag or title names, so a sample can be found by it.
    named = [{"id": c.id, "kind": c.kind, **({"tag": short(c.tag, 80)} if c.tag else {}),
              **({"title": short(c.alias, 80)} if c.alias else {})}
             for c in document.content_controls() if c.tag or c.alias]
    if named:
        data["tagged"] = named[:30]
    if len(data["headings"]) == 80:
        data["headings_note"] = "the first 80 headings; word_read a range for the rest"
    result = Result(summary=f"Described {entry.name}", data=data)
    result.memory = _memory(call)        # round 58: the application's, never the model's
    return result


def _accepted_size(document: Any, table: Any) -> dict:
    from ..edit import provenance as _provenance
    from .objects import _table_entry

    entry = _table_entry(document, table.id)
    rows = len(_provenance.accepted_rows(entry)) if entry is not None else len(table.rows)
    return {"rows": rows, "columns": len(document.accepted_columns(table.id)[0])}


def _kept_together(document: Any, table: Any) -> dict:
    """``keep_together: true`` where a table's rows keep it on one page (round 25, W2:
    word_format_table ``keep_together``), with ``keep_with_next: true`` where its last row
    keeps with the block after it too (W5); nothing where they do not."""
    from ..edit.table_format import kept_together, kept_with_next, row_keeps

    if len(table.rows) < 2:
        return {}
    rows = row_keeps(document, document._table_entry(table.id)[1].element)
    if not kept_together(rows):
        return {}
    return {"keep_together": True, **({"keep_with_next": True} if kept_with_next(rows) else {})}


def _cell_text(table: Any, row: int, column: int) -> str:
    try:
        return table.cell(row, column).text
    except IndexError:
        return ""


def drawing_kind(drawing: Any) -> str:
    return getattr(drawing, "kind", None) or "picture"


def _drawings(document: Any) -> list[Any]:
    seen = []
    for picture in document.pictures():
        seen.append(document.drawing(picture.id))
    known = {d.id for d in seen}
    for chart in document.charts():
        identifier = getattr(chart, "id", None)
        if identifier and identifier not in known:
            seen.append(document.drawing(identifier))
            known.add(identifier)
    return seen


@word_tool("word_read", "The document as Markdown with an id comment before each block. Paged: pass next_cursor back. view markup shows tracked changes, comments and fields in place.",
           {"doc": DOC,
            "range": string("Block, cell, span p:A..t:B (p:A.. to end), heading h:p:A or section s:2. Default body.", optional=True),
            "view": string("final: changes accepted; markup: changes, comments, fields in place. Default final.",
                           enum=["final", "markup", "original"], optional=True),
            "stories": string("all adds headers, footers, notes. Default body.",
                              enum=["body", "all"], optional=True),
            "cursor": CURSOR},
           refs=('range',), group="core")
def word_read(call: Any, doc: str, range: str | None = None, view: str = "final",
              stories: str = "body", cursor: str | None = None) -> Result:
    from ..rust import reads

    given = reads._refs(call, range)
    return reads.serve(call, "word_read", {"range": given, "view": view, "stories": stories, "cursor": cursor},
                       lambda: _python_word_read(call, doc, range, view, stories, cursor), addresses=(given,))


def _python_word_read(call: Any, doc: str, range: str | None, view: str, stories: str,
                      cursor: str | None) -> Result:
    """``word_read`` by docx-agent's own model (the Rust session's fallback)."""
    if range is not None and stories == "all":
        raise ToolError("invalid_arguments", "stories=all reads whole stories: give no range with it",
                        field="range")
    entry = call.entry
    range = resolve(call, range)
    key = ("markdown", range, view, stories)
    read = lambda: call.document.to_markdown(range, view=view, stories=stories)  # noqa: E731
    # Inside a batch the version moves only at its end: read the state as it is now.
    markdown = read() if call.in_batch else entry.cached(entry.check_cache, key, read)
    limit = max(2000, call.limits.max_result_chars - _PAGE_MARGIN)
    text, next_cursor = page_text(markdown, cursor=cursor, limit=limit)
    result = Result(summary=f"Read {range or 'the document'} ({view})", data=text,
                    next_cursor=next_cursor)
    result.total = len(markdown)
    return result


@word_tool("word_inspect", "Exact formatting of blocks: style, runs, effective font, size, spacing, indents, list, revisions; each table cell's fill and drawn text colour; with layout, where each is placed.",
           {"doc": DOC,
            "target": string("A block, cell t:X/c1,1, span p:A..t:B (p:A.. to the end) or section."),
            "layout": boolean("Also each block's page, top and bottom. Default false.",
                              optional=True)},
           refs=('target',), group="word_text")
def word_inspect(call: Any, doc: str, target: str, layout: bool = False) -> Result:
    from ..rust import reads

    given = reads._refs(call, target)
    # (with layout: placed by the session's own layout of the version, its engine set to the
    # document's fonts and locale here; where it has none -- docx-agent's Rust layout off, or
    # options it does not take -- placed by docx-agent's own layout, as before)
    engine = reads.session_layout(call) if layout else None
    return reads.serve(call, "word_inspect", {"target": given, "layout": layout or None},
                       lambda: _python_word_inspect(call, doc, target, layout), addresses=(given,),
                       layout=engine, why_not="no layout in the session" if layout and engine is None else None)


def _python_word_inspect(call: Any, doc: str, target: str, layout: bool) -> Result:
    """``word_inspect`` by docx-agent's own model (the Rust session's fallback)."""
    from ..state import state as state_of

    state = state_of(call.document, resolve(call, target), layout=layout, placed=_placed_by_worker(call))
    data = {key: tool_names(state[key], "effective") for key in ("cell", "styles", "blocks", "notes", "comments",
                                                                  "revisions")
            if state.get(key)}
    return Result(summary=f"Inspected {target}", data=data)


def _placed_by_worker(call: Any) -> Any:
    """``word_inspect``'s placements (:func:`docx_agent.state.state`'s ``placed``) read in
    the worker holding the document's layout (round 59, R134:
    :func:`docx_agent.layout.held_answers`, the call's lock let go while it lays out where
    it may): the first ``layout`` of a 340-page document's table brought the whole layout
    here, hundreds of megabytes kept after the call.  ``None`` from it where the document
    has no such worker: placed from its layout here, as before."""
    from ..layout import held_answers

    def placed(asked: list) -> Any:
        found = held_answers(call.document, {"where": asked}, call.unlocked)
        if found is None:
            return None
        reflow = found["reflow"]
        stopped = reflow.get("stopped")
        return found["where"], {"page_count": reflow["pages"],
                                "pages_known": reflow["pages"] if reflow["complete"] else None,
                                "stopped": None if stopped is None else {
                                    "page": stopped["page"], "reason": stopped["reason"], "at": stopped["at"]}}

    return placed


TOOLS = [word_read, word_inspect]
