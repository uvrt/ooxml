Lane H7a: `Version.problems()` gives the version's validation problems as `(code, part, detail)`.
`Version.summary()` gives what `open_document` says of a Word document (JSON). Both read the
version in Rust. `Version.id_state(number, last, gone=, drawings_gone=, earlier=, said=)` gives
the version's id state from the ids the session holds, with the host's numbers. Its record is
docx-agent's `Document.id_state` record; the deflate bytes may differ, and either side opens the
other's state.
