"""What a call reads of its document's layouts made already (moved out of
:mod:`docx_agent.tools._base`, lane P2c5): the layout a change's reflow is measured from
(:func:`seen_before`), the bytes of the call's version when the session
has them (:func:`state_bytes`), a check's facts made where the layouts are held
(:func:`held_facts`) and the layout of the current state when it was made
(:func:`cached_layout`).

Must not: lay a document out on its own account (only :func:`cached_layout` with ``fetch``
fetches one that was made), or change a document.
"""

from __future__ import annotations

from typing import Any

from ..layout import cache_key, known_summary, layout_key, layout_options, summary_for


def seen_before(document: Any, data: bytes | None = None) -> Any:
    """What a changing call's reflow is measured from: the summary of the layout of the
    document's current state when it was laid out already -- here, or (round 38) in the
    worker that made a check's facts -- else ``None``.  ``data``: the state's bytes, when
    the caller has them (:func:`state_bytes`)."""
    if not getattr(document, "_layout_summaries", None) and not getattr(document, "_layouts", None):
        # Lane G: nothing laid out for the document (its checks answered elsewhere): no
        # layout of its current state either, without writing its bytes out to look.
        return None
    options = layout_options(document, {})
    data = document.to_bytes() if data is None else data
    key = layout_key(data, options)
    found = known_summary(document, key)
    if found is not None:
        return found
    conversion = document._layouts.get(cache_key(data, options))
    return summary_for(document, key, conversion.layout) if conversion is not None else None


def state_bytes(call: Any) -> bytes | None:
    """The bytes of the call's document as it is now, when the session has them already
    (round 45): its version's snapshot (``DocumentEntry.snapshot``, kept when the change
    before ended), as long as nothing changed the document since -- outside a batch, or
    before any op of the batch changed it.  ``None``: serialize it (``to_bytes``)."""
    entry = call.entry
    kept = getattr(entry, "_snapshot", None)
    if kept is None or kept[0] != entry.version or entry.doc_id in call.context.changed_at:
        return None
    # (0.117.0: a committed version's bytes are made when asked; not made here for this.)
    return kept[1] if isinstance(kept[1], (bytes, bytearray)) else None


def held_facts(call: Any, request: dict, *, unlocked: Any = None, timed: Any = None) -> Any:
    """:func:`docx_agent.layout.check_facts` of the call's document, ``(facts, summary)``,
    made in the worker holding its layouts -- with the call's lock let go where it may (a
    read outside a batch, as before), and (round 59, R134) holding it -- a batch's read,
    which never waits for another's layout in progress -- where a toolbox's workers hold
    them (:func:`~docx_agent.layout.runner_of`).  ``None``: lay it out here, as before (a
    batch's read of a document no worker lays out; a converter that cannot)."""
    from ..layout import check_facts, runner_of

    if not call.may_unlock and runner_of(call.document) is None:
        return None
    return check_facts(call.document, unlocked if unlocked is not None else call.unlocked, request, timed=timed)


def cached_layout(document: Any, *, fetch: bool = True) -> Any:
    """The layout of the document's current state if it was laid out already, else ``None``.
    Round 38: laid out in the worker that made a check's facts, it is fetched from there
    (nothing laid out again) -- ``fetch`` false: only whether it was (``True``)."""
    options = layout_options(document, {})
    data = document.to_bytes()
    conversion = document._layouts.get(cache_key(data, options))
    if conversion is not None:
        return conversion.layout
    if known_summary(document, layout_key(data, options)) is None:
        return None
    return document.layout() if fetch else True


__all__ = ["cached_layout", "held_facts", "seen_before", "state_bytes"]
