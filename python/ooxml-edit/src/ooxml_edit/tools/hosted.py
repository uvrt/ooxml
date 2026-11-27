"""The Rust host behind the :class:`~.dispatch.Toolbox` (Phase 3, E1): the facade's Rust path.

A format whose documents the Rust host holds says so with :attr:`~.session.DocumentFormat.host`
(a format library's): an object with

- ``make(settings, pages) -> Host``: the host (the core's), made with the toolbox's settings
  (JSON text: ``limits`` by :class:`~.limits.Limits`' field names, ``roles``, ``log_arguments``,
  ``allow_new_problems``, ``numbers``, ``own_tools``), laying its documents out with ``pages``;
- ``pages(font_dirs, locale) -> PageSetup``: what documents are laid out with for
  font folders and a locale (the toolbox's, or a session's own);
- ``catalogue() -> Sequence[str]``: the names of the host's tools;
- ``held_kinds() -> Sequence[str]``: the kinds whose documents the host's sessions name while
  the Python host holds them (until Phase 4);
- ``opened(session, doc)``: what the format says of a document the application opened (a
  warning), given the host's session and its id;
- ``refused(exc) -> (why, message) | None``: an exception the host's session raised opening a
  document, read: ``why`` one of ``argument``, ``limit``, ``unreadable``, ``state``; ``None``
  for any other.

A toolbox that :func:`runs_on_host` makes one :class:`Hosting`: one host per
toolbox (decision D7; one per other set of limits, font folders or locale a session is given),
and a
:class:`HostedSession` -- today's :class:`~.session.Session`, its documents, blobs, outputs,
image budgets, datasets (:mod:`.hosted_data`) and log the Rust ``HostSession``'s -- per
:meth:`~.dispatch.Toolbox.session`.  A document of a format the Python host holds (until Phase
4) has its entry there too, its id from the Rust session's one counter and its room counted
there, a stand-in in the Rust session (decision D3).  Each call is routed (:meth:`Hosting.route`):

- by the documents it names: those the Rust host holds in Rust, with the interpreter lock let
  go; those the Python host holds in the Python dispatcher;
- a call naming none by its tool: the host's catalogue in Rust; a Python-held format's tool,
  and the new or opened document of such a format, in Python;
- an application's own tool (``tool(...)``, decision D1) in the Python dispatcher -- refused
  when it names a document the Rust host holds (:meth:`HostedSession.held_in_python`), its
  datasets, outputs and images kept in the Rust session -- and inside a Rust ``batch`` or
  ``read_batch`` refused with the way out "call it alone";
- a ``read_batch`` of both hosts' documents in Python, each op of the Rust host's a Rust call;
  a ``batch`` of both refused with the way out "two batches" (decision D2).

A :class:`~.results.Result` is built from the reply's JSON (:class:`HostResult`), its
``ToolError`` from the reply's error, and the call's record is appended to the session's log.

Nothing calls back from Rust into Python: the session clock's moment of a call is handed over
as a value (its writes' date), the blobs are the host's own, and what a call reads of the Python
host's documents is resolved first and handed over as values (:mod:`.hosted_values`).

Must not: run a tool's work itself, or keep a copy of a document the host holds.
"""

from __future__ import annotations

import dataclasses
import datetime as _dt
import json
import threading
import time
from concurrent.futures import Future, ThreadPoolExecutor
from contextlib import contextmanager
from typing import TYPE_CHECKING, Any, Iterable, Iterator, Mapping, Sequence

from ..errors import ArgumentError
from .hosted_data import HostDatasets, tool_error
from .hosted_values import held_values
from . import sources
from .limits import LimitError, Limits
from .logs import CallRecord, log_call, log_internal, logger
from .results import Image, Result, ToolError
from .session import (Blob, Output, SavedDocument, Session, SessionView, StateRefused,
                      _require_bytes, utc_now)

if TYPE_CHECKING:
    from .dispatch import Toolbox


#: How many times a call is made again with what it asked of the Python host's documents that
#: was not resolved before it (:mod:`.hosted_values`): one is enough for every call measured.
MISSED_ROUNDS = 2


