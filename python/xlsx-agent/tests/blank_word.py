"""A new blank Word document's bytes for the tests that put a workbook beside one, made by
docx-agent's ``new_document`` through the Toolbox (the Rust core is internal: decision D6)."""

from __future__ import annotations

import datetime as dt


def blank_word() -> bytes:
    """A blank A4 document, en-US, made on 2026-11-23 at 09:00 UTC."""
    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    moment = dt.datetime(2026, 11, 23, 9, 0, tzinfo=dt.timezone.utc)
    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
        session = box.session(clock=lambda: moment)
        made = box.dispatch(session, "new_document", {"kind": "docx", "size": "A4"})
        assert made.ok, made.to_json()
        return session.save(made.created[0]).data
