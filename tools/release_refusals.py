"""What the release notes say of each form the Word tools refuse as not supported yet.

crates/ooxml-oracle/known-differences.toml's ``[[not_supported]]`` entries name each form (a
glob on ``details.not_supported``) with its cause, written for whoever ports it: the forms the
Rust tools refuse when the gates call them directly.  The release notes (tools/release_notes.py,
"Refused at this release, returning later") say instead which calls the team gets refused
through docx-agent's Toolbox -- where the tool layer answers some forms first (a reference's
value, a dataset's rows, ``check``'s ``scale`` and ``numbers``) and every document is the
session's own and laid out by its engine.

READER has one entry per TOML form (tools/test_changelog.py holds the keys equal): what the
team is refused, or ``None`` where no call through the Toolbox has been seen refused in that
form (served there, or not reached with a made-up document yet) -- such a form is not listed.
python/docx-agent/tests/test_release_refusals.py makes, through the Toolbox, a call refused in
each listed form.  A lane that serves a form removes its entry here and in the TOML; a lane
that adds a form adds both, and a case to that test where it lists the form.

Entries that say the same are listed once.  Standard library only.
"""

from __future__ import annotations

#: What is refused, by form: a call or argument, said for the team that calls the tools;
#: ``None``: not listed (see the module's text).
READER: dict[str, str | None] = {
    "word_edit_table *":
        "`word_edit_table` `sync` from a single value, and any edit of a table in which a cell's paragraph "
        "mark is deleted while its row stays (Accept All would join the next cell's text into it)",
    "check: *":
        "`check`'s layout facts (`include: [\"layout\"]`) of a document the page layout cannot lay out "
        "throughout (for example a VML shape without a size in a footnote)",
    "render: *": None,  # the Toolbox's documents are the session's own and laid out by its engine
    "* of a document the session does not own": None,  # likewise
    "word_read final view of Accept All's cell ends":
        "`word_read`'s final view of a document with a table cell whose paragraph mark is deleted while its "
        "row stays (a pending deletion of a whole row is read)",
    "word_inspect of Accept All's cell ends":
        "`word_inspect` of a document with a table cell whose paragraph mark is deleted while its row stays",
    "* by reference": None,  # the tool layer writes a reference's value, or resolves a $name, first
    "replace_text *":
        "`replace_text` with `regex` or `range`, an empty `find`, or styled text in `replace` "
        "(`**`, `*`, `[text]{Style}`, escapes)",
    "word_delete a *":
        "`word_delete` of a picture (a `d:` target) or of a heading alone (an `h:` target without a range)",
    "word_insert_markdown *":
        "`word_insert_markdown` with `like`, or a table from a dataset's or another document's rows "
        "(`from`; one field of one row is served)",
    "a picture in Markdown":
        "A picture in Markdown given to `word_insert_markdown`",
    "a footnote in Markdown":
        "A footnote in Markdown given to `word_insert_markdown`",
    "* at replace:":
        "`word_insert_markdown` `at` a `replace:` place",
    "* into an empty container": None,
    "a block of * without an id": None,
    "ranges sharing a paragraph with a range across paragraphs": None,
    "word_drawings *":
        "`word_drawings` with `ref` (naming the drawing for later calls)",
    "edit_chart*":
        "`edit_chart` with values from a dataset or table (`from`)",
    "*a footnote or endnote the layout cannot lay out": None,
    "copy_from *": None,
    "a list number docx-agent cannot read": None,
    "a theme whose colours docx-agent cannot read": None,
    "a copied style *": None,
    "word_controls fill *": None,
    "word_changes accept moving the document's only table record*": None,
    "a document without a settings part":
        "`word_headers_footers` with `type: \"even\"` on a document without a settings part",
    "the section of *: a section given by what a block holds":
        "`word_headers_footers` given a section by something a block holds (a bookmark, link, drawing, "
        "note, comment, field or change) instead of by a block",
    "word_template reflow without the layout": None,  # the session's engine lays every document out
    "a field update that needs the layout": None,  # likewise
    "no layout": None,  # likewise
    "a page cross-reference without the layout": None,  # likewise
    "the layout of this version: *": None,
    "a table of contents whose end is not in a paragraph beside its begin": None,
}
