"""The Rust session's own page engines (lane H7b): each Word document the session holds is
laid out by an engine the session keeps (``ooxml_core.Session.pages``), made from what it is
set to lay the document out with (``ooxml_core.PageSetup``: the fonts, the locale, the memory
budget).

Whenever a call asks for a document's engine (:func:`session_pages`) the document is set to
the font folders and the locale it is drawn in (``Session.set_pages``): its own, as
:func:`docx_agent.layout.layout_options` resolves them -- a document given other folders, or
another language, is laid out with those --, the setting made again only where they changed
since.  From then on the session lays it out with them whenever a call needs its pages, a
call's own layout given or not.

The Rust layout is docx-agent's to switch (:func:`docx_agent.pages.available`): where it is
off, no document gets an engine here, and the calls that read the pages are answered as
without one.
"""

from __future__ import annotations

from typing import Any


def session_pages(mirror: Any, document: Any, *, make: bool = True) -> tuple[Any, str | None]:
    """The page engine the session lays ``mirror``'s document out with (an
    ``ooxml_core.WordPages``: the session's own), set to lay it out with the font folders and
    the locale of ``document`` (the view the toolbox holds of it); ``(None, why)`` where the
    Rust layout is off or does not lay out with them.  ``make`` false: only where a call has
    asked for the document's engine before (lane P5m: an edit's checks measure the reflow
    with an engine the document has, never one made for them)."""
    import docx2svg
    from docx2svg import rust

    from .. import pages
    from ..layout import layout_options

    if not pages.available():
        return None, "no Rust layout"
    options = docx2svg.ConvertOptions(**layout_options(document, {}))
    why = rust.unsupported_options(options)
    if why is not None:
        return None, why
    pages.budget()      # (the process's budget, made with docx-agent's limit before an engine counts in it)
    key = rust.engine_key(options)
    with mirror.lock:
        if not make and getattr(mirror, "pages_key", None) is None:
            return None, "no engine laid the document out"
        if getattr(mirror, "pages_key", None) != key:
            mirror.session.set_pages(mirror.doc, rust.page_setup(options))
            mirror.pages_key = key
    found = mirror.session.pages(mirror.doc)
    return (found, None) if found is not None else (None, "no engine in the session")
