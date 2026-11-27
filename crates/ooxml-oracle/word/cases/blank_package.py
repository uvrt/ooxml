"""A new blank Word document's package as the Rust core makes it (``new_document``'s bytes,
before a session opens them): the made-up documents the tests and the gates start from.  The
oracle's harness is the internal layer, which may read the core (``ooxml_core`` is internal,
decision D6); docx-agent's tests reach it through ``word_docs.blank``."""

from __future__ import annotations

from datetime import datetime, timezone


def blank(*, page: str = "A4", landscape: bool = False, language: str = "en-US",
          separators: tuple[str, str] = (".", ","), title: str | None = None, author: str | None = None,
          created: datetime) -> bytes:
    """The package: its page, its text's language, its regional separators (decimal symbol
    and list separator), its title, author and creation date."""
    import ooxml_core

    stamp = created.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return bytes(ooxml_core.new_document(page=(page, landscape), language=language, separators=separators,
                                         properties=(title, author, stamp)))
