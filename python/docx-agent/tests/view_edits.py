"""Edits a test makes to an open document with docx-agent's library, outside a tool call
(lane P2e6).

A document the Rust session owns is the session's: the Python view
(``session.entry(doc).document``) is read from its head, and what a library call changes
in the view reaches the session only as a step committed through it.  A test that sets the
document up that way (a style changed, a paragraph inserted, a control added) does it in
:func:`edited`, which commits it as one library step, so the calls that follow -- served by
the session or not -- see it.  A document no session owns is edited in place, as before.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any


@contextmanager
def edited(session: Any, doc: str = "d1") -> Iterator[Any]:
    """The view of ``session``'s document ``doc`` to edit with the library: what the block
    changes is one step of the document when it ends."""
    entry = session.entry(doc)
    with entry.batch():
        yield entry.document
