"""Word's Rust host (Phase 3, E1): the :attr:`~ooxml_edit.tools.DocumentFormat.host` of the docx
format, which :mod:`ooxml_edit.tools.hosted` makes a toolbox's Rust host with
(:func:`ooxml_edit.tools.hosted.rust_host`).

The host lays Word documents out with the font folders and locale of the toolbox (or a
session's own), its pages drawn by ``render`` with the same folders and the system's fonts;
the application's settings of the format (``docx_format``: ``min_text_size``, ``typography``)
are the host's Word format's, which every call reads.
"""

from __future__ import annotations

import json
import os
import threading
from typing import Any


class TemplateOpened(UserWarning):
    """A template (``.dotx``, ``.dotm``) was opened to edit: edits change the template
    itself; ``new_document`` with the template's blob makes a document from it."""


def layout_memory() -> int:
    """What the process's page engines may keep, in bytes: ``DOCX_AGENT_LAYOUT_MEMORY`` (MB)
    where it is set to a positive number, else 256 MB."""
    try:
        value = float(os.environ.get("DOCX_AGENT_LAYOUT_MEMORY", ""))
    except ValueError:
        value = 0.0
    return int(value * 2 ** 20) if value > 0 else 256 * 2 ** 20


class WordHosting:
    """How a toolbox's Rust host holds Word documents, with the application's settings of
    the format (``min_text_size``: points, or points by kind; ``typography``)."""

    _catalogue: "tuple[str, ...] | None" = None
    _catalogue_lock = threading.Lock()

    def __init__(self, *, min_text_size: Any = None, typography: bool = True) -> None:
        self.min_text_size = min_text_size
        self.typography = bool(typography)

    def make(self, settings: str, pages: Any) -> Any:
        """The Rust host of a toolbox: ``settings`` (JSON text) as :mod:`ooxml_edit.tools.hosted`
        writes them, its documents laid out with ``pages``."""
        from ooxml_edit.tools.hosted import rust_host

        minimum = None if self.min_text_size is None else json.dumps(self.min_text_size)
        return rust_host(settings, pages=pages, min_text_size=minimum, typography=self.typography)

    def pages(self, font_dirs: Any, locale: Any) -> Any:
        """What documents are laid out and drawn with for ``font_dirs`` (``None``:
        ``OOXML_FONT_DIRS``) and ``locale`` (``None``: en-US): docx2svg's page setup."""
        import docx2svg
        from docx2svg import rust
        from ooxml_common.fonts.office import user_font_dirs

        dirs = [str(path) for path in user_font_dirs(list(font_dirs) if font_dirs else None)]
        if locale is None:
            from ooxml_common.locales import EN_US

            locale = EN_US
        options = docx2svg.ConvertOptions(font_dirs=dirs, locale=locale)
        rust.memory_budget(layout_memory())     # (the process's budget, made with docx-agent's limit first)
        return rust.page_setup(options, raster=rust.page_raster((), dirs))

    def opened(self, session: Any, doc: str) -> None:
        """A template opened to edit warns (:class:`TemplateOpened`):
        edits change the template itself."""
        import warnings

        kind = session.document(doc)[3]
        if kind in ("dotx", "dotm"):
            warnings.warn(TemplateOpened(
                f"this file is a template (.{kind}): editing it edits the template. To make a document "
                "from it use new_document with its blob; saving as docx writes a document either way"),
                stacklevel=4)

    @staticmethod
    def refused(exc: BaseException) -> "tuple[str, str] | None":
        """An open the host refused, read: ``(why, message)``; ``None`` for anything else."""
        from ooxml_edit.tools.hosted import open_refusal

        return open_refusal(exc)

    @staticmethod
    def held_kinds() -> tuple[str, ...]:
        """The kinds whose documents the host's sessions name while the Python host holds them
        (until Phase 4 moves each to Rust)."""
        from ooxml_edit.tools.hosted import host_held_kinds

        return host_held_kinds()

    def catalogue(self) -> tuple[str, ...]:
        """The names of the host's tools (Word's and the session's own)."""
        with WordHosting._catalogue_lock:
            if WordHosting._catalogue is None:
                from ooxml_edit.tools.hosted import host_tools

                WordHosting._catalogue = host_tools()
            return WordHosting._catalogue


#: The format's host with the default settings.
HOSTING = WordHosting()