def runs_on_host(toolbox: "Toolbox") -> bool:
    """Whether ``toolbox``'s sessions run on the Rust host: a format of it is one the Rust
    host holds (:attr:`~.session.DocumentFormat.host`), and each other is of a kind the Rust
    session names while the Python host holds its documents (its maker's ``held_kinds``).  The
    application's own tools run beside it, in the facade (decision D1), their datasets,
    outputs and images the Rust session's; so do the Python host's documents (D3).

    A toolbox with no format the Rust host holds (the format libraries held in Python alone)
    keeps the plain :class:`~.session.Session` and its stores until Phase 4 deletes them.  One
    holding a format the Rust host holds beside a Python-held format of a kind the Rust session
    cannot name is refused (``ArgumentError``, a ``ValueError``): the Rust host's documents have
    no other host to run on (Phase 3 E2).
    """
    formats = list(toolbox.formats.values())
    hosted = [fmt.kind for fmt in formats if fmt.host is not None]
    if not hosted:
        return False
    named = tuple(_hosting(toolbox).held_kinds())
    unnamed = [fmt.kind for fmt in formats if fmt.host is None and fmt.kind not in named]
    if unnamed:
        held = (f"it names {', '.join(repr(kind) for kind in named)} documents" if named
                else "it names no other kind")
        raise ArgumentError(
            f"{', '.join(hosted)} documents run on the Rust host, whose session cannot name a "
            f"{unnamed[0]!r} document ({held}): give the {unnamed[0]!r} format a toolbox of its own, "
            f"or use a format of a kind the Rust session names")
    return True


def held_kinds(toolbox: "Toolbox") -> list[str]:
    """The kinds of ``toolbox``'s formats whose documents the Python host holds."""
    return [kind for kind, fmt in toolbox.formats.items() if fmt.host is None]


def own_tools(toolbox: "Toolbox") -> list[str]:
    """The tools of ``toolbox`` the application registered itself: none of its formats'
    host's catalogue, and none a format the Python host holds offers (:func:`format_tools`)."""
    hosted = set(_hosting(toolbox).catalogue())
    held = format_tools(toolbox)
    return [name for name in toolbox.tools if name not in hosted and name not in held]


