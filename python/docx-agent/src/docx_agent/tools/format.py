"""The docx :class:`~ooxml_edit.tools.DocumentFormat`: how the tool layer recognises a Word
document, the Rust session that opens and owns it (:class:`docx_agent.rust.owner.RustOwner`:
its versions, its steps, its ids, its saved bytes), the hooks that read it -- each the
session's reading of the version asked (:mod:`docx_agent.rust.hooks`), never a Python view
-- and the Word system-prompt fragment (mechanics only: no house style)."""

from __future__ import annotations

import io
import zipfile
from typing import Any

from ooxml_edit.tools import DocumentFormat, ToolError

from ..rust import hooks
from ..rust.edits import serve_owned
from ..rust.owner import OWNER, TemplateOpened, session_validation
from ._base import checks, problem_key
from .table_data import own_source, source_hint

KIND = "docx"
EXTENSIONS = (".docx", ".docm", ".dotx", ".dotm")

PROMPT = """\
Word documents
- describe first: headings with ids, sections, styles, comments, revisions, fields, \
tables, pages. word_read gives Markdown with an id comment before each block; pass a range \
to read part of it. Several reads go in one read_batch.
- Addresses: p:3B212964 a paragraph; p:3B212964@4:11 characters 4-11 of it; p:A@5 a \
position; p:A..t:B the blocks from A through B; h:p:A a heading with every block under it \
(to the next heading of its level or higher), wherever a span is taken; t: table, \
c: comment, rev: revision, fn:/en: note, d: drawing, cc: content control, fld: field, \
s:2 a section (s:body the last), \
header1/footer1 stories. A paragraph's id can change when it is edited: the result's \
renamed maps old to new; the old id keeps working.
- Never paste word_read's Markdown into a text tool: it is escaped. Pass plain text.
- A figure from a dataset or table goes into text as {{d:<id>/<key>/<column>}} or \
{{t:<table>/<key>/<column>}}, :0.0 after it to round, a date pattern such as :d mmmm yyyy \
for a date: the tool writes the value in the \
locale and reports its source. A text value too, verbatim: in word_insert_markdown its \
Markdown (from: d:<id>/#12/<column>, or each {{...}} alone between blank lines); never retype it.
- To change words inside a paragraph, prefer replace_text with expect one, or \
word_insert_text and word_delete with find; word_set_text rewrites a whole paragraph. \
word_format sets styles and run or paragraph formatting. A new table is a Markdown \
table through word_insert_markdown.
- Tracked changes: word_set_tracking on makes every later edit a revision by its author; \
word_changes lists, accepts and rejects them. \
Comments, replies and resolving are never revisions.
- Fields (TOC, page numbers, cross-references) are caches: after changing headings or \
moving text, word_fields update.
- Renders show the final view (revisions accepted, comments hidden); read markup with \
word_read view markup.
- Before saving: check with include validate, reflow and fields; update stale fields.
"""

#: Claude's strict mode fits only some tools (20 tools, 24 optional parameters and, measured,
#: about 32 free-text strings per request): the writing tools the reference transcripts
#: (w1-w6, batch-30) call most -- 23 of their calls, 24 optional parameters, 23 strings.
#: word_changes counts its list calls too.  edit_chart left the list in T4: with its add
#: action it has 14 optional parameters, more than half the request's.  X1's from,
#: from_columns and header took word_insert_markdown to seven, and edit_smartart, which
#: filled the last two places, out of the strict set (23 optional parameters now).
STRICT_FIRST = ("replace_text", "word_changes", "word_set_tracking", "word_insert_markdown",
                "word_template")


def _opened_by_the_session(data: bytes, state: Any = None) -> Any:
    """The format's ``open`` and ``open_with_state``: never reached -- the session opens a
    Word document's bytes itself (the owner's ``open_data``), and the tool layer holds no
    document of its own."""
    raise ToolError("internal", "a Word document is opened by the Rust session (RustOwner.open_data), "
                    "never by the tool layer")


