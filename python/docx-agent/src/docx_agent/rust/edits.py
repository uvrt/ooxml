"""docx-agent's editing tools answered by the Rust session that owns the document.

Every editing tool (what its edits read from the host declared by the Rust tool and
exported with the registry: :func:`docx_agent.rust.served.hosting`; its arguments as the
session takes them: :func:`docx_agent.rust.served.arguments_for`) runs there: the call's arguments, its ``$name``
refs resolved, go to ``Session.edit`` with the call's caller, its date (the session clock's,
to the second) and the document's tracking mode; the versions the call's edits made are the
call's step (:meth:`~ooxml_edit.tools.owned.OwnedEntry.served`, numbered in the order the
session made them), its reply the result.

Such a call runs without the document's lock (:func:`serve_owned`, the format's ``serve``:
:mod:`ooxml_edit.tools.served`), beside the others the session serves -- the session prepares
each edit on a snapshot and commits it in a short section of its own.  What ooxml-edit runs
under the lock (a call with an idempotence key, a batch's op) is served the same way there
(:func:`serve`).

No fallback: the session's refusal is the call's answer, in docx-agent's words with its way
out (:mod:`.refusals`); a form it does not support yet says so (``details.not_supported``).
"""

from __future__ import annotations

import datetime as _dt
import json
import threading
import time
from collections import Counter
from typing import Any, Callable

from ooxml_edit.tools import Result, ToolError
from ooxml_edit.tools.session import StepOwner
from ooxml_edit.tools.session import folded_warning

from . import reads
from . import served as _served
from .owner import context_of, documents, options
from .refusals import refusal
from .served import Adapted, written_json

_COUNTS_LOCK = threading.Lock()
_SERVED: Counter = Counter()


def served() -> dict[str, int]:
    """The editing calls answered by the Rust session, by tool, since the process started."""
    with _COUNTS_LOCK:
        return dict(_SERVED)


def _count(tool: str) -> None:
    with _COUNTS_LOCK:
        _SERVED[tool] += 1


class _Blobs:
    """The session's blobs as ``CallOptions(blobs=...)`` asks them (lane P2c14): Rust calls
    back with the interpreter lock taken back, for the blob a call names."""

    def __init__(self, session: Any) -> None:
        self.session = session

    def blob(self, handle: str) -> tuple[str, str, bytes] | None:
        try:
            found = self.session.blob(handle)
        except Exception:  # noqa: BLE001 -- no such blob: the tool refuses, listing them
            return None
        return found.name, found.mime, bytes(found.data)

    def listed(self) -> list[tuple[str, str]]:
        return [(b["handle"], b["name"]) for b in self.session.describe()["blobs"]]


def _written(call: Any) -> str | None:
    """The values the dispatcher wrote into the call's arguments for references
    (``call.facts``), as ``Session.edit`` takes them; ``None`` without any."""
    return written_json(getattr(call, "facts", None) or (), {})