def format_tools(toolbox: "Toolbox") -> dict[str, str]:
    """The tools of ``toolbox`` a format the Python host holds offers, outside the host's
    catalogue, each with that format's kind: its library's (:attr:`~.registry.Tool.library`),
    or one whose handlers serve that kind."""
    hosted = set(_hosting(toolbox).catalogue())
    kinds = held_kinds(toolbox)
    out: dict[str, str] = {}
    for name, tool in toolbox.tools.items():
        if name in hosted:
            continue
        kind = tool.library if tool.library in kinds else next(
            (k for k in tool.handlers if k in kinds), None)
        if kind is not None:
            out[name] = kind
    return out


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
             "numbers": _rules(toolbox.numbers), "own_tools": list(own),
             "format_tools": format_tools(toolbox),
             "held": [{"kind": kind, "prefixes": sources.prefixes(toolbox.formats).get(kind, [])}
                      for kind in held_kinds(toolbox)]}
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
        #: The host's tools.
        self.catalogue = frozenset(self.maker.catalogue())
        #: The tools that may read the Python host's documents' rows or values: those
        #: declaring sources, datasets, references or reads of every document's values, and
        #: batches (:mod:`.hosted_values`).
        self.reading = frozenset(
            name for name, found in toolbox.tools.items()
            if found.composite or found.sources or found.facts or found.datasets or found.value_reads is not None)
        #: The host's tools that name one document, by ``doc``, and are no batch.
        self.one_doc = frozenset(
            name for name, found in toolbox.tools.items()
            if name in self.catalogue and not found.composite and found.documents == ("doc",))
        #: The tools of the formats the Python host holds, outside the host's catalogue.
        self.formats = frozenset(format_tools(toolbox))
        #: The kinds whose documents the Python host holds (until Phase 4).
        self.kinds = frozenset(held_kinds(toolbox))
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

    def route(self, session: "HostedSession", name: Any, arguments: Any) -> "str | ToolError":
        """Where a call of ``name`` runs: ``"python"`` -- the facade's Python dispatcher -- for
        an application's own tool (decision D1) and for a call naming only documents the Python
        host holds (or a tool of its formats'), else ``"rust"``; the host reads every other name,
        an unknown one too, with its hints.  A ``read_batch`` of both runs in Python, each op of
        the Rust host's documents a Rust call; a ``batch`` of both is refused (decision D2)."""
        if not self.own and not self.kinds:
            return "rust"                       # (every tool the host's: an unknown one too)
        if type(name) is str and name in self.own:
            return "python"                     # (an application's own tool: D1)
        if name in self.one_doc and type(arguments) is dict:
            doc = arguments.get("doc")          # (the common call: a host tool naming its one document)
            if isinstance(doc, str):
                return "python" if doc in session._held else "rust"
        tool = self._tool(name)
        if tool is None:
            return "rust"
        if tool.name in self.own:
            return "python"
        if not self.kinds:
            return "rust"
        parsed = _parsed(arguments)
        if parsed is None:
            return "rust" if tool.name in self.catalogue else "python"
        if not tool.composite:
            side = self._side(session, tool, parsed)
            return side or ("rust" if tool.name in self.catalogue else "python")
        ops = [op for op in parsed.get("ops") or () if isinstance(op, Mapping)]
        tools = [self._tool(op.get("tool")) for op in ops]
        sides = [self._side(session, found, op.get("arguments")) for found, op in zip(tools, ops)]
        if "rust" not in sides and any(found is not None and found.name in self.formats for found in tools):
            return "python"                 # (a Python-held format's tools alone: its host's batch)
        if "python" not in sides:
            return "rust"
        if "rust" not in sides or tool.name == "read_batch":
            return "python"
        return _two_batches(ops, sides)

    def _tool(self, name: Any) -> Any:
        if not isinstance(name, str):
            return None
        found = self.toolbox.tools.get(name)
        if found is not None:
            return found                        # (the common call: a tool's own name)
        try:
            return self.toolbox.tools[self.toolbox.tool_named(name)]
        except (ToolError, KeyError):
            return None

    def _side(self, session: "HostedSession", tool: Any, arguments: Any) -> str | None:
        """Where one call (or op) runs: by the documents it names -- a tool of the host's
        catalogue in Rust unless every one is the Python host's, any other tool (a Python-held
        format's, an application's own) in Python; one naming none by its tool (the host's
        catalogue, the kind a ``new_document`` makes or an ``open_document`` opens) -- ``None``
        for a tool outside the catalogue naming none, or no tool: it goes with the batch's
        other ops (inside a Rust batch an application's own tool is refused, "call it alone":
        D1; a Python-held format's, "send it as its own call or in its own batch")."""
        if tool is None:
            return None
        given = _parsed(arguments) or {}
        if tool.name in self.one_doc and isinstance(given.get("doc"), str):
            return "python" if given["doc"] in session._held else "rust"
        try:
            docs = [doc for doc in tool.doc_ids(given) if isinstance(doc, str)]
        except Exception:  # noqa: BLE001 -- arguments a dispatcher refuses in its words
            docs = []
        if tool.name not in self.catalogue:
            return "python" if docs else None
        if docs:
            return "python" if all(doc in session._held for doc in docs) else "rust"
        if tool.name == "new_document":
            return "python" if given.get("kind") in self.kinds else "rust"
        if tool.name == "open_document" and isinstance(given.get("blob"), str):
            found = session.rust.blob(given["blob"])
            if found is not None:
                kind = given.get("kind") if isinstance(given.get("kind"), str) else None
                blob_name, _mime, data = found
                if session.python_held_format(bytes(data), blob_name, kind) is not None:
                    return "python"
        return "rust"

    def dispatch(self, session: "Session | SessionView", name: Any, arguments: Any, *,
                 roles: Any = None, context: Mapping[str, Any] | None = None) -> Result:
        """One call (:meth:`~.dispatch.Toolbox.dispatch`), routed (:meth:`route`)."""
        from .dispatch import _caller

        base, found, caller = _caller(session, roles, context)
        assert isinstance(base, HostedSession), "a session of the toolbox's host"
        where = self.route(base, name, arguments)
        if where == "python":
            try:
                return self.toolbox._dispatch(session, name, arguments, roles=roles, context=context)
            finally:
                if base._held:
                    base.sync_held()
        if isinstance(where, ToolError):
            return self._refused(base, name, arguments, where, found, caller)
        return self.call(base, name, arguments, found, caller)

    def call(self, base: "HostedSession", name: Any, arguments: Any, roles: Sequence[str],
             caller: Mapping[str, Any], *, logged: bool = True) -> "HostResult":
        """A call the Rust host runs, with what it reads of the documents the Python host
        holds (:mod:`.hosted_values`); ``logged``: its record kept in the session's log (not
        for a ``read_batch``'s op, which the batch's record is)."""
        comments: list[dict[str, Any]] = []
        text = _arguments_text(arguments, comments)
        narrowed = list(roles) if tuple(roles) != tuple(base.roles) else None
        given_name = name if isinstance(name, str) else json.dumps(name)
        context = json.dumps(dict(caller), default=str) if caller else None
        if not base._held:                  # (the common session: no document the Python host holds)
            result = HostResult.of(*base.rust.call(given_name, text, roles=narrowed, context=context,
                                                   date=_call_date(base.clock)))
        else:
            result = self._held_call(base, given_name, name, arguments, text, narrowed, context)
        if result.ok and result.tool == "describe" and result.doc is not None:
            result.memory = base.memory_now(result.doc)
        if logged:
            base.kept(result, comments)
        return result

    def _held_call(self, base: "HostedSession", given_name: str, name: Any, arguments: Any, text: str,
                   narrowed: list[str] | None, context: str | None) -> "HostResult":
        """:meth:`call` in a session holding documents of the Python host: with what the call
        reads of them (:mod:`.hosted_values`; none for a tool that reads no sources,
        datasets, references or values, a refusal of its arguments among them), what it
        missed noted, and made again with that resolved -- where it changed nothing."""
        held = held_values(self.toolbox, base, name, arguments) if name in self.reading else None
        for _round in range(MISSED_ROUNDS + 1):
            result = HostResult.of(*base.rust.call(
                given_name, text, roles=narrowed, context=context, date=_call_date(base.clock),
                held=held.json() if held is not None else None, holding=True))
            if not result.missed or held is None or result.changed_parts or not held.add(result.missed):
                break
        return result

    def read_op(self, session: "HostedSession", op: Any, roles: Sequence[str],
                caller: Mapping[str, Any]) -> "tuple[Result, bool] | None":
        """A Python ``read_batch``'s op of a document the Rust host holds, run as a Rust call:
        its result and whether its tool pages (``cursor``); ``None`` for an op the Python
        host runs."""
        if not isinstance(op, Mapping):
            return None
        tool = self._tool(op.get("tool"))
        if self._side(session, tool, op.get("arguments")) != "rust":
            return None
        result = self.call(session, tool.name, op.get("arguments") or {}, roles, caller, logged=False)
        return result, "cursor" in tool.schema.get("properties", {})

    def _refused(self, base: "HostedSession", name: Any, arguments: Any, error: ToolError,
                 roles: Sequence[str], caller: Mapping[str, Any]) -> Result:
        """A call the facade refuses itself (a ``batch`` of both hosts' documents), logged as
        the dispatcher logs one."""
        result = Result.failure(error)
        result.tool = name if isinstance(name, str) else ""
        self.toolbox._record(base, result.tool, [], arguments, result, {}, roles, caller)
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


