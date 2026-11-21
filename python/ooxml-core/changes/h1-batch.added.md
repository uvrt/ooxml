Lane H1: step groups. `Session.open_group(doc)` gives a `StepGroup`. The commits made until
`StepGroup.close(context=...)` are one undo step of the context's caller. `mark()` and
`revert(mark)` take back what was committed after a mark (by default since the group
opened), as a new version that no redo brings back, with the tracking mode as it was then.
Plain redo is held while the group is open and given back when it commits nothing. Undo
and redo are refused while a group is open. This is built on `ooxml_store::Store`'s
`open_group`, `group_mark`, `revert_group` and `close_group`.
