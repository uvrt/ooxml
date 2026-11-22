Lane E2: `word_format` `keep_together` true is served by the Rust session (no fallback to the
Python handler); `word_format_table` is handed the document's page engine, so its
`keep_together` is served too.
Inside a `batch`, `word_format_table` `keep_together` is measured once the batch's last op ran
(round 28, W8): the adapter defers it and asks the session `block_pages` for the table's pages,
with the call's writing settings, and places docx-agent's "taller than a page" warning in the
op's warnings.
