"""Structure: ``word_move``, ``word_copy_from``, ``word_sections``, ``word_headers_footers``,
``word_fields``, ``word_notes`` and ``word_links`` -- the Rust host's, with the roles the
tool layer gives their listing actions (``Tool.action_roles``)."""

from __future__ import annotations

from ooxml_edit.tools import REVIEW

#: What the tool layer holds of each tool beside its Rust definition.
HOST = {name: {"action_roles": {"list": REVIEW}}
        for name in ("word_sections", "word_fields", "word_notes", "word_links")}
