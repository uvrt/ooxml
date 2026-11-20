"""docx-agent's editing tools answered by the Rust session that owns the document (phase P2;
lane P2c4: the session is the only owner, :mod:`.owner`; lane P2c6: side by side).

A tool with a Rust editing handler (:data:`SERVED`: ``word_set_text``, ``replace_text``,
``word_insert_text``, ``word_delete``, ``word_format``, ``word_lists``, ``word_format_table``,
``word_styles``, ``word_follow_sample``, ``word_insert_markdown``, ``word_fields``,
``word_headers_footers``, ``word_template``, ``word_move``, ``word_sections``, ``word_notes``,
``word_links``, and each one ported after them)
runs there, ahead of docx-agent's handler and its wrapper
(:func:`docx_agent.tools._base.wrap`): the call's arguments, its ``$name`` refs resolved, go to
``Session.edit`` with the call's caller and the document's tracking mode; the versions the
call's edits made are the call's step (:meth:`~ooxml_edit.tools.owned.OwnedEntry.served`,
numbered in the order the session made them), its reply the result.  The Python view of the
document is not read.

Such a call runs without the document's lock (:func:`serve_owned`, the format's ``serve``:
:mod:`ooxml_edit.tools.served`), beside the others the session serves -- the session prepares
each edit on a snapshot and commits it in a short section of its own.  Where its checks
compare layouts (Python lays them out) it runs under the lock (:func:`serve`).

The Python handler answers instead -- on the view, committed through the bridge, counted by
tool and reason (:func:`fallbacks`, ``edit_fallbacks{tool,reason}``), never silently -- where
the session does not answer as it yet: an op of a ``batch`` (one step), a call on a draft,
values written by reference, a ref name that would be folded, a refusal (the session's carries its own way out, tool rule 2,
which today's does not), a session given its own clock (the Rust session dates revisions by
the system's), and what the session says it leaves to the host (its ``unported``: a find
within a block, a picture, regex, ...).  The Rust handler changes nothing in each of those.
"""

from __future__ import annotations

import json
import threading
import time
from collections import Counter
from typing import Any, Callable

from ooxml_edit.tools import Result
from ooxml_edit.tools.session import REF
from ooxml_edit.tools.session import StepOwner
from ooxml_edit.tools.session import utc_now

from . import reads
from .owner import context_of

_COUNTS_LOCK = threading.Lock()
_FALLBACKS: Counter = Counter()
_SERVED: Counter = Counter()


def fallbacks() -> dict[str, dict[str, int]]:
    """``edit_fallbacks{tool,reason}``: the editing calls of a ported tool answered by the
    Python handler, by tool and reason, since the process started."""
    with _COUNTS_LOCK:
        out: dict[str, dict[str, int]] = {}
        for (tool, reason), n in sorted(_FALLBACKS.items()):
            out.setdefault(tool, {})[reason] = n
        return out


def served() -> dict[str, int]:
    """The editing calls answered by the Rust session, by tool, since the process started."""
    with _COUNTS_LOCK:
        return dict(_SERVED)


def _set_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {"target": reads._refs(call, arguments.get("target")), "text": arguments.get("text"),
            "paragraph_style": arguments.get("paragraph_style"),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


def _replace_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "replace": arguments.get("replace"),
            "expect": arguments.get("expect"), "regex": arguments.get("regex") or None,
            "slides": arguments.get("slides"), "range": reads._refs(call, arguments.get("range")),
            "stories": arguments.get("stories", "body")}


