"""Word tools for an agent tool layer (``ooxml_edit.tools``): definitions and handlers.

An application puts these in a :class:`ooxml_edit.tools.Toolbox` with the docx
:data:`FORMAT` (and, beside them, another library's tools for its documents)::

    from ooxml_edit.tools import Toolbox
    from docx_agent.tools import TOOLS, FORMAT, GROUPS

    toolbox = Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)
    session = toolbox.session(clock=my_clock)
    d1 = session.open(docx_bytes, name="agreement.docx")
    tools = toolbox.definitions("anthropic", groups="core")
    openai_tools = toolbox.definitions("openai-responses")   # or "openai-chat"
    result = toolbox.dispatch(session, "describe", {"doc": d1})

Every Word tool is the Rust session's (``ooxml_core``): its definition is the session's spec
(``ooxml_core.tool_specs``, :func:`docx_agent.rust.session.tool_of`) with what only the tool
layer holds of it (the roles of its actions, the calls of it that only read), and its handler
hands the call to the session (:mod:`.handlers`), a reading tool with the arguments the host
adapts first.  The shared tools (``open_document``, ``save_document``, ``find_text``,
``render``...) are one definition for every format (``ooxml_edit.tools.shared``, the session's
specs byte for byte); this package gives their Word handlers (:mod:`.shared`).

Everything is in memory: documents and inputs are bytes under handles, saved files go to
the application.  Lengths are points.  Every mutating call is one undo step, written as
tracked changes when ``word_set_tracking`` turned tracking on, and returns ``checks``: the
validation delta against the document as opened, and the pages it reflowed.

How to run them with each provider (Claude, OpenAI Responses, Chat Completions):
``README.md`` in this package.
"""

from __future__ import annotations

import dataclasses
import json
from collections.abc import Mapping
from typing import Any

from ooxml_edit.tools import CORE, DocumentFormat, ToolGroup, shared as _shared

from ..rust.session import tool_of
from ._base import block_of, blocks_between
from .format import FORMAT, KIND, PROMPT
from . import handlers, objects, read, review, structure, style
from . import shared

GROUPS = [
    *_shared.GROUPS,
    ToolGroup("word_text", "Word: inspect formatting, insert and delete text, write Markdown."),
    ToolGroup("word_review", "Word: tracking, tracked changes (list, accept, reject), comments."),
    ToolGroup("word_structure", "Word: move and copy sections, page setup, headers and footers, "
              "fields and TOC, notes, links and bookmarks."),
    ToolGroup("word_objects", "Word: tables, pictures and other drawings, text boxes, content controls."),
    ToolGroup("word_style", "Word: formatting text and paragraphs, lists, the style sheet, templates."),
]

#: The Word tools, in the roadmap's order (the strict plan tries tools in order).
WORD_ORDER = (
    "word_read", "word_inspect",
    "word_set_text", "word_insert_text", "word_delete", "word_insert_markdown", "word_format",
    "word_set_tracking", "word_changes", "word_comments",
    "word_move", "word_copy_from", "word_sections", "word_headers_footers", "word_fields", "word_notes",
    "word_links",
    "word_edit_table", "word_format_table", "word_drawings", "word_controls",
    "word_lists", "word_styles", "word_template", "word_design_facts", "word_compare_blocks",
    "word_follow_sample",
)

#: The Word tools' own handlers (the rest: :func:`.handlers.edit` for a changing tool,
#: :func:`.handlers.read` for a reading one).
HANDLERS = {**read.HANDLERS, **style.HANDLERS}

#: What the tool layer holds of a Word tool beside its Rust definition.
HOST: dict[str, dict[str, Any]] = {**review.HOST, **structure.HOST, **objects.HOST, **style.HOST}


def _word_tools() -> list[Any]:
    """The Word tools from the Rust session's specs, in :data:`WORD_ORDER`."""
    import ooxml_core

    specs = {spec["name"]: spec for spec in json.loads(ooxml_core.tool_specs(KIND))}
    missing = [name for name in WORD_ORDER if name not in specs]
    if missing:
        raise RuntimeError(f"ooxml_core has no spec of {', '.join(missing)}: install the ooxml-core this "
                           "docx-agent was built with")
    out = []
    for name in WORD_ORDER:
        tool = tool_of(specs[name], KIND)
        handler = HANDLERS.get(name) or (handlers.edit if tool.mutates else handlers.read)
        out.append(dataclasses.replace(tool, handlers={KIND: handler}, **HOST.get(name, {})))
    return out


