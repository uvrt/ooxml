"""A document's layout is the Rust session's: a check lays out the version it reads with the
document's page engine (:mod:`docx_agent.rust.engines`), each version from the one before,
so no layout owner of docx-agent's waits for or makes layouts.

:func:`read_version` is the format's ``read_version``: a read is of the version at its call.
"""

from __future__ import annotations

from typing import Any


def read_version(toolbox: Any, session: Any, entry: Any, tool: Any, arguments: Any) -> Any:
    """ooxml-edit's ``DocumentFormat.read_version``: ``None`` -- the call reads the version it
    was given (the session lays that version out itself)."""
    return None


def layout_waited() -> dict[str, float]:
    """The time a check waited for a layout owner, by part: none.  (Goes with its caller,
    the Python check, in lane CUT-A2.)"""
    return {}
