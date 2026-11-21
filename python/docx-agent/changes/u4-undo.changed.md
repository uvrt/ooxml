Lane U4: `undo` on a Word document is answered by the Rust session's `undo` tool, with today's
replies; who made each step is the session's record. The caller's context goes to the session in
the order given (`CallContext(context=...)`).
Lane U5: "nothing (of yours) to undo/redo" now lists the document's steps and who made each in
`valid_options`, and the undo or redo the other way in `try_call` where that one works. A session
with its own clock undoes through the same tool.
