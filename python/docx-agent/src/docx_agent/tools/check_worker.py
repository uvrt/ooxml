"""``check``'s layout facts made where the layout is held (round 38).

A check's facts about the layout -- the page count, the coverage, each range's pages and
extent, the layout facts of the pages asked for -- were read in the caller's process from a
layout pickled to it from the worker that made it, and mapped to ids there: about 0.4 s to
read the pickle and as much again to map it, for every check, in the one interpreter every
agent's calls share.  :func:`check_bytes` runs in the worker holding the document's layout
(:mod:`docx_agent.layout`, "layouts held where they were made") and sends back only the
facts: it maps the layout to ids on a copy of the document opened from the very bytes laid
out (ids are the bytes' own: ``w14:paraId``, or a paragraph's place), so the facts are the
ones the caller would have read, for the version it checked.  The caller keeps what stays
with the document: the problems (``validate``), the fields, the comparison with the last
check (from a small :class:`~docx_agent.layout.LayoutSummary`, its ids followed through the
document's aliases there).

Lane L4b: where the document's version is laid out in Rust, its facts are made in the
caller's process instead (:func:`docx_agent.layout.check_facts`), by :func:`facts_of` on the
layout mapped to the caller's own copy of the version -- no worker holds a Rust layout.  This
task is left for the versions docx2svg's Python layout lays out (the Rust layout's fallbacks).
"""

from __future__ import annotations

import time
from typing import Any


from ..layout import (LayoutSummary, _put_held, extents_of, held_layout, made_facts, opened_layout,
                      page_infos_of, placements_of)


def check_bytes(data: bytes, options: dict, state: tuple, request: dict) -> dict:
    """In a worker: the layout of ``data`` -- the one held, or laid out from the one found
    (:func:`docx_agent.layout.held_layout`) -- and the facts ``request`` asks of it:

    * always ``reflow``: ``pages``, ``complete``, ``stopped``, ``coverage`` (as ``check``
      reports them) and ``warnings`` (``layout_warnings``' text); ``stats``, the pages and
      body lines ``app.xml`` is saved with;
    * ``summary`` (asked): the layout's :class:`~docx_agent.layout.LayoutSummary`;
    * ``ranges`` (each range's address and paragraph ids, :meth:`docx_agent.scope.Scope.spec`):
      ``range`` (each as reported, with its extent) and ``on`` (every page they are on);
      with ``keeps`` (round 40), each range's keep tried (:mod:`.keep_trials`);
    * ``layout`` (asked): the layout facts of ``pages`` -- the ranges' pages among them
      when ranges are given -- with ``min_size`` and ``styles``;
    * ``pages_of`` (round 57, R124: ids): ``{id: [pages]}`` for those drawn;
    * ``where`` (round 59, R134: lists of ids): each list's placements
      (:func:`~docx_agent.layout.placements_of`), and ``extents`` (``{name: ids}``): each
      group's :meth:`~docx_agent.layout.DocumentLayout.extent`;
    * ``page_infos`` (round 59, R135: ids): ``{id: page}`` of those drawn, the page each
      starts on as a page reference reads it (:func:`~docx_agent.layout.page_infos_of`).

    Plus how the layout was found or made (:func:`docx_agent.layout.made_facts`), and
    (round 47, R82) ``worker``: the seconds this task spent laying out (``layout``) and
    reading the facts (``facts``) -- the caller's ``timing`` counts the rest of its wait
    for the task as waiting for the worker."""
    held, how, seconds = held_layout(data, options, state)
    try:
        out = made_facts(held, how, seconds)
        began = time.perf_counter()
        out.update(_facts(held, data, options, request))
        out["worker"] = {"layout": seconds, "facts": time.perf_counter() - began}
    finally:
        _put_held(state[1], held, state[3])    # a layout of these bytes either way
    return out


# A task ooxml-edit's subprocess workers may run (the mark, as the layout's tasks set it).
check_bytes.__ooxml_edit_task__ = True  # type: ignore[attr-defined]


def _facts(held: Any, data: bytes, options: dict, request: dict) -> dict:
    def summary() -> LayoutSummary:
        made = held.extra.get("summary")
        if made is None:
            made = held.extra["summary"] = LayoutSummary(opened_layout(held, data, options),
                                                         held.extra.get("summary before"), keep=True)
        return made

    return facts_of(opened_layout(held, data, options), held.laid[0], data, options, request, summary)


def facts_of(layout: Any, raw: Any, data: bytes, options: dict, request: dict, summary: Any) -> dict:
    """The facts ``request`` asks (:func:`check_bytes`' list) of ``layout`` -- a
    :class:`~docx_agent.layout.DocumentLayout` of ``data`` laid out with ``options``, ``raw``
    its docx2svg (or Rust) layout -- ``summary()`` giving its summary where asked: in a
    worker (:func:`check_bytes`) or, for a version laid out in Rust, in the caller."""
    from ..layout_facts import layout_facts
    from ..scope import Scope
    from ._base import short
    from .scoped import range_pages

    reflow: dict[str, Any] = {"pages": layout.page_count, "complete": layout.pages_known is not None}
    if layout.stopped is not None:
        reflow["stopped"] = {"page": layout.stopped.page, "reason": layout.stopped.reason,
                             "at": layout.stopped.at}
    reflow["coverage"] = layout.coverage_facts()
    out: dict[str, Any] = {
        "reflow": reflow,
        "warnings": [short(str(w), 160) for w in layout.warnings[:10]] if layout.warnings else None,
        "stats": ({"pages": layout.pages_known,
                   "lines": sum(len(layout._body(index)) for index in range(layout.page_count))}
                  if layout.pages_known is not None else {})}
    if request.get("summary"):
        out["summary"] = summary()
    pages = request.get("pages")
    scoped_pages = pages
    scope = None
    if request.get("ranges"):
        scope = Scope.of_ids(layout._document, request["ranges"])
        out["range"], on = range_pages(scope, layout, extent=True)
        out["on"] = on
        if request.get("keeps"):
            # Round 40: what each range's keep would do, laid out from this very layout.
            from .keep_trials import keep_effects

            caps = request["keeps"]
            keep_effects(raw, data, options, layout, scope, out["range"],
                         trials=caps["trials"], seconds=caps["seconds"])
        scoped_pages = on if pages is None else [p for p in on if p in pages]
    if request.get("pages_of") is not None:
        # Round 57 (R124): the pages of some blocks -- a check of numbers says each text's
        # first page -- read here, so the layout never goes to the caller for them.
        out["pages_of"] = {identifier: pages for identifier in request["pages_of"]
                           for pages in [layout.pages_of(identifier)] if pages}
    if request.get("where") is not None:
        # Round 59 (R134): where blocks are placed (word_inspect's layout, a table kept
        # together), read here: the layout never goes to the caller for them.
        out["where"] = placements_of(layout, request["where"])
    if request.get("extents") is not None:
        # Round 59 (R134): how much of each page blocks take (a span kept together).
        out["extents"] = extents_of(layout, request["extents"])
    if request.get("page_infos") is not None:
        # Round 59 (R135): the page each block starts on, as a page reference to it reads it.
        out["page_infos"] = page_infos_of(layout, request["page_infos"])
    if request.get("layout"):
        out["layout"] = layout_facts(layout, pages=scoped_pages, min_size=request.get("min_size"),
                                     styles=request.get("styles"), scope=scope)
    return out
