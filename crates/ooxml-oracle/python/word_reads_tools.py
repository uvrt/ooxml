"""``describe`` and ``word_read`` for ``word_reads_diff.py``: docx-agent's toolbox against the
Rust tools (``word-reads``), case by case.  ``describe``'s fields the Rust core does not give
yet (``ooxml_tools_word::NOT_DESCRIBED``) and the application-only ``memory`` are left out of
the comparison; every other field, and every ``word_read`` reply, must be equal."""

from __future__ import annotations

import json

#: describe's data keys the Rust core does not give (and why: ooxml_tools_word::NOT_DESCRIBED).
NOT_DESCRIBED = {"pages", "problems"}

#: word_read's cases: (name, arguments beyond doc).
READS = (("final", {}), ("markup", {"view": "markup"}), ("original", {"view": "original"}),
         ("all", {"stories": "all"}))


def _toolbox():
    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    return Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS)


def python_replies(toolbox, path, what: str) -> dict:
    session = toolbox.session()
    doc = session.open(path.read_bytes(), name=path.name)
    out = {}
    try:
        cases = [("describe", "describe", {})] if what == "describe" else \
            [(name, "word_read", arguments) for name, arguments in READS]
        for name, tool, arguments in cases:
            reply = json.loads(json.dumps(toolbox.dispatch(session, tool, {"doc": doc, **arguments}).to_json()))
            out[name] = (tool, arguments, reply)
    finally:
        session.close(doc)
    return out


def comparable(tool: str, reply: dict) -> dict:
    if reply.get("error") is not None:
        return {"error": {"code": reply["error"].get("code"), "message": reply["error"].get("message")}}
    data = reply.get("data")
    if tool == "describe":
        data = {k: v for k, v in data.items() if k not in NOT_DESCRIBED}
        revisions = data.get("revisions")
        if isinstance(revisions, dict):
            data["revisions"] = {k: v for k, v in revisions.items() if k != "changes"}
    out = {"summary": reply.get("summary"), "data": data}
    if tool == "word_read":
        out["next_cursor"] = reply.get("next_cursor")
    return out


def check_tools(args, run_rust, report, files_of) -> int:
    toolbox = _toolbox()
    expected, manifest = {}, []
    for path in files_of(args):
        try:
            replies = python_replies(toolbox, path, args.what)
        except Exception as error:  # noqa: BLE001 -- a test instrument: say what failed
            print(f"{path}: python failed: {error}")
            continue
        for case, (tool, arguments, reply) in replies.items():
            expected[(str(path), case)] = comparable(tool, reply)
            manifest.append({"file": str(path), "case": case, "tool": tool, "arguments": {"doc": "d1", **arguments}})
    rust = {}
    for key, line in run_rust(args.rust, manifest).items():
        reply = line["reply"]
        tool = next(m["tool"] for m in manifest if (m["file"], m["case"]) == key)
        rust[key] = comparable(tool, reply)
    return report(expected, rust)
