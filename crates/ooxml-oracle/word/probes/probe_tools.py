"""The probes' inputs made through the tools (the tool host docx-agent serves, the Rust core
behind it), not a document API: a new blank document, and a document after a list of tool
calls.

Must not: import pytest or docx-agent's library beyond its tool host (``docx_agent.tools``).
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import probe_paths  # noqa: E402,F401  (docx-agent's test helpers on the path)
import word_docs  # noqa: E402

_BOX: Any = None


def box() -> Any:
    """One tool host for the probe's process, no workers."""
    global _BOX
    if _BOX is None:
        from docx_agent.tools import FORMAT, GROUPS, TOOLS
        from ooxml_edit.tools import Toolbox

        _BOX = Toolbox(list(TOOLS), formats=[FORMAT], groups=GROUPS, workers=0)
    return _BOX


def new(created: str = "2026-10-04T00:00:00Z", *, language: str = "en-US",
        separators: tuple[str, str] = (".", ",")) -> bytes:
    """A new blank document, as ``new_document`` makes one."""
    import ooxml_core

    return bytes(ooxml_core.new_document(page=("A4", False), language=language, separators=separators,
                                         properties=(None, None, created)))


class Session:
    """A document open as ``d1`` in a tool session: calls on it, its ids, its bytes."""

    def __init__(self, data: bytes, name: str = "probe.docx") -> None:
        self.box = box()
        self.session = self.box.session()
        self.session.open(data, name)

    def call(self, tool: str, **arguments: Any) -> Any:
        result = self.box.dispatch(self.session, tool, {"doc": "d1", **arguments})
        if not result.ok:
            raise RuntimeError(f"{tool}: {result.to_json()}")
        return result

    def paragraph_ids(self) -> list[str]:
        return word_docs.paragraph_ids(self.box, self.session, "d1")

    def blocks(self) -> list[tuple[str, str]]:
        return word_docs.blocks(self.box, self.session, "d1")

    def data(self) -> bytes:
        return word_docs.saved(self.session, "d1")


def edited(data: bytes, calls: list[tuple[str, dict]], *, name: str = "probe.docx") -> bytes:
    """``data`` after each tool call of ``calls`` (``(tool, arguments)``, on ``d1``)."""
    session = Session(data, name)
    for tool, arguments in calls:
        session.call(tool, **arguments)
    return session.data()
