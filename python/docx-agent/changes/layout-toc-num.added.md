Lane LF6: a table of contents over numbered headings is rebuilt by the Rust field update
too: each numbered heading's entry shows its list number -- then a tab, with a left stop after
the number measured with the layout's fonts, or a space -- as `_TocLooks.number_stop` writes
it; it was left to the Python handler. A generated fixture, `toc-numbered.docx`
(`tools/make_toc_fixtures.py`), holds headings numbered every way.
