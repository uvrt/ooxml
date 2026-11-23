"""A refusal of the Rust session, answered as the call's reply (lane P2e1): a refusal is
not a fallback.

The Rust tools word their refusals as docx-agent's handlers do (code, message and field; the
differentials hold them to it) and carry their way out besides (``valid_options``,
``try_call``: tool rule 2).  A served call the session refuses is answered with that
refusal, passed through whole (:func:`refusal`), never handed to the Python handler for its
wording -- an id a change removed too: the session words it from its retired-id record, the
change named by its step's state number.

Must not: word a refusal.
"""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import ToolError


def refusal(error: dict[str, Any]) -> ToolError:
    """The session's refusal ``error`` (its envelope's ``error``) as the call's
    :class:`ToolError`, key for key."""
    return ToolError(error["code"], error["message"], field=error.get("field"),
                     valid_options=error.get("valid_options"), details=error.get("details"),
                     try_call=error.get("try_call"))
