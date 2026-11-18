"""A document's pages laid out in Rust (phase PL, lane L4): docx2svg's layout without its
Python objects.

Every layout docx-agent makes of a version -- a check's, ``describe``'s pages, a render's, a
fields update's, the live view's -- is first asked of the document's Rust engine
(:class:`docx2svg.rust.Engine`, one per document lineage and layout options in each process
that lays it out): it lays the version out from the last one it laid out, with the
interpreter lock let go, and holds it in Rust (:class:`docx2svg.rust.RustLayout`). Nothing is
pickled, and nothing of the layout is a Python object but the pages a reader reads.  Where
the Rust layout does not lay a version out as docx2svg does yet, :func:`lay_out` answers
``None`` and the caller lays it out as before (docx2svg's Python layout, frozen as that
fallback): counted by reason, :func:`fallbacks` (``layout_fallbacks{reason}``).

The engines are kept per lineage, the most recently used :data:`ENGINES` of them; a
document closed or collected drops its own (:func:`forget`, from
:func:`docx_agent.layout.forget_lineage`).

**Memory (lane L4b).**  One engine per document per process, and only in the process that
owns the document: a worker process makes none (``make=False`` there,
:func:`docx_agent.layout.held_layout`), so the layouts a worker still makes are docx2svg's
Python fallbacks alone.  The live view's versions and a check's keep trials are laid out
by the document's own engine.  Every engine counts in the process's memory budget
(:func:`budget`, ``docx2svg.rust.memory_budget``, sized by
:data:`docx_agent.layout.LAYOUT_MEMORY`): its memos as held, its pages' paint within the
limit; a document's kept layouts reserve their weight from it too (:func:`account`).

**Finalizers.**  A document collected lets its engines go through :mod:`docx_agent.finalizers`:
its finalizer only hands :func:`forget` over, done (:func:`~docx_agent.finalizers.drain`) at
the start of the next :func:`lay_out`, :func:`known`, :func:`forget` or :func:`clear`, before
:data:`_LOCK` is taken -- never inside it, where the collector may fire.
"""

from __future__ import annotations

import threading
from collections import OrderedDict
from typing import Any

from . import finalizers

#: How many documents' engines one process keeps (the least recently used let go first).
ENGINES = 16

_ENGINES: "OrderedDict[tuple, Any]" = OrderedDict()
_LOCK = threading.Lock()


def available() -> bool:
    """Whether ``ooxml_core`` has the Rust layout."""
    from docx2svg import rust

    return rust.available()


def budget() -> Any:
    """The process's ``ooxml_core.MemoryBudget`` (``docx2svg.rust.memory_budget``), made with
    :data:`docx_agent.layout.LAYOUT_MEMORY` bytes."""
    from docx2svg import rust

    from .layout import LAYOUT_MEMORY

    return rust.memory_budget(LAYOUT_MEMORY)


def account() -> Any:
    """A new account of :func:`budget` (``ooxml_core.BudgetAccount``): what one document's
    kept layouts reserve."""
    return budget().account()


def memory() -> dict[str, int]:
    """The process's budget now: ``limit``, ``used`` (pages' paint and kept layouts) and
    ``held`` (the engines' memos), bytes."""
    found = budget()
    return {"limit": found.limit, "used": found.used, "held": found.held}


def _engine(lineage: str, convert_options: Any, make: bool = True) -> Any:
    from docx2svg import rust

    # Only the table is read under the lock: whether an engine fits the options is Python
    # work (the fonts' folders), which allocates -- and may run the collector -- so it is
    # asked outside.
    with _LOCK:
        mine = [(key, engine) for key, engine in _ENGINES.items() if key[0] == lineage]
    for key, engine in mine:
        if engine.fits(convert_options):
            with _LOCK:
                if key in _ENGINES:
                    _ENGINES.move_to_end(key)
            return engine
    if not make:
        return None
    budget()
    made = rust.Engine(convert_options)
    with _LOCK:
        found = _ENGINES.setdefault((lineage, made.key), made)
        _ENGINES.move_to_end((lineage, made.key))
        while len(_ENGINES) > ENGINES:
            _ENGINES.popitem(last=False)
    return found


def lay_out(data: bytes, options: dict, lineage: str, *, make: bool = True) -> Any:
    """The version ``data`` laid out in Rust with ``options`` (docx2svg's
    :class:`~docx2svg.ConvertOptions` keywords), by the engine of ``lineage`` -- a
    :class:`docx2svg.rust.RustLayout` -- or ``None`` where docx2svg's Python layout lays it
    out instead (counted).  ``make`` false: only by an engine this process has already (a
    worker process has none: ``None``, not counted)."""
    finalizers.drain()
    if not available():
        return None
    import docx2svg

    convert_options = docx2svg.ConvertOptions(**options)
    from docx2svg import rust

    why = rust.unsupported_options(convert_options)
    if why is not None:
        rust.count_fallback(why)
        return None
    engine = _engine(lineage, convert_options, make)
    if engine is None:
        return None
    made = engine.lay_out(data, convert_options)
    if made is not None:
        made.lineage = lineage
    return made


def known(data: bytes, options: dict, lineage: str) -> Any:
    """The Rust layout of ``data`` where the engine of ``lineage`` laid it out last (the
    layout owner's flight, an earlier call), else ``None``: nothing is laid out, nothing
    counted."""
    finalizers.drain()
    if not available():
        return None
    import docx2svg

    engine = _engine(lineage, docx2svg.ConvertOptions(**options), make=False)
    return engine.known(data) if engine is not None else None


def forget(lineage: str) -> None:
    """Let go of the engines of ``lineage``: the document is closed or collected (never from
    a finalizer itself: see the module's notes)."""
    finalizers.drain()
    with _LOCK:
        for key in [k for k in _ENGINES if k[0] == lineage]:
            del _ENGINES[key]


def clear() -> None:
    """Let go of every engine."""
    finalizers.drain()
    with _LOCK:
        _ENGINES.clear()


def fallbacks() -> dict[str, int]:
    """``layout_fallbacks{reason}`` of this process (:func:`docx2svg.rust.fallbacks`)."""
    from docx2svg import rust

    return rust.fallbacks()


def is_rust(layout: Any) -> bool:
    """Whether ``layout`` (docx2svg's, as a :class:`~docx_agent.layout.DocumentLayout` holds
    it) is laid out in Rust."""
    from docx2svg import rust

    return isinstance(layout, rust.RustLayout)
