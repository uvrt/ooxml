"""docx-agent's reading tools answered by the Rust session that owns the document: every
read (``describe``, ``word_read``, ``word_inspect``, ``find_text``, ``word_compare_blocks``,
``word_design_facts``, ``check``, ``render``, ``save_document``, ``edit_chart`` read and,
through them, the ops of ``read_batch``) runs there with the interpreter lock let go.

**The versions.**  The session owns every Word document a toolbox session holds
(:mod:`.owner`): its versions are kept by the tool layer's numbers as the session makes them
(:meth:`Mirror.keep`), and a read reads the version its call was given -- a committed
version's reader's (``entry.live``: the owner's entry), or within a batch what the batch's
ops made so far.

**No fallback.**  The session's answer is the call's answer, and its refusal too, passed
through whole (:mod:`.refusals`): a form it does not support yet says so
(``details.not_supported``) with its way out.  Nothing is answered by another path.

Must not: read a Python document, or word a refusal.
"""

from __future__ import annotations

import json
import threading
from collections import Counter, OrderedDict
from contextlib import contextmanager
from typing import Any, Callable, Iterator

from ooxml_edit.tools import Result, ToolError

from .refusals import refusal

#: The versions of a document kept for the calls reading them.
KEPT = 4

_SESSION: list[Any] = []
_SESSION_LOCK = threading.Lock()
_COUNTS_LOCK = threading.Lock()
_SERVED: Counter = Counter()


def session() -> Any:
    """The process's Rust session (an ``ooxml_core.Session``), made when first needed."""
    with _SESSION_LOCK:
        if not _SESSION or _SESSION[0] is None:
            import ooxml_core

            _SESSION[:] = [ooxml_core.Session()]
        return _SESSION[0]


def use_session(given: Any) -> Any:
    """Hold the documents opened from now on in ``given`` (an ``ooxml_core.Session``: an
    application's own; ``None``: the process's, made again when next needed), and give back
    the one used before (``None`` where none was made yet)."""
    with _SESSION_LOCK:
        before = _SESSION[0] if _SESSION else None
        _SESSION[:] = [given]
        return before


def served() -> dict[str, int]:
    """The reading calls answered by the session, by tool, since the process started."""
    with _COUNTS_LOCK:
        return dict(_SERVED)


def _count(tool: str) -> None:
    with _COUNTS_LOCK:
        _SERVED[tool] += 1


def close(held: Any, doc: str) -> None:
    """Close ``doc`` in the session ``held``; nothing where it is closed already."""
    try:
        held.close(doc)
    except Exception:  # noqa: BLE001 -- closed already
        pass


class Mirror:
    """One document the session owns: the session and its id there, the versions kept by
    the tool layer's number of each, and the application's settings of its format."""

    def __init__(self, held: Any, doc: str, settings: dict[str, Any] | None = None) -> None:
        self.session = held
        self.doc = doc
        self.versions: OrderedDict[int, Any] = OrderedDict()
        #: The session's version -> the tool layer's number of it, for every version kept
        #: (:meth:`keep`).
        self.numbers: dict[int, int] = {}
        self.lock = threading.Lock()
        #: The application's settings (``docx_format``: ``min_text_size``, ``typography``).
        self.settings: dict[str, Any] = dict(settings or {})
        #: The version the document was opened as, the number it was opened at and the
        #: highest its history gave (:meth:`.owner.RustOwner.opened`).
        self.opened: Any = None
        self.numbers_opened: tuple[int, int] = (0, 0)
        #: The tracking mode the session edits the document under (:mod:`.edits`).
        self.tracking: dict[str, Any] | None = None
        #: What the document's page engine was last set to (:mod:`.engines`).
        self.pages_key: Any = None

    def keep(self, version: Any, first: int | None = None) -> None:
        """Keep ``version`` (an ``ooxml_core.Version``) as the document's version
        ``version.state``; ``first``: the session's first version of the step, where it made
        several (a batch's ops), each numbered so too."""
        number = version.state
        self.versions[number] = version
        for seq in range(first if first is not None else version.number, version.number + 1):
            self.numbers[seq] = number
        self.versions.move_to_end(number)
        while len(self.versions) > KEPT:
            self.versions.popitem(last=False)


