"""The Rust host behind the :class:`~.dispatch.Toolbox` (Phase 3, E1): the facade's Rust path.

A format whose documents the Rust host holds says so with :attr:`~.session.DocumentFormat.host`
(a format library's): an object with

- ``make(settings, pages) -> Host``: the host (the core's), made with the toolbox's settings
  (JSON text: ``limits`` by :class:`~.limits.Limits`' field names, ``roles``, ``log_arguments``,
  ``allow_new_problems``, ``numbers``, ``own_tools``), laying its documents out with ``pages``;
- ``pages(font_dirs, locale) -> PageSetup``: what documents are laid out with for
  font folders and a locale (the toolbox's, or a session's own);
- ``catalogue() -> Sequence[str]``: the names of the host's tools;
- ``opened(session, doc)``: what the format says of a document the application opened (a
  warning), given the host's session and its id;
- ``refused(exc) -> (why, message) | None``: an exception the host's session raised opening a
  document, read: ``why`` one of ``argument``, ``limit``, ``unreadable``, ``state``; ``None``
  for any other.

A toolbox that :func:`runs_on_host` makes one :class:`Hosting`: one host per
toolbox (decision D7; one per other set of limits, font folders or locale a session is given),
and a
:class:`HostedSession` -- today's :class:`~.session.Session`, its documents, blobs, outputs and
log the Rust ``HostSession``'s -- per :meth:`~.dispatch.Toolbox.session`.  Each call is routed
(:meth:`Hosting.route`): a tool of the host's catalogue runs in Rust, with the interpreter lock
let go; an application's own tool (``tool(...)``, decision D1) runs here, in the Python
dispatcher, and inside a Rust ``batch`` or ``read_batch`` is refused with the way out "call it
alone".  A :class:`~.results.Result` is built from the reply's JSON (:class:`HostResult`), its
``ToolError`` from the reply's error, and the call's record is appended to the session's log.

Nothing calls back from Rust into Python: the session clock's moment of a call is handed over
as a value (its writes' date), the blobs are the host's own.

Must not: run a tool's work itself, or keep a copy of a document the host holds.
"""

from __future__ import annotations

import dataclasses
import datetime as _dt
import json
import threading
import time
from concurrent.futures import Future, ThreadPoolExecutor
from types import MappingProxyType
from typing import TYPE_CHECKING, Any, Iterable, Mapping, Sequence

from ..errors import ArgumentError
from .limits import LimitError, Limits
from .logs import CallRecord, log_call, log_internal, logger
from .results import Image, Result, ToolError
from .session import (Blob, Output, SavedDocument, Session, SessionView, StateRefused,
                      _require_bytes, utc_now)

if TYPE_CHECKING:
    from .dispatch import Toolbox


def runs_on_host(toolbox: "Toolbox", forced: bool | None = None) -> bool:
    """Whether ``toolbox``'s sessions run on the Rust host.

    The rule until Phase 3 E1b, in one place: every format is one the Rust host holds
    (:attr:`~.session.DocumentFormat.host`), and the toolbox has no tool of the application's
    own -- a session holding a Python-held document, or a tool writing Python datasets, keeps
    the host's formats on the Python host until the session's stores are shared (E1b drops both clauses,
    and :class:`~.dispatch.Toolbox`'s ``_rust_host`` with them).

    ``forced``: the toolbox's ``_rust_host`` (a test's, for a toolbox with tools of its own).
    """
    if not toolbox.formats or any(fmt.host is None for fmt in toolbox.formats.values()):
        return False
    if forced is not None:
        return forced
    return not own_tools(toolbox)


def own_tools(toolbox: "Toolbox") -> list[str]:
    """The tools of ``toolbox`` the application registered itself: none of its formats'
    host's catalogue."""
    hosted = set(_hosting(toolbox).catalogue())
    return [name for name in toolbox.tools if name not in hosted]


