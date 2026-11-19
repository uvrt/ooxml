"""The Python view of a Word document the Rust session owns (lane P2c4): a docx-agent
``Document`` made current from the session's head when Python reads it
(:class:`ooxml_edit.tools.owned.OwnedEntry`).

:func:`follow` brings a view to a later version of the session: the parts that differ, put in
as the session wrote them (each parsed when Python first reads it), the ids the session gave
since (never given again here) and its renames (followed here), and the version's number.  No
part is read to find ids: the session says them.

Must not: make an undo step of its own (the steps are the session's), or read a part the
version did not change.
"""

from __future__ import annotations

from typing import Any


def follow(document: Any, parts: dict[str, bytes | None], given: list[int],
           renamed: list[tuple[str, str]], number: int, last: int) -> None:
    """Bring ``document`` to the session's version ``number`` (``last``: the highest number
    given): ``parts`` changed since the version it is (``None`` removes one), ``given`` the
    id values the session gave since, ``renamed`` its renames since (old id, id now)."""
    package = document.package
    for name, data in parts.items():
        if data is None:
            if package.has_part(name):
                package.remove_part(name)
        else:
            package.replace_part(name, data)
    document.history.follow(number, last)
    # The per-change id scan read the parts as lxml wrote them: these came as bytes.
    document.__dict__.pop("_id_scan", None)
    if given:
        # (A view whose ids were never read reads them once here, as its first Python use
        # would: the session's given values join them.)
        document._remember_ids(set(given))
    if renamed:
        document._rename(dict(renamed))
    document._invalidate()


__all__ = ["follow"]
