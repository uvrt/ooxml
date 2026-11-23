"""Style: ``word_format``, ``word_lists``, ``word_styles``, ``word_template``,
``word_design_facts``, ``word_compare_blocks`` and ``word_follow_sample`` -- the Rust
session's; the readers' arguments with their refs resolved, and the roles and reading calls
the tool layer gives the others (``Tool.action_roles``, ``Tool.reads``)."""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import REVIEW, Result

from ..rust import reads


def word_compare_blocks(call: Any, doc: str, blocks: list[str], reference: dict,
                        cursor: str | None = None) -> Result:
    """Paged by difference; the reference's document at the version the call reads."""
    return reads.serve(call, "word_compare_blocks", {"blocks": [reads.refs(call, block) for block in blocks],
                                                     "reference": reference, "cursor": cursor},
                       others=("reference.doc",))


def _previewing(arguments: Any) -> bool:
    """``word_follow_sample``'s calls that only read: a ``preview``."""
    return bool(arguments.get("preview"))


HANDLERS = {"word_compare_blocks": word_compare_blocks}

#: What the tool layer holds of each tool beside its Rust definition.
HOST = {
    "word_styles": {"action_roles": {"list": REVIEW, "describe": REVIEW}},
    "word_follow_sample": {"reads": _previewing},
}
