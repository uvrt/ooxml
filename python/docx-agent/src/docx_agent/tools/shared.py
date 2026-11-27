"""The shared tools (``ooxml_edit.tools.shared``) for Word documents: one definition each for
every format, dispatched by the document's kind.

``open_document``, ``close_document``, ``undo``, ``batch``, ``read_blob`` and ``read_batch`` are
the session's own (``shared.SESSION_TOOLS``, ``shared.READ_BATCH``); this module offers the others
for docx -- ``new_document``, ``save_document``, ``describe``, ``find_text``, ``replace_text``,
``render``, ``check``, ``edit_chart`` and ``set_properties`` -- whose calls on a Word document
run on the Rust host, as every Word tool's do.  The shared definitions are the Rust host's specs
of these tools byte for byte (a test holds them so); they are kept as ooxml-edit's so that a
toolbox with another library's tools merges them into one tool."""

from __future__ import annotations

from ooxml_edit.tools import shared

from .specs import KIND, on_host

#: The shared tools docx offers, each handled on the Rust host.
HANDLERS = {name: on_host for name in (
    "new_document", "save_document", "describe", "find_text", "replace_text", "render", "check",
    "edit_chart", "set_properties")}

#: The shared tools docx offers, in the order the toolbox lists them.  The session's own tools
#: follow (``read_batch``: many reads in one call, a reader's tool).
TOOLS = [shared.handler(name, kind=KIND)(handler) for name, handler in HANDLERS.items()] \
    + [*shared.SESSION_TOOLS, shared.READ_BATCH]
