"""The host goldens' generated streams (Phase 3, lane ph3-diff F2): each feature of the host --
what ``ooxml-host`` takes over from the Python host -- driven through docx-agent's Toolbox (and
xlsx-agent's, for a workbook the Python host holds), recorded by host_goldens' hook.

A *scenario* is one session: a function given a :class:`Run` (its toolbox, session and calls).
Scenarios are grouped by feature; each feature is one stream, ``docx/features/<feature>`` (the
``docx`` kit) or ``mixed/features/<feature>`` (``docx+xlsx``).  ``host_goldens.py
record-features OUT`` records them all, ``bless`` keeps what replays.

The calls are a model's: mistakes included (a misspelt ref, an unknown tool, a refused op),
since the host's answer to a mistake is as much its contract as its answer to a good call.
Documents come from the repository's fixtures, or are made here (a workbook, a blank
document), so nothing new is kept but what a scenario makes.
"""

from __future__ import annotations

import random
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import host_goldens

FIXTURES = host_goldens.ORACLE / "fixtures"
FIVE_PAGES = FIXTURES / "wordto" / "sample-5pages.docx"
WITH_TABLE = FIXTURES / "wordto" / "sample-with-table.docx"
ONE_PAGE = FIXTURES / "wordto" / "sample-1page.docx"


@dataclass
class Scenario:
    feature: str
    name: str
    run: Callable[["Run"], None]
    kit: str = "docx"
    toolbox: dict = field(default_factory=dict)
    session: dict = field(default_factory=dict)


SCENARIOS: list[Scenario] = []


def scenario(feature: str, kit: str = "docx", toolbox: dict | None = None, session: dict | None = None):
    def register(fn: Callable[["Run"], None]) -> Callable[["Run"], None]:
        SCENARIOS.append(Scenario(feature, fn.__name__, fn, kit, toolbox or {}, session or {}))
        return fn
    return register


class Run:
    """One scenario's toolbox and session, and its calls."""

    def __init__(self, box: Any, session: Any) -> None:
        self.box = box
        self.s = session

    def open(self, path: Path, name: str | None = None) -> str:
        return self.s.open(path.read_bytes(), name or path.name)

    def open_bytes(self, data: bytes, name: str) -> str:
        return self.s.open(data, name)

    def blob(self, data: bytes, name: str) -> str:
        return self.s.add_blob(data, name)

    def call(self, tool: str, arguments: Any = None, *, on: Any = None, **kw: Any) -> Any:
        return self.box.dispatch(on or self.s, tool, {} if arguments is None else arguments, **kw)

    def ops(self, *ops: tuple[str, dict], atomic: bool | None = None, on: Any = None) -> Any:
        arguments: dict = {"ops": [{"tool": tool, "arguments": args} for tool, args in ops]}
        if atomic is not None:
            arguments["atomic"] = atomic
        return self.call("batch", arguments, on=on)


def _limits(given: dict) -> Any:
    from ooxml_edit.tools.limits import Limits

    return Limits(**given)


def run_scenario(item: Scenario) -> None:
    tools, formats, groups = host_goldens.kit_parts(item.kit)
    from ooxml_edit.tools import Toolbox

    settings = dict(item.toolbox)
    if "limits" in settings:
        settings["limits"] = _limits(settings["limits"])
    with Toolbox(tools, formats=formats, groups=groups, workers=0, **settings) as box:
        session_settings = dict(item.session)
        if "limits" in session_settings:
            session_settings["limits"] = _limits(session_settings["limits"])
        session = box.session(**session_settings)
        item.run(Run(box, session))


def blank() -> bytes:
    import ooxml_core

    return bytes(ooxml_core.new_document(page=("A4", False), language="en-US", separators=(".", ","),
                                         properties=(None, None, "2026-11-23T09:00:00Z")))


def workbook() -> bytes:
    from xlsx_agent import Workbook

    return Workbook.from_tables({
        "Sales": {"columns": ["Region", "Quarter", "Units", "Revenue"], "table": "Sales", "rows": [
            ["North", "Q1", 120, 3600.5], ["North", "Q2", 135, 4050.0], ["South", "Q1", 80, 2400.0],
            ["South", "Q2", 95, 2850.25], ["East", "Q1", 60, 1800.0], ["East", "Q2", 70, 2100.0]]},
        "Notes": {"columns": ["Ref", "Body"], "table": "Notes", "rows": [
            ["N-1", "Revenue rose in every region."], ["N-2", "East opened in Q1."]]}}).to_bytes()


