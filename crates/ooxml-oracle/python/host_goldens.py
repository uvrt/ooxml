"""Host goldens (Phase 3, lane ph3-diff F1): today's Python host recorded, so that the Rust host
(``ooxml-host``) can be shown to answer every call the same way.

The Python host is :class:`ooxml_edit.tools.Toolbox` with docx-agent's tools.  A recorder hook
(:class:`Recorder`, installed by :func:`install`) watches its sessions and writes each as a
*case*: the documents and blobs the application gave it, then every call with its normalised
result, the saved packages' part digests among them.  The streams are JSON lines in
``crates/ooxml-oracle/host/`` (:data:`HOST`), each line one event of one case::

    {"case": ID, "at": 0, "event": "case", "source": "Python host at <sha>", "test": ...,
     "toolbox": {...}, "session": {...}}
    {"case": ID, "at": 1, "event": "open", "name": "a.docx", "input": {...}, "returned": "d1"}
    {"case": ID, "at": 2, "event": "blob", "name": "draft.md", "input": {...}, "returned": "b1"}
    {"case": ID, "at": 3, "event": "call", "tool": ..., "arguments": {...}, "result": {...},
     "outputs": [{"name", "format", "doc", "validate", "parts": {part: digest}}]}
    {"case": ID, "at": 4, "event": "calls", "calls": [[tool, arguments], ...], "results": [...]}
    ... "undo" / "redo" (doc, steps, scope; returned), "close" (doc; returned), "save" (doc;
    saved: name, kind, version, parts), "new_round"

A call made through a view (:meth:`Session.view`) carries ``view: {"roles", "context"}``, and
``roles``/``context`` when the dispatch narrowed them.  An application call that raised has
``raised`` (the exception's type and message) in place of its result.

``input``: the bytes the application gave, by digest -- ``{"sha256", "path"}``, the path a
repository file with those bytes (``$ROOT/...``) or a file of ``host/inputs`` (``$HOST/...``).

``toolbox``/``session``: what the replay makes them with -- docx-agent's ``TOOLS``, ``FORMAT``
and ``GROUPS`` always (a case whose toolbox has other tools or formats is not recorded: the
application's own tools stay in Python, decision D1), plus the settings that differ from the
defaults (``limits``, ``roles``, ``locale``, ``numbers``, ``block_edits``,
``allow_new_problems``, ``batch_defers``...) and the session's clock.  A ``kit`` other than
``docx`` names the libraries: ``docx+xlsx`` adds xlsx-agent's tools and format (a workbook the
Python host holds beside a Word document; those streams are under ``mixed/`` and xlsx-agent's
suite replays them).  Settings the application changes between calls are a ``settings`` event.

**Concurrent cases.**  A session whose calls overlapped (threads) is recorded with
``concurrent: true``, each call's ``span`` (the recorder's ticks at its start and end) and a
``final`` event (its open documents' part digests once the calls ended).  Its replay is a
serial one in commit order (:func:`expected_of`): the calls that changed something as
``change`` events (``ok`` and ``changed_parts``), ordered by overlap and version, then the final
state -- the batch concurrency oracle (Phase 3 design §7.5), frozen.  Generated streams
(host_features.py, ``record-features``) cover each host feature; tests/test_host_concurrency.py
runs the oracle live on new seeds.

**A result, normalised** (:func:`normalised_result`): what the model reads (``to_json``), plus
the application's ``touched``, ``touched_by_op`` (its keys as strings) and ``changed_parts``.
``duration``, ``timing``, ``block`` and ``memory`` are out: they are how long and on what the call
ran, never the same twice.

**Normalisations** -- what would otherwise differ between two runs of the same host, each
applied when recording and again when replaying:

* *Timing.*  ``Result.duration``/``timing``/``block``/``memory`` are not recorded, nor any
  ``timing`` key of a result's ``data`` (:data:`TIMING_KEYS`, at any depth: a batch's ops).
* *Clock.*  A session made without a clock runs on :data:`FIXED_CLOCK` while recording (and so
  when replayed), so every date a tool writes -- tracked changes, comments, ``docProps/core.xml``
  -- is the same.  A session's own clock is recorded by its value; one whose value changes
  between calls is not recorded (``clock varies``).
* *Faces.*  The layout finds faces in the repository's open faces alone (:data:`FACES`,
  pptx2svg-fonts: Carlito, Caladea, Arimo, Tinos, Cousine...), never in the machine's folders
  (:func:`pin_faces`): page counts, reflow and field page numbers are the same on a Mac with
  Office and on a Linux runner.
* *Pool.*  Replays run the work in-process (``workers=0``) whatever the recording used: the
  pool decides where work runs, not its answer.
* *Packages.*  A saved package is its parts' digests (sha256 of the uncompressed bytes, an
  embedded package by its own parts' digests), not the zip's bytes: two deflate
  implementations write the same parts in other bytes.  For the same reason a saved file's
  ``size`` (``save_document``'s ``data.size``, an output's size) is out (:data:`ENCODING_FIELDS`).
* *Paths.*  Paths of the repository or the home folder in a call or a result are portable
  (``$ROOT``, ``$HOME``, ``goldens.portable``); a temporary folder's path is ``$TMP``.

*Not normalised: the host's own nondeterminism.*  An answer that differs between two runs of
the same host is a finding, not a normalisation: the case is not kept (below), and its why is in
``host/skipped.json``.  The one found when F1 was recorded: an unknown-argument refusal names the
first unknown argument in a set's order, which Python's hash seed decides (``word_style`` with
both ``style`` and ``targets`` unknown) -- the Rust host should name them in the call's order.

**Kept only what replays.**  :func:`bless` replays every recorded case twice before it keeps
it: a case whose replay differs from its recording -- a test that changed a document behind the
toolbox's back -- is left out, with why, in the bless's
summary.  The replay (:func:`replay_case`, ``tests/test_host_goldens.py``) then holds the host to
0 differences on every runner.

**Known differences** (``host/known-differences.toml``, :func:`known_entries`): an entry
admits a difference in some fields of some calls -- ``stream`` and ``case`` globs, optionally
``tool``, ``at`` and ``arguments`` (a subset), and ``fields`` (the paths that may differ, e.g.
``result.warnings``) -- and must give its ``rule`` (the tool design rule that makes the new
answer right: rule 1 only, decision D4) and its ``cause``.

Standard library only at import (the gates' environment); recording and replaying import the
Python host.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fnmatch
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import weakref
import zipfile
from collections import Counter
from dataclasses import fields as dataclass_fields
from dataclasses import is_dataclass
from pathlib import Path
from typing import Any, Callable, Iterable

sys.path.insert(0, str(Path(__file__).resolve().parent))

import goldens  # noqa: E402

ORACLE = goldens.ORACLE
ROOT = goldens.ROOT
#: The host goldens: ``<format>/<kind>/<name>.jsonl`` streams, ``inputs/``, known differences.
HOST = ORACLE / "host"
INPUTS = HOST / "inputs"
KNOWN = HOST / "known-differences.toml"
DOCX_AGENT = ROOT / "python" / "docx-agent"
TRANSCRIPTS = DOCX_AGENT / "tests" / "goldens" / "transcripts"
#: The faces every layout of a recording or replay finds, and no others (:func:`pin_faces`).
FACES = ROOT / "python" / "pptx2svg" / "packages" / "pptx2svg-fonts" / "src" / "pptx2svg_fonts" / "files"
#: The clock of a session made without one, while recording and replaying.
FIXED_CLOCK = dt.datetime(2026, 1, 5, 9, 0, tzinfo=dt.timezone.utc)
SOURCE = "Python host at {sha}"
#: Data fields about how long a call took, anywhere in a result's data.
TIMING_KEYS = frozenset({"timing"})
#: Data fields about how a file is encoded (its zip), not what it holds, by tool.
ENCODING_FIELDS = {"save_document": frozenset({"size"})}
#: The tool design rules a known difference may give (decision D4: rule 1, acceptance).
ALLOWED_RULES = frozenset({1})
#: The user decisions a known difference may give instead (one problems shape; optimistic
#: batches, both 2026-11-25).
ALLOWED_DECISIONS = frozenset({"structured-problems-2026-11-25", "optimistic-batches-2026-11-25",
                               "stamp-on-open-2026-11-26"})
#: Repository folders whose files an input may name instead of a copy in host/inputs.
REPOSITORY_INPUTS = (ORACLE / "fixtures", ORACLE / "corpus", ORACLE / "cases" / "inputs",
                     DOCX_AGENT / "tests" / "goldens")


# -- JSON ------------------------------------------------------------------------------------


def canonical(value: Any) -> str:
    return goldens.canonical(value)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class NotJson(Exception):
    """A value the streams cannot hold (the case is not recorded)."""


def plain(value: Any) -> Any:
    """``value`` as JSON holds it (tuples as lists); :class:`NotJson` for anything else."""
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise NotJson(f"the number {value!r}")
        return value
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in value]
    if isinstance(value, dict) or hasattr(value, "items") and callable(value.items):
        out = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise NotJson(f"a key {key!r}")
            out[key] = plain(item)
        return out
    raise NotJson(f"a {type(value).__name__}")


_TMP_ROOTS = tuple(dict.fromkeys(os.path.realpath(p) for p in (tempfile.gettempdir(), "/tmp", "/private/tmp")))


def portable(value: Any) -> Any:
    """Paths made portable: ``$ROOT``/``$HOME`` (goldens.portable) and ``$TMP``."""
    if isinstance(value, str):
        for root in _TMP_ROOTS:
            for spelling in (root, root.removeprefix("/private")):
                if spelling not in ("", "/") and value.startswith(spelling + "/"):
                    return "$TMP" + value[len(spelling):]
        return goldens.portable(value)
    if isinstance(value, list):
        return [portable(item) for item in value]
    if isinstance(value, dict):
        return {key: portable(item) for key, item in value.items()}
    return value


def _without_timing(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: _without_timing(item) for key, item in value.items() if key not in TIMING_KEYS}
    if isinstance(value, list):
        return [_without_timing(item) for item in value]
    return value


# -- results and packages --------------------------------------------------------------------


def normalised_result(result: Any) -> dict:
    """A :class:`~ooxml_edit.tools.Result` as the streams hold it (the module's notes)."""
    body = plain(result.to_json())
    if isinstance(body.get("data"), (dict, list)):
        data = _without_timing(body["data"])
        encoded = ENCODING_FIELDS.get(result.tool, frozenset())
        if encoded and isinstance(data, dict):
            data = {key: value for key, value in data.items() if key not in encoded}
        body["data"] = data
    app = {"touched": plain(result.touched),
           "touched_by_op": {str(key): plain(value) for key, value in sorted(result.touched_by_op.items())},
           "changed_parts": plain(result.changed_parts)}
    body["app"] = {key: value for key, value in app.items() if value}
    return portable(body)


def part_digests(data: bytes) -> dict[str, str]:
    """A package's parts by name: each the sha256 of its uncompressed bytes, or, for an
    embedded package (a zip holding ``[Content_Types].xml``), ``package:`` and the digest of
    its own parts' digests."""
    out = {}
    with zipfile.ZipFile(io.BytesIO(data)) as package:
        names = package.namelist()
        if len(set(names)) != len(names):
            raise ValueError(f"a part is in the package twice: {sorted(names)}")
        for name in sorted(names):
            out[name] = _part_digest(package.read(name))
    return out


def _part_digest(body: bytes) -> str:
    if body.startswith(b"PK\x03\x04"):
        try:
            with zipfile.ZipFile(io.BytesIO(body)) as inner:
                if "[Content_Types].xml" in inner.namelist():
                    return "package:" + sha256(canonical(part_digests(body)).encode())
        except zipfile.BadZipFile:
            pass
    return sha256(body)


def described_output(output: Any) -> dict:
    """An :class:`~ooxml_edit.tools.session.Output`: its description less the zip's size, and
    its parts' digests (a file that is no package: its digest)."""
    try:
        parts: Any = part_digests(output.data)
    except zipfile.BadZipFile:
        parts = {"": sha256(output.data)}
    return portable({"name": output.name, "format": output.format, "doc": output.doc,
                     "validate": plain(output.validate), "parts": parts})


# -- inputs ----------------------------------------------------------------------------------

_REPOSITORY_INDEX: dict[str, str] | None = None
_INDEX_LOCK = threading.Lock()


def _repository_index() -> dict[str, str]:
    """sha256 -> the repository file with those bytes, among :data:`REPOSITORY_INPUTS`."""
    global _REPOSITORY_INDEX
    with _INDEX_LOCK:
        if _REPOSITORY_INDEX is None:
            found: dict[str, str] = {}
            for folder in REPOSITORY_INPUTS:
                if folder.is_dir():
                    for path in sorted(folder.rglob("*")):
                        if path.is_file() and path.suffix.lower() in (".docx", ".dotx", ".docm", ".xlsx", ".png",
                                                                      ".jpg", ".jpeg", ".md", ".csv", ".txt",
                                                                      ".svg", ".emf", ".gif", ".pptx"):
                            found.setdefault(sha256(path.read_bytes()), "$ROOT/" + path.relative_to(ROOT).as_posix())
            _REPOSITORY_INDEX = found
        return _REPOSITORY_INDEX


def keep_input(data: bytes, name: str, inputs: Path) -> dict:
    """Where the replay finds ``data``: a repository file with these bytes, else a copy in
    ``inputs`` (``$HOST/inputs/<sha256>.<suffix>``)."""
    digest = sha256(data)
    known = _repository_index().get(digest)
    if known is not None:
        return {"sha256": digest, "path": known}
    suffix = Path(name).suffix.lower()
    suffix = suffix if suffix and len(suffix) <= 6 and suffix[1:].isalnum() else ".bin"
    target = inputs / f"{digest}{suffix}"
    if not target.exists():
        inputs.mkdir(parents=True, exist_ok=True)
        partial = target.with_name(f".{target.name}.{os.getpid()}.{threading.get_ident()}")
        partial.write_bytes(data)
        os.replace(partial, target)
    return {"sha256": digest, "path": f"$HOST/inputs/{target.name}"}


def input_bytes(spec: dict, host: Path = HOST) -> bytes:
    path = spec["path"]
    if path.startswith("$HOST/"):
        found = host / path[len("$HOST/"):]
        if not found.exists():          # a staged recording's inputs are blessed into HOST
            found = HOST / path[len("$HOST/"):]
    else:
        found = Path(goldens.local(path))
    data = found.read_bytes()
    if sha256(data) != spec["sha256"]:
        raise ValueError(f"{path} is not the input recorded ({spec['sha256'][:12]})")
    return data


# -- the faces -------------------------------------------------------------------------------


def pin_faces() -> None:
    """Every Word layout of this process finds faces in :data:`FACES` alone (the module's
    notes, *Faces*): docx2svg's folders, read by the session's page engines."""
    from docx2svg import rust

    if getattr(rust.font_dirs, "_host_goldens", False):
        return

    def pinned(options: Any) -> tuple[Path, ...]:   # noqa: ARG001 -- every layout the same
        return (FACES,)

    pinned._host_goldens = True  # type: ignore[attr-defined]
    rust.font_dirs = pinned
    with rust._FONTS_LOCK:
        rust._FONTS.clear()


# -- configuration ---------------------------------------------------------------------------


def _differing(value: Any, default: Any) -> dict:
    """A frozen dataclass's fields that differ from ``default``'s, as JSON."""
    return {f.name: plain(getattr(value, f.name)) for f in dataclass_fields(value)
            if getattr(value, f.name) != getattr(default, f.name)}


def _setting(value: Any, default: Any) -> Any:
    if is_dataclass(value):
        return _differing(value, default)
    return plain(value)


#: The kits a recorded toolbox may be: the libraries' tools, formats and groups, by name.
#: ``docx``: docx-agent's; ``docx+xlsx``: docx-agent's and xlsx-agent's (a workbook held by the
#: Python host beside a Word document: datasets, sources and facts across them).
KITS = ("docx", "docx+xlsx")


def kit_parts(kit: str) -> tuple[list, list, list]:
    """``(tools, formats, groups)`` of ``kit``."""
    from docx_agent import tools as docx

    if kit == "docx":
        return list(docx.TOOLS), [docx.FORMAT], list(docx.GROUPS)
    if kit == "docx+xlsx":
        from xlsx_agent import tools as xlsx

        return [*docx.TOOLS, *xlsx.TOOLS], [docx.FORMAT, xlsx.FORMAT], [*docx.GROUPS, *xlsx.GROUPS]
    raise ValueError(f"no kit {kit!r}")


_REFERENCE: dict[str, Any] = {}


def reference_toolbox(kit: str = "docx") -> Any:
    """``kit``'s toolbox with every default: what a recorded toolbox is compared with."""
    if kit not in _REFERENCE:
        from ooxml_edit.tools import Toolbox

        tools, formats, groups = kit_parts(kit)
        _REFERENCE[kit] = Toolbox(tools, formats=formats, groups=groups, workers=0)
    return _REFERENCE[kit]


def kit_of(box: Any) -> str:
    """The kit ``box`` is; :class:`NotJson` (why) for none."""
    why = "the application's own tools"
    for kit in KITS:
        try:
            ref = reference_toolbox(kit)
        except ImportError:
            continue
        if list(box.formats) != list(ref.formats):
            continue
        if any(box.formats[k] is not ref.formats[k] for k in box.formats):
            why = "the application's own formats"
            continue
        if [_tool_key(t) for t in box.tools.values()] != [_tool_key(t) for t in ref.tools.values()]:
            theirs = set(box.tools) ^ set(ref.tools)
            why = "the application's own tools" + (f" ({', '.join(sorted(theirs))})" if theirs else "")
            continue
        if list(box.groups) != list(ref.groups):
            why = "other tool groups"
            continue
        return kit
    if not any(set(box.formats) == set(reference_toolbox(k).formats) for k in KITS if k in _REFERENCE):
        why = "the application's own formats" if why == "the application's own tools" else why
    raise NotJson(why)


def _tool_key(tool: Any) -> tuple:
    return (tool.name, tuple(sorted((str(kind), id(handler)) for kind, handler in tool.handlers.items())),
            canonical(plain(tool.schema)))


#: Toolbox settings the streams hold (the others are where the work runs: the pool).
TOOLBOX_SETTINGS = ("limits", "allow_new_problems", "log_arguments", "font_dirs", "locale", "roles", "numbers",
                    "block_edits", "batch_defers", "strict_first")
SESSION_SETTINGS = ("limits", "font_dirs", "locale", "roles", "numbers")


def toolbox_config(box: Any) -> dict:
    """The settings a replay makes ``box`` again with (its ``kit`` when not ``docx``);
    :class:`NotJson` (why) for a toolbox the streams cannot hold: other tools, formats or
    groups, its own error map."""
    kit = kit_of(box)
    ref = reference_toolbox(kit)
    if box._errors != ref._errors:
        raise NotJson("the application's own error map")
    if box.font_dirs:
        raise NotJson("the application's font folders (the faces are pinned)")
    out: dict = {} if kit == "docx" else {"kit": kit}
    for name in TOOLBOX_SETTINGS:
        value, default = getattr(box, name), getattr(ref, name)
        if value != default:
            out[name] = _setting(value, default)
    return out


def session_config(session: Any) -> dict:
    ref = reference_session()
    if session.font_dirs:
        raise NotJson("the application's font folders (the faces are pinned)")
    out = {}
    for name in SESSION_SETTINGS:
        value, default = getattr(session, name), getattr(ref, name)
        if value != default:
            out[name] = _setting(value, default)
    return out


_REF_SESSION: Any = None


def reference_session() -> Any:
    global _REF_SESSION
    if _REF_SESSION is None:
        from docx_agent.tools import FORMAT
        from ooxml_edit.tools.session import Session

        _REF_SESSION = Session([FORMAT], clock=lambda: FIXED_CLOCK)
    return _REF_SESSION


def _built(kind: str, given: dict) -> Any:
    """A setting from its recorded fields."""
    if kind == "limits":
        from ooxml_edit.tools.limits import Limits

        return Limits(**{k: tuple(v) if isinstance(v, list) else v for k, v in given.items()})
    if kind == "locale":
        from ooxml_common.locales import EN_US
        import dataclasses

        return dataclasses.replace(EN_US, **given)
    if kind == "numbers":
        from ooxml_edit.tools.numbers import NumberRules

        return NumberRules(**{k: tuple(v) if isinstance(v, list) else v for k, v in given.items()})
    if kind in ("roles", "strict_first", "font_dirs"):
        return None if given is None else list(goldens.local(given))
    return given


def apply_settings(target: Any, config: dict | None, reference: Any, names: tuple[str, ...]) -> None:
    """A ``settings`` event: each of ``names`` set on ``target`` as ``config`` gives it, the
    default (``reference``'s) where it gives none."""
    if config is None:
        return
    for name in names:
        if name in config:
            value = _built(name, config[name])
            if name == "roles":
                value = tuple(value)
            setattr(target, name, value)
        else:
            setattr(target, name, getattr(reference, name))


def make_toolbox(config: dict) -> Any:
    from ooxml_edit.tools import Toolbox

    tools, formats, groups = kit_parts(config.get("kit", "docx"))
    settings = {name: _built(name, value) for name, value in config.items() if name not in ("batch_defers", "kit")}
    box = Toolbox(tools, formats=formats, groups=groups, workers=0, **settings)
    if "batch_defers" in config:
        box.batch_defers = config["batch_defers"]
    return box


def make_session(box: Any, config: dict, clock: str) -> Any:
    at = dt.datetime.fromisoformat(clock)
    settings = {name: _built(name, value) for name, value in config.items()}
    return box.session(clock=lambda: at, **settings)


# -- the recorder ----------------------------------------------------------------------------


class _Case:
    """One session's events as they are recorded."""

    def __init__(self, test: str, number: int) -> None:
        self.test = test
        self.number = number
        self.events: list[dict] = []
        self.toolbox: dict | None = None
        self.toolbox_seen: Any = None
        self.session: dict | None = None
        #: The settings as the last call found them (a ``settings`` event when they change).
        self.toolbox_now: dict | None = None
        self.session_now: dict | None = None
        self.clocks: set[str] = set()
        self.tainted: str | None = None
        self.active = 0
        self.lock = threading.Lock()
        self.calls = 0
        #: Outputs the call in flight added.
        self.outputs_now: list[dict] = []
        self.recorded_clock: Any = None
        self.fixed_clock = False
        self.formats: list = []
        #: Calls overlapped: the session is kept (``keep``) until its final state is taken.
        self.concurrent = False
        self.session_ref: Any = None
        self.keep: Any = None
        self.final: dict | None = None

    def taint(self, why: str) -> None:
        if self.tainted is None:
            self.tainted = why


class Recorder:
    """The hook: patches the Python host's ``Session`` and ``Toolbox`` so that every session
    made while it is installed is recorded as a case (the module's notes).  ``test()`` names
    the cases a session belongs to (pytest's current test, a transcript's name)."""

    def __init__(self, inputs: Path, test: Callable[[], str]) -> None:
        self.inputs = inputs
        self.test = test
        self.cases: list[_Case] = []
        self._by_session: "weakref.WeakKeyDictionary[Any, _Case]" = weakref.WeakKeyDictionary()
        self._numbers: Counter = Counter()
        self._lock = threading.Lock()
        self._undo: list[Callable[[], None]] = []
        #: Whether this thread is inside a recorded call: the host's own calls of the API it
        #: makes then are part of that call, not the application's.
        self._inside = threading.local()
        self._ticks = 0

    # -- installing

    def install(self) -> "Recorder":
        from ooxml_edit.tools import dispatch as dispatch_module
        from ooxml_edit.tools import session as session_module

        pin_faces()
        Session, SessionView, Toolbox = session_module.Session, session_module.SessionView, dispatch_module.Toolbox
        recorder = self

        def patch(owner: Any, name: str, make: Callable[[Callable], Callable]) -> None:
            original = owner.__dict__[name]
            setattr(owner, name, make(original))
            self._undo.append(lambda: setattr(owner, name, original))

        def init(original):
            def __init__(session, formats=(), *, clock=None, **kwargs):
                case = recorder._new_case()
                fixed = clock is None
                given = clock or (lambda: FIXED_CLOCK)

                def recorded_clock():
                    value = given()
                    with case.lock:
                        case.clocks.add(value.isoformat() if hasattr(value, "isoformat") else repr(value))
                    return value

                original(session, formats, clock=recorded_clock, **kwargs)
                case.recorded_clock = recorded_clock
                case.fixed_clock = fixed
                case.formats = list(formats)
                case.session_ref = weakref.ref(session)
                with recorder._lock:
                    recorder._by_session[session] = case
            return __init__

        def app_call(kind: str, original, describe: Callable[..., dict], answer: Callable[[Any], Any]):
            def wrapper(session, *args, **kwargs):
                case = recorder._case_of(session)
                if case is None or recorder._is_inside():
                    return original(session, *args, **kwargs)
                event = {"event": kind}
                try:
                    event.update(describe(*args, **kwargs))
                except NotJson as why:
                    case.taint(f"{kind}: {why}")
                except TypeError:
                    case.taint(f"{kind}: arguments it cannot read")
                if isinstance(session, SessionView):
                    event["view"] = recorder._view(session, case)
                with recorder._active(case):
                    try:
                        out = original(session, *args, **kwargs)
                    except Exception as exc:
                        event["raised"] = f"{type(exc).__name__}: {exc}"
                        recorder._add(case, event)
                        raise
                try:
                    event.update(answer(out))
                except NotJson as why:
                    case.taint(f"{kind}: {why}")
                recorder._add(case, event)
                return out
            return wrapper

        def open_args(data, name, *, kind=None, source="app", doc_id=None, state=None):
            return {"name": name, "kind": kind, "source": source, "doc_id": doc_id, "state": plain(state),
                    "input": keep_input(bytes(data), name, recorder.inputs)}

        def blob_args(data, name, mime=None):
            return {"name": name, "mime": mime, "input": keep_input(bytes(data), name, recorder.inputs)}

        def history_args(doc_id, steps=1, *, scope=None, caller=None):
            if caller:
                raise NotJson("a caller given to the session, not through a view")
            return {"doc": doc_id, "steps": steps, "scope": scope}

        def returned(value):
            return {"returned": plain(value)}

        def saved(value):
            return {"saved": {"doc": value.doc, "name": value.name, "kind": value.kind, "version": value.version,
                              "parts": part_digests(value.data), "state": plain(value.state)}}

        patch(Session, "__init__", init)
        patch(Session, "open", lambda o: app_call("open", o, open_args, returned))
        patch(Session, "add_blob", lambda o: app_call("blob", o, blob_args, returned))
        patch(Session, "close", lambda o: app_call("close", o, lambda doc_id: {"doc": doc_id}, returned))
        patch(Session, "save", lambda o: app_call("save", o, lambda doc_id: {"doc": doc_id}, saved))
        patch(Session, "new_round", lambda o: app_call("new_round", o, lambda: {}, lambda _v: {}))
        for name in ("undo", "redo"):
            patch(Session, name, lambda o, name=name: app_call(name, o, history_args, returned))
            patch(SessionView, name, lambda o, name=name: app_call(name, o, history_args, returned))
        for name, why in (("adopt", "a document adopted from Python objects"),
                          ("keep_datasets", "datasets kept by the application"),
                          ("drop_dataset", "a dataset dropped by the application"),
                          ("commit", "a commit by the application"),
                          ("owned_history", "history read by the application")):
            patch(Session, name, lambda o, why=why: self._tainting(o, why))

        def add_output(original):
            def wrapper(session, output):
                case = recorder._case_of(session)
                if case is not None:
                    if not case.active:
                        case.taint("an output added by the application")
                    else:
                        try:
                            case.outputs_now.append(described_output(output))
                        except Exception as exc:  # noqa: BLE001 -- a case the streams cannot hold
                            case.taint(f"an output: {exc}")
                return original(session, output)
            return wrapper

        patch(Session, "add_output", add_output)

        def dispatch(original):
            def wrapper(box, session, name, arguments, *, roles=None, context=None):
                case = recorder._case_of(session)
                if case is None or recorder._is_inside():
                    return original(box, session, name, arguments, roles=roles, context=context)
                event = recorder._call_event(case, box, session, roles, context)
                try:
                    event.update({"tool": plain(name), "arguments": goldens.portable(plain(arguments))})
                except NotJson as why:
                    case.taint(f"arguments: {why}")
                start = recorder._tick()
                with recorder._active(case):
                    try:
                        result = original(box, session, name, arguments, roles=roles, context=context)
                    except Exception as exc:
                        event["raised"] = f"{type(exc).__name__}: {exc}"
                        recorder._add(case, event)
                        raise
                    finally:
                        event["outputs"] = case.outputs_now
                event["span"] = [start, recorder._tick()]
                try:
                    event["result"] = normalised_result(result)
                except NotJson as why:
                    case.taint(f"a result: {why}")
                recorder._add(case, event)
                return result
            return wrapper

        def dispatch_many(original):
            def wrapper(box, session, calls, *, max_workers=8, roles=None, context=None):
                case = recorder._case_of(session)
                if case is None or recorder._is_inside():
                    return original(box, session, calls, max_workers=max_workers, roles=roles, context=context)
                event = recorder._call_event(case, box, session, roles, context)
                event["event"] = "calls"
                try:
                    event["calls"] = goldens.portable(plain([[name, arguments] for name, arguments in calls]))
                except (NotJson, TypeError, ValueError) as why:
                    case.taint(f"calls: {why}")
                start = recorder._tick()
                with recorder._active(case):
                    try:
                        results = original(box, session, calls, max_workers=max_workers, roles=roles,
                                           context=context)
                    except Exception as exc:
                        event["raised"] = f"{type(exc).__name__}: {exc}"
                        recorder._add(case, event)
                        raise
                    finally:
                        event["outputs"] = case.outputs_now
                event["span"] = [start, recorder._tick()]
                try:
                    event["results"] = [normalised_result(result) for result in results]
                except NotJson as why:
                    case.taint(f"a result: {why}")
                recorder._add(case, event)
                return results
            return wrapper

        patch(Toolbox, "dispatch", dispatch)
        patch(Toolbox, "dispatch_many", dispatch_many)
        return self

    def uninstall(self) -> None:
        while self._undo:
            self._undo.pop()()

    # -- recording

    def _new_case(self) -> _Case:
        test = self.test()
        with self._lock:
            self._numbers[test] += 1
            case = _Case(test, self._numbers[test])
            self.cases.append(case)
        return case

    def _case_of(self, session: Any) -> _Case | None:
        base = getattr(session, "session", session) if type(session).__name__ == "SessionView" else session
        with self._lock:
            return self._by_session.get(base)

    def _tick(self) -> int:
        """The next number of the recorder's clock of call starts and ends: a concurrent case's
        calls that did not overlap keep their order (:func:`expected_of`)."""
        with self._lock:
            self._ticks += 1
            return self._ticks

    def _is_inside(self) -> bool:
        return getattr(self._inside, "depth", 0) > 0

    @contextlib.contextmanager
    def _active(self, case: _Case):
        with case.lock:
            if case.active and not case.concurrent:
                # Calls from several threads at once: a concurrent case, held to a serial
                # replay of its changes in commit order (:func:`expected_of`).
                case.concurrent = True
                case.keep = case.session_ref() if case.session_ref is not None else None
            case.active += 1
            case.outputs_now = []
        self._inside.depth = getattr(self._inside, "depth", 0) + 1
        try:
            yield
        finally:
            self._inside.depth -= 1
            with case.lock:
                case.active -= 1

    def _tainting(self, original: Callable, why: str) -> Callable:
        recorder = self

        def wrapper(session, *args, **kwargs):
            case = recorder._case_of(session)
            if case is not None and not recorder._is_inside():
                case.taint(why)
            return original(session, *args, **kwargs)
        return wrapper

    def _view(self, view: Any, case: _Case) -> dict:
        try:
            return {"roles": list(view.roles), "context": plain(dict(view.context))}
        except NotJson as why:
            case.taint(f"a view's context: {why}")
            return {}

    def _call_event(self, case: _Case, box: Any, session: Any, roles: Any, context: Any) -> dict:
        event: dict = {"event": "call"}
        try:
            config = toolbox_config(box)
        except NotJson as why:
            case.taint(str(why))
            config = {}
        settings: dict = {}
        if case.toolbox_seen is None:
            case.toolbox_seen = box
            case.toolbox = case.toolbox_now = config
        elif case.toolbox_seen is not box:
            case.taint("calls through two toolboxes")
        elif config != case.toolbox_now:
            settings["toolbox"] = case.toolbox_now = config
        base = session.session if type(session).__name__ == "SessionView" else session
        try:
            now = session_config(base)
        except NotJson as why:
            case.taint(f"the session: {why}")
            now = {}
        if case.session is None:
            case.session = case.session_now = now
        elif case.session_now != now:
            settings["session"] = case.session_now = now
        if settings:            # the application set the toolbox's or session's settings since
            self._add(case, {"event": "settings", **settings})
        if base.clock is not case.recorded_clock:
            case.taint("the session's clock replaced")
        if case.toolbox is not None and case.formats != list(
                reference_toolbox(case.toolbox.get("kit", "docx")).formats.values()):
            case.taint("a session of other formats")
        if type(session).__name__ == "SessionView":
            event["view"] = self._view(session, case)
        try:
            if roles is not None:
                event["roles"] = [roles] if isinstance(roles, str) else plain(list(roles))
            if context is not None:
                event["context"] = plain(dict(context))
        except NotJson as why:
            case.taint(f"a call's context: {why}")
        return event

    def _add(self, case: _Case, event: dict) -> None:
        with case.lock:
            if event["event"] in ("call", "calls"):
                case.calls += 1 if event["event"] == "call" else len(event.get("calls") or ())
            case.events.append(event)

    # -- the cases

    def settle(self) -> None:
        """Each concurrent case's final state, taken once its calls have ended (a test's end):
        the part digests of every document its session holds open."""
        for case in self.cases:
            if case.concurrent and case.final is None and case.keep is not None:
                session, case.keep = case.keep, None
                self._inside.depth = getattr(self._inside, "depth", 0) + 1
                try:
                    case.final = {doc: part_digests(session.save(doc).data) for doc in sorted(session.documents)}
                    self._add(case, {"event": "final", "documents": case.final})
                except Exception as exc:  # noqa: BLE001 -- a case whose end cannot be read
                    case.taint(f"its final state: {type(exc).__name__}: {exc}")
                finally:
                    self._inside.depth -= 1

    def finished(self, source: str, stream: Callable[[_Case], str]) -> tuple[dict[str, list[dict]], list[dict]]:
        """``({stream: lines}, skipped)``: each recorded case with calls, as lines; the cases not
        recorded, each ``{"case", "why"}``."""
        self.settle()
        streams: dict[str, list[dict]] = {}
        skipped = []
        for case in self.cases:
            if not case.calls:
                continue
            name = stream(case)
            case_id = f"{case.test}#{case.number}"
            if case.tainted is None and len(case.clocks) > 1:
                case.taint("clock varies")
            if case.tainted is None and case.concurrent and case.final is None:
                case.taint("calls from several threads at once, its final state not taken")
            if case.tainted is not None:
                skipped.append({"stream": name, "case": case_id, "why": case.tainted, "calls": case.calls})
                continue
            clock = FIXED_CLOCK.isoformat() if case.fixed_clock else next(iter(case.clocks))
            head = {"case": case_id, "at": 0, "event": "case", "source": source, "test": case.test,
                    "toolbox": case.toolbox or {}, "session": {**(case.session or {}), "clock": clock}}
            if case.concurrent:
                head["concurrent"] = True
            lines = streams.setdefault(name, [])
            lines.append(head)
            for at, event in enumerate(case.events, 1):
                if not case.concurrent:
                    event = {key: value for key, value in event.items() if key != "span"}
                lines.append({"case": case_id, "at": at, **event})
        return streams, skipped


def install(inputs: Path, test: Callable[[], str]) -> Recorder:
    return Recorder(inputs, test).install()


def host_sha() -> str:
    """The commit the Python host was recorded at: the last that changed the packages or the
    crates (the oracle's own files left out), with ``+dirty`` when they have changes."""
    def git(*args: str) -> str:
        return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True,
                              check=False).stdout.strip()

    paths = ["python", "crates", ":(exclude)crates/ooxml-oracle"]
    sha = git("log", "-1", "--format=%H", "--", *paths) or "unknown"
    dirty = git("status", "--porcelain", "--untracked-files=no", "--", *paths)
    return sha[:12] + ("+dirty" if dirty else "")


