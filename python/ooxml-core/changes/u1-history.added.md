Lane U1: the store numbers a document's states as the tool layer numbers them
(`ooxml_store::Version::number`): the state opened 0, a step one more than the highest number
given, a plain undo or redo the number of the state it brings back; given in publish order in the
store's commit section. `Store::numbered(n)` finds the head-most kept state with a number. The
session's `Opened::number` reads it.
A step group is one step and one number (lane U2): its commits carry one more than the highest
number given when it opened, given for good only when it closes having made a step; a revert of
all its commits brings back the number it opened on.
