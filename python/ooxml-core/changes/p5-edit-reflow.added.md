Lane P5m: `Session.edit` measures the reflow a changing call's checks report, after the call's
edits, where the options' page engine laid out the version its first edit was applied to (or the
document's last check saw it): docx-agent's `reflow_json` form, `"stale: call check"` past 20
pages, or a `timeout: …` where laying the new version out passed the call's `layout_timeout`. It
is the fifth item of what the call made (JSON text, `None` where none was measured).
Lane P5o: the measure never waits for the page engine and is never dropped for it: where another
call holds the engine, the version before and the version after are laid out on a spare engine of
its kind (`WordPages`' seed: the same fonts, options and budgets, nothing laid out) and compared
there; what the engine laid out last is read from that seed without its lock.
