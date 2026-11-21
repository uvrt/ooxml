Lane U3: an owned document's history is its owner's. `OwnedHistory` keeps no steps, numbers
or heads of its own: its version is the owner's head's state number (`head.state`), a state
is found by its number through the owner (`numbered`), and steps, `can_undo`, `can_redo` and
`next_redo` are read from the owner's list (`numbers`). Calls the owner serves side by side are
kept and counted as they end, in any order, so no call waits for an earlier one to be numbered
first; what is counted per change (the changes kept for reads, the part stamps, the version the
last change left) is ordered by the owner's count of its versions, so a step that ends after a
later one is never taken for a change made outside the tools. The owner protocol gains
`numbered`, `highest`, `numbers` and `continue_numbers` (a document opened again goes on with
its numbers); `keep` takes the head alone; `Served.before` carries the state a served step went
from.
