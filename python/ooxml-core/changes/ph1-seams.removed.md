Architecture review phase 1 (lane ph1-seams): the host seams the port left are gone.
`Session.set_last_check` (a check the host made itself: every check is the session's),
`Version.ids_since` and `Version.changed_since` (what a Python view of the document followed) and the `updates` count in
`Session.stats()` (commits of parts through the bridge). `Session.last_check` and
`Session.last_laid` give the version they saw as a number, never `None`. `tool_hosting(kind)`
no longer says `served` for each tool: every Word tool is the session's; it says what a tool's
calls read from the host (`blobs`, `layout`, `keeps_written`).
Instead: nothing replaces `set_last_check`, `ids_since` and `changed_since` (`parts_changed_since` names the parts) (no host check or view is left to
use them); read `served` as true for every tool `tool_specs` lists.
