"""docx2svg's settings for the layout in Rust (``ooxml_core``): what a document is laid out
and drawn with for a set of :class:`~docx2svg.ConvertOptions`.

:func:`word_pages` makes the ``ooxml_core.WordPages`` engine a conversion lays one document
out with; :func:`page_setup` the ``ooxml_core.PageSetup`` a session's engines are made from
(docx-agent).  Both lay out with the machine's fonts as Word finds them (:func:`word_fonts`,
scanned once per set of folders), the locale, the decimal tab and the glyph size, counted in
the process's :func:`memory_budget`; :func:`engine_key` says which options lay out alike.

Nothing here lays a document out or draws it: ``ooxml_core`` does, and what it cannot lay out
yet it refuses (with why).
"""

from __future__ import annotations

import sys
import threading
from pathlib import Path
from typing import Any, Sequence

from ooxml_common.fonts import office
from ooxml_common.locales import EN_US


def available() -> bool:
    """Whether ``ooxml_core`` has the Rust layout."""
    try:
        ooxml_core = core()
    except ImportError:
        return False
    return hasattr(ooxml_core, "WordPages")


def core() -> Any:
    """``ooxml_core``, the Rust core's module."""
    import importlib

    return importlib.import_module("ooxml_core")


# -- fonts ---------------------------------------------------------------------------------


def font_dirs(options) -> tuple[Path, ...]:
    """The folders a layout with ``options`` finds faces in, in Word's order: Word's own
    (macOS: the system's, Word's bundle, Office's cloud fonts; elsewhere the system's), then
    the application's (``options.font_dirs``, else ``OOXML_FONT_DIRS``) with the folders under
    them."""
    word = office.font_dirs(office.WORD) if sys.platform == "darwin" else office.system_font_dirs()
    return tuple(word) + office.with_subfolders(office.user_font_dirs(options.font_dirs))


_FONTS: dict = {}
_FONTS_LOCK = threading.Lock()


def _fonts_key(options) -> tuple:
    """``(folders, substitutes, substitute)``: the folders as ``(location, path)`` pairs, the
    substitutions (Word's face lowercased -> the face used; Word's measured ones only where
    ``options.substitute_fonts``), and whether absent faces are substituted at all."""
    folders = tuple(("bundle" if Path(d) == office.WORD_FONTS else "folder", str(d)) for d in font_dirs(options))
    substitute = bool(options.substitute_fonts)
    substitutes = dict(core().WordFonts.default_substitutes()) if substitute else {}
    substitutes.update({name.lower(): value for name, value in (options.font_substitutes or {}).items()})
    return folders, tuple(substitutes.items()), substitute


def word_fonts(options) -> Any:
    """The machine's fonts as ``options`` find them (an ``ooxml_core.WordFonts``: Word's
    folders, then the application's), scanned once per set of folders and substitutes."""
    key = _fonts_key(options)
    with _FONTS_LOCK:
        found = _FONTS.get(key)
    if found is not None:
        return found
    folders, substitutes, substitute = key
    made = core().WordFonts(list(folders), substitutes=list(substitutes), recorded_faces=substitute,
                            fallbacks=substitute)
    with _FONTS_LOCK:
        return _FONTS.setdefault(key, made)


#: The options the Rust layout does not take, each with why.
_REFUSED = {
    "advances": "the layout measures with the font files themselves",
    "metrics": "the layout measures with the font files themselves",
    "decorations": "the layout measures with the font files themselves",
    "names": "the SVG names each face as its font file does",
}


def unsupported_options(options) -> str | None:
    """Why the Rust layout does not lay out with ``options`` -- ``ConvertOptions.advances``,
    ``metrics``, ``decorations`` and ``names`` are no longer taken -- with the way out; or
    ``None``."""
    given = [name for name in _REFUSED if getattr(options, name, None) is not None]
    if not given:
        return None
    return (f"ConvertOptions.{', '.join(given)}: no longer taken ({_REFUSED[given[0]]}). Leave "
            f"{'it' if len(given) == 1 else 'them'} None and name the faces instead: font_dirs (folders to "
            "find them in) or font_substitutes (a face to use for an absent one)")


_RASTERS: dict = {}


def page_raster(files: Sequence[str] = (), dirs: Sequence[str] = (), *, skip_system_fonts: bool = False,
                sans_serif_family: str | None = None) -> Any:
    """An ``ooxml_core.PageRaster`` handed ``files``, the files under ``dirs`` and the
    system's fonts unless ``skip_system_fonts`` -- read once per process for each set (as a
    session's raster is): a face installed later is seen after a restart."""
    key = (tuple(files), tuple(dirs), skip_system_fonts, sans_serif_family)
    with _FONTS_LOCK:
        found = _RASTERS.get(key)
    if found is not None:
        return found
    made = core().PageRaster(list(files), list(dirs), skip_system_fonts=skip_system_fonts,
                             sans_serif_family=sans_serif_family)
    with _FONTS_LOCK:
        return _RASTERS.setdefault(key, made)


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
            _BUDGET.append(core().MemoryBudget(int(limit) if limit else DEFAULT_MEMORY))
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


def _engine_args(options) -> tuple[Any, dict]:
    """The fonts and the keywords ``WordPages`` and ``PageSetup`` take for ``options``."""
    why = unsupported_options(options)
    if why is not None:
        raise ValueError(why)
    _, decimal, glyph_size, language, thousands, date_order, list_separator = _engine_key(options)
    return word_fonts(options), {"locale": language, "decimal": decimal, "glyph_size": glyph_size,
                                 "numbers": (thousands, date_order, list_separator),
                                 "budget": memory_budget()}


def page_setup(options, *, raster: Any = None) -> Any:
    """What a Rust session lays its documents out with for ``options`` (an
    ``ooxml_core.PageSetup``: the fonts as :func:`word_fonts` finds them, the engines'
    locale, decimal tab and glyph size, counted in :func:`memory_budget`) -- each document's
    engine then the session's own (``ooxml_core.Session(pages=...)``, ``Session.set_pages``);
    ``raster`` (a :func:`page_raster`) the faces a host's ``render`` draws its pages with.

    Raises :class:`ValueError` for options the Rust layout does not take
    (:func:`unsupported_options`)."""
    fonts, keywords = _engine_args(options)
    return core().PageSetup(fonts, **keywords, raster=raster)


def word_pages(options) -> Any:
    """An engine laying one document out with ``options`` (an ``ooxml_core.WordPages``, made
    as :func:`page_setup`'s engines are): what a conversion lays its document out with.

    Raises :class:`ValueError` for options the Rust layout does not take
    (:func:`unsupported_options`)."""
    fonts, keywords = _engine_args(options)
    return core().WordPages(fonts, **keywords)


__all__: Sequence[str] = ("DEFAULT_MEMORY", "available", "core", "engine_key", "font_dirs", "memory_budget",
                          "page_raster", "page_setup", "unsupported_options", "word_fonts", "word_pages")
