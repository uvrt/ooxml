Lane U1: the store numbers a document's states as the tool layer numbers them
(`ooxml_store::Version::number`): the state opened 0, a step one more than the highest number
given, a plain undo or redo the number of the state it brings back; given in publish order in the
store's commit section. `Store::numbered(n)` finds the head-most kept state with a number. The
session's `Opened::number` reads it.
A step group is one step and one number (lane U2): its commits carry one more than the highest
number given when it opened, given for good only when it closes having made a step; a revert of
all its commits brings back the number it opened on.
Lane U3: `Version.state` (the state number; `number` stays the session's count of versions);
`Session.numbered(doc, n)`, `Session.highest(doc)`, `Session.numbers(doc)` (the highest number,
the steps undo can reach and redo can bring back, each `(before, after, seq, caller)`) and
`Session.continue_numbers(doc, state, highest)` for a document opened again. `Session.edit`'s
made tuple ends with the state number the call's step went from. A changing call's
`result.version` and `Session.undo`/`redo`'s version are state numbers (`Store::listed`,
`Store::highest`, `Store::continue_numbers`; `StepInfo::number`/`from_number`).