def _parsed(arguments: Any) -> Mapping[str, Any] | None:
    """A call's arguments as a mapping; ``None`` for text that is no JSON object."""
    if type(arguments) is dict:             # (the common call)
        return arguments
    if isinstance(arguments, str):
        try:
            arguments = json.loads(arguments)
        except ValueError:
            return None
    return arguments if isinstance(arguments, Mapping) else None


def _two_batches(ops: Sequence[Mapping[str, Any]], sides: Sequence[str | None]) -> ToolError:
    """The refusal of a changing ``batch`` whose ops change documents of both hosts (decision
    D2): the way out, two batches -- the call with the first one's ops, the second's listed."""
    rust = [i for i, side in enumerate(sides) if side != "python"]
    python = [i for i, side in enumerate(sides) if side == "python"]
    first, second = (rust, python) if rust[0] < python[0] else (python, rust)
    return ToolError(
        "refused", f"this batch changes documents two hosts hold (ops {_listed(first)} and ops "
        f"{_listed(second)}), and one batch is one host's: send them as two batches, the first with "
        f"ops {_listed(first)}, the second with ops {_listed(second)}", field="ops",
        details={"batches": [first, second]},
        try_call={"tool": "batch", "arguments": {"ops": [dict(ops[i]) for i in first]}})