def detect(data: bytes, name: str) -> bool:
    if not data.startswith(b"PK"):
        return False
    if name.lower().endswith(EXTENSIONS):
        return True
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            return "word/document.xml" in archive.namelist()
    except zipfile.BadZipFile:
        return False


#: The references a deck resolves in its own text that a Word document refuses in words of
#: its own (``DocumentFormat.own_facts``), with no deck format loaded too: pptx-agent's
#: ``{{slide:s:262}}``, a slide's number.
OWN_REFUSED = {"slide:": "slide references are for decks; a Word document has no slides"}

#: Tool names a model writes that ooxml-edit's reading of names (its tools' names and
#: actions, verbs and plurals read) does not settle, or settles wrongly -- a link is a
#: hyperlink, not a header's link to the previous one; a page break is a paragraph's
#: ``page_break_before`` (``DocumentFormat.tool_hints``; each key covers its spellings with
#: another verb, a plural or no prefix).  Wording only.
TOOL_HINTS = {
    "word_add_image": "word_drawings insert_picture",
    "word_add_textbox": "word_drawings insert_text_box",
    "word_add_table": "word_insert_markdown",
    "word_add_heading": "word_insert_markdown",
    "word_add_paragraph": "word_insert_markdown",
    "word_add_bullet": "word_lists add",
    "word_add_link": "word_links add_hyperlink",
    "word_add_url": "word_links add_hyperlink",
    "word_add_page_break": "word_format",
    "word_set_font": "word_format",
    "word_apply_style": "word_format",
    "word_set_paragraph_style": "word_format",
    "word_format_cell": "word_format_table",
    "word_shade_cell": "word_format_table",
    "word_track_changes": "word_set_tracking",
    "word_enable_tracking": "word_set_tracking",
    "word_add_header": "word_headers_footers set",
    "word_add_footer": "word_headers_footers set",
    "word_add_page_number": "word_headers_footers set",
    "word_set_margins": "word_sections set",
    "word_set_orientation": "word_sections set",
    "word_set_page_size": "word_sections set",
    "word_set_columns": "word_sections set",
    "word_add_checkbox": "word_controls insert",
    "word_add_dropdown": "word_controls insert",
    "word_add_content_control": "word_controls insert",
    "word_find": "find_text",
    "word_replace": "replace_text",
    "word_find_replace": "replace_text",
    "word_save": "save_document",
    "word_upgrade": "word_template upgrade_to_modern",
}


def summary(document: Any) -> dict[str, Any]:
    """What ``open_document`` says of a Word document: cheap facts, no layout -- the
    session's reading of the version (:func:`docx_agent.rust.hooks.summary`)."""
    return hooks.summary(document)


def touched(document: Any, change: Any) -> list[dict[str, Any]]:
    """The format's ``touched``: for a document the owner holds, what a change touched is the
    owner's to say between the versions it went from and to (``RustOwner.touched``), which
    the tool layer asks for every docx entry; this hook is never handed a document."""
    raise ToolError("internal", "a Word document's changes are said by the Rust session (RustOwner.touched)")


def number_texts(call: Any, scope: Any) -> list[Any]:
    """The texts a check of numbers reads, the session's (:func:`docx_agent.rust.hooks.number_texts`)."""
    return hooks.number_texts(call, call.subject, scope)


def scale_values(call: Any, scale: Any, scope: Any) -> list[Any]:
    """The values a scale's targets name, the session's (:func:`docx_agent.rust.hooks.scale_values`)."""
    return hooks.scale_values(call, call.subject, scale, scope)


FORMAT = DocumentFormat(
    kind=KIND, open=_opened_by_the_session, detect=detect, problems=hooks.problems, problem_key=problem_key,
    validation=session_validation, warnings=(TemplateOpened,), prompt=PROMPT, summary=summary, checks=checks,
    strict_first=STRICT_FIRST, source_hint=source_hint, own_source=own_source, read_provenance=hooks.provenance,
    own_facts=dict(OWN_REFUSED), tool_hints=TOOL_HINTS, touched=touched, number_texts=number_texts,
    scale_values=scale_values, state=hooks.id_state, open_with_state=_opened_by_the_session, owner=OWNER,
    serve=serve_owned)