def _date(call: Any) -> str:
    """The moment the call's writes are dated by (a revision's, a comment's): the session's
    clock, UTC, to the second -- as docx-agent's handlers date them (lane P2e1)."""
    moment = call.now()
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=_dt.timezone.utc)
    return moment.astimezone(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def call_context(call: Any, entry: Any) -> Any:
    """The ``ooxml_core.CallContext`` a served call of ``entry`` runs with: the caller, the
    host's limits and limit on laying out, and its writing settings (dated by :func:`_date`,
    spaced unless the application's ``typography`` is off, read under the session's locale
    where it has one: else the document's own language) -- the one context of the call's edit
    and of what its adapter asks the session after it."""
    typography = reads.owned(entry).handle.settings.get("typography")
    return context_of(call.caller, call.limits, call.limits.layout_timeout, date=_date(call),
                      typography=typography is not False, locale=entry.locale)


def _kept_refs(call: Any, arguments: dict[str, Any]) -> tuple[dict[str, Any], dict[str, str]]:
    """``arguments`` with each ref name they give (``ref``, ``items[].ref``) as the call keeps it
    (ooxml-edit's ``Call.ref_name``: a camelCase or hyphenated name folded; one that folds to
    none, or to another address's ref, refused before the edit, so the refusal changes
    nothing -- lane H2), and the names folded, ``{kept: given}``.  A ref that reads as an
    address or a place (:func:`~docx_agent.tools.positions.is_place`) is left to the tool,
    which refuses it naming the argument it belongs in (``at``), as docx-agent's handler does."""
    from ..tools.positions import is_place

    folded: dict[str, str] = {}

    def kept(holder: dict[str, Any]) -> dict[str, Any]:
        name = holder.get("ref")
        if not isinstance(name, str) or not name or is_place(name):
            return holder
        ref = call.ref_name(name)
        if ref == name:
            return holder
        folded[ref] = name
        return {**holder, "ref": ref}

    out = kept(arguments)
    items = out.get("items")
    if isinstance(items, list):
        out = {**out, "items": [kept(item) if isinstance(item, dict) else item for item in items]}
    return out, folded


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
        reply, _app, made = mirror.session.edit("word_set_tracking", json.dumps(arguments), options(context=context))
        error = json.loads(reply).get("error")
        if made is None and error is not None:
            # (Never so: the host keeps only a mode word_set_tracking took, lane H2.)  The
            # session's refusal is the call's: nothing is changed.
            raise ToolError(error["code"], error["message"], field=error.get("field"),
                            valid_options=error.get("valid_options"), details=error.get("details"))
        mirror.tracking = wanted
    if made is not None:
        first, head, _touched, _validation, before, _reflow = made
        entry.served(first, head, StepOwner(dict(call.caller), "word_set_tracking"), counted=counted,
                     before=before)


def _set_mode(entry: Any, given: dict[str, Any], context: Any, names: dict[str, str]) -> tuple[str, str, Any]:
    """``word_set_tracking`` served: the session's tracking mode of the document and the
    host's (``entry.tracking``, which later calls are made under and :func:`_tracking` keeps
    the session to) set together, under the mirror's lock, where the session sets it."""
    mirror = entry.handle
    with mirror.lock:
        reply, app, made = mirror.session.edit("word_set_tracking", json.dumps(given), options(
            context=context, documents=documents(names=names)))
        if json.loads(reply).get("error") is None:
            wanted = {"on": True, "author": given.get("author")} if given.get("on") else None
            entry.tracking = wanted
            mirror.tracking = wanted
    return reply, app, made


def _host_tables(session: Any) -> Callable[[], list[tuple[str, str, str]]]:
    """The tables of the open documents the host holds itself (a workbook's ``t:stock``), as
    ``CallDocuments``' ``tables`` asks them: ``(name, doc, kind)``, the name as a ``from``
    takes it (:func:`ooxml_edit.tools.sources.offered`).  Called by the session only where a
    refusal names one (round 56, R120: a table not in the document), as docx-agent's
    ``_no_table`` reads them only then: reading another document may wait for it."""

    def tables() -> list[tuple[str, str, str]]:
        from ooxml_edit.tools import sources

        held = dict(session.documents)
        if len(held) < 2:
            return []
        found = []
        for name in sources.offered(session)[1]:
            try:
                handle = sources.qualify(name, held, session.formats)[0]
            except ToolError:
                continue
            found.append((name, handle, held[handle].kind))
        return found

    return tables


def owned_entry(call: Any) -> Any:
    """The call's document's entry, the session's (refused as ``internal`` otherwise)."""
    return reads.owned(call.entry)


def _by_reference(call: Any) -> None:
    """A tool given values by reference that does not keep them yet: refused as not
    supported (the values the dispatcher wrote are the host's, the record of where they went
    the tool's)."""
    if getattr(call, "facts", None) and call.tool.name not in _served.hosting().keeps_written:
        form = f"{call.tool.name} by reference"
        raise ToolError("refused", f"{form} is not supported yet; write the value itself",
                        details={"not_supported": form})


def _layout(call: Any, entry: Any, measured: bool) -> Any:
    """The page engine a call of a tool whose edits read the pages
    (:attr:`~docx_agent.rust.served.Hosting.laid`) edits with; for the others, where the
    call's checks take the reflow the session measures (``measured``: served without the
    lock) -- an engine the document has already, never one made for them --; else ``None``."""
    from .engines import existing_pages, session_pages

    if call.tool.name in _served.hosting().laid:
        return session_pages(entry)
    return existing_pages(entry) if measured else None


def _names(call: Any, entry: Any, arguments: dict[str, Any]) -> dict[str, str]:
    """The host's names for the documents a call names, each the session's id of it
    (``Session.edit``'s ``names``): the arguments name documents as the host does, and so
    does the reply.  Another document held as a reader of a committed version of one the
    session owns (``live``) is that document."""
    mirror = entry.handle
    names = {entry.doc_id: mirror.doc}
    for doc in call.tool.doc_ids(arguments):
        if doc in names:
            continue
        other = call.entries.get(doc)
        if other is None:
            continue                # (the tool refuses a document not open, naming those that are)
        other = reads.owned(other)
        if other.handle.session is not mirror.session:
            raise ToolError("refused", f"{doc} is held by another Rust session: open both in one",
                            details={"not_supported": "a document in another session"})
        names[doc] = other.handle.doc
    return names


def _timing(call: Any, arguments: dict[str, Any]) -> Any:
    """The ``Timing`` of a call whose result says where its time went (``word_fields``
    update), or ``None``."""
    from ..tools._base import Timing

    if call.tool.name == "word_fields" and arguments.get("action") == "update":
        return Timing()
    return None


def _edit(call: Any, entry: Any, arguments: dict[str, Any], counted: bool) -> tuple[Result, Any, list[Any]]:
    """The call's edit made by the session: its result, what it made (``Session.edit``'s
    ``(first, head, touched, validation, before, reflow)``, ``None`` where it changed
    nothing), and the markers its checks get (its ``Timing``).  ``counted``: as for
    :meth:`~ooxml_edit.tools.owned.OwnedEntry.served`."""
    import ooxml_core

    mirror = entry.handle
    tool = call.tool.name
    _by_reference(call)
    # the host's limit on laying the document out holds in an edit as in a read
    context = call_context(call, entry)
    arguments, folded = _kept_refs(call, arguments)
    _tracking(call, entry, context, counted)
    adapted = _served.arguments_for(call, tool, arguments)
    written = adapted.written if isinstance(adapted, Adapted) else _written(call)
    sources = adapted.sources if isinstance(adapted, Adapted) else None
    given = {**{k: v for k, v in adapted.items() if v is not None}, "doc": entry.doc_id}
    names = _names(call, entry, arguments)
    layout = _layout(call, entry, measured=counted)
    timing = _timing(call, arguments)
    started = time.perf_counter()
    blobs = _Blobs(call.session) if tool in _served.hosting().blobs else None
    if tool == "word_set_tracking":
        reply, _app, made = _set_mode(entry, given, context, names)
    else:
        laid = ooxml_core.CallLayout(layout) if layout is not None else None
        reply, _app, made = mirror.session.edit(tool, json.dumps(given), options(
            context=context, layout=laid, blobs=blobs, written=written,
            documents=documents(names=names, tables=_host_tables(call.session), sources=sources)))
    if timing is not None:
        timing.add("update", time.perf_counter() - started)
    _count(tool)
    envelope = json.loads(reply)
    error = envelope.get("error")
    if made is None and error is not None:
        raise refusal(error)
    result = _result(envelope)
    refs = envelope.get("refs") or {}
    for name, address in refs.items():
        result.refs[call.define_ref(name, address)] = address
    # (first, as the call's own warnings come before the tool's)
    result.warnings[:0] = [folded_warning(given, ref) for ref, given in folded.items() if ref in refs]
    if timing is None:
        timing = _replied_timing(result)
    if timing is None:
        return result, made, []
    if isinstance(result.data, dict):
        result.data["timing"] = timing.parts      # (the checks add theirs)
    return result, made, [timing]


def _replied_timing(result: Result) -> Any:
    """The ``Timing`` of a reply that says where its time went itself (its ``data.timing``),
    which the call's checks add their parts to; ``None`` without one."""
    from ..tools._base import Timing

    parts = result.data.get("timing") if isinstance(result.data, dict) else None
    if not isinstance(parts, dict):
        return None
    timing = Timing()
    timing.parts.update(parts)
    return timing


class Made:
    """A check's marker (:attr:`~ooxml_edit.tools.served.Served.touched`): the version a
    call the session served made, its validation (``(new, fixed, baseline)``, as
    ``Session.validation`` gives it) and the ``reflow`` the session measured after the
    call's edits (JSON text, ``None`` where it measured none), which its checks report."""

    def __init__(self, head: Any, validation: tuple[list[str], list[str], int], reflow: str | None = None) -> None:
        self.head = head
        new, fixed, baseline = validation
        self.validation = {"new": list(new), "fixed": list(fixed), "baseline": baseline}
        self.reflow = json.loads(reflow) if reflow is not None else None


def serve_owned(call: Any, arguments: dict[str, Any]) -> Any:
    """The format's ``serve``: the call's edit made by the session beside the others it
    serves, without the document's lock (:mod:`ooxml_edit.tools.served`)."""
    from ooxml_edit.tools.served import Served

    entry = owned_entry(call)
    result, made, markers = _edit(call, entry, arguments, counted=True)
    if made is None:
        return Served(result, None, None, list(markers))
    first, head, step, validation, before, reflow = made
    return Served(result, first, head, [*markers, Made(head, validation, reflow)],
                  None if step is None else json.loads(step), before)


def serve(call: Any, arguments: dict[str, Any]) -> Result:
    """Under the document's lock (a call with an idempotence key, a batch's op): the call's
    edit made by the session, its versions the call's step."""
    entry = owned_entry(call)
    with entry.serving():
        result, made, markers = _edit(call, entry, arguments, counted=False)
        for marker in markers:
            call.touch(marker)
        if made is not None:
            first, head, _step, validation, before, reflow = made
            call.touch(Made(head, validation, reflow))
            entry.served(first, head, before=before)
    return result


def _result(envelope: dict[str, Any]) -> Result:
    # a listing's page too (word_comments list): its cursor and its total
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"),
                    next_cursor=envelope.get("next_cursor"))
    if envelope.get("total") is not None:
        result.total = envelope["total"]
    for key in ("changed", "created", "removed"):
        getattr(result, key).extend(envelope.get(key) or [])
    result.renamed.update(envelope.get("renamed") or {})
    result.warnings.extend(envelope.get("warnings") or [])
    return result