def _listed(indexes: Sequence[int]) -> str:
    return ", ".join(str(i) for i in indexes)


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
    #: What the call asked of the documents the Python host holds that was not resolved for
    #: it (``[{"doc", "address"}]``; :mod:`.hosted_values`).
    missed: list[dict[str, str]] | None = dataclasses.field(default=None, repr=False)

    @classmethod
    def of(cls, text: str, files: Sequence[tuple[str, str, bytes]], app: tuple) -> "HostResult":
        touched, by_op, changed_parts, timing, memory, budget, record, missed = app
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
        if missed is not None:
            fields["missed"] = json.loads(missed)
        return cls(envelope=envelope, record=record, ok=envelope["ok"] is True, tool=record[0], budget=budget,
                   **fields)

    def to_json(self) -> dict[str, Any]:
        return json.loads(json.dumps(self.envelope))


#: How a refused open reads in Python: the host's reason (:meth:`maker.refused`) as the Python
#: host's exception.
_REFUSALS = {"argument": ArgumentError, "limit": LimitError, "unreadable": ArgumentError,
             "state": StateRefused}


@dataclasses.dataclass(frozen=True)
class HostDocument:
    """A document the Rust host holds, as the Python dispatcher lists the open documents
    (:attr:`HostedSession.documents`): its id, kind and name; no Python entry of it."""

    doc_id: str
    kind: str
    name: str


class _Documents(Mapping):
    """:attr:`HostedSession.documents`: the Rust session's ids in the order opened, each the
    Python host's entry or a :class:`HostDocument`."""

    def __init__(self, session: "HostedSession") -> None:
        self._session = session
        self._ids = list(session.rust.documents())

    def __getitem__(self, doc: str) -> Any:
        held = self._session._held.get(doc)
        if held is not None:
            return held
        if doc not in self._ids:
            raise KeyError(doc)
        kind, name, _version, _file = self._session.rust.document(doc)
        return HostDocument(doc, kind, name)

    def __iter__(self) -> Any:
        return iter(self._ids)

    def __len__(self) -> int:
        return len(self._ids)

    def __contains__(self, doc: object) -> bool:
        return doc in self._ids


