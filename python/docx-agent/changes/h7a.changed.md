Lane H7a: for a document the Rust session owns, the format's `summary`, `problems`, `state`,
`closed` and `restored` hooks no longer read the Python view. They get ooxml-edit's
`OwnedDocument` and answer from the session's reading of the version
(`docx_agent.rust.hooks`): the same summary and problems as before, and the same id-state
record. `closed` and `restored` let go only of what was kept for a view made in this process.
