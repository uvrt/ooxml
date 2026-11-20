"""A session whose documents the Python handlers answer, for tests comparing a served
call's reply with docx-agent's own (lane P2e1: a session's own clock no longer keeps a call
from the Rust session, which dates writes by it too)."""

from __future__ import annotations

import os
from typing import Any

ENV = "DOCX_AGENT_RUST"


def python_session(toolbox: Any, data: bytes, name: str, **options: Any) -> tuple[Any, str]:
    """A new session of ``toolbox`` (``options``: ``Toolbox.session``'s) holding ``data`` as
    ``name``, the document opened with the Rust session off: no owner serves its calls."""
    was = os.environ.get(ENV)
    os.environ[ENV] = "0"
    try:
        session = toolbox.session(**options)
        return session, session.open(data, name)
    finally:
        if was is None:
            del os.environ[ENV]
        else:
            os.environ[ENV] = was
