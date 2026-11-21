Lane G8: the editing tools the Rust session serves, and what their edits read from the host
(blobs, the page engine, values written by reference), come from the registry's export
(`ooxml_core.tool_hosting`, declared by each Rust tool), not from lists in `rust/edits.py`
(`SERVED`, `BLOBS`, `LAID`, `KEEPS_WRITTEN` are gone: `docx_agent.rust.served.hosting()`).
A tool whose arguments need the host first has an adapter registered with
`@adapter("<tool>")` in its group's module of `docx_agent.rust.served`; the others go as
given. The same tools are served as before (`tests/test_served_hosting.py`).