def owned(entry: Any) -> Any:
    """The entry the session owns that ``entry`` is, or reads a committed version of."""
    for found in (entry, getattr(entry, "live", None)):
        if isinstance(getattr(found, "handle", None), Mirror):
            return found
    raise ToolError("internal", f"{getattr(entry, 'doc_id', 'the document')} is not held by the Rust session")


def setting(entry: Any, name: str) -> Any:
    """An application setting of the document's format (``docx_format``), or ``None``."""
    return owned(entry).handle.settings.get(name)


def refs(call: Any, value: Any) -> Any:
    """``value`` with its ``$name`` refs resolved (a span's ends too), as the tools resolve
    them; a heading's range is the session's to expand."""
    if isinstance(value, str) and ".." in value and "$" in value:
        first, _, last = value.partition("..")
        return "..".join(call.resolve_ref(end.strip()) if end.strip().startswith("$") else end
                         for end in (first, last))
    if isinstance(value, str) and value.startswith("$"):
        return call.resolve_ref(value)
    return value


def version_read(entry: Any) -> tuple[Mirror, Any]:
    """The session's copy of the document ``entry`` is, and the version of it a call reads:
    within a batch what its ops made so far; else the entry's version."""
    found = owned(entry)
    mirror = found.handle
    if found is entry and getattr(entry.history, "grouped", False):
        return mirror, entry.history.reading_head()
    with mirror.lock:
        version = mirror.versions.get(entry.version)
    if version is None:
        version = mirror.session.numbered(mirror.doc, entry.version)
    if version is None:
        raise ToolError("internal", f"version {entry.version} of {entry.doc_id} is no longer held")
    return mirror, version


def _with_others(call: Any, arguments: dict[str, Any], others: tuple[str, ...],
                 held: Any) -> tuple[dict[str, str], list[Any]]:
    """The host's names of the other documents the call names (at a path of ``others``,
    ``reference.doc``), each the session's id of it, and the versions of them the call
    reads.  The call's own document named again is the tool's."""
    versions: list[Any] = []
    names: dict[str, str] = {}
    for given in (found for path in others for found in at_path(arguments, path.split("."))):
        if not isinstance(given, str) or given == getattr(call.entry, "doc_id", None) or given in names:
            continue
        entry = call.entries.get(given)
        if entry is None:
            raise ToolError("not_found", f"no open document {given}", valid_options=sorted(call.entries))
        mirror, version = version_read(entry)
        if mirror.session is not held:
            raise ToolError("refused", f"{given} is held by another Rust session: open both in one",
                            details={"not_supported": "a document in another session"})
        names[given] = mirror.doc
        versions.append(version)
    return names, versions


def at_path(value: Any, path: list[str]) -> list[Any]:
    """The values at ``path`` in ``value`` (``reference.doc``; a part ending ``[]`` is each
    item of a list: ``copies[].reference.doc``)."""
    if not path:
        return [value]
    head, rest = path[0], path[1:]
    if head.endswith("[]"):
        items = value.get(head[:-2]) if isinstance(value, dict) else None
        return [found for item in items or () for found in at_path(item, rest)] \
            if isinstance(items, list) else []
    return at_path(value.get(head), rest) if isinstance(value, dict) else []


