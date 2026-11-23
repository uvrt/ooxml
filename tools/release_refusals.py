"""What the release notes say of each form the Word tools refuse as not supported yet.

crates/ooxml-oracle/known-differences.toml's ``[[not_supported]]`` entries name each form (a
glob on ``details.not_supported``) with its cause, written for whoever ports it.  The release
notes (tools/release_notes.py, "Refused at this release, returning later") say instead, for
the team that calls the tools, which calls are refused, by READER here: one entry per form,
the same keys as the TOML's forms (tools/test_changelog.py holds them equal).  A lane that
serves a form removes its entry from both files; a lane that adds a form adds both.

Entries that say the same are listed once.  Standard library only.
"""

from __future__ import annotations

#: What is refused, by form: a call or argument, said for the team that calls the tools.
READER: dict[str, str] = {
    "word_edit_table *":
        "`word_edit_table`: `sync` from a single value; any edit of a table in which a cell's paragraph "
        "mark is deleted while its row stays (Accept All would join the next cell's text into it); "
        "deleting a row or column that holds a footnote, endnote or comment reference; splitting a cell "
        "in a row that reaches past the table's grid",
    "check: *":
        "`check` with `scale` or with a deck's parts in `include` (`fit`, `collisions`, `facts`, `design`, "
        "`numbers`); `copies` against a document held only as a file package; the layout parts when no "
        "page layout is at hand or the layout cannot lay out the version; a document held only for reading",
    "render: *":
        "`render` when no page layout is at hand, or of a document held only for reading",
    "* of a document the session does not own":
        "`word_changes` and `save_document` of a document held only for reading",
    "word_read final view of Accept All's cell ends":
        "`word_read`'s final view of a document with a table cell whose paragraph mark is deleted while its "
        "row stays (a pending deletion of a whole row is read)",
    "word_inspect of Accept All's cell ends":
        "`word_inspect` of a document with a table cell whose paragraph mark is deleted while its row stays",
    "* by reference":
        "A value or address given by reference (`$name`, `{{...}}`) to `word_set_text`, `word_insert_text`, "
        "`word_insert_markdown` or `word_delete`",
    "replace_text *":
        "`replace_text` with `regex`, `range` or `slides`, an empty `find`, styled text in `replace` "
        "(`**`, `*`, `[text]{Style}`, escapes), or in the body only while headers, footers or notes "
        "match too",
    "word_delete a *":
        "`word_delete` of a picture (a `d:` target) or of a heading alone (an `h:` target without a range)",
    "word_insert_markdown *":
        "`word_insert_markdown` with `like` (or a style map naming a paragraph), or content from data "
        "(`source`, `blob`, `from_columns`, `header`, `date_format`)",
    "a picture in Markdown":
        "A picture in inserted Markdown",
    "a footnote in Markdown":
        "A footnote in inserted Markdown",
    "* at replace:":
        "Placing new content `at` a `replace:` place",
    "* into an empty container":
        "Placing new content into an empty container",
    "a block of * without an id":
        "Deleting a block that has no id",
    "ranges sharing a paragraph with a range across paragraphs":
        "An untracked edit of several ranges in which a range across paragraphs shares a paragraph with "
        "another",
    "word_drawings *":
        "`word_drawings` with `ref`, inserting an SVG picture or a picture not in the session's blobs, "
        "and listing a chart inside a group",
    "edit_chart*":
        "`edit_chart` with values from a table (`from`, `columns`, `date_format`, `blanks`), and on "
        "charts whose data or embedded workbook the tools cannot read or write",
    "*a footnote or endnote the layout cannot lay out":
        "`check`, `render`, the live view and `word_inspect`'s layout of a document with a frame, a text "
        "box, a drawing that text wraps beside, or a STYLEREF field in a footnote or endnote",
    "copy_from *":
        "`word_copy_from` to a place given by reference, or from a source held only as a file package",
    "a list number docx-agent cannot read":
        "`word_copy_from` of a list whose numbering the tools cannot read",
    "a theme whose colours docx-agent cannot read":
        "`word_copy_from` from a document whose theme colours the tools cannot read",
    "a copied style *":
        "`word_copy_from` of a style without a kind, or based on no style",
    "word_controls fill *":
        "`word_controls` `fill` of a control around a table's rows or cells, of a block-level control "
        "without a paragraph, or of a control bound by an XPath other than child steps or to a part's root",
    "word_changes accept moving the document's only table record*":
        "`word_changes` `accept` that moves the document's only record of where a table's values came from",
    "a document without a settings part":
        "`word_headers_footers` on a document without a settings part",
    "the section of *: a section given by what a block holds":
        "`word_headers_footers` given a section by something a block holds (a link, bookmark, drawing, "
        "note, comment, field or change) instead of by a block",
    "word_template reflow without the layout":
        "`word_template` `reflow` when no page layout is at hand",
    "a field update that needs the layout":
        "`word_fields` updates that need page numbers when no page layout is at hand",
    "no layout":
        "`word_fields` updates that need page numbers when no page layout is at hand",
    "a page cross-reference without the layout":
        "Inserting a page cross-reference when no page layout is at hand",
    "the layout of this version: *":
        "`word_fields` updates that need page numbers on a version the page layout cannot lay out",
    "a table of contents whose end is not in a paragraph beside its begin":
        "Updating a table of contents whose end is not in a paragraph beside its start",
}