# -- refs ------------------------------------------------------------------------------------


@scenario("refs")
def refs_defined_used_folded_and_misspelt(r: Run) -> None:
    r.open(FIVE_PAGES)
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "## Summary\n\nOne line.", "ref": "summary"})
    r.call("word_read", {"doc": "d1", "range": "$summary"})
    r.call("word_read", {"doc": "d1", "range": "$Summary"})          # folded
    r.call("word_read", {"doc": "d1", "range": "$sumary"})           # misspelt
    r.call("word_read", {"doc": "d1", "range": "$nothing"})
    r.call("word_set_text", {"doc": "d1", "target": "$summary", "text": "Summary, again"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "after:$summary", "markdown": "Below it.", "ref": "below"})
    r.call("word_read", {"doc": "d1", "range": "$summary..$below"})


@scenario("refs")
def refs_and_undo(r: Run) -> None:
    r.open(FIVE_PAGES)
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "First new.", "ref": "first"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Second new.", "ref": "second"})
    r.call("undo", {"doc": "d1"})
    r.call("word_read", {"doc": "d1", "range": "$second"})          # its block was undone
    r.call("word_read", {"doc": "d1", "range": "$first"})
    r.call("undo", {"doc": "d1", "redo": True})
    r.call("word_read", {"doc": "d1", "range": "$second"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Again.", "ref": "first"})   # redefined
    r.call("word_read", {"doc": "d1", "range": "$first"})


@scenario("refs")
def refs_owned_by_views_and_scoped_undo(r: Run) -> None:
    r.open(FIVE_PAGES)
    writer = r.s.view(context={"agent": "writer"})
    other = r.s.view(context={"agent": "other"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Writer's line.", "ref": "mine"}, on=writer)
    r.call("word_read", {"doc": "d1", "range": "$mine"}, on=writer)
    r.call("word_read", {"doc": "d1", "range": "$mine"}, on=other)
    r.call("word_set_text", {"doc": "d1", "target": "$mine", "text": "Other's change"}, on=other)
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Other's line.", "ref": "mine"}, on=other)
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Writer again."}, on=writer)
    r.call("undo", {"doc": "d1"}, on=other)       # the other's latest, not the writer's
    r.call("word_read", {"doc": "d1", "range": "$mine"}, on=writer)
    r.call("undo", {"doc": "d1", "steps": 2}, on=writer)
    r.call("describe", {"doc": "d1"})


# -- datasets, sources and facts from a workbook ------------------------------------------------


@scenario("datasets", kit="docx+xlsx")
def datasets_derived_and_written(r: Run) -> None:
    r.open_bytes(blank(), "report.docx")
    r.open_bytes(workbook(), "sales.xlsx")
    r.call("xlsx_derive", {"doc": "d2", "from": "t:Sales", "id": "by_region", "group_by": ["Region"],
                           "measures": [{"fn": "sum", "column": "Units", "as": "Units"},
                                        {"fn": "sum", "column": "Revenue", "as": "Revenue"}]})
    r.call("xlsx_derive", {"doc": "d2", "from": "t:Sales", "id": "top", "group_by": ["Region"],
                           "measures": [{"fn": "sum", "column": "Revenue", "as": "Revenue"}],
                           "sort": [{"column": "Revenue", "desc": True}], "top": 2})
    r.call("xlsx_derive", {"doc": "d2", "from": "d:by_region", "id": "worst", "sort": [{"column": "Nope"}]})
    r.call("xlsx_query", {"table": "d:by_region"})
    r.call("xlsx_query", {"table": "d:by_regoin"})                                   # misspelt dataset
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "d:by_region", "ref": "table"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end",
                                    "markdown": "North sold {{d:by_region/North/Units}} units for "
                                                "{{d:by_region/North/Revenue}}."})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "d:nothing"})
    r.call("check", {"doc": "d1", "include": ["numbers"]})
    made = r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "| Region | Units |\n| --- | --- |\n"
                                           "| North | 0 |\n| South | 0 |\n| East | 0 |"})
    table = next((c for c in made.created if c.startswith("t:")), "t@body/1") if made.ok else "t@body/1"
    r.call("word_edit_table", {"doc": "d1", "table": table, "action": "set_cells", "from": "d:by_region",
                               "from_columns": ["Region", "Units"], "index": 1})
    r.call("word_edit_table", {"doc": "d1", "table": table, "action": "set_cells", "from": "d:by_region",
                               "from_columns": ["Region", "Units"], "index": 1, "header": [], "grow": True})
    r.call("check", {"doc": "d1", "include": ["numbers"]})
    r.call("xlsx_trace", {"doc": "d2", "rows": ["d:top/#1"]})