def serve(call: Any, tool: str, arguments: dict[str, Any], *,
          answer: Callable[[dict[str, Any]], Result] | None = None,
          others: tuple[str, ...] = (), layout: Any = None, raster: Any = None,
          images: "list[tuple[str, int, int]] | None" = None,
          context: "Callable[[], dict[str, Any]] | None" = None,
          files: "list[tuple[str, str, bytes]] | None" = None) -> Result:
    """``tool``'s answer from the Rust session on the version ``call`` reads (``answer``
    makes the :class:`Result` of its envelope); its refusal raised as the call's.
    ``others``: the paths of ``arguments`` naming another open document; ``layout``: the
    document's page engine for a call that reads the pages, laying out within the host's
    ``layout_timeout``; ``raster``: the faces ``render`` draws with and ``images`` the
    image budgets as they stand; ``context``: what the call context adds; the files the call
    gives the application appended to ``files`` as ``(name, format, bytes)``."""
    import ooxml_core

    from .owner import context_of

    mirror, version = version_read(call.entry)
    names, pinned = _with_others(call, arguments, others, mirror.session)
    # the documents named as the host names them: the reply says those names
    names[call.entry.doc_id] = mirror.doc
    given = {**{k: v for k, v in arguments.items() if v is not None}, "doc": call.entry.doc_id}
    made = context_of(None, call.limits, call.limits.layout_timeout if layout is not None else None,
                      **(context() if context is not None else {}))
    laid = None
    if layout is not None or raster is not None:
        laid = ooxml_core.CallLayout(layout, raster=raster, image_budgets=images)
    options = ooxml_core.CallOptions(
        context=made, documents=ooxml_core.CallDocuments(at=[version, *pinned], names=names), layout=laid)
    # A read that lays out or draws runs with the document's lock let go (outside a batch,
    # where ooxml-edit lets it go): it reads the version taken above, and an edit meanwhile
    # does not wait for it.
    with _unlocked(call, laid is not None):
        reply, given_files, app = mirror.session.call(tool, json.dumps(given), options)
    _count(tool)
    envelope = json.loads(reply)
    error = envelope.get("error")
    if error is not None:
        raise refusal(error)
    if files is not None:
        app = json.loads(app)
        files.extend((found["name"], found["format"], data)
                     for found, data in zip(app.get("files") or [], given_files))
    return (answer or _result)(envelope)


@contextmanager
def _unlocked(call: Any, slow: bool) -> Iterator[None]:
    """The call's document locks let go for a slow read (``slow``: one that lays out or
    draws) where the call may let them go (:meth:`ooxml_edit.tools.Call.unlocked`), else
    held."""
    if not slow:
        yield
        return
    with call.unlocked():
        yield


def _result(envelope: dict[str, Any]) -> Result:
    result = Result(summary=envelope.get("summary", ""), data=envelope.get("data"),
                    next_cursor=envelope.get("next_cursor"))
    result.total = envelope.get("total")
    result.warnings.extend(envelope.get("warnings") or [])
    return result


def session_layout(call: Any) -> Any:
    """The page engine the session lays the document ``call`` reads out with
    (:func:`.engines.session_pages`)."""
    from .engines import session_pages

    return session_pages(owned(call.entry))


# -- check --------------------------------------------------------------------------------------


def serve_check(call: Any, arguments: dict[str, Any]) -> Result:
    """``check`` answered by the session: the application's ``min_text_size`` (one number)
    as its ``min_size`` where the check gives none; laid out by the document's page engine
    where the check reads the layout.  The session keeps what the check saw for the next
    one's ``since_last_check``."""
    entry = owned(call.entry)
    if arguments.get("range") is not None:
        arguments = {**arguments, "range": refs(call, arguments["range"])}
    minimum = entry.handle.settings.get("min_text_size")
    if minimum is not None and arguments.get("min_size") is None:
        if isinstance(minimum, bool) or not isinstance(minimum, (int, float)):
            raise ToolError("refused", "a check with the application's min_text_size by kind is not supported "
                            "yet; give min_size", field="min_size", try_call={"tool": "check", "arguments": {
                                **arguments, "doc": call.entry.doc_id, "min_size": 9}, "fill": ["min_size"]},
                            details={"not_supported": "check: min_text_size by kind"})
        arguments = {**arguments, "min_size": minimum}
    from .engines import session_pages

    layout = session_pages(entry) if laid_check(arguments) else None
    held = layout is not None and call.in_batch
    before = _laid(layout) if held else 0
    result = serve(call, "check", arguments, layout=layout, others=("copies[].reference.doc",))
    if held and _laid(layout) > before:
        # Between a batch's changes (after the last one ooxml-edit runs it once the batch is
        # done, the lock let go): it laid the document out holding it.
        call.warn(BATCH_CHECK)
    return result


