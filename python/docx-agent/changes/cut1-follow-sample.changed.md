Lane CUT1: `word_follow_sample` with a sample in another open document is served by the Rust
session (its arguments name the documents as the host does), no longer by the Python handler.
The tests that meet a form whose port is deferred until after the release are strict xfails
naming the form (`tests/deferred_forms.py`); the tests of docx2svg's Python layout
(`python_layout`) are skipped.