@scenario("datasets", kit="docx+xlsx")
def sources_named_by_table_and_cell(r: Run) -> None:
    r.open_bytes(blank(), "report.docx")
    r.open_bytes(workbook(), "sales.xlsx")
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "t:Sales", "from_columns": ["Region", "Units"]})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "t:Notes/#2/Body"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "t:Notes/N-1/Body"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "from": "t:Nope"})
    r.call("xlsx_read", {"doc": "d2", "target": "Sales!A1:D3", "sources": True})
    r.call("xlsx_derive", {"doc": "d2", "from": "t:Sales", "id": "units", "group_by": ["Region"],
                           "measures": [{"fn": "sum", "column": "Units", "as": "Units"}]})
    r.call("xlsx_read", {"doc": "d2", "target": "d:units", "sources": True})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "South: {{d:units/South/Units}} units.\n\n"
                                    "Note: {{t:Notes/#2/Body}}", "ref": "facts"})
    r.call("word_read", {"doc": "d1", "range": "$facts"})
    r.call("check", {"doc": "d1", "include": ["numbers"], "range": ["$facts"]})
    r.call("describe", {"doc": "d1"})


@scenario("datasets", kit="docx+xlsx")
def recorded_values_and_a_table_synced(r: Run) -> None:
    r.open_bytes(blank(), "report.docx")
    r.open_bytes(workbook(), "sales.xlsx")
    r.call("xlsx_record_values", {"doc": "d2", "table": "t:Notes", "key": "Ref", "source": "Body", "column": "Tone",
                                  "id": "tone", "values": [{"key": "N-1", "value": "good"},
                                                           {"key": "N-2", "value": "neutral"}]})
    r.call("xlsx_query", {"table": "d:tone"})
    r.call("xlsx_derive", {"doc": "d2", "from": "t:Sales", "id": "q", "group_by": ["Region"],
                           "measures": [{"fn": "sum", "column": "Units", "as": "Units"}]})
    made = r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "| Region | Units |\n| --- | --- |\n"
                                           "| North | 1 |\n| West | 2 |"})
    table = next((c for c in made.created if c.startswith("t:")), "t@body/1") if made.ok else "t@body/1"
    r.call("xlsx_set_cells", {"doc": "d2", "cells": [{"address": "Sales!C2", "number": 150}]})
    r.call("xlsx_derive", {"doc": "d2", "from": "t:Sales", "id": "q", "group_by": ["Region"],
                           "measures": [{"fn": "sum", "column": "Units", "as": "Units"}]})
    r.call("word_edit_table", {"doc": "d1", "table": table, "action": "sync", "from": "d:q", "key": "Region"})
    r.call("word_edit_table", {"doc": "d1", "table": table, "action": "sync", "from": "d:q", "key": "Region",
                               "remove": True})
    r.call("word_edit_table", {"doc": "d1", "table": table, "action": "sync", "from": "d:q", "key": "Region"})
    r.call("undo", {"doc": "d2"})
    r.call("check", {"doc": "d1", "include": ["numbers"]})


# -- batch -----------------------------------------------------------------------------------


@scenario("batch")
def batch_partial_and_atomic(r: Run) -> None:
    r.open(FIVE_PAGES)
    good = ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Added.", "ref": "added"})
    bad = ("word_set_text", {"doc": "d1", "target": "p:00000000", "text": "No such paragraph"})
    after = ("word_set_text", {"doc": "d1", "target": "$added", "text": "Added, changed."})
    r.ops(good, bad, after)
    r.ops(good, bad, after, atomic=True)
    r.call("word_read", {"doc": "d1", "range": "$added"})
    r.call("undo", {"doc": "d1"})                                      # the batch is one step
    r.ops()                                                            # nothing given
    r.ops(("word_read", {"doc": "d1", "range": "p@body/1"}), ("describe", {"doc": "d1"}))   # reads only


