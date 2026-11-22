Lane U4: `undo` is a tool of the session's own (beside `read_batch`): the session's undo or redo
of a document's latest steps, or with a named caller that caller's own, in today's words -- the
later step and who made it when a caller's latest change is not the latest (`entangled`), and a
caller's own redo refused as `entangled` while another caller's undo is the next to redo. The
session keeps who made each step (tool and caller, by state number) as a call, a bridged step or
a step group's close is numbered; `CallContext(context=...)` gives the caller's context in the
order the host gave it (the replies name it so), `StepGroup.close(tool=...)` the batch's tool.
A caller's own undo or redo checks and moves the history in one step of the store's
(`Store::undo_newest`, `Store::redo_next`), so no commit lands between them.
Lane U5: "nothing (of yours) to undo/redo" is refused with its way out (tool rule 2), a deliberate
change from docx-agent's reply: `valid_options` lists the document's steps, newest first, each with
who made it (`version`, `tool`, `context`; `yours`, `undone`), then the oldest state kept
(`{"version": n, "oldest": true}`), at most 10 on each side of the head (`details.left_out` counts
the rest); `try_call` is the undo or redo the other way where that one would work. The way-out-less
`ToolError::nothing_to_offer` is gone.