def _hosting(toolbox: "Toolbox") -> Any:
    """The formats' one host maker."""
    makers = {id(fmt.host): fmt.host for fmt in toolbox.formats.values() if fmt.host is not None}
    if len(makers) != 1:
        raise ArgumentError("the formats' documents are held by more than one Rust host; one toolbox "
                            "has one host")
    return next(iter(makers.values()))


def _settings(toolbox: "Toolbox", limits: Limits, own: Sequence[str]) -> str:
    """The host's settings as JSON text: the toolbox's (with ``limits``)."""
    given = {"limits": dataclasses.asdict(limits), "roles": list(toolbox.roles),
             "log_arguments": bool(toolbox.log_arguments),
             "allow_new_problems": bool(toolbox.allow_new_problems),
             "numbers": _rules(toolbox.numbers), "own_tools": list(own)}
    return json.dumps(given)


def _rules(rules: Any) -> dict[str, Any]:
    """:class:`~.numbers.NumberRules` as the host reads them."""
    return {"ignore": list(rules.ignore), "whole_numbers": rules.whole_numbers, "texts": rules.texts,
            "per_text": rules.per_text}


def _call_date(clock: Any) -> str:
    """The moment a call's writes are dated by, read of the session's ``clock``: the system's
    own clock read straight to the second (the default, :func:`~.session.utc_now`), else the
    application's clock's moment (:func:`_date`)."""
    if clock is utc_now:
        return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    return _date(clock())


def _date(moment: _dt.datetime) -> str:
    """The moment a call's writes are dated by: UTC, to the second."""
    if moment.tzinfo is not None:
        moment = moment.astimezone(_dt.timezone.utc)
    return f"{moment.year:04d}-{moment.month:02d}-{moment.day:02d}T{moment.hour:02d}:{moment.minute:02d}:{moment.second:02d}Z"


class Hosting:
    """A toolbox's Rust host and how its calls are routed."""

    def __init__(self, toolbox: "Toolbox") -> None:
        self.toolbox = toolbox
        self.maker = _hosting(toolbox)
        #: The application's own tools, which run here (decision D1).
        self.own = frozenset(own_tools(toolbox))
        self._hosts: dict[tuple[str, Any, str], Any] = {}
        self._lock = threading.Lock()

    def host(self, limits: Limits, font_dirs: Any, locale: Any) -> Any:
        """The host of sessions with ``limits``, laying their documents out with
        ``font_dirs`` and ``locale``: the toolbox's one (made on first use), or one for each
        other set a session is given."""
        settings = _settings(self.toolbox, limits, sorted(self.own))
        key = (settings, font_dirs, repr(locale))
        with self._lock:
            found = self._hosts.get(key)
            if found is None:
                found = self._hosts[key] = self.maker.make(settings, self.maker.pages(font_dirs, locale))
            return found

    def session(self, **given: Any) -> "HostedSession":
        """:meth:`~.dispatch.Toolbox.session`'s session, on the host."""
        return HostedSession(self, self.toolbox.formats.values(), **given)

    # -- routing -------------------------------------------------------------------------------

    def route(self, name: Any) -> str:
        """Where a call of ``name`` runs: ``"python"`` for an application's own tool (the
        facade's Python dispatcher), else ``"rust"`` -- the host reads every other name, an
        unknown one too, with its hints."""
        if not self.own or not isinstance(name, str):
            return "rust"
        try:
            found = self.toolbox.tool_named(name)
        except ToolError:
            return "rust"
        return "python" if found in self.own else "rust"

    def dispatch(self, session: "Session | SessionView", name: Any, arguments: Any, *,
                 roles: Any = None, context: Mapping[str, Any] | None = None) -> Result:
        """One call (:meth:`~.dispatch.Toolbox.dispatch`), routed."""
        if self.route(name) == "python":
            return self.toolbox._dispatch(session, name, arguments, roles=roles, context=context)
        from .dispatch import _caller

        base, found, caller = _caller(session, roles, context)
        assert isinstance(base, HostedSession), "a session of the toolbox's host"
        comments: list[dict[str, Any]] = []
        text = _arguments_text(arguments, comments)
        narrowed = list(found) if tuple(found) != tuple(base.roles) else None
        reply, files, app = base.rust.call(
            name if isinstance(name, str) else json.dumps(name), text, roles=narrowed,
            context=json.dumps(dict(caller), default=str) if caller else None,
            date=_call_date(base.clock))
        result = HostResult.of(reply, files, app)
        if result.ok and result.tool == "describe" and result.doc is not None:
            result.memory = base.memory_now(result.doc)
        base.kept(result, comments)
        return result

    def dispatch_many(self, session: "Session | SessionView", calls: Sequence[tuple[str, Any]], *,
                      max_workers: int = 8, roles: Any = None,
                      context: Mapping[str, Any] | None = None) -> list[Result]:
        """One turn's calls (:meth:`~.dispatch.Toolbox.dispatch_many`): a thread pool over
        :meth:`dispatch`, the Rust work with the interpreter lock let go.  A call waits for the
        turn's earlier calls of its documents (both only reading: side by side), so every
        result is the one the calls in the order given would have."""
        if not calls:
            return []
        box = self.toolbox
        documents = [_documents_named(arguments) for _name, arguments in calls]
        reads = [bool(documents[i]) and box._side_by_side(name, arguments)
                 for i, (name, arguments) in enumerate(calls)]
        futures: list[Future[Result]] = []
        with ThreadPoolExecutor(max_workers=max(1, min(max_workers, len(calls)))) as executor:
            for index, (name, arguments) in enumerate(calls):
                earlier = [futures[j] for j in range(index)
                           if not (documents[j] and documents[index] and not documents[j] & documents[index])
                           and not (reads[j] and reads[index])]
                futures.append(executor.submit(self._after, earlier, session, name, arguments, roles,
                                               context))
            return [future.result() for future in futures]

    def _after(self, earlier: list[Future[Result]], session: Any, name: Any, arguments: Any, roles: Any,
               context: Any) -> Result:
        for future in earlier:
            future.exception()      # wait; its outcome is its own
        return self.dispatch(session, name, arguments, roles=roles, context=context)