# -- streams ---------------------------------------------------------------------------------


def write_stream(path: Path, lines: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(canonical(line) + "\n" for line in lines), encoding="utf-8")


def read_stream(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def streams(host: Path = HOST) -> dict[str, Path]:
    """Every stream below ``host``: its name (``docx/tests/test_x``) -> its file."""
    return {path.relative_to(host).with_suffix("").as_posix(): path
            for path in sorted(host.rglob("*.jsonl")) if "inputs" not in path.relative_to(host).parts}


def cases_of(lines: list[dict]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for line in lines:
        out.setdefault(line["case"], []).append(line)
    return out


# -- replay ----------------------------------------------------------------------------------


def _session_or_view(session: Any, view: dict | None) -> Any:
    if not view:
        return session
    return session.view(roles=view.get("roles"), context=view.get("context") or None)


def _changing(result: dict | None) -> bool:
    return bool(result and result.get("ok") and (result.get("app") or {}).get("changed_parts"))


def _commit_order(changes: list[tuple[tuple, dict]]) -> list[dict]:
    """A concurrent case's changes in the order they were committed.  The calls are grouped by
    overlap (each call's span: the recorder's tick when it started and when it ended): calls
    whose spans chain together ran at once, and a group that ended before the next began came
    first.  Within a group the version each call left orders them -- a commit's version is one
    past the one before it.  An undo does not advance the version (it goes back), so undos run
    beside other changes cannot be ordered here: the recorded scenarios undo only once their
    writers have ended."""
    by_start = sorted(changes, key=lambda item: (item[0][0], item[0][3]))
    groups: list[list[tuple[tuple, dict]]] = []
    end = None
    for item in by_start:
        if end is None or item[0][0] > end:
            groups.append([])
            end = item[0][1]
        else:
            end = max(end, item[0][1])
        groups[-1].append(item)
    return [change for group in groups for _key, change in sorted(group, key=lambda item: (item[0][2], item[0][3]))]


def expected_of(lines: list[dict]) -> list[dict]:
    """What a replay of a case must give.  A case of calls on one thread: its lines.  A
    concurrent case (calls from several threads at once): its calls that changed something,
    as ``change`` events in the order their versions were committed (each answer its ``ok``
    and ``changed_parts``), in the slots the calls took among the application's own events,
    and its ``final`` state -- what a serial replay in commit order must give; the reads, and
    calls that changed nothing, are left out (when they ran decided what they saw)."""
    head = lines[0]
    if not head.get("concurrent"):
        return lines
    changes: list[tuple[tuple, dict]] = []
    shape: list[dict | None] = []
    for event in lines[1:]:
        kind = event["event"]
        if kind == "call":
            items = [(event["tool"], event["arguments"], event.get("result"))]
        elif kind == "calls":
            items = [(c[0], c[1], r) for c, r in zip(event.get("calls") or [], event.get("results") or [])]
        else:
            shape.append(event)
            continue
        for tool, arguments, result in items:
            if not _changing(result):
                continue
            change = {"event": "change", "tool": tool, "arguments": arguments,
                      "result": {"ok": True, "changed_parts": result["app"]["changed_parts"]}}
            for key in ("view", "roles", "context"):
                if key in event:
                    change[key] = event[key]
            span = event.get("span") or [len(changes), len(changes)]
            changes.append(((span[0], span[1], result.get("version") or 0, len(changes)), change))
            shape.append(None)
    ordered = iter(_commit_order(changes))
    out = [head]
    for at, event in enumerate(shape, 1):
        body = next(ordered) if event is None else {k: v for k, v in event.items() if k not in ("case", "at")}
        out.append({"case": head["case"], "at": at, **body})
    return out


def replay_case(lines: list[dict], host: Path = HOST) -> list[dict]:
    """A case's events as the Python host answers them now: the lines it was recorded as, with
    each answer (``returned``, ``saved``, ``result``/``results``, ``outputs``, ``raised``) made
    again; a concurrent case's :func:`expected_of`, run serially."""
    pin_faces()
    lines = expected_of(lines)
    head, events = lines[0], lines[1:]
    replayed = [head]
    with make_toolbox(head["toolbox"]) as box:
        session = make_session(box, {k: v for k, v in head["session"].items() if k != "clock"},
                               head["session"]["clock"])
        outputs_now: list = []
        session.on_output = outputs_now.append
        for event in events:
            out = {key: value for key, value in event.items()
                   if key not in ("returned", "saved", "result", "results", "outputs", "raised", "documents")}
            target = _session_or_view(session, event.get("view"))
            outputs_now.clear()
            try:
                kind = event["event"]
                if kind == "open":
                    value = session.open(input_bytes(event["input"], host), event["name"], kind=event["kind"],
                                         source=event["source"], doc_id=event["doc_id"], state=event["state"])
                    out["returned"] = plain(value)
                elif kind == "blob":
                    out["returned"] = session.add_blob(input_bytes(event["input"], host), event["name"],
                                                       event["mime"])
                elif kind == "close":
                    out["returned"] = plain(session.close(event["doc"]))
                elif kind == "save":
                    saved = session.save(event["doc"])
                    out["saved"] = {"doc": saved.doc, "name": saved.name, "kind": saved.kind,
                                    "version": saved.version, "parts": part_digests(saved.data),
                                    "state": plain(saved.state)}
                elif kind in ("undo", "redo"):
                    out["returned"] = plain(getattr(target, kind)(event["doc"], event["steps"], scope=event["scope"]))
                elif kind == "new_round":
                    session.new_round()
                elif kind == "settings":
                    apply_settings(box, event.get("toolbox"), reference_toolbox(head["toolbox"].get("kit", "docx")),
                                   TOOLBOX_SETTINGS)
                    apply_settings(session, event.get("session"), reference_session(), SESSION_SETTINGS)
                elif kind == "call":
                    result = box.dispatch(target, goldens.local(event["tool"]), goldens.local(event["arguments"]),
                                          roles=event.get("roles"), context=event.get("context"))
                    out["outputs"] = [described_output(o) for o in outputs_now]
                    out["result"] = normalised_result(result)
                elif kind == "change":
                    result = box.dispatch(target, goldens.local(event["tool"]), goldens.local(event["arguments"]),
                                          roles=event.get("roles"), context=event.get("context"))
                    out["result"] = {"ok": result.ok, "changed_parts": plain(result.changed_parts)}
                elif kind == "final":
                    out["documents"] = {doc: part_digests(session.save(doc).data) for doc in sorted(session.documents)}
                elif kind == "calls":
                    results = box.dispatch_many(target, [tuple(call) for call in goldens.local(event["calls"])],
                                                roles=event.get("roles"), context=event.get("context"))
                    out["outputs"] = [described_output(o) for o in outputs_now]
                    out["results"] = [normalised_result(result) for result in results]
                else:
                    raise ValueError(f"an event the replay does not know: {kind}")
            except Exception as exc:  # noqa: BLE001 -- compared with what the recording raised
                if event["event"] in ("call", "calls"):
                    out["outputs"] = [described_output(o) for o in outputs_now]
                out["raised"] = f"{type(exc).__name__}: {exc}"
            replayed.append(out)
    return replayed


def _paths(value: Any, prefix: str = "") -> dict[str, Any]:
    """``value`` flattened to its leaves by path (``result.data.items[0].text``)."""
    if isinstance(value, dict) and value:
        out = {}
        for key, item in value.items():
            out.update(_paths(item, f"{prefix}.{key}" if prefix else key))
        return out
    if isinstance(value, list) and value:
        out = {}
        for index, item in enumerate(value):
            out.update(_paths(item, f"{prefix}[{index}]"))
        return out
    return {prefix: value}


def differences(recorded: list[dict], replayed: list[dict]) -> list[dict]:
    """Where a replay differs from its recording: per event, the leaf paths that differ."""
    out = []
    for index in range(max(len(recorded), len(replayed))):
        want = recorded[index] if index < len(recorded) else {}
        have = replayed[index] if index < len(replayed) else {}
        if want == have:
            continue
        a, b = _paths(want), _paths(have)
        paths = sorted(path for path in set(a) | set(b) if a.get(path, KeyError) != b.get(path, KeyError))
        out.append({"at": want.get("at", have.get("at")), "event": want.get("event", have.get("event")),
                    "tool": want.get("tool"), "arguments": want.get("arguments"), "paths": paths,
                    "want": {p: a.get(p) for p in paths[:5]}, "have": {p: b.get(p) for p in paths[:5]}})
    return out


# -- known differences -----------------------------------------------------------------------


def known_entries(path: Path = KNOWN) -> list[dict]:
    """The known differences; an entry without a ``cause`` or an allowed ``rule`` is refused."""
    if not path.exists():
        return []
    text = path.read_text(encoding="utf-8")
    try:
        import tomllib
    except ImportError:                       # Python 3.10: only an empty file is read
        if any(line.strip() and not line.lstrip().startswith("#") for line in text.splitlines()):
            raise RuntimeError("host/known-differences.toml has entries: reading them needs Python 3.11+")
        return []
    entries = tomllib.loads(text).get("difference", [])
    for entry in entries:
        if not entry.get("cause"):
            raise ValueError(f"a known difference without a cause: {entry}")
        if entry.get("rule") not in ALLOWED_RULES and entry.get("decision") not in ALLOWED_DECISIONS:
            raise ValueError(f"a known difference whose rule is not one of {sorted(ALLOWED_RULES)} and whose "
                             f"decision is not one of {sorted(ALLOWED_DECISIONS)}: {entry}")
        if not entry.get("fields"):
            raise ValueError(f"a known difference that names no fields: {entry}")
    return entries


def _subset(have: Any, want: Any) -> bool:
    if isinstance(want, dict):
        return isinstance(have, dict) and all(k in have and _subset(have[k], v) for k, v in want.items())
    return have == want


def unlisted(stream: str, case: str, found: list[dict], entries: list[dict]) -> tuple[list[dict], set[int]]:
    """``found`` (:func:`differences`) less what known differences admit, and the indices of
    the entries used."""
    left, used = [], set()
    for difference in found:
        paths = list(difference["paths"])
        for index, entry in enumerate(entries):
            if not fnmatch.fnmatchcase(stream, entry.get("stream", "*")) or \
                    not fnmatchcase_case(case, entry.get("case", "*")):
                continue
            if "tool" in entry and entry["tool"] != difference.get("tool"):
                continue
            if "at" in entry and entry["at"] != difference["at"]:
                continue
            if "arguments" in entry and not _subset(difference.get("arguments"), entry["arguments"]):
                continue
            admitted = [p for p in paths if any(p == f or p.startswith(f + ".") or p.startswith(f + "[")
                                                for f in entry["fields"])]
            if admitted:
                used.add(index)
                paths = [p for p in paths if p not in admitted]
        if paths:
            left.append({**difference, "paths": paths})
    return left, used


def fnmatchcase_case(case: str, pattern: str) -> bool:
    return fnmatch.fnmatchcase(case, pattern)


# -- recording -------------------------------------------------------------------------------


def transcript_inputs(transcript: dict) -> Path:
    if "fixture" in transcript:
        return (ORACLE / "fixtures" / transcript["fixture"]).parent
    return DOCX_AGENT / "tests" / "goldens" / "trial" / "_inputs" / transcript["task"] / "input"


def record_transcripts(out: Path, names: Iterable[str] = ()) -> int:
    """The docx transcripts' calls, recorded into ``out`` (``docx/transcripts/<task>.jsonl``)."""
    sys.path.insert(0, str(DOCX_AGENT / "tests"))
    import goldens_replay  # docx-agent's tests: the generated inputs

    from docx_agent.tools import FORMAT, GROUPS, TOOLS
    from ooxml_edit.tools import Toolbox

    current = {"test": ""}
    recorder = install(out / "inputs", lambda: current["test"])
    try:
        for path in sorted(TRANSCRIPTS.glob("*.json")):
            if names and path.stem not in names:
                continue
            transcript = json.loads(path.read_text(encoding="utf-8"))
            inputs = transcript_inputs(transcript)
            current["test"] = f"transcripts/{path.stem}"
            clock = dt.datetime.fromisoformat(transcript["clock"])
            with Toolbox(TOOLS, formats=[FORMAT], groups=GROUPS, workers=0) as box:
                session = box.session(clock=lambda: clock)
                for name in transcript.get("open", []):
                    session.open((inputs / name).read_bytes(), name)
                for name in transcript.get("blobs", []):
                    session.add_blob(goldens_replay.blob_bytes(inputs, name), name)
                for call in transcript["calls"]:
                    box.dispatch(session, call["tool"], call["arguments"])
                session.take_outputs()
    finally:
        recorder.uninstall()
    found, skipped = recorder.finished(SOURCE.format(sha=host_sha()),
                                       lambda case: "docx/" + case.test)
    for name, lines in found.items():
        write_stream(out / f"{name}.jsonl", lines)
    write_skipped(out, "docx/transcripts", skipped)
    print(f"recorded {sum(len(cases_of(v)) for v in found.values())} transcript case(s) into {out}")
    return 0


def record_features(out: Path, names: Iterable[str] = ()) -> int:
    """The generated host-feature streams (host_features.py), recorded into ``out``:
    ``docx/features/<feature>.jsonl``, or ``mixed/features/<feature>.jsonl`` for a scenario
    with a workbook the Python host holds."""
    import host_features

    current = {"test": ""}
    kits: dict[str, str] = {}
    recorder = install(out / "inputs", lambda: current["test"])
    try:
        for item in host_features.SCENARIOS:
            if names and item.feature not in names and item.name not in names:
                continue
            current["test"] = f"features/{item.feature}::{item.name}"
            kits[current["test"]] = item.kit
            host_features.run_scenario(item)
            recorder.settle()
    finally:
        recorder.uninstall()

    def stream(case: _Case) -> str:
        kit = kits.get(case.test, "docx")
        return ("docx/" if kit == "docx" else "mixed/") + case.test.split("::")[0]

    found, skipped = recorder.finished(SOURCE.format(sha=host_sha()), stream)
    for name, lines in found.items():
        write_stream(out / f"{name}.jsonl", lines)
    write_skipped(out, "features", skipped)
    print(f"recorded {sum(len(cases_of(v)) for v in found.values())} feature case(s) into {out}"
          + (f"; {len(skipped)} not recorded" if skipped else ""))
    for item in skipped:
        print(f"  not recorded: {item['case']}: {item['why']}")
    return 0


def write_skipped(out: Path, name: str, skipped: list[dict]) -> None:
    if skipped:
        path = out / "skipped" / f"{name.replace('/', '_')}.{os.getpid()}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(skipped, indent=1), encoding="utf-8")


def record_tests(out: Path, files: list[str], workers: str = "auto") -> int:
    """docx-agent's Toolbox tests run with the hook (``host_recorder``, a pytest plugin): each
    test file's sessions into ``out`` (``docx/tests/<file>.jsonl``)."""
    tests = DOCX_AGENT / "tests"
    if not files:
        files = sorted(p.name for p in tests.glob("test_*.py") if "Toolbox" in p.read_text(encoding="utf-8"))
    env = {**os.environ, "HOST_GOLDENS_OUT": str(out.resolve()),
           "PYTHONPATH": os.pathsep.join(filter(None, [str(Path(__file__).resolve().parent),
                                                        str(Path(__file__).resolve().parent / "tests"),
                                                        os.environ.get("PYTHONPATH")]))}
    command = [sys.executable, "-m", "pytest", "-q", "-p", "host_recorder", "-p", "no:cacheprovider",
               "-n", workers, "--dist", "loadfile", *[f"tests/{name}" for name in files]]
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, cwd=DOCX_AGENT, env=env, check=False).returncode


# -- bless -----------------------------------------------------------------------------------


def _merge_staged(staged: Path) -> dict[str, list[dict]]:
    """The staged streams, a test file's parts (one per pytest worker) merged in case order."""
    merged: dict[str, list[dict]] = {}
    for path in sorted(staged.rglob("*.jsonl")):
        rel = path.relative_to(staged)
        if rel.parts[0] in ("inputs", "skipped"):
            continue
        name = rel.with_suffix("").as_posix()
        name = name.split("@", 1)[0]                     # docx/tests/test_x@gw3 -> docx/tests/test_x
        merged.setdefault(name, []).extend(read_stream(path))
    for name, lines in merged.items():
        order = list(dict.fromkeys(line["case"] for line in lines))
        by_case = cases_of(lines)
        merged[name] = [line for case in order for line in sorted(by_case[case], key=lambda l: l["at"])]
    return merged


def _summarise(name: str, old: list[dict], new: list[dict]) -> list[str]:
    before, after = cases_of(old), cases_of(new)
    out = []
    added = sorted(set(after) - set(before))
    removed = sorted(set(before) - set(after))
    changed = sorted(case for case in set(before) & set(after) if before[case][1:] != after[case][1:])
    if added:
        out.append(f"  {name}: {len(added)} case(s) new")
    if removed:
        out.append(f"  {name}: {len(removed)} case(s) gone: " + ", ".join(removed[:5]))
    for case in changed:
        found = differences(before[case], after[case])
        tools = Counter(d.get("tool") or d["event"] for d in found)
        fields = Counter(p.split("[")[0].split(".")[0] + ("." + p.split(".")[1] if p.count(".") else "")
                         for d in found for p in d["paths"])
        out.append(f"  {name} {case}: {len(found)} event(s) changed ({', '.join(f'{t} {n}' for t, n in tools.most_common(5))};"
                   f" fields {', '.join(f'{f} {n}' for f, n in fields.most_common(5))})")
    return out


def _replays_the_same(work: tuple[str, list[dict], Path]) -> tuple[str, list[dict], list[dict]]:
    """A stream's cases that replay to their recording twice, and those that do not (why)."""
    name, lines, staged = work
    kept, dropped = [], []
    for case, events in cases_of(lines).items():
        try:
            first = replay_case(events, staged)
            second = replay_case(events, staged)
            entries = known_entries()
            found = (unlisted(name, case, differences(expected_of(events), first), entries)[0]
                     or unlisted(name, case, differences(first, second), entries)[0])
            why = "replays differently: " + ", ".join(found[0]["paths"][:3]) if found else None
        except Exception as exc:  # noqa: BLE001 -- a case the replay cannot run is not kept
            why = f"replay failed: {type(exc).__name__}: {exc}"
        if why:
            dropped.append({"stream": name, "case": case, "why": why, "calls": None})
        else:
            kept.extend(events)
    return name, kept, dropped


def bless(staged: Path, host: Path = HOST, *, write: bool = True, reason: str | None = None,
          only: Iterable[str] = (), jobs: int = 1, cases: Iterable[str] = ()) -> int:
    """Turn a recording (``staged``) into the host goldens: each case replayed twice and kept only
    where both replays equal the recording; the change summary printed (cases new, gone, and
    each changed case's events by tool and field); with ``write``, the streams and their inputs
    written into ``host`` (streams ``only`` names, or all recorded) and streams no recording
    covers left as they are.  With ``cases`` (case ids, ``test_x::test_y#1``), only those cases
    are blessed (:func:`bless_cases`).  A recording of a host with uncommitted changes needs
    ``reason``."""
    merged = _merge_staged(staged)
    if only:
        merged = {name: lines for name, lines in merged.items() if any(fnmatch.fnmatchcase(name, o) for o in only)}
    wanted = set(cases)
    if wanted:
        merged = {name: [line for line in lines if line["case"] in wanted] for name, lines in merged.items()}
        merged = {name: lines for name, lines in merged.items() if lines}
    sources = {line["source"] for lines in merged.values() for line in lines if line["event"] == "case"}
    if any(source.endswith("+dirty") for source in sources) and not reason:
        print(f"refused: recorded from a host with uncommitted changes ({', '.join(sorted(sources))}); "
              "commit them, or give --reason")
        return 1
    if wanted:
        return bless_cases(staged, merged, wanted, host, write=write, jobs=jobs)
    kept: dict[str, list[dict]] = {}
    dropped: list[dict] = []
    for path in sorted((staged / "skipped").glob("*.json")) if (staged / "skipped").is_dir() else ():
        dropped.extend(json.loads(path.read_text(encoding="utf-8")))
    from concurrent.futures import ProcessPoolExecutor

    with ProcessPoolExecutor(max_workers=max(1, jobs)) as pool:
        checked = pool.map(_replays_the_same, [(name, lines, staged) for name, lines in sorted(merged.items())])
        for name, good, bad in checked:
            if good:
                kept[name] = good
            dropped.extend(bad)
    lines_out = []
    for name in sorted(set(kept) | set(merged)):
        old = read_stream(host / f"{name}.jsonl") if (host / f"{name}.jsonl").exists() else []
        lines_out.extend(_summarise(name, old, kept.get(name, [])))
    cases = sum(len(cases_of(v)) for v in kept.values())
    calls = sum(len(line.get("calls") or ()) if line["event"] == "calls" else 1
                for v in kept.values() for line in v if line["event"] in ("call", "calls"))
    print(f"host goldens: {len(kept)} stream(s), {cases} case(s), {calls} call(s) kept; "
          f"{len(dropped)} case(s) not kept")
    for why, count in Counter(d["why"].split(":")[0] if d["why"].startswith("replay") else d["why"]
                              for d in dropped).most_common():
        print(f"  not kept ({count}): {why}")
    print("changes:" if lines_out else "changes: none")
    for line in lines_out:
        print(line)
    if write:
        for name, lines in kept.items():
            write_stream(host / f"{name}.jsonl", lines)
            for line in lines:
                for spec in _inputs_of(line):
                    if spec["path"].startswith("$HOST/"):
                        source = staged / spec["path"][len("$HOST/"):]
                        target = host / spec["path"][len("$HOST/"):]
                        if source.exists() and not target.exists():
                            target.parent.mkdir(parents=True, exist_ok=True)
                            shutil.copyfile(source, target)
        for name in set(merged) - set(kept):
            stale = host / f"{name}.jsonl"
            if stale.exists():
                stale.unlink()
        write_skipped_list(host, set(merged) | {d["stream"] for d in dropped}, dropped)
        _drop_unused_inputs(host)
    return 0


def spliced(stream: str, lines: list[dict]) -> str:
    """A stream's text (``stream``) with the cases of ``lines`` written in place of their old
    lines -- a case it lacks at its end -- and every other line kept as it is, byte for byte."""
    new = cases_of(lines)
    out: list[str] = []
    placed: set[str] = set()
    for text in stream.splitlines(keepends=True):
        case = json.loads(text)["case"] if text.strip() else None
        if case not in new:
            out.append(text)
        elif case not in placed:
            out.extend(canonical(line) + "\n" for line in new[case])
            placed.add(case)
    for case, case_lines in new.items():
        if case not in placed:
            out.extend(canonical(line) + "\n" for line in case_lines)
    return "".join(out)


def bless_cases(staged: Path, merged: dict[str, list[dict]], wanted: set[str], host: Path = HOST, *,
                write: bool = True, jobs: int = 1) -> int:
    """:func:`bless` of the cases ``wanted`` alone (``merged``: their recorded lines, by stream):
    each replayed twice as every blessed case is; where all replay the same, each is written in
    its stream in place of its old lines (a case the stream lacks goes at its end) with its own
    inputs, and every other line of the stream is left as it was, byte for byte.  A case named
    and not recorded, or not replaying the same, refuses the whole bless (nothing written)."""
    recorded = {line["case"] for lines in merged.values() for line in lines}
    missing = sorted(wanted - recorded)
    if missing:
        print("refused: not recorded: " + ", ".join(missing))
        return 1
    from concurrent.futures import ProcessPoolExecutor

    kept: dict[str, list[dict]] = {}
    dropped: list[dict] = []
    with ProcessPoolExecutor(max_workers=max(1, jobs)) as pool:
        for name, good, bad in pool.map(_replays_the_same, [(n, ls, staged) for n, ls in sorted(merged.items())]):
            kept[name] = good
            dropped.extend(bad)
    if dropped:
        print("refused: replays differently: " + ", ".join(f"{d['case']} ({d['why']})" for d in dropped))
        return 1
    lines_out = []
    for name, lines in sorted(kept.items()):
        path = host / f"{name}.jsonl"
        old = [line for line in read_stream(path) if line["case"] in wanted] if path.exists() else []
        lines_out.extend(_summarise(name, old, lines))
    print(f"host goldens: {len(kept)} stream(s), {len(recorded)} case(s) blessed alone")
    print("changes:" if lines_out else "changes: none")
    for line in lines_out:
        print(line)
    if not write:
        return 0
    for name, lines in kept.items():
        path = host / f"{name}.jsonl"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(spliced(path.read_text(encoding="utf-8") if path.exists() else "", lines), encoding="utf-8")
        for line in lines:
            for spec in _inputs_of(line):
                if spec["path"].startswith("$HOST/"):
                    source = staged / spec["path"][len("$HOST/"):]
                    target = host / spec["path"][len("$HOST/"):]
                    if source.exists() and not target.exists():
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(source, target)
    skipped = host / "skipped.json"
    if skipped.exists():
        earlier = json.loads(skipped.read_text(encoding="utf-8"))
        left = [d for d in earlier if not (d["case"] in wanted and d["stream"] in kept)]
        if left != earlier:
            skipped.write_text(json.dumps(left, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    _drop_unused_inputs(host)
    return 0


def write_skipped_list(host: Path, recorded: set[str], dropped: list[dict]) -> None:
    """``host/skipped.json``: the cases not kept, with why -- this bless's for the streams it
    recorded, the earlier list's for every other stream."""
    path = host / "skipped.json"
    earlier = json.loads(path.read_text(encoding="utf-8")) if path.exists() else []
    kept = [d for d in earlier if d["stream"] not in recorded]
    path.write_text(json.dumps(sorted(kept + dropped, key=lambda d: (d["stream"], d["case"])), indent=1,
                               ensure_ascii=False) + "\n", encoding="utf-8")


def _inputs_of(line: dict) -> list[dict]:
    return [line["input"]] if isinstance(line.get("input"), dict) else []


def _drop_unused_inputs(host: Path) -> None:
    used = {Path(spec["path"]).name for path in streams(host).values() for line in read_stream(path)
            for spec in _inputs_of(line)}
    folder = host / "inputs"
    if folder.is_dir():
        for path in folder.iterdir():
            if path.name not in used:
                path.unlink()


# -- the command -----------------------------------------------------------------------------


def run_replay(names: list[str]) -> int:
    entries = known_entries()
    bad = 0
    total = Counter()
    for name, path in streams().items():
        if names and not any(fnmatch.fnmatchcase(name, n) for n in names):
            continue
        for case, events in cases_of(read_stream(path)).items():
            total["cases"] += 1
            left, _used = unlisted(name, case, differences(expected_of(events), replay_case(events)), entries)
            if left:
                bad += 1
                print(f"DIFFERS {name} {case}:")
                for item in left[:3]:
                    print("   ", json.dumps(item, ensure_ascii=False)[:600])
    print(f"replayed {total['cases']} case(s): {bad} with unlisted differences")
    return 1 if bad else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    t = sub.add_parser("record-transcripts", help="record the docx transcripts into OUT")
    t.add_argument("out", type=Path)
    t.add_argument("names", nargs="*")
    f = sub.add_parser("record-features", help="record the generated host-feature streams into OUT")
    f.add_argument("out", type=Path)
    f.add_argument("names", nargs="*", help="features or scenarios (default: all)")
    r = sub.add_parser("record-tests", help="record docx-agent's Toolbox tests into OUT (pytest with the hook)")
    r.add_argument("out", type=Path)
    r.add_argument("files", nargs="*")
    r.add_argument("-n", default="auto", help="pytest-xdist workers")
    b = sub.add_parser("bless", help="keep what replays of a recording in OUT, and print the changes")
    b.add_argument("staged", type=Path)
    b.add_argument("--check", action="store_true", help="print the changes, write nothing")
    b.add_argument("--reason", help="why a recording of uncommitted host code is blessed")
    b.add_argument("--only", nargs="*", default=[], help="stream globs to bless (default: all recorded)")
    b.add_argument("-j", type=int, default=1, help="streams replayed at once (processes)")
    b.add_argument("--case", action="append", default=[], dest="cases",
                   help="bless only this case (test_x::test_y#1; repeatable): every other case's lines kept as they are")
    b.add_argument("--cases-from", type=Path, help="a file of case ids, one a line, blessed as --case")
    p = sub.add_parser("replay", help="replay the host goldens against this host")
    p.add_argument("names", nargs="*", help="stream globs (default: all)")
    args = parser.parse_args(argv)
    if args.command == "record-transcripts":
        return record_transcripts(args.out, args.names)
    if args.command == "record-features":
        return record_features(args.out, args.names)
    if args.command == "record-tests":
        return record_tests(args.out, args.files, args.n)
    if args.command == "bless":
        cases = list(args.cases)
        if args.cases_from:
            cases += [line.strip() for line in args.cases_from.read_text(encoding="utf-8").splitlines() if line.strip()]
        return bless(args.staged, write=not args.check, reason=args.reason, only=args.only, jobs=args.j,
                     cases=cases)
    return run_replay(args.names)


if __name__ == "__main__":
    sys.exit(main())