#: Every Word-specific tool, in the roadmap's order.
WORD_TOOLS = _word_tools()

TOOLS = shared.TOOLS + WORD_TOOLS

CORE_TOOLS = [t.name for t in TOOLS if t.group == CORE]

#: The kinds of text ``docx_format``'s ``min_text_size`` sets a minimum for.
TEXT_KINDS = ("body", "table", "footnote", "endnote", "header", "footer", "caption")


def _minimums(value: Any) -> "float | dict[str, float] | None":
    """``min_text_size`` as the session takes it: ``None``, points for all text, or points by
    kind (``None`` values left out); a ``ValueError`` for anything else, here rather than at
    the first check."""
    if value is None:
        return None

    def points(kind: str, given: Any) -> float:
        if isinstance(given, bool) or not isinstance(given, (int, float)) or not 1 <= given <= 400:
            raise ValueError(f"min_text_size{f'[{kind!r}]' if kind else ''} {given!r}: points between "
                             f"1 and 400, or a mapping of {', '.join(TEXT_KINDS)} to points")
        return float(given)

    if isinstance(value, Mapping):
        unknown = [kind for kind in value if kind not in TEXT_KINDS]
        if unknown:
            raise ValueError(f"min_text_size has no kind {unknown[0]!r}; the kinds are {', '.join(TEXT_KINDS)}")
        found = {kind: points(kind, given) for kind, given in value.items() if given is not None}
        if not found:
            raise ValueError(f"min_text_size is an empty mapping; name {', '.join(TEXT_KINDS)}, or give "
                             "one number")
        return found
    return points("", value)


def docx_format(*, min_text_size: "float | dict[str, float] | None" = None, assets: Any = None,
                typography: bool = True) -> DocumentFormat:
    """:data:`FORMAT` with the application's own settings (round 17, P6) -- never tool
    arguments, so the definitions the model is sent stay the same; ``deck_format``'s
    counterpart.  The settings go to the Rust session that owns the format's documents
    (:class:`docx_agent.rust.owner.RustOwner`), which every call of them reads:

    ``min_text_size`` (points): ``check`` reports ``small_text``, the text drawn smaller
    (its layout facts, as ``min_size`` does), and the tools that write content
    (``word_insert_markdown``, ``word_edit_table``) the text they wrote that its styles make
    smaller.  One number is the minimum for all text; a mapping sets it per kind -- ``body``,
    ``table`` (text in a table cell), ``footnote``, ``endnote``, ``header``, ``footer``,
    ``caption`` (a paragraph in the Caption style) -- and a kind it leaves out is not
    checked::

        toolbox = Toolbox(TOOLS, formats=[docx_format(min_text_size=10)], groups=GROUPS)
        docx_format(min_text_size={"body": 10, "table": 9, "footnote": 8, "caption": 9})

    ``assets``: an asset catalogue is not supported yet (refused, ``ValueError``): give a
    picture to ``word_drawings`` as an image blob.

    ``typography`` (round 35, R21; default on): the text a call writes gets the spacing
    Word's proofing of its language expects around ``: ; ! ?`` and guillemets -- in French,
    a no-break space (U+00A0) -- and the result says how many spaces it wrote; ``False``
    writes the text as given.  ``check`` reports the places still without it either way
    (``punctuation_spacing``)::

        toolbox = Toolbox(TOOLS, formats=[docx_format(typography=False)], groups=GROUPS)
    """
    from ..rust.owner import RustOwner

    if assets is not None:
        raise ValueError("an asset catalogue is not supported yet: give word_drawings a picture as an image blob")
    from .hosting import WordHosting

    minimums = _minimums(min_text_size)
    owner = RustOwner(min_text_size=minimums, typography=bool(typography))
    host = WordHosting(min_text_size=minimums, typography=bool(typography))
    return dataclasses.replace(FORMAT, owner=owner, host=host)


__all__ = ["CORE_TOOLS", "FORMAT", "GROUPS", "PROMPT", "TOOLS", "WORD_TOOLS", "block_of", "blocks_between",
           "docx_format"]
