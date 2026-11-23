"""The Rust session's own page engines: each Word document the session holds is laid out by
an engine the session keeps (``ooxml_core.Session.pages``), made from what it is set to lay
the document out with (``ooxml_core.PageSetup``: the fonts, the locale, the memory budget).

Whenever a call asks for a document's engine (:func:`session_pages`) the document is set to
the font folders and the locale its entry is drawn in (``Session.set_pages``) -- the
session's (``Toolbox(font_dirs=..., locale=...)``), ``OOXML_FONT_DIRS`` and en-US where none
-- the setting made again only where they changed since.  From then on the session lays it
out with them whenever a call needs its pages.
"""

from __future__ import annotations

import os
from typing import Any


def layout_memory() -> int:
    """What the process's engines may keep, in bytes: ``DOCX_AGENT_LAYOUT_MEMORY`` (MB)
    where it is set to a positive number, else 256 MB."""
    try:
        value = float(os.environ.get("DOCX_AGENT_LAYOUT_MEMORY", ""))
    except ValueError:
        value = 0.0
    return int(value * 2 ** 20) if value > 0 else 256 * 2 ** 20


def font_dirs_of(entry: Any) -> list[str]:
    """The font folders ``entry``'s document lays out and renders with: the session's, else
    ``OOXML_FONT_DIRS`` (:func:`ooxml_common.fonts.office.user_font_dirs`), as strings."""
    from ooxml_common.fonts.office import user_font_dirs

    given = getattr(entry, "font_dirs", None)
    return [str(path) for path in user_font_dirs(list(given) if given else None)]


def locale_of(entry: Any) -> Any:
    """The regional settings ``entry``'s document is drawn and edited in: the session's, else
    en-US -- never the operating system's."""
    locale = getattr(entry, "locale", None)
    if locale is None:
        from ooxml_common.locales import EN_US

        return EN_US
    return locale


def session_pages(entry: Any) -> Any:
    """The page engine the session lays ``entry``'s document out with (an
    ``ooxml_core.WordPages``: the session's own), set to the entry's font folders and
    locale."""
    import docx2svg
    from docx2svg import rust

    from ooxml_edit.tools import ToolError

    mirror = entry.handle
    options = docx2svg.ConvertOptions(font_dirs=font_dirs_of(entry), locale=locale_of(entry))
    rust.memory_budget(layout_memory())     # (the process's budget, made with docx-agent's limit first)
    key = rust.engine_key(options)
    with mirror.lock:
        if mirror.pages_key != key:
            mirror.session.set_pages(mirror.doc, rust.page_setup(options))
            mirror.pages_key = key
    found = mirror.session.pages(mirror.doc)
    if found is None:
        raise ToolError("internal", f"the session made no page engine for {entry.doc_id}")
    return found


def existing_pages(entry: Any) -> Any:
    """The document's page engine where a call asked for it before (an edit's checks measure
    the reflow with an engine the document has, never one made for them), else ``None``."""
    if entry.handle.pages_key is None:
        return None
    return session_pages(entry)
