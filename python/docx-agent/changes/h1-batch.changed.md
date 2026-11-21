Lane H1: the ops of a `batch` are served by the Rust session (the "batch op" fallback is
gone). The batch is one step group of the session's (`ooxml_core.Session.open_group`): one
undo step, all or none where atomic, and a failed op taken back alone where not. Its reads
read what its earlier ops made, and a Python op's change is in the session before the next
op runs. Every session version of a step made of several is numbered as the step
(`rust.reads.Mirror.keep(first=...)`), so the ids its earlier versions retired are in the
record (`retired_ids`).