@scenario("batch")
def batch_trailing_reads_and_failed_op_refs(r: Run) -> None:
    r.open(FIVE_PAGES)
    r.ops(("word_insert_markdown", {"doc": "d1", "at": "after:p:00000000", "markdown": "Lost.", "ref": "lost"}),
          ("word_set_text", {"doc": "d1", "target": "$lost", "text": "Uses the failed op's ref"}),
          ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Kept.", "ref": "kept"}),
          ("word_read", {"doc": "d1", "range": "$kept"}),
          ("find_text", {"doc": "d1", "text": "Kept"}))
    r.ops(("word_set_text", {"doc": "d1", "target": "$kept", "text": "Kept, edited."}),
          ("word_read", {"doc": "d1", "range": "$kept"}),
          ("render", {"doc": "d1", "pages": [1]}))                    # render cannot run in a batch
    r.ops(("no_such_tool", {"doc": "d1"}), ("word_add_paragraph", {"doc": "d1", "text": "hint"}))


@scenario("batch", toolbox={"limits": {"max_batch_ops": 3}})
def batch_max_ops(r: Run) -> None:
    r.open(ONE_PAGE)
    op = ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "x"})
    r.ops(op, op, op)
    r.ops(op, op, op, op)
    r.call("read_batch", {"ops": [{"tool": "describe", "arguments": {"doc": "d1"}}] * 4})


@scenario("batch", toolbox={"limits": {"batch_timeout": 0.0}})
def batch_timeout(r: Run) -> None:
    r.open(ONE_PAGE)
    op = ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "late"})
    r.ops(op, op, op)
    r.ops(op, op, atomic=True)
    r.call("read_batch", {"ops": [{"tool": "describe", "arguments": {"doc": "d1"}}] * 2})


@scenario("batch")
def batch_multi_document(r: Run) -> None:
    r.open(ONE_PAGE)
    r.open(WITH_TABLE)
    r.ops(("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "In one.", "ref": "one"}),
          ("word_insert_markdown", {"doc": "d2", "at": "end", "markdown": "In two.", "ref": "two"}),
          ("word_read", {"doc": "d1", "range": "$one"}),
          ("word_read", {"doc": "d2", "range": "$two"}))
    r.ops(("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Atomic one."}),
          ("word_set_text", {"doc": "d2", "target": "p:00000000", "text": "fails"}), atomic=True)
    r.ops(("word_insert_markdown", {"doc": "d3", "at": "end", "markdown": "No such document."}),
          ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "Still runs."}))
    r.call("undo", {"doc": "d2"})
    r.call("describe", {"doc": "d1"})
    r.call("describe", {"doc": "d2"})


# -- read_batch ------------------------------------------------------------------------------


@scenario("read_batch", toolbox={"limits": {"max_result_chars": 2000, "max_read_batch_chars": 4000}})
def read_batch_budgets_and_resend(r: Run) -> None:
    r.open(FIVE_PAGES)
    reads = [{"tool": "word_read", "arguments": {"doc": "d1", "range": f"p@body/{n}"}} for n in range(1, 8)]
    big = [{"tool": "word_read", "arguments": {"doc": "d1"}} for _ in range(4)]
    r.call("read_batch", {"ops": reads})
    first = r.call("read_batch", {"ops": big})
    resend = (first.data or {}).get("resend") if first.ok and isinstance(first.data, dict) else None
    if resend:
        r.call(resend["tool"], resend.get("arguments") or {})
    r.call("read_batch", {"ops": [{"tool": "word_set_text", "arguments": {"doc": "d1", "target": "p@body/1",
                                                                         "text": "a change"}}]})
    r.call("read_batch", {"ops": [{"tool": "render", "arguments": {"doc": "d1", "pages": [1], "width": 400}},
                                  {"tool": "check", "arguments": {"doc": "d1"}}]})
    r.call("read_batch", {"ops": []})


# -- undo and redo ---------------------------------------------------------------------------


@scenario("undo")
def undo_redo_and_limits(r: Run) -> None:
    r.open(FIVE_PAGES)
    for n in range(3):
        r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": f"Step {n}."})
    r.call("undo", {"doc": "d1", "steps": 2})
    r.call("undo", {"doc": "d1", "redo": True})
    r.call("undo", {"doc": "d1", "steps": 99})
    r.call("undo", {"doc": "d1", "steps": 10})
    r.call("undo", {"doc": "d1", "redo": True, "steps": 10})
    r.call("undo", {"doc": "d1", "redo": True})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "After the redo."})
    r.call("undo", {"doc": "d1", "redo": True})                         # nothing to redo now
    r.call("undo", {"doc": "d9"})


