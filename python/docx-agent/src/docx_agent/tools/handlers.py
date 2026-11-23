"""The handlers every Word tool shares: each call is the Rust session's.

- :func:`edit` -- a tool that changes the document: a changing call is the session's edit
  (:func:`docx_agent.rust.edits.serve`, under the document's lock; ooxml-edit asks the
  format's ``serve`` first, without it), and a call of it that only reads (``word_changes``
  list, ``word_follow_sample`` preview, ``edit_chart`` read: ``Tool.reads``) the session's
  read of the version the call reads (:func:`read_action`);
- :func:`read` -- a reading tool with no argument the host adapts: the session's read.

A tool whose arguments need the host first (refs, the application's data) has a handler of
its own in its group's module, calling :func:`docx_agent.rust.reads.serve` with them.
"""

from __future__ import annotations

from typing import Any

from ooxml_edit.tools import Result

from ..rust import edits, reads


def others_of(tool: Any) -> tuple[str, ...]:
    """The paths of a tool's arguments that name another open document (``reference.doc``,
    ``sample.doc``): its ``documents`` but ``doc``."""
    documents = tool.documents
    if callable(documents):
        return ()
    return tuple(path for path in documents if path != "doc")


def read_action(call: Any, arguments: dict[str, Any]) -> Result:
    """A reading call of a changing tool: the session's read of the version the call reads,
    its arguments as the session takes them (the tool's adapter, :mod:`docx_agent.rust.served`)."""
    from ..rust.served import arguments_for

    tool = call.tool
    given = dict(arguments_for(call, tool.name, arguments))
    return reads.serve(call, tool.name, given, others=others_of(tool))


def edit(call: Any, **arguments: Any) -> Result:
    """The handler of a tool that changes the document."""
    if call.changing:
        return edits.serve(call, arguments)
    return read_action(call, arguments)


def read(call: Any, **arguments: Any) -> Result:
    """The handler of a reading tool whose arguments the session takes as given."""
    tool = call.tool
    given = {key: value for key, value in arguments.items() if key != "doc"}
    return reads.serve(call, tool.name, given, others=others_of(tool))
