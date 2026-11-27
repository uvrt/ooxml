"""Review: ``word_set_tracking``, ``word_changes`` (list, preview, accept and reject tracked
changes) and ``word_comments`` -- the Rust host's, with the roles the tool layer gives
their actions (what a reviewer may call: ``Tool.action_roles``)."""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import ACCEPT, REVIEW, shared


def _listing(arguments: Any) -> bool:
    """``word_changes``' calls that only read: ``list`` and ``preview``."""
    return arguments.get("action") in ("list", "preview")


#: What the tool layer holds of each tool beside its Rust definition.
HOST = {
    "word_changes": {"reads": _listing,
                     "action_roles": {"list": REVIEW, "preview": ACCEPT, "accept": ACCEPT, "reject": ACCEPT},
                     "review_core": True,
                     "action_fields": {"ids": ("accept", "reject", "preview"),
                                       "all": ("accept", "reject", "preview")}},
    "word_comments": {"action_roles": dict(shared.COMMENT_ACTION_ROLES), "call_role": shared.comment_role,
                      "review_core": True},
}
