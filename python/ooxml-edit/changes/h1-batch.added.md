Lane H1: a batch on a document whose owner has step groups (`open_group`, `group_mark`,
`revert_group`, `close_group`; see `tools.owned`) is one step group of the owner's. Each op
is a sub-step (`DocumentEntry.op_step`, a no-op for other documents). A call the owner
serves commits into the group. A Python tool's change to the view is committed as the op
ends, so the next op reads it. A failed op is taken back alone, or the whole group where
the batch is atomic, and undo and redo stay as they were. The batch ends as one step. The
group is closed on every path, an exception's too, and while it is open the view and the
batch's reads follow its head (`OwnedHistory.reading_head`). The owner's `keep` is given
the step's first version (`first`), so every version of a step made of several is
numbered as the step.
