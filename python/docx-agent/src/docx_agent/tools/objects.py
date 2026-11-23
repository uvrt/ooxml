"""Objects: tables (``word_edit_table``, ``word_format_table``; a new table is Markdown,
through ``word_insert_markdown``), pictures and other drawings (``word_drawings``) and
content controls (``word_controls``) -- the Rust session's, with the roles the tool layer
gives their listing actions (``Tool.action_roles``)."""

from __future__ import annotations

from ooxml_edit.tools import REVIEW

#: What the tool layer holds of each tool beside its Rust definition.
HOST = {name: {"action_roles": {"list": REVIEW}} for name in ("word_drawings", "word_controls")}
