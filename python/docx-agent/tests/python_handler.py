"""A session whose documents the Python handlers answer, for tests comparing a served
call's reply with docx-agent's own (lane P2e1: a session's own clock no longer keeps a call
from the Rust session, which dates writes by it too)."""

from __future__ import annotations

from typing import Any

from docx_agent.rust import reads


def python_session(toolbox: Any, data: bytes, name: str, **options: Any) -> tuple[Any, str]:
    """A new session of ``toolbox`` (``options``: ``Toolbox.session``'s) holding ``data`` as
    ``name``, opened with no Rust session to own it: the Python handlers answer its calls."""
    kept = reads._session()          # (made first, so it is the one given back)
    reads.use_session(None)
    try:
        session = toolbox.session(**options)
        return session, session.open(data, name)
    finally:
        reads.use_session(kept)