def _arguments_text(arguments: Any, comments: list[dict[str, Any]]) -> str:
    """A call's arguments as JSON text, a batch's ops' comments taken out for the log (as
    the Python host keeps them, :func:`~.dispatch._op_comments`)."""
    from .dispatch import _op_comments

    if type(arguments) is dict and "ops" not in arguments:     # (the common call: no batch)
        try:
            return json.dumps(arguments, ensure_ascii=False)
        except (TypeError, ValueError):
            return json.dumps(arguments, ensure_ascii=False, default=str)
    if isinstance(arguments, str):
        try:
            parsed = json.loads(arguments)
        except ValueError:
            return arguments            # (the host refuses it, in its words)
    else:
        parsed = arguments
    if isinstance(parsed, dict) and "ops" in parsed:
        parsed = _op_comments(parsed, comments)
    try:
        return json.dumps(parsed, ensure_ascii=False)
    except (TypeError, ValueError):
        return json.dumps(parsed, ensure_ascii=False, default=str)


def _documents_named(arguments: Any) -> frozenset[str]:
    """The documents a call names, as far as can be told before the host reads it: its
    ``doc`` and its ops' (a batch's)."""
    try:
        parsed = json.loads(arguments) if isinstance(arguments, str) else arguments
    except ValueError:
        return frozenset()
    if not isinstance(parsed, Mapping):
        return frozenset()
    found = {parsed["doc"]} if isinstance(parsed.get("doc"), str) else set()
    for op in parsed.get("ops") or ():
        given = op.get("arguments") if isinstance(op, Mapping) else None
        if isinstance(given, Mapping) and isinstance(given.get("doc"), str):
            found.add(given["doc"])
    return frozenset(found)


