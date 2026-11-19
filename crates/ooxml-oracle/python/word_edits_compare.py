"""How the Word edits' harness compares docx-agent's results with the Rust core's
(word_edits_diff.py): what a reply must say alike, and the rules under which two results
that differ in form are equal."""

from __future__ import annotations

from word_structure_cases import settle as _structure_settle


def reply(reply: dict) -> dict:
    """What a reply says that both sides must say alike: an error's code, message and
    field (the way out is Rust's rule-2 addition); else the summary, data, ids and warnings."""
    if reply.get("error"):
        error = reply["error"]
        return {"error": {k: error.get(k) for k in ("code", "message", "field")}}
    return {k: reply.get(k) for k in ("summary", "data", "changed", "created", "removed", "renamed", "warnings",
                                      "total", "next_cursor") if reply.get(k) not in (None, [], {})}


def settle(want: dict, have) -> tuple:
    """``want`` (changed in place) and ``have`` as they are compared: the Rust side's result,
    or ``want`` where the two are equal by rule; and the rule (unported, rewritten) or None."""
    if want.pop("structure", False) and isinstance(have, dict):
        _structure_settle(want, have)
    if isinstance(have, dict) and "reply" in have:
        if have.get("unported"):
            return want, "unported"     # the host answers it its own way (Done::unported)
        have["reply"] = reply(have["reply"])
        have.pop("unported", None)
        shorten(want["parts"], have["parts"])
        return have, None
    written = want.pop("touched_as_written", None)
    reserialized = want.pop("reserialized", [])
    if isinstance(have, dict) and "parts" in have and "parts" in want:
        for name in reserialized:
            if name not in have["parts"]:
                want["parts"].pop(name, None)   # no change, only lxml's serialization
                if have.get("touched") == written:
                    want["touched"] = written
    if isinstance(have, dict) and "refused" in have and "refused" in want:
        return want, None   # both refused: the messages are each side's own words
    if isinstance(have, dict) and "parts" in have and "parts" in want:
        shorten(want["parts"], have["parts"])
        if have.get("touched") != want["touched"] and have.get("touched") == written:
            want["touched"] = written
            return have, "rewritten"    # equal once the body before is as lxml writes it
    return have, None


def shorten(want: dict, have: dict, context: int = 160) -> None:
    """Two parts that differ, cut to the stretch around their first difference."""
    for name in set(want) & set(have):
        a, b = want[name], have[name]
        if a == b:
            continue
        at = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        start = max(0, at - context)
        want[name] = f"@{at}: {a[start:at + context]}"
        have[name] = f"@{at}: {b[start:at + context]}"
