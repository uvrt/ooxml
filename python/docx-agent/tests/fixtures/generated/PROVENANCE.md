# docx-agent's own fixtures

| File | SHA-256 |
| --- | --- |
| `ids-and-markup.docx` | `3df20ba9fcbe5badf9bb7a950ae80652b244b8484da665dccab7afd1a24d74df` |
| `mode14.docx` | `77be777abae6193af48cb3c970c7a4c67a2a3a4a1c147acc569c89a7fc7240cc` |
| `controls.docx` | `fc2f42cc7f052802a23f82f3fa25d0d39c65bf54c7f8db674aba0042d23934b4` |
| `lists-and-styles.docx` | `94c56b4994c7f50480d634cbf099d808c7ded428b5a8c254bd289fd3567fabaf` |
| `review-follow.docx` | `32b604fc864d48b02ff62b6fbf9e1b433572ca31b8734bd98bdfdb72eb63f715` |

Written by `tools/make_fixtures.py` in this repository, deterministically (running it
again writes the same bytes), every part by hand: the styles are a handful written there,
not a template's, and nothing in them comes from Word or any other application. Licence:
this repository's, MIT.

`ids-and-markup.docx` (compatibility mode 15) holds every paraId edge case -- paraIds
as Word writes them, missing, repeated three times, out of range, one without a textId --
and the markup reading text has to walk: a hyperlink, complex and simple fields, inline and
block-level content controls, an insertion, a deletion and a move by another author, a
bookmark across two paragraphs, tabs, breaks, a symbol and the special hyphens, a table
with a horizontal and a vertical merge, a header, a footer, a footnote and a comment.
`mode14.docx` is three paragraphs in compatibility mode 14.
`controls.docx` (mode 15) holds a content control of every kind as Word 365 writes them --
plain text showing its placeholder, rich text with a title and a tag, a drop-down list whose
items' display texts and values differ, a combo box holding text of its own, a date, a checked
check box, content- and control-locked ones, one holding a tracked insertion, a data-bound one
with its custom XML part, a block-level one of two paragraphs with a bookmark displaced around
it, one in a table cell and one in the header -- for `word_controls`.
`lists-and-styles.docx` (mode 15) is what E1's edits work on: a template localised to Dutch
(the style ids Dutch Word writes -- `Standaard`, `Kop1`, `Zwaar` -- with their English
names), numbered lists over one abstract definition (one restarted), a list a paragraph
style gives, an unused list instance, formatting that changes mid-word with runs split by
rsids and a proofing mark, an external and an internal hyperlink, a bookmark, and an inline
picture -- a 4x4 one-colour PNG written by the same script.
`review-follow.docx` (mode 15) is what a review settles beyond its records: after plain
paragraphs and a plain table (what edit sets over every fixture work on), data-bound
controls (text, a drop-down list, a date whose node lacks its time) showing tracked changes,
and a table with tracked cell insertions that a comment describing them ("Tracked structural
change: ...", as docx-agent wrote before its round 57) is anchored in, beside an ordinary
comment; each bound node holds what its control shows -- for `word_changes`.
