Lane CUT2: the module is the Rust session's API alone. The prototype's functions and classes
are gone with the crates behind them (`crates/core`, `crates/word`): `Snap`, `Owner`,
`SnapshotE`, `Parser`, `open_document`, `commit`, `call`, `close_document`, `check`,
`field_pages`, `documents`, `spin`, `spin_held`, `noop`, `opcodes`, the tree and body digests
(`tree_digests`, `unit_starts`, `index_walk`, `index_walks`, `body_others`, `BodySpans`,
`body_blocks`, `common_ends`, `common_prefix_of`, `common_suffix_of`, `Starts`, `write_plan`,
`write_assemble`). `Session.number_version` is gone too: the session numbers each step it
ends (a call, a step group, an undo or redo, a revert) in its record of retired ids itself.