@scenario("undo")
def undo_entangled_between_callers(r: Run) -> None:
    r.open(FIVE_PAGES)
    a = r.s.view(context={"agent": "a"})
    b = r.s.view(context={"agent": "b"})
    r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "A's paragraph.", "ref": "a_par"}, on=a)
    r.call("word_set_text", {"doc": "d1", "target": "$a_par", "text": "B edits A's paragraph."}, on=b)
    r.call("undo", {"doc": "d1"}, on=a)                    # A's step: B's later step rests on it
    r.call("undo", {"doc": "d1"}, on=b)
    r.call("undo", {"doc": "d1"}, on=a)
    r.call("undo", {"doc": "d1", "redo": True}, on=a)
    r.s.undo("d1")                                         # the application's undo: the latest
    a.undo("d1")
    r.call("describe", {"doc": "d1"})


# -- roles and views -------------------------------------------------------------------------


@scenario("roles", session={"roles": ["review", "resolve", "accept", "edit"]})
def roles_through_views(r: Run) -> None:
    r.open(FIVE_PAGES)
    reviewer = r.s.view(roles=["review"], context={"agent": "reviewer"})
    resolver = r.s.view(roles=["review", "resolve"], context={"agent": "resolver"})
    r.call("word_set_tracking", {"doc": "d1", "on": True, "author": "Writer"})
    r.call("word_set_text", {"doc": "d1", "target": "p@body/1", "text": "Tracked change"})
    r.call("word_set_text", {"doc": "d1", "target": "p@body/1", "text": "Refused"}, on=reviewer)
    r.call("word_comments", {"doc": "d1", "action": "add", "items": [{"target": "p@body/1", "text": "Why?"}]},
           on=reviewer)
    r.call("word_comments", {"doc": "d1", "action": "list"}, on=reviewer)
    r.call("word_changes", {"doc": "d1", "action": "accept_all"}, on=reviewer)
    r.call("word_changes", {"doc": "d1", "action": "accept"}, on=reviewer)
    r.call("word_changes", {"doc": "d1", "action": "list"}, on=resolver)
    r.ops(("word_comments", {"doc": "d1", "action": "add", "items": [{"target": "p@body/2", "text": "ok"}]}),
          ("word_set_text", {"doc": "d1", "target": "p@body/2", "text": "no"}), on=reviewer)
    r.call("word_read", {"doc": "d1", "range": "p@body/1"}, on=reviewer, roles=["review"])
    r.call("word_set_text", {"doc": "d1", "target": "p@body/1", "text": "narrowed"}, roles=["review"])


@scenario("roles", session={"roles": ["review"]})
def a_review_session(r: Run) -> None:
    r.open(ONE_PAGE)
    r.call("describe", {"doc": "d1"})
    r.call("word_set_text", {"doc": "d1", "target": "p@body/1", "text": "no"})
    r.call("word_insert_text", {"doc": "d1", "items": [{"target": "p@body/1", "where": "after", "text": "x"}]})
    r.call("undo", {"doc": "d1"})
    r.call("save_document", {"doc": "d1", "name": "reviewed.docx", "format": "docx"})


# -- aliases, withdrawn tools and hints -------------------------------------------------------


@scenario("names")
def aliases_namespaces_withdrawn_and_hints(r: Run) -> None:
    r.open(ONE_PAGE)
    r.call("functions.describe", {"doc": "d1"})
    r.call("replace_text", {"doc": "d1", "find": "the", "replace": "the"})
    r.call("word_replace_text", {"doc": "d1", "find": "the", "replace": "the"})
    r.call("edit_smartart", {"doc": "d1"})
    r.call("word_find_asset", {"doc": "d1"})
    r.call("word_add_image", {"doc": "d1"})
    r.call("word_add_table", {"doc": "d1"})
    r.call("word_add_page_break", {"doc": "d1"})
    r.call("word_red", {"doc": "d1"})
    r.call("totally_unknown", {"doc": "d1"})
    r.ops(("functions.word_read", {"doc": "d1", "range": "p@body/1"}), ("edit_smartart", {"doc": "d1"}))
    r.call("read_batch", {"ops": [{"tool": "word_add_image", "arguments": {"doc": "d1"}},
                                  {"tool": "describe", "arguments": {"doc": "d1"}}]})


@scenario("names")
def unknown_arguments_named(r: Run) -> None:
    r.open(ONE_PAGE)
    r.call("word_read", {"doc": "d1", "rnage": "p@body/1"})
    r.call("word_set_text", {"doc": "d1", "target": "p@body/1", "txt": "x"})
    # Two unknown arguments: today's host names the first in a set's order, which Python's
    # hash seed decides (host/known-differences.toml); the Rust host names them in call order.
    r.call("word_style", {"doc": "d1", "style": "Heading 2", "targets": ["p@body/1"]})