#: An envelope's fields a :class:`HostResult` reads, as :class:`~.results.Result` names them.
_ENVELOPE_FIELDS = ("doc", "version", "summary", "changed", "created", "removed", "renamed", "warnings",
                    "checks", "data", "next_cursor", "refs", "datasets", "total")


@dataclasses.dataclass
class HostResult(Result):
    """A Rust call's reply as a :class:`~.results.Result`: :meth:`to_json` is the envelope
    the host wrote, key for key (``try_call`` included), so :meth:`~.results.Result.to_text`
    and the adapters send exactly it; what only the application reads is the reply's
    application part, and :attr:`record` the host's record of the call (its fields as
    ``HostSession.call`` gives them)."""

    envelope: dict[str, Any] = dataclasses.field(default_factory=dict)
    record: tuple | None = dataclasses.field(default=None, repr=False)

    @classmethod
    def of(cls, text: str, files: Sequence[tuple[str, str, bytes]], app: tuple) -> "HostResult":
        touched, by_op, changed_parts, timing, memory, budget, record = app
        envelope = json.loads(text)
        fields: dict[str, Any] = {key: envelope[key] for key in _ENVELOPE_FIELDS if key in envelope}
        error = envelope.get("error")
        if error is not None:
            fields["error"] = ToolError(error["code"], error["message"], field=error.get("field"),
                                        valid_options=error.get("valid_options"),
                                        details=error.get("details"), try_call=error.get("try_call"))
            fields["summary"] = error["message"]
        if files:
            described = {image.get("label"): image for image in envelope.get("images") or ()}
            images = []
            for name, format, data in files:
                image = described.get(name)
                if format == "png" and image is not None:
                    images.append(Image(bytes(data), image["width"], image["height"], label=name))
            fields["images"] = images
        if touched is not None:
            fields["touched"] = json.loads(touched)
        if by_op is not None:
            fields["touched_by_op"] = {int(op): list(t) for op, t in json.loads(by_op).items()}
        if changed_parts is not None:
            fields["changed_parts"] = json.loads(changed_parts)
        if timing is not None:
            total, lock_wait, work, held, cpu = timing
            fields["duration"] = total
            fields["timing"] = {"total": total, "lock_wait": lock_wait, "work": work, "cpu": cpu, "held": held}
        if memory is not None:
            fields["memory"] = json.loads(memory)
        return cls(envelope=envelope, record=record, ok=envelope["ok"] is True, tool=record[0], budget=budget,
                   **fields)

    def to_json(self) -> dict[str, Any]:
        return json.loads(json.dumps(self.envelope))


#: How a refused open reads in Python: the host's reason (:meth:`maker.refused`) as the Python
#: host's exception.
_REFUSALS = {"argument": ArgumentError, "limit": LimitError, "unreadable": ArgumentError,
             "state": StateRefused}


