Word's part of the session is its own crate, `ooxml-session-word` (architecture review Phase 3, lane ph3-format, PR
A2): Word documents' opening, state, tool calls, checks, save gate, live view and page engines. `ooxml-session` is
format-generic and depends on no Word crate; a session is made with its formats (Word's, and the kinds Rust does not
own yet as packages). Tool results and the Python API unchanged.
