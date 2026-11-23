"""docx2svg's settings for the layout in Rust (``ooxml_core``): what a Rust session lays its
Word documents out with for a set of :class:`~docx2svg.ConvertOptions`.

:func:`page_setup` makes the ``ooxml_core.PageSetup`` a session's engines are made from:
the machine's fonts as docx2svg finds them (:func:`word_fonts`, scanned once per set of
folders), the locale, the decimal tab and the glyph size, counted in the process's
:func:`memory_budget`; :func:`engine_key` says which options lay out alike.

Nothing here lays a document out: the session's engine does, in Rust, and what it cannot lay
out yet its tools refuse as not supported (with a way out).  docx2svg's own layout
(:func:`docx2svg.lay_out_docx`) is a separate, pure-Python renderer; neither one stands in
for the other.
"""

from __future__ import annotations

import threading
from pathlib import Path
from typing import Any, Sequence

from ooxml_common.locales import EN_US


def available() -> bool:
    """Whether ``ooxml_core`` has the Rust layout."""
    try:
        ooxml_core = _core()
    except ImportError:
        return False
    return hasattr(ooxml_core, "WordPages")


def _core() -> Any:
    """``ooxml_core``, the Rust core's module: an optional dependency of docx2svg."""
    import importlib

    return importlib.import_module("ooxml_core")


# -- fonts ---------------------------------------------------------------------------------

_FONTS: dict = {}
_FONTS_LOCK = threading.Lock()


def _fonts_key(options) -> tuple:
    from . import _font_dirs
    from .fonts import SUBSTITUTES, WORD_FONTS

    folders = tuple(("bundle" if Path(d) == WORD_FONTS else "folder", str(d)) for d in _font_dirs(options))
    substitutes = dict(SUBSTITUTES) if options.substitute_fonts else {}
    substitutes.update({name.lower(): value for name, value in (options.font_substitutes or {}).items()})
    return folders, tuple(substitutes.items()), bool(options.substitute_fonts)


def word_fonts(options) -> Any:
    """The machine's fonts as ``options`` find them (docx2svg's ``_fonts``: Word's folders,
    then the application's), scanned once per set of folders and substitutes."""
    ooxml_core = _core()

    key = _fonts_key(options)
    with _FONTS_LOCK:
        found = _FONTS.get(key)
    if found is not None:
        return found
    folders, substitutes, substitute = key
    made = ooxml_core.WordFonts(list(folders), substitutes=list(substitutes), recorded_faces=substitute,
                                fallbacks=substitute)
    with _FONTS_LOCK:
        return _FONTS.setdefault(key, made)


def unsupported_options(options) -> str | None:
    """Why the Rust layout does not lay out with ``options`` (it measures with the fonts
    alone, draws text as text, and always carries on past a block it cannot lay out), or
    ``None``."""
    if options.advances is not None or options.metrics is not None or options.decorations is not None \
            or options.names is not None:
        return "custom measurement"
    if options.text != "text":
        return "outlined text"
    from . import paginate

    if not paginate.CARRY_ON:
        return "layout stops"
    return None


# -- the memory budget (lane L4b) ------------------------------------------------------------

#: What the process's engines keep that can be made again (their pages' paint) and what a
#: host's own caches reserve (layouts kept for readers) by default, bytes.
DEFAULT_MEMORY = 256 * 2 ** 20

_BUDGET: list = []
_BUDGET_LOCK = threading.Lock()


def memory_budget(limit: int | None = None) -> Any:
    """The process's ``ooxml_core.MemoryBudget``: every engine counts in it (its memos as
    held, its pages' paint within the limit), and a host reserves what it keeps of the
    layouts from it (``memory_budget().account()``).  Made on first use with ``limit``
    bytes (default :data:`DEFAULT_MEMORY`); a later ``limit`` is ignored."""
    with _BUDGET_LOCK:
        if not _BUDGET:
            _BUDGET.append(_core().MemoryBudget(int(limit) if limit else DEFAULT_MEMORY))
        return _BUDGET[0]


# -- the engine ------------------------------------------------------------------------------


def _engine_key(options) -> tuple:
    """What an engine is made for: the fonts, the decimal separator, the glyph size, and the
    locale's language, thousands separator, date order and list separator (a chart's
    numbers and dates are written in them)."""
    locale = options.locale or EN_US
    return (_fonts_key(options), locale.decimal_separator, options.glyph_size, locale.language,
            locale.thousands_separator, locale.date_order, locale.list_separator)


def engine_key(options) -> tuple:
    """What an engine for ``options`` is made for: two options with the same key lay out
    alike (a host sets a document's engine again only where its key changed)."""
    return _engine_key(options)


def page_setup(options) -> Any:
    """What a Rust session lays its documents out with for ``options`` (an
    ``ooxml_core.PageSetup``: the fonts as :func:`word_fonts` finds them, the engines'
    locale, decimal tab and glyph size, counted in :func:`memory_budget`) -- each document's
    engine then the session's own (``ooxml_core.Session(pages=...)``, ``Session.set_pages``).

    Raises :class:`ValueError` for options the Rust layout does not take
    (:func:`unsupported_options`)."""
    why = unsupported_options(options)
    if why is not None:
        raise ValueError(f"the Rust layout does not lay out with these options: {why}")
    _, decimal, glyph_size, language, thousands, date_order, list_separator = _engine_key(options)
    return _core().PageSetup(word_fonts(options), locale=language, decimal=decimal, glyph_size=glyph_size,
                             numbers=(thousands, date_order, list_separator), budget=memory_budget())


__all__: Sequence[str] = ("DEFAULT_MEMORY", "available", "engine_key", "memory_budget", "page_setup",
                          "unsupported_options", "word_fonts")