class HostedSession(Session):
    """A :class:`~.session.Session` whose documents, blobs, outputs and call log the Rust
    host's session (:attr:`rust`, the core's ``HostSession``) holds: today's signatures,
    its work done in Rust."""

    def __init__(self, hosting: Hosting, formats: Iterable[Any], **given: Any) -> None:
        self.hosting = hosting
        from .session import font_dirs_of, locale_of

        limits = given.get("limits") or hosting.toolbox.limits
        font_dirs, locale = font_dirs_of(given.get("font_dirs")), given.get("locale")
        #: The Rust host's session.
        self.rust = hosting.host(limits, font_dirs, locale_of(locale)).session(locale=locale)
        super().__init__(formats, **given)
        if self.numbers != hosting.toolbox.numbers:
            self.rust.set_number_rules(json.dumps(_rules(self.numbers)))

    # The open documents are the Rust session's: their ids, each with its kind.
    @property                               # type: ignore[override]
    def documents(self) -> Mapping[str, str]:
        """The open documents, ``{id: kind}`` in id order (a copy): the host holds them."""
        return MappingProxyType({doc: self.rust.document(doc)[0] for doc in self.rust.documents()})

    @documents.setter
    def documents(self, value: Any) -> None:
        if value:                           # (Session.__init__ starts it empty)
            raise AttributeError("a session on the Rust host opens its documents with open()")

    # The roles are the Rust session's (it holds every call to them).
    @property                               # type: ignore[override]
    def roles(self) -> tuple[str, ...]:
        return self._roles

    @roles.setter
    def roles(self, value: Any) -> None:
        from .registry import role_set

        roles = role_set(value)
        self.rust.set_roles(list(roles))
        self._roles = roles                 # (as the Rust session holds them: read per call)

    # -- documents -------------------------------------------------------------------------------

    def open(self, data: bytes, name: str, *, kind: str | None = None, source: str = "app",
             doc_id: str | None = None, state: Any = None) -> str:
        """:meth:`Session.open`, the document held by the host."""
        data = _require_bytes(data, "open")
        if isinstance(state, (str, bytes, dict, list, tuple)) and not state:
            state = None
        try:
            doc = self.rust.open(data, name=name, kind=kind, source=source, doc_id=doc_id, state=state)
        except Exception as exc:  # noqa: BLE001 -- the host's refusal, read by its maker
            refused = self.hosting.maker.refused(exc)
            if refused is None:
                raise
            why, message = refused
            raise _REFUSALS.get(why, ArgumentError)(message) from None
        self.hosting.maker.opened(self.rust, doc)
        return doc

    def adopt(self, document: Any, kind: str, name: str, *, source: str = "new", size: int = 0,
              doc_id: str | None = None) -> str:
        raise ArgumentError(f"a {kind} document is held by the Rust host from its bytes: open its bytes")

    def _named(self, doc_id: str) -> str:
        """``doc_id``, an open document's; else ``not_found`` naming the open ones (as
        :meth:`Session.entry` refuses)."""
        open_now = self.rust.documents()
        if doc_id not in open_now:
            raise ToolError("not_found", f"no open document {doc_id!r}", field="doc", valid_options=open_now)
        return doc_id

    def kind_of(self, doc_id: str) -> str:
        """The kind of the open document ``doc_id``, as its format names it."""
        return self.rust.document(self._named(doc_id))[0]

    def entry(self, doc_id: str) -> Any:
        raise ArgumentError(f"{doc_id} is held by the Rust host, which keeps no Python entry of it: "
                            "read it with a tool, or save it")

    def refs(self, doc_id: str) -> dict[str, str]:
        """:meth:`Session.refs`: the document's refs as they are now, a copy."""
        return json.loads(self.rust.refs(self._named(doc_id)))

    def read_refs(self, doc_id: str) -> dict[str, str]:
        """:meth:`Session.read_refs`: the host's refs are those of the version committed."""
        return self.refs(doc_id)

    def save(self, doc_id: str) -> SavedDocument:
        doc, name, kind, version, data, state = self.rust.save(self._named(doc_id))
        return SavedDocument(doc=doc, name=name, kind=kind, version=version, data=bytes(data), state=state)

    def close(self, doc_id: str) -> list[str]:
        return list(self.rust.close(self._named(doc_id)))

    def undo(self, doc_id: str, steps: int = 1, *, scope: str | None = None,
             caller: Mapping[str, Any] | None = None) -> int:
        return self._rust_history(doc_id, steps, scope, False, caller)

    def redo(self, doc_id: str, steps: int = 1, *, scope: str | None = None,
             caller: Mapping[str, Any] | None = None) -> int:
        return self._rust_history(doc_id, steps, scope, True, caller)

    def _rust_history(self, doc_id: str, steps: int, scope: str | None, redo: bool,
                      caller: Mapping[str, Any] | None) -> int:
        if scope is not None:
            kind = self.kind_of(doc_id)
            raise ToolError("invalid_arguments", f"{doc_id} ({kind}) undoes document-wide only: leave scope out",
                            field="scope")
        count, error = self.rust.history(self._named(doc_id), steps, redo=redo,
                                         context=json.dumps(dict(caller), default=str) if caller else None)
        if error is not None:
            error = json.loads(error)
            raise ToolError(error["code"], error["message"], field=error.get("field"),
                            valid_options=error.get("valid_options"), details=error.get("details"),
                            try_call=error.get("try_call"))
        return count

    # -- datasets: the Rust HostSession's from E1b -------------------------------------------------

    def keep_datasets(self, entries: Iterable[Any]) -> None:
        if list(entries):           # (keeping none keeps nothing)
            raise NotImplementedError("a session on the Rust host keeps the application's datasets from Phase 3 "
                                      "E1b: until then, keep them in a toolbox with a tool of the application's own")

    def dataset(self, address: str, *, field: str | None = None) -> Any:
        raise NotImplementedError("a session on the Rust host holds its datasets from Phase 3 E1b")

    def dataset_ids(self) -> list[str]:
        raise NotImplementedError("a session on the Rust host holds its datasets from Phase 3 E1b")

    def dataset_entries(self) -> list[Any]:
        raise NotImplementedError("a session on the Rust host holds its datasets from Phase 3 E1b")

    def drop_dataset(self, address: str) -> None:
        raise NotImplementedError("a session on the Rust host holds its datasets from Phase 3 E1b")

    # -- blobs and outputs -----------------------------------------------------------------------

    def add_blob(self, data: bytes, name: str, mime: str | None = None) -> str:
        data = _require_bytes(data, "add_blob")
        try:
            return self.rust.add_blob(data, name, mime)
        except ValueError as exc:
            raise LimitError(str(exc)) from None

    def blob(self, handle: str) -> Blob:
        found = self.rust.blob(handle)
        if found is None:
            raise ToolError("not_found", f"no blob {handle!r}",
                            valid_options=[f"{h} ({n})" for h, n, _m, _s in self.rust.blobs()])
        name, mime, data = found
        return Blob(handle, name, mime, bytes(data))

    def memory_now(self, doc_id: str) -> dict[str, float]:
        """What this process holds, in MB (``describe``'s, the application's): ``document``,
        what the host keeps of the document, and the process's own (:func:`.memory.process_memory`)."""
        from .memory import process_memory

        return {"document": round(self.rust.document_memory(doc_id) / 2 ** 20, 1), **process_memory()}

    def new_round(self) -> None:
        self.rust.new_round()

    def kept(self, result: Result, comments: Sequence[Mapping[str, Any]]) -> None:
        """What a call answered kept here: the files its tools saved, as outputs; its record,
        in the log (logged as the Python host logs it)."""
        if result.tool == "save_document":     # (the one tool that saves files for the application)
            for name, format, doc, data, validate, state in self.rust.take_outputs():
                self.add_output(Output(name, format, bytes(data), doc=doc,
                                       validate=json.loads(validate) if validate else None, state=state))
        made = _record_of(result, comments)
        if made is None:
            return
        self.log.append(made)
        if logger.isEnabledFor(20):     # logging.INFO
            log_call(made)
        if result.error is not None and result.error.code == "internal":
            log_internal(made, result.error.message, result.error.details.get("trace"))


def _record_of(result: Result, comments: Sequence[Mapping[str, Any]]) -> CallRecord | None:
    """The host's record of ``result``'s call as :class:`~.logs.CallRecord`."""
    record = getattr(result, "record", None)
    if record is None:
        return None
    tool, docs, digest, shape, ok, code, summary, duration, version, arguments, roles, context = record
    return CallRecord(tool=tool, docs=list(docs), digest=digest, shape=json.loads(shape), ok=ok, code=code,
                      summary=summary, duration=duration, version=version,
                      arguments=json.loads(arguments) if arguments is not None else None, roles=roles,
                      context=json.loads(context) if context is not None else None,
                      comments=[dict(c) for c in comments] or None)


__all__ = ["HostResult", "HostedSession", "Hosting", "own_tools", "runs_on_host"]
