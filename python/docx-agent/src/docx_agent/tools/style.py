"""Style: ``word_format``, ``word_lists``, ``word_styles``, ``word_template``,
``word_design_facts``, ``word_compare_blocks`` and ``word_follow_sample`` -- the Rust host's,
with the roles and reading calls the tool layer gives them (``Tool.action_roles``,
``Tool.reads``)."""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import REVIEW


def _previewing(arguments: Any) -> bool:
    """``word_follow_sample``'s calls that only read: a ``preview``."""
    return bool(arguments.get("preview"))


#: What the tool layer holds of each tool beside its Rust definition.
HOST = {
    "word_styles": {"action_roles": {"list": REVIEW, "describe": REVIEW}},
    "word_follow_sample": {"reads": _previewing},
}
