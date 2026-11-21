Lane U4: an owned document's undo and redo are its owner's own undo tool (owner protocol:
`history(handle, name, steps, redo, caller)` replaces `undo`/`redo`; `close_group` takes the
batch's `tool`). The tool layer keeps no owners or own undos for such a document; `undo`'s reply
is the owner's, and `Session.undo`/`redo` count its steps (`Session.owned_history`).
Lane U5: `ToolError(try_call=...)` keeps a call that would work, and the reply carries it as
`try_call`; an owned document's undo refusal passes the owner's on (its steps and who made each).
