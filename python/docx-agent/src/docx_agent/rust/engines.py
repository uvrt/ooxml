"""The Rust session's own page engines (lane H7b): each Word document the session holds is
laid out by an engine the session keeps (``ooxml_core.Session.pages``), made from what it is
set to lay the document out with (``ooxml_core.PageSetup``: the fonts, the locale, the memory
budget).

The first time a call asks for a document's engine (:func:`session_pages`) the document is
set to its toolbox's font folders and locale (``Session.set_pages``; the folders
``OOXML_FONT_DIRS`` and en-US where the toolbox names none); from then on the session lays it
out with them whenever a call needs its pages, a call's own layout given or not.  Nothing here
reads the Python document: the folders and the locale are the toolbox session's
(``Session.font_dirs``, ``Session.locale``).
"""

from __future__ import annotations

from typing import Any


def _options(font_dirs: Any, locale: Any) -> Any:
    """docx2svg's options for laying out with ``font_dirs`` (``None``: the default folders)
    in ``locale`` (``None``: en-US), as :func:`docx_agent.layout.layout_options` gives them."""
    import docx2svg
    from ooxml_common.fonts.office import user_font_dirs
    from ooxml_common.locales import EN_US

    return docx2svg.ConvertOptions(font_dirs=[str(path) for path in user_font_dirs(font_dirs)],
                                   locale=locale if locale is not None else EN_US)


def session_pages(mirror: Any, toolbox: Any) -> tuple[Any, str | None]:
    """The page engine the session lays ``mirror``'s document out with (an
    ``ooxml_core.WordPages``: the session's own), laid out with ``toolbox``'s font folders and
    locale (an :class:`ooxml_edit.tools.Session`'s ``font_dirs`` and ``locale``); ``(None,
    why)`` where the Rust layout does not lay out with them."""
    from docx2svg import rust

    if not rust.available():
        return None, "no Rust layout"
    options = _options(getattr(toolbox, "font_dirs", None), getattr(toolbox, "locale", None))
    why = rust.unsupported_options(options)
    if why is not None:
        return None, why
    from ..pages import budget

    budget()        # (the process's budget, made with docx-agent's limit before an engine counts in it)
    key = rust.engine_key(options)
    with mirror.lock:
        if getattr(mirror, "pages_key", None) != key:
            mirror.session.set_pages(mirror.doc, rust.page_setup(options))
            mirror.pages_key = key
    found = mirror.session.pages(mirror.doc)
    return (found, None) if found is not None else (None, "no engine in the session")
