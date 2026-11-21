Phase P2, lane P2e2: `word_delete` with `find` and `within` (or `find` with a `target`,
searched within it, said in a warning) and `word_insert_text` with `find` and `within` are
served by the Rust session: the find searches the paragraphs a block, span, heading's range
or other scope holds, as `Document.anchor(text, within=)`; `within` and `target` naming other
blocks is refused as docx-agent refuses it. Before, the session answered them `unported`.
