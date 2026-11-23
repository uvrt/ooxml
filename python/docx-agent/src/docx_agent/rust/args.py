"""Arguments every Word read reads alike: ``range`` as a list of addresses and ``pages`` (or
``slides`` read as pages)."""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import ToolError


def ranges_of(value: Any) -> list[str]:
    """``range`` as a list of addresses: one string is one range (rule 1: a list's one
    item, said either way); ``[]``, ``""`` and blanks are no range (a strict decoder's
    placeholders)."""
    if value is None:
        return []
    items = [value] if isinstance(value, str) else list(value)
    out: list[str] = []
    for item in items:
        if isinstance(item, str) and item.strip() and item.strip() not in out:
            out.append(item.strip())
    return out


def pages_of(call: Any, slides: list | None, pages: list[int] | None) -> list[int] | None:
    """``pages``, or ``slides`` read as pages: a model that renders decks too often names a
    document's pages ``slides``.  Given both, ``pages`` is the document's and ``slides`` is
    ignored.  The result's warnings say which."""
    if slides is None:
        return pages
    if pages is not None:
        call.warn("slides ignored: a document has pages")
        return pages
    call.warn("slides read as pages: a document has pages")
    out = []
    for index, item in enumerate(slides):
        # A deck's slides may be ids (s:256); a document's pages are numbers only.
        if isinstance(item, str) and not item.strip().isdigit():
            raise ToolError("invalid_arguments", f"slides[{index}] is {item!r}: a document has pages, "
                            "numbered from 1 (pages: [1])", field="pages")
        out.append(int(item))
    return out
