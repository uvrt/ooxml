"""docx-agent: Word documents for a model, as tool calls on the Rust host.

Every Word document is held by the Rust host (``ooxml_edit.tools.hosted``): it opens, edits, lays out,
checks, renders and saves it. docx-agent is the tool layer on it, for ``ooxml_edit.tools``::

    from ooxml_edit.tools import Toolbox
    from docx_agent.tools import FORMAT, GROUPS, TOOLS

    with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS) as toolbox:
        session = toolbox.session()
        d1 = session.open(docx_bytes, name="report.docx")
        result = toolbox.dispatch(session, "word_read", {"doc": d1})

and the live view of a session's document (``docx_agent.liveview``). The Python ``Document``
API of 0.1 is gone (CHANGELOG.md, 0.2.0).
"""

__version__ = "0.2.4"

__all__ = ["__version__"]