#: What a check that laid the document out between a batch's changes says: the way out.
BATCH_CHECK = ("check laid the document out between the batch's changes, holding it: other calls waited. "
               "Put check after the batch's last change (or call it alone): there it lets the document go "
               "while it lays out")


def _laid(pages: Any) -> int:
    """The versions the page engine ``pages`` laid out so far, whole or again in part."""
    stats = dict(pages.stats())
    return int(stats["full"] + stats["relaid"])


def laid_check(arguments: dict[str, Any]) -> bool:
    """Whether a check with these arguments reads the layout: ``reflow`` or ``layout``
    asked (or the default: none, or an empty list), or what the layout's facts measure
    (``range``, ``min_size``, ``styles``)."""
    include = arguments.get("include")
    if not include or "reflow" in include or "layout" in include:
        return True
    return any(arguments.get(name) not in (None, "", []) for name in ("range", "min_size", "styles"))


# -- render -------------------------------------------------------------------------------------

#: The faces pages are drawn with in Rust, by the font folders they were made with
#: (``ooxml_core.PageRaster``: the system's fonts and the folders, read once).
_RASTERS: dict[tuple[str, ...], Any] = {}
_RASTERS_LOCK = threading.Lock()


def host_raster(font_dirs: "list[str]") -> Any:
    """The ``ooxml_core.PageRaster`` ``render`` draws a document's pages from -- the
    system's fonts and ``font_dirs``, read once per set of folders."""
    import ooxml_core

    dirs = tuple(font_dirs)
    with _RASTERS_LOCK:
        found = _RASTERS.get(dirs)
    if found is None:
        made = ooxml_core.PageRaster([], list(dirs))
        with _RASTERS_LOCK:
            found = _RASTERS.setdefault(dirs, made)
    return found


def image_budgets(call: Any) -> list[tuple[str, int, int]]:
    """The session's and the round's image budgets as they stand for ``call``, each
    ``(name, limit, used)`` where the host's limits set one."""
    limits, held = call.limits, call.session
    budgets = []
    if limits.image_budget is not None:
        budgets.append(("session", limits.image_budget, held.images_used))
    if limits.image_budget_per_round is not None:
        budgets.append(("round", limits.image_budget_per_round, held.images_this_round))
    return budgets


def serve_render(call: Any, arguments: dict[str, Any]) -> Result:
    """``render`` answered by the session -- laid out by the document's page engine, its
    pages drawn in Rust -- each image a file of the reply, made an image of the call here."""
    from ooxml_edit.tools.render import png_size

    from .engines import font_dirs_of, session_pages

    entry = owned(call.entry)
    files: list[tuple[str, str, bytes]] = []
    answer = serve(call, "render", arguments, layout=session_pages(entry), raster=host_raster(font_dirs_of(entry)),
                   images=image_budgets(call), files=files)
    for name, _format, data in files:
        width, height = png_size(data)
        call.image(data, width, height, label=name)
    return answer


# -- what the last check saw --------------------------------------------------------------------


def last_check(call: Any) -> tuple[Any, int | None]:
    """What the last check of the document ``call`` reads saw, kept by the session:
    ``(seen, number)`` -- ``seen`` ``(version, summary)`` or ``None`` where no check saw one
    yet; ``number`` the session's number of the version ``call`` reads."""
    mirror, version = version_read(call.entry)
    return mirror.session.last_check(mirror.doc), version.number