class HostedSession(Session):
    """A :class:`~.session.Session` whose documents, blobs, outputs, image budgets, datasets
    and call log the Rust host's session (:attr:`rust`, the core's ``HostSession``) holds:
    today's signatures, its work done in Rust.  The application's own tools, which run here
    (decision D1), keep their datasets and outputs and count their images in it too."""

    def __init__(self, hosting: Hosting, formats: Iterable[Any], **given: Any) -> None:
        self.hosting = hosting
        from .session import font_dirs_of, locale_of

        limits = given.get("limits") or hosting.toolbox.limits
        font_dirs, locale = font_dirs_of(given.get("font_dirs")), given.get("locale")
        #: The Rust host's session.
        self.rust = hosting.host(limits, font_dirs, locale_of(locale)).session(locale=locale)
        #: The entries of the documents the Python host holds, by the Rust session's ids.
        self._held: dict[str, Any] = {}
        #: Each one's state number as the Rust session was last told it (:meth:`sync_held`).
        self._told: dict[str, int] = {}
        #: The session's datasets, the Rust session's (:mod:`.hosted_data`).
        self.data = HostDatasets(self.rust, before=self.sync_held)
        super().__init__(formats, **given)
        if self.numbers != hosting.toolbox.numbers:
            self.rust.set_number_rules(json.dumps(_rules(self.numbers)))

    # The open documents: the Rust session names them all (one counter, one room); those of a
    # format the Python host holds have their entries here too (until Phase 4).
    @property                               # type: ignore[override]
    def documents(self) -> Mapping[str, Any]:
        """The open documents by id, in the order opened: the entry of each the Python host
        holds (:class:`~.session.DocumentEntry`), a :class:`HostDocument` of each the Rust host
        holds."""
        return _Documents(self)

    @documents.setter
    def documents(self, value: Any) -> None:
        if value:                           # (Session.__init__ starts it empty)
            raise AttributeError("a session on the Rust host opens its documents with open()")

    def _entries(self) -> dict[str, Any]:
        return self._held

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
        """:meth:`Session.open`: a document of the Rust host's formats held by it; one of a
        format the Python host holds (:func:`python_held_format`) held here, its id the Rust
        session's (decision D3)."""
        data = _require_bytes(data, "open")
        fmt = self.python_held_format(data, name, kind)
        if fmt is not None:
            doc = self._hold(fmt.kind, name, len(data), source, doc_id)
            try:
                return super().open(data, name, kind=fmt.kind, source=source, doc_id=doc, state=state)
            except BaseException:
                self.rust.close(doc)
                raise
        if isinstance(state, (str, bytes, dict, list, tuple)) and not state:
            state = None
        try:
            doc = self.rust.open(data, name=name, kind=kind, source=source, doc_id=doc_id, state=state)
        except Exception as exc:  # noqa: BLE001 -- the host's refusal, read by its maker
            raise self._refused(exc) from None
        self.hosting.maker.opened(self.rust, doc)
        return doc

    def adopt(self, document: Any, kind: str, name: str, *, source: str = "new", size: int = 0,
              doc_id: str | None = None) -> str:
        """:meth:`Session.adopt`, for a format the Python host holds: its id the Rust session's."""
        fmt = self.formats.get(kind)
        if fmt is None or fmt.host is not None:
            raise ArgumentError(f"a {kind} document is held by the Rust host from its bytes: open its bytes")
        if doc_id is not None and doc_id not in self._held and doc_id in self.rust.documents():
            # (:meth:`open`'s: the id held for it already)
            return super().adopt(document, kind, name, source=source, size=size, doc_id=doc_id)
        doc = self._hold(kind, name, size, source, doc_id)
        try:
            return super().adopt(document, kind, name, source=source, size=size, doc_id=doc)
        except BaseException:
            self.rust.close(doc)
            raise

    def python_held_format(self, data: bytes, name: str, kind: str | None) -> Any:
        """The format the Python host holds a document of ``kind`` (or of these bytes) in;
        ``None`` for one of the Rust host's (whose refusal words any other)."""
        if kind is not None:
            fmt = self.formats.get(kind)
            return fmt if fmt is not None and fmt.host is None else None
        return next((fmt for fmt in self.formats.values() if fmt.host is None and fmt.detect(data, name)), None)

    def _hold(self, kind: str, name: str, size: int, source: str, doc_id: str | None) -> str:
        """The Rust session's id of a document the Python host is to hold (decision D3), its
        room counted there."""
        try:
            return self.rust.hold(kind, name=name, size=size, source=source, doc_id=doc_id)
        except Exception as exc:  # noqa: BLE001 -- the host's refusal, read by its maker
            raise self._refused(exc) from None

    def _refused(self, exc: Exception) -> Exception:
        refused = self.hosting.maker.refused(exc)
        if refused is None:
            return exc
        why, message = refused
        return _REFUSALS.get(why, ArgumentError)(message)

    # The Python host's documents' state numbers, as the Rust session was last told them.
    def sync_held(self) -> None:
        """Tell the Rust session the state each document the Python host holds is at now:
        what its datasets' staleness and the rows read of it compare with.  Told wherever such
        a document's state can move -- after a call the Python dispatcher runs, as
        :meth:`lock` lets go, at :meth:`commit`, undo, redo and close -- so a call the Rust
        host runs needs no telling first."""
        for doc, entry in list(self._held.items()):
            version = entry.version
            if self._told.get(doc) != version:
                self.rust.held_moved(doc, version)
                self._told[doc] = version

    def lock(self, doc_ids: Iterable[str]) -> Any:
        """:meth:`Session.lock`; in a session holding documents of the Python host, what the
        application or a call changed under it told the Rust session as the locks go
        (:meth:`sync_held`)."""
        return self._lock_telling(doc_ids) if self._held else super().lock(doc_ids)

    @contextmanager
    def _lock_telling(self, doc_ids: Iterable[str]) -> Iterator[list[Any]]:
        try:
            with super().lock(doc_ids) as entries:
                yield entries
        finally:
            self.sync_held()

    def commit(self, doc_id: str) -> int:
        """:meth:`Session.commit`, the version told the Rust session (:meth:`sync_held`)."""
        if not self._held:
            return super().commit(doc_id)
        try:
            return super().commit(doc_id)
        finally:
            self.sync_held()

    def _free_id(self, doc_id: str) -> None:
        """(The Rust session gave the id, the one counter.)"""

    def _room_for_document(self, size: int) -> None:
        """(The Rust session counts the room.)"""

    def _named(self, doc_id: str) -> str:
        """``doc_id``, an open document's; else ``not_found`` naming the open ones (as
        :meth:`Session.entry` refuses)."""
        open_now = self.rust.documents()
        if doc_id not in open_now:
            raise ToolError("not_found", f"no open document {doc_id!r}", field="doc", valid_options=open_now)
        return doc_id

    def held_here(self, doc_ids: Iterable[str]) -> list[str]:
        return [doc for doc in doc_ids if doc in self._held]

    def held_in_python(self, tool: Any, doc_ids: Iterable[str]) -> None:
        """Refuse a call the Python dispatcher runs that names a document the Rust host holds:
        there is no Python document of it to hold.  A format's tool is refused as on any
        document of another kind; an application's own tool (decision D1) with the way out."""
        doc_ids = [doc for doc in doc_ids if doc not in self._held]
        if not doc_ids:
            return
        open_now = set(self.rust.documents())
        held = [doc for doc in doc_ids if doc in open_now]
        if not held:
            return
        kind = self.kind_of(held[0])
        if any(found for found in tool.handlers if found is not None) and tool.handler_for(kind) is None:
            raise ToolError("invalid_arguments", f"{tool.name} does not work on a {kind} document",
                            field=tool.documents[0] if not callable(tool.documents) and tool.documents else None,
                            valid_options=[f"{e.doc_id} ({e.kind})" for e in self.documents.values()
                                           if tool.handler_for(e.kind) is not None])
        raise ToolError(
            "refused", f"{tool.name} is the application's own tool, and {', '.join(held)} is held by the "
            "Rust host, which keeps no Python document of it: read or change it with the host's tools (an "
            "application's tool that needs a document's bytes names no document and reads them with "
            "session.save)", field="doc", details={"held_by_host": held})

    def kind_of(self, doc_id: str) -> str:
        """The kind of the open document ``doc_id``, as its format names it."""
        held = self._held.get(doc_id)
        return held.kind if held is not None else self.rust.document(self._named(doc_id))[0]

    def entry(self, doc_id: str) -> Any:
        held = self._held.get(doc_id)
        if held is not None:
            return held
        self._named(doc_id)
        raise ArgumentError(f"{doc_id} is held by the Rust host, which keeps no Python entry of it: "
                            "read it with a tool, or save it")

    def provenance(self, doc_id: str) -> dict[str, dict[str, Any]]:
        """:meth:`Session.provenance`: where the values placed in ``doc_id`` from sources came
        from, as the host reads the records its head keeps (the Python host's format, of its
        own); empty for a document that keeps none."""
        if doc_id in self._held:
            return super().provenance(doc_id)
        found = self.rust.version(self._named(doc_id)).records()
        if found is None:
            return {}
        return {str(address): dict(record) for address, record in json.loads(found).get("provenance", ())}

    def refs(self, doc_id: str) -> dict[str, str]:
        """:meth:`Session.refs`: the document's refs as they are now, a copy."""
        if doc_id in self._held:
            return super().refs(doc_id)
        return json.loads(self.rust.refs(self._named(doc_id)))

    def read_refs(self, doc_id: str) -> dict[str, str]:
        """:meth:`Session.read_refs`: the host's refs are those of the version committed."""
        if doc_id in self._held:
            return super().read_refs(doc_id)
        return self.refs(doc_id)

    def save(self, doc_id: str) -> SavedDocument:
        if doc_id in self._held:
            return super().save(doc_id)
        doc, name, kind, version, data, state = self.rust.save(self._named(doc_id))
        return SavedDocument(doc=doc, name=name, kind=kind, version=version, data=bytes(data), state=state)

    def close(self, doc_id: str) -> list[str]:
        if doc_id in self._held:
            self.sync_held()
            super().close(doc_id)       # (its entry let go here; its datasets frozen by the Rust session)
            self._told.pop(doc_id, None)
        return list(self.rust.close(self._named(doc_id)))

    def undo(self, doc_id: str, steps: int = 1, *, scope: str | None = None,
             caller: Mapping[str, Any] | None = None) -> int:
        if doc_id in self._held:
            return self._python_history(super().undo, doc_id, steps, scope, caller)
        return self._rust_history(doc_id, steps, scope, False, caller)

    def redo(self, doc_id: str, steps: int = 1, *, scope: str | None = None,
             caller: Mapping[str, Any] | None = None) -> int:
        if doc_id in self._held:
            return self._python_history(super().redo, doc_id, steps, scope, caller)
        return self._rust_history(doc_id, steps, scope, True, caller)

    def _python_history(self, run: Any, doc_id: str, steps: int, scope: str | None,
                        caller: Mapping[str, Any] | None) -> int:
        try:
            return run(doc_id, steps, scope=scope, caller=caller)
        finally:
            self.sync_held()

    def _rust_history(self, doc_id: str, steps: int, scope: str | None, redo: bool,
                      caller: Mapping[str, Any] | None) -> int:
        if scope is not None:
            kind = self.kind_of(doc_id)
            raise ToolError("invalid_arguments", f"{doc_id} ({kind}) undoes document-wide only: leave scope out",
                            field="scope")
        count, error = self.rust.history(self._named(doc_id), steps, redo=redo,
                                         context=json.dumps(dict(caller), default=str) if caller else None)
        if error is not None:
            raise tool_error(error)
        return count

    # -- datasets: the Rust session's (decision D3; :mod:`.hosted_data`) ----------------------

    @property                               # type: ignore[override]
    def datasets(self) -> Mapping[str, Any]:
        """The datasets by id, each read from the Rust session when asked for."""
        return self.data.mapping()

    @datasets.setter
    def datasets(self, value: Any) -> None:
        if value:                           # (Session.__init__ starts it empty)
            raise AttributeError("a session on the Rust host keeps its datasets with keep_datasets()")

    def keep_datasets(self, entries: Iterable[Any]) -> None:
        self.data.keep(entries)

    def dataset(self, address: str, *, field: str | None = None) -> Any:
        return self.data.get(address, field=field)

    def dataset_ids(self) -> list[str]:
        return self.data.ids()

    def dataset_entries(self) -> list[Any]:
        return list(self.data.entries())

    def drop_dataset(self, address: str) -> None:
        self.data.drop(address)

    # -- images: the Rust session's budgets -----------------------------------------------------

    def images_remaining(self) -> int | None:
        return self.rust.images_remaining()

    def check_images(self, count: int) -> None:
        refused = self.rust.images(count)
        if refused is not None:
            raise tool_error(refused)

    def reserve_images(self, count: int) -> None:
        refused = self.rust.images(count, reserve=True)
        if refused is not None:
            raise tool_error(refused)

    def release_images(self, count: int) -> None:
        self.rust.release_images(count)

    # -- blobs and outputs -----------------------------------------------------------------------

    def add_blob(self, data: bytes, name: str, mime: str | None = None) -> str:
        data = _require_bytes(data, "add_blob")
        try:
            return self.rust.add_blob(data, name, mime)
        except ValueError as exc:
            raise LimitError(str(exc)) from None

    def add_output(self, output: Output) -> None:
        """A file a tool of the application's own host saved: handed to ``on_output``, else
        kept in the Rust session until taken."""
        if self.on_output is not None:
            self.on_output(output)
            return
        if output.state is not None and not isinstance(output.state, str):
            raise TypeError("an output's state on the Rust host is text (its format's)")
        self.rust.add_output(output.name, output.format, output.data, doc=output.doc,
                             validate=json.dumps(output.validate) if output.validate is not None else None,
                             state=output.state)

    def take_outputs(self) -> list[Output]:
        """The outputs saved since the last take, handed over and forgotten."""
        return self._outputs(keep=False)

    @property                               # type: ignore[override]
    def outputs(self) -> list[Output]:
        """The outputs saved since the last take, kept in the Rust session (a copy)."""
        return self._outputs(keep=True)

    @outputs.setter
    def outputs(self, value: Any) -> None:
        if value:                           # (Session.__init__ starts it empty)
            raise AttributeError("a session on the Rust host keeps its outputs: add_output() adds one")

    def _outputs(self, *, keep: bool) -> list[Output]:
        return [Output(name, format, bytes(data), doc=doc, validate=json.loads(validate) if validate else None,
                       state=state)
                for name, format, doc, data, validate, state in self.rust.take_outputs(keep=keep)]

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
        if result.tool == "save_document" and self.on_output is not None:
            # (the one tool that saves files for the application: handed over now, as the
            # application asked; else the Rust session keeps them until taken)
            for output in self.take_outputs():
                self.on_output(output)
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