def _insert_text(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    items = arguments.get("items")
    return {**{k: arguments.get(k) for k in ("find", "where", "at", "text", "ref")},
            "target": reads._refs(call, arguments.get("target")),
            "within": reads._refs(call, arguments.get("within")),
            "items": None if items is None else [
                {**item, "target": reads._refs(call, item.get("target"))} if isinstance(item, dict) else item
                for item in items]}


def _delete(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"find": arguments.get("find"), "within": reads._refs(call, arguments.get("within")),
            "target": reads._refs(call, arguments.get("target")),
            "collapse_space": arguments.get("collapse_space"), "force": arguments.get("force")}


def _refs_in(call: Any, value: Any) -> Any:
    """A list of addresses (or one) with its ``$name`` refs resolved."""
    if isinstance(value, list):
        return [reads._refs(call, item) for item in value]
    return reads._refs(call, value)


def _own(arguments: dict[str, Any]) -> dict[str, Any]:
    """The call's arguments but its document (the session's id goes in its place)."""
    return {k: v for k, v in arguments.items() if k != "doc"}


def _format(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    if arguments.get("keep_together"):
        # Blocks kept together are measured against a page (Python lays them out).
        raise _Fallback("keep_together")
    find = arguments.get("find")
    finds = find if isinstance(find, list) else None if find is None else [find]
    return {**_own(arguments), "targets": _refs_in(call, arguments.get("targets")),
            "like": reads._refs(call, arguments.get("like")),
            "find": None if finds is None else [
                {**item, "within": reads._refs(call, item.get("within"))} if isinstance(item, dict) else item
                for item in finds]}


def _lists(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "targets": _refs_in(call, arguments.get("targets")),
            "join": reads._refs(call, arguments.get("join"))}


def _format_table(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "table": reads._refs(call, arguments.get("table"))}


def _styles(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return _own(arguments)


def _follow_sample(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    sample = arguments.get("sample")
    if isinstance(sample, dict):
        if sample.get("doc") not in (None, arguments.get("doc")):
            # (the session names its documents its own way: the host's ids are not its)
            raise _Fallback("a sample in another document")
        sample = {"block": reads._refs(call, sample.get("block"))}
    return {**_own(arguments), "sample": sample, "copies": _refs_in(call, arguments.get("copies"))}


def _insert_markdown(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    if getattr(call.entry.held, "min_text_size", None) is not None:
        # The text a call writes below the application's minimums is measured in Python
        # (``note_small_text``); the session has no minimums.
        raise _Fallback("a minimum text size")
    given = _own(arguments)
    blob = given.get("blob")
    if isinstance(blob, str) and given.get("markdown") is None and given.get("from") is None:
        # The blob is the host's: its text goes as the Markdown, read as the tool reads it.
        try:
            given["markdown"] = call.blob(blob).data.decode("utf-8")
        except UnicodeDecodeError:
            raise _Fallback("a blob that is not UTF-8") from None
        del given["blob"]
    given["at"] = _blocks_at(call, given.get("at"))
    return given


def _blocks_at(call: Any, at: Any) -> Any:
    """A placing tool's ``at`` with its ``$name`` refs resolved as the tool resolves them
    (``text._at``: a span's last block after it, its first before it); a ref naming a
    heading's range is the Python view's to expand."""
    if not isinstance(at, str) or "$" not in at:
        return at
    head, _, rest = at.partition(":")
    if head not in ("after", "before", "replace"):
        rest = at
    if any("h:" in call.resolve_ref(part) for part in rest.split("..") if part.startswith("$")):
        raise _Fallback("a heading's range by reference")
    from ..tools.text import _at

    return _at(call, at)


def _fields(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("after", "before", "at")},
            "ids": _refs_in(call, arguments.get("ids"))}


def _headers_footers(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "section": reads._refs(call, arguments.get("section"))}


def _template(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return _own(arguments)


def _sided(call: Any, value: Any) -> Any:
    """A place that may say its side (``before:X``, ``after:X``; docx-agent's ``_place``)
    with its ``$name`` refs resolved."""
    if isinstance(value, str):
        for side in ("after:", "before:"):
            if value.startswith(side):
                return side + reads._refs(call, value[len(side):])
    return reads._refs(call, value)


def _move(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "after": _sided(call, arguments.get("after")),
            **{k: reads._refs(call, arguments.get(k)) for k in ("section_heading", "blocks", "before",
                                                                 "after_section")}}


def _sections(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "after": _sided(call, arguments.get("after")),
            **{k: reads._refs(call, arguments.get(k)) for k in ("before", "section")}}


def _notes(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), **{k: reads._refs(call, arguments.get(k)) for k in ("at", "note")}}


def _links(call: Any, arguments: dict[str, Any]) -> dict[str, Any]:
    return {**_own(arguments), "target": reads._refs(call, arguments.get("target"))}


#: The tools the session edits with, and each one's arguments as the session takes them.
SERVED: dict[str, Callable[[Any, dict[str, Any]], dict[str, Any]]] = {
    "word_set_text": _set_text,
    "replace_text": _replace_text,
    "word_insert_text": _insert_text,
    "word_delete": _delete,
    "word_format": _format,
    "word_lists": _lists,
    "word_format_table": _format_table,
    "word_styles": _styles,
    "word_follow_sample": _follow_sample,
    "word_insert_markdown": _insert_markdown,
    "word_fields": _fields,
    "word_headers_footers": _headers_footers,
    "word_template": _template,
    "word_move": _move,
    "word_sections": _sections,
    "word_notes": _notes,
    "word_links": _links,
}

#: The served tools whose edits read the document's pages (a field's result, a header's page
#: number, the upgrade's reflow): the session lays the versions out with the document's page
#: engine (lane P2c13, :func:`_pages`).
LAID = frozenset({"word_fields", "word_headers_footers", "word_template"})


def _ref_names(arguments: dict[str, Any]) -> list[str]:
    """The ref names a call gives (``ref``, ``items[].ref``)."""
    items = arguments.get("items") if isinstance(arguments.get("items"), list) else []
    names = [arguments.get("ref"), *(item.get("ref") for item in items if isinstance(item, dict))]
    return [name for name in names if isinstance(name, str) and name]


class _Fallback(Exception):
    """The call is answered by the Python handler, for this reason."""


def owned_entry(call: Any) -> Any:
    """The call's document's entry when the session owns it and serves the call's tool."""
    entry = call.entry
    if call.tool.name not in SERVED or entry is None or getattr(entry, "owner", None) is None:
        return None
    return entry


def _why_not(call: Any, entry: Any, arguments: dict[str, Any]) -> str | None:
    if entry.live is not None or entry.pinned is not None:
        return "a draft"
    if any(not REF.match(name) for name in _ref_names(arguments)):
        # folded with a warning, or refused, by the tool layer as the Python handler names it
        return "a ref name to fold"
    if call.in_batch:
        return "batch op"
    if getattr(call, "facts", None):
        return "values by reference"
    if getattr(call.session, "clock", utc_now) is not utc_now:
        # Revisions are dated by the session's clock: the Rust session's is the system's.
        return "the session's own clock"
    return None


def will_serve(call: Any, arguments: dict[str, Any]) -> bool:
    """Whether the session is asked first for the changing call (lane P2c13): then its work
    ahead (a layout in Python, the lock let go) is not done -- the session lays out in Rust;
    where it then leaves the call to the Python handler, that lays out under the lock."""
    entry = owned_entry(call)
    if entry is None or not call.changing or call.owner_declined:
        return False
    if _why_not(call, entry, arguments) is not None:
        return False
    return call.tool.name not in LAID or _pages(entry) is not None


def _pages(entry: Any) -> Any:
    """The document's page engine (:func:`docx_agent.pages.word_pages`), or ``None``."""
    from ..pages import word_pages

    return word_pages(entry.held)[0]


def _layout(call: Any, entry: Any) -> Any:
    """The page engine a call of a :data:`LAID` tool edits with; ``None`` for the others."""
    if call.tool.name not in LAID:
        return None
    from ..pages import word_pages

    found, why = word_pages(entry.held)
    if found is None:
        raise _Fallback(f"the layout: {why}")
    return found


def _tracking(call: Any, entry: Any, context: Any, counted: bool) -> None:
    """The session's tracking mode of the document made the call's (``entry.tracking``); a
    version that made (the settings written) is a step of its own, ``word_set_tracking``'s."""
    mirror = entry.handle
    with mirror.lock:
        wanted = entry.tracking or None
        if mirror.tracking == wanted:
            return
        arguments: dict[str, Any] = {"doc": mirror.doc, "on": bool(wanted)}
        if wanted and wanted.get("author"):
            arguments["author"] = wanted["author"]
        reply, _app, made = mirror.session.edit("word_set_tracking", json.dumps(arguments), context=context)
        if made is None and json.loads(reply).get("error") is not None:
            raise _Fallback("tracking")
        mirror.tracking = wanted
    if made is not None:
        first, head, _touched, _validation = made
        entry.served(first, head, StepOwner(dict(call.caller), "word_set_tracking"), counted=counted)


def _layout_before(call: Any, entry: Any) -> Any:
    """What the call's reflow is measured from, as docx-agent's wrapper takes it -- only where
    the view is current (the version before the edit; ``entry.held``: read as it is, no step
    of the view's opened for it); ``None`` where there is none."""
    from ..tools.laid import laid_before, seen_before, state_bytes

    if not entry.view_current:
        return None
    document = entry.held
    return seen_before(document, state_bytes(call)) or laid_before(document)


def _hand_engine(call: Any, entry: Any, before: Any, changed: bool) -> None:
    """The toolbox's engine on the view as it is (``entry.held``), as docx-agent's wrapper
    engages it for a Python edit: where the call's checks compare layouts (a layout before of
    at most ``REFLOW_PAGES`` pages), handed to them -- they make the view current, after the
    step; elsewhere, where the call ``changed`` the document, for the relayout on commit its
    checks start (lane P2c13), which lets it go.  Under the document's lock."""
    from ..tools._base import engage_document, hand_to_checks

    if _compares(before):
        engage_document(call, entry.held)
        hand_to_checks(entry.held)
    elif changed:
        engage_document(call, entry.held)


def _engine(call: Any, entry: Any) -> Any:
    """The engine a call served without the lock lays its version out with on commit (lane
    P2c13): the toolbox's, as :func:`_hand_engine` engages it, not put on the view (the
    document's lock is not held)."""
    from ..tools._base import Engine

    return Engine.of(call.toolbox.pool, call.limits.layout_timeout, entry.held)


def _compares(before: Any) -> bool:
    """Whether a call's checks compare layouts, from ``before`` (Python lays them out)."""
    from ..tools._base import REFLOW_PAGES

    return before is not None and before.count <= REFLOW_PAGES


def _timing(call: Any, arguments: dict[str, Any]) -> Any:
    """The ``Timing`` of a call whose result says where its time went (round 45, R67:
    ``word_fields`` update), or ``None``."""
    from ..tools._base import Timing

    if call.tool.name == "word_fields" and arguments.get("action") == "update":
        return Timing()
    return None


def _edit(call: Any, entry: Any, arguments: dict[str, Any], counted: bool) -> tuple[Result, Any, list[Any]]:
    """The call's edit made by the session: its result, what it made (``Session.edit``'s
    ``(first, head, touched, validation)``, ``None`` where it changed nothing), and the
    markers its checks get (its ``Timing``, :func:`_timing`).  ``counted``: as for
    :meth:`~ooxml_edit.tools.owned.OwnedEntry.served`."""
    mirror = entry.handle
    tool = call.tool.name
    context = context_of(call.caller)
    _tracking(call, entry, context, counted)
    given = {"doc": mirror.doc, **{k: v for k, v in SERVED[tool](call, arguments).items()
                                   if v is not None}}
    layout = _layout(call, entry)
    timing = _timing(call, arguments)
    started = time.perf_counter()
    reply, app, made = mirror.session.edit(tool, json.dumps(given), context=context, layout=layout)
    if timing is not None:
        timing.add("update", time.perf_counter() - started)
    envelope = json.loads(reply)
    error = envelope.get("error")
    unported = json.loads(app).get("unported") or []
    if made is None and error is not None:
        raise _Fallback(f"refusal: {error.get('code')}")
    if made is None and unported:
        raise _Fallback(unported[0])
    result = _result(envelope)
    for name, address in (envelope.get("refs") or {}).items():
        call.define_ref(name, address)        # the call's refs, as docx-agent's remember
        result.refs[name] = address
    if timing is None:
        return result, made, []
    if isinstance(result.data, dict):
        result.data["timing"] = timing.parts      # (the checks add theirs)
    return result, made, [timing]


def _count(tool: str, why: str | None) -> None:
    with _COUNTS_LOCK:
        if why is None:
            _SERVED[tool] += 1
        else:
            _FALLBACKS[(tool, why)] += 1


class Made:
    """A check's marker (:attr:`~ooxml_edit.tools.served.Served.touched`): the version a
    call the session served made, and its validation (``(new, fixed, baseline)``, as
    ``Session.validation`` gives it), which its checks report; the ``engine`` its relayout on
    commit lays the version out with (lane P2c13)."""

    def __init__(self, head: Any, validation: tuple[list[str], list[str], int], engine: Any = None) -> None:
        self.head = head
        self.engine = engine
        new, fixed, baseline = validation
        self.validation = {"new": list(new), "fixed": list(fixed), "baseline": baseline}


def serve_owned(call: Any, arguments: dict[str, Any]) -> Any:
    """The format's ``serve`` (lane P2c6): the call's edit made by the session beside the
    others it serves, without the document's lock (:mod:`ooxml_edit.tools.served`).  ``None``
    where the call's checks compare layouts (Python lays them out, under the lock: the
    session edits there); a ``Declined`` where the Python handler answers (counted)."""
    from ooxml_edit.tools.served import Declined, Served

    entry = owned_entry(call)
    if entry is None:
        return None
    tool = call.tool.name
    why = _why_not(call, entry, arguments)
    current = entry.view_current
    if why is None:
        before = _layout_before(call, entry)
        if _compares(before):
            return None
        try:
            result, made, markers = _edit(call, entry, arguments, counted=True)
        except _Fallback as fallback:
            why = str(fallback)
    if why is not None:
        _count(tool, why)
        return Declined(why)
    _count(tool, None)
    touched: list[Any] = [*([_before(before)] if current else []), *markers]
    if made is None:
        return Served(result, None, None, touched)
    first, head, step, validation = made
    return Served(result, first, head, [*touched, Made(head, validation, _engine(call, entry))],
                  None if step is None else json.loads(step))


def _before(layout: Any) -> Any:
    from ..tools._base import Before

    return Before(layout)


def serve(call: Any, arguments: dict[str, Any], python: Callable[[], Any]) -> Any:
    """Under the document's lock: the call's edit made by the session, or ``python()``'s where
    the session does not answer as it yet (counted, with why).  Only for a call
    :func:`owned_entry` names.  A call the session was tried for without the lock
    (:attr:`~ooxml_edit.tools.dispatch.Call.owner_declined`) goes to ``python()`` at once."""
    entry = owned_entry(call)
    tool = call.tool.name
    if call.owner_declined:
        return python()
    try:
        why = _why_not(call, entry, arguments)
        if why is not None:
            raise _Fallback(why)
        before = _layout_before(call, entry)
        if entry.view_current:
            call.touch(_before(before))
        with entry.serving():
            result, made, markers = _edit(call, entry, arguments, counted=False)
            for marker in markers:
                call.touch(marker)
            if made is not None:
                entry.served(made[0], made[1])
        _hand_engine(call, entry, before, made is not None)
    except _Fallback as fallback:
        _count(tool, str(fallback))
        return python()
    _count(tool, None)
    return result


def _result(envelope: dict[str, Any]) -> Result:
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"))
    for key in ("changed", "created", "removed"):
        getattr(result, key).extend(envelope.get(key) or [])
    result.renamed.update(envelope.get("renamed") or {})
    result.warnings.extend(envelope.get("warnings") or [])
    return result