# -- retry keys ------------------------------------------------------------------------------


@scenario("keys")
def retry_keys(r: Run) -> None:
    r.open(ONE_PAGE)
    insert = {"doc": "d1", "at": "end", "markdown": "Once only.", "key": "k-1"}
    r.call("word_insert_markdown", insert)
    r.call("word_insert_markdown", insert)                             # a retry: nothing new
    r.call("word_insert_markdown", {**insert, "markdown": "Other text, same key."})
    r.call("word_insert_markdown", {**insert, "key": "k-2"})
    r.ops(("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "In a batch.", "key": "k-3"}),
          ("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": "In a batch.", "key": "k-3"}))
    r.call("undo", {"doc": "d1"})
    r.call("word_insert_markdown", {**insert, "key": "k-2"})          # after its undo
    r.call("describe", {"doc": "d1"})


# -- the image budget ------------------------------------------------------------------------


@scenario("images", toolbox={"limits": {"image_budget": 3, "image_budget_per_round": 2,
                                         "max_images_per_call": 2}})
def image_budget(r: Run) -> None:
    r.open(FIVE_PAGES)
    r.call("render", {"doc": "d1", "pages": [1], "width": 300, "fast": True})
    r.call("render", {"doc": "d1", "pages": [1, 2, 3], "width": 300, "fast": True})   # over per call
    r.call("render", {"doc": "d1", "pages": [2], "width": 300, "fast": True})        # over the round
    r.s.new_round()
    r.call("render", {"doc": "d1", "pages": [2], "width": 300, "fast": True})
    r.call("render", {"doc": "d1", "pages": [3], "width": 300, "fast": True})        # over the session
    r.call("read_batch", {"ops": [{"tool": "render", "arguments": {"doc": "d1", "pages": [1], "width": 300,
                                                                   "fast": True}}]})


# -- concurrency: random writers with batches ---------------------------------------------------


def random_writers(r: Run, seed: int, writers: int = 4, rounds: int = 6) -> None:
    """Writers on threads, each a seeded random mix of lone edits, batches (some atomic, some
    with a failing op) and reads of its own paragraph (no undo: an undo beside other changes
    leaves no commit order to replay, host_goldens._commit_order): a concurrent case, whose final state a
    serial replay of its changes in commit order must give (host_goldens.expected_of)."""
    r.open(FIVE_PAGES)
    made = [r.call("word_insert_markdown", {"doc": "d1", "at": "end", "markdown": f"Writer {n}'s paragraph.",
                                            "ref": f"w{n}"}) for n in range(writers)]
    targets = [next((c for c in m.created if c.startswith("p:")), "p@body/1") for m in made]
    errors: list[BaseException] = []
    start = threading.Barrier(writers)      # all at once, so their calls overlap

    def work(n: int) -> None:
        rng = random.Random(seed * 100 + n)
        view = r.s.view(context={"agent": f"w{n}"})
        try:
            start.wait(timeout=60)
            for k in range(rounds):
                pick = rng.random()
                text = f"Writer {n}, round {k}"
                if pick < 0.35:
                    r.call("word_set_text", {"doc": "d1", "target": targets[n], "text": text}, on=view)
                elif pick < 0.7:
                    ops = [("word_set_text", {"doc": "d1", "target": targets[n], "text": text}),
                           ("word_insert_text", {"doc": "d1", "items": [{"target": targets[n], "where": "after",
                                                                       "text": f" ({k})"}]})]
                    if rng.random() < 0.4:
                        ops.insert(1, ("word_set_text", {"doc": "d1", "target": "p:00000000", "text": "x"}))
                    r.ops(*ops, atomic=rng.random() < 0.5, on=view)
                else:
                    r.call("word_read", {"doc": "d1", "range": targets[n]}, on=view)
        except BaseException as exc:  # noqa: BLE001 -- reported by the scenario
            errors.append(exc)

    threads = [threading.Thread(target=work, args=(n,)) for n in range(writers)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=600)
    if errors:
        raise errors[0]


for _seed in (1, 2, 3):
    def _writers(r: Run, seed: int = _seed) -> None:
        random_writers(r, seed)
    _writers.__name__ = f"random_writers_with_batches_{_seed}"
    scenario("concurrency")(_writers)
