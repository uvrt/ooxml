"""The datasets of a session on the Rust host (Phase 3, decision D3): the Rust ``HostSession``
keeps them, and the application's own host -- its tools (decision D1), and until Phase 4 the
Python host of the formats the Rust host does not hold yet -- writes and reads them through the
binding.

- :func:`entry_json`: a :class:`~.datasets.DatasetEntry` a call (or the application) keeps,
  as the session's store reads it: its values (:func:`dataset_json`), its id, origin and
  time, and where it came from (a document at a version, the sources it also read).
- :class:`HostDatasets`: a session's datasets as Python reads them.  Each is a
  :class:`KeptEntry`, a frozen view of the store's entry: its values are those kept (the
  application's own :class:`~.datasets.Dataset`, exactly as given, where it kept them here;
  else read from the store's JSON, :func:`dataset_of`); whether it is stale or closed is
  the store's, asked when read.  A view is of one keeping: the store gives every keeping a
  serial never given again, and a view, and the values copied, are kept by id only with
  that serial -- a dataset kept again, or dropped, is read fresh.

Must not: decide a dataset's staleness (the store does, against the documents' heads), or
keep a value the store has not kept.
"""

from __future__ import annotations

import dataclasses
import datetime as _dt
import json
import math
import threading
from typing import Any, Iterable, Iterator, Mapping

from .datasets import DATASET_PREFIX, Dataset, DatasetEntry, dataset_id
from .results import ToolError


def tool_error(text: str) -> ToolError:
    """A refusal the host wrote as JSON, as Python's :class:`~.results.ToolError`."""
    error = json.loads(text)
    return ToolError(error["code"], error["message"], field=error.get("field"),
                     valid_options=error.get("valid_options"), details=error.get("details"),
                     try_call=error.get("try_call"))


# -- values ------------------------------------------------------------------------------------


@dataclasses.dataclass(frozen=True)
class OtherValue:
    """A value the store holds only by its kind (``a Decimal``, ``not a finite number``):
    one JSON has no form for, read back from a dataset Python did not keep itself."""

    kind: str

    def __str__(self) -> str:
        return self.kind


def cell_json(value: Any) -> Any:
    """A dataset's value as the store reads it: JSON's own where it has one, a date, a
    datetime and a time as their parts, a whole number past 64 bits as its digits,
    anything else by its kind (:func:`.sources.what`)."""
    from .sources import what

    if value is None or isinstance(value, (bool, str)):
        return value
    if isinstance(value, int):
        return value if -(2 ** 63) <= value < 2 ** 64 else {"int": str(value)}
    if isinstance(value, float):
        return value if math.isfinite(value) else {"other": what(value)}
    if isinstance(value, _dt.datetime):
        return {"datetime": [value.year, value.month, value.day, value.hour, value.minute,
                             value.second, value.microsecond]}
    if isinstance(value, _dt.date):
        return {"date": [value.year, value.month, value.day]}
    if isinstance(value, _dt.time):
        return {"time": [value.hour, value.minute, value.second, value.microsecond]}
    if isinstance(value, OtherValue):
        return {"other": value.kind}
    return {"other": what(value)}


def value_of(given: Any) -> Any:
    """A value :func:`cell_json` wrote, read back."""
    if not isinstance(given, dict):
        return given
    (kind, parts), = given.items()
    if kind == "date":
        return _dt.date(*parts)
    if kind == "datetime":
        return _dt.datetime(*parts)
    if kind == "time":
        return _dt.time(*parts)
    if kind == "int":
        return int(parts)
    return OtherValue(parts)


def dataset_json(dataset: Dataset) -> dict[str, Any]:
    """``dataset`` as the store reads it: every field but its ``detail`` (the producer's
    own, which no other library reads); Markdown worked out now."""
    markdown = dataset.markdown
    return {"columns": list(dataset.columns),
            "rows": [[cell_json(v) for v in row] for row in dataset.rows],
            "sources": ([[list(cell) for cell in row] for row in dataset.sources]
                        if dataset.sources is not None else None),
            "keys": dataset.keys,
            "types": list(dataset.types) if dataset.types is not None else None,
            "cache": dict(dataset.cache) if dataset.cache else None,
            "formats": list(dataset.formats) if dataset.formats is not None else None,
            "markdown": ([[row, column, text] for (row, column), text in markdown.items()
                          if text is not None] if markdown else None)}


def dataset_of(given: Mapping[str, Any]) -> Dataset:
    """A dataset the store wrote as JSON (:func:`dataset_json`'s form)."""
    markdown = given.get("markdown")
    return Dataset(tuple(given["columns"]), tuple(tuple(value_of(v) for v in row) for row in given["rows"]),
                   sources=given.get("sources"), keys=given.get("keys") or 0, types=given.get("types"),
                   cache=given.get("cache"), formats=given.get("formats"),
                   markdown={(r, c): text for r, c, text in markdown} if markdown else None)


# -- entries -----------------------------------------------------------------------------------


def _invalidated(dataset: Dataset) -> int:
    return (dataset.cache or {}).get("invalidated", 0)


def _source_json(entry: DatasetEntry, id: str, dataset: Dataset) -> dict[str, Any]:
    """Where ``entry``'s values came from, as the store reads it, for the dataset ``id`` of
    ``dataset``'s values."""
    kept = getattr(entry, "store_source", None)
    if kept is not None:            # (a view's, or one derived from a view: the store's own)
        return {**kept, "id": id, "invalidated": _invalidated(dataset)}
    return {"id": id, "doc": entry.doc, "kind": entry.kind, "name": entry.name,
            "version": entry.version, "closed": entry.closed, "for_good": bool(entry._stale),
            "invalidated": _invalidated(dataset),
            "also": [_source_json(other, other.id, other.dataset) for other in entry.also]}


def _moment(value: Any) -> str | None:
    if value is None or isinstance(value, str):
        return value
    if isinstance(value, _dt.datetime) and value.tzinfo is not None:
        value = value.astimezone(_dt.timezone.utc)
    return value.strftime("%Y-%m-%dT%H:%M:%SZ")


def entry_json(entry: DatasetEntry) -> dict[str, Any]:
    """A dataset kept, as the store reads it (``DatasetEntry::from_json``)."""
    return {"id": entry.id, "origin": entry.origin, "defined_at": _moment(entry.defined_at),
            "dataset": dataset_json(entry.dataset),
            "source": _source_json(entry, entry.id, entry.dataset)}


class KeptEntry(DatasetEntry):
    """A dataset the store keeps, as Python reads it: frozen at one keeping (:attr:`serial`);
    whether it is stale or closed asked of the store when read (as it was last told, once
    the store keeps another dataset under its id)."""

    def __init__(self, store: "HostDatasets", serial: int, meta: Mapping[str, Any], dataset: Dataset,
                 stale: bool, closed: bool) -> None:
        source = meta["source"]
        super().__init__(meta["id"], dataset, doc=source.get("doc"), kind=source.get("kind"),
                         name=source.get("name"), version=source.get("version"),
                         defined_at=meta.get("defined_at"), origin=meta.get("origin"))
        self.serial = serial
        #: Where its values came from, as the store said: what one derived from it inherits.
        self.store_source: dict[str, Any] = dict(source)
        self._store = store
        self._now = (stale, closed)

    def _state(self) -> tuple[bool, bool]:
        found = self._store.state(self.id, self.serial)
        if found is not None:
            self._now = found
        return self._now

    @property
    def stale(self) -> bool:                    # type: ignore[override]
        return self._state()[0]

    @property
    def closed(self) -> bool:                   # type: ignore[override]
        return self._state()[1]

    @property
    def source_version(self) -> int | None:
        return None if self.closed else self.version

    def follow_live(self) -> None:
        """The store follows its source itself."""

    def document_closed(self, doc: str) -> None:
        """The store keeps its fate when its source closes."""

    def source_closed(self) -> None:
        """The store keeps its fate when its source closes."""

    def mark_stale(self) -> None:
        raise TypeError(f"{self.address} is the session's, kept on the Rust host: derive it again")

    def derived(self, id: str, dataset: Dataset, *, defined_at: Any = None) -> DatasetEntry:
        """A dataset derived from this one: its source, version and fate the store's."""
        made = DatasetEntry(id, dataset, doc=self.doc, kind=self.kind, name=self.name, version=self.version,
                            defined_at=defined_at, origin=self.address)
        made.store_source = self.store_source        # type: ignore[attr-defined]
        made._stale = self.stale
        return made


class HostDatasets:
    """A session's datasets on the Rust host (its ``HostSession``, :attr:`rust`), as Python
    reads and keeps them."""

    def __init__(self, rust: Any) -> None:
        self.rust = rust
        self._lock = threading.Lock()
        #: By id: the serial of the keeping and its values, as Python has them.
        self._values: dict[str, tuple[int, Dataset]] = {}
        #: By id: the view of the keeping read last.
        self._views: dict[str, KeptEntry] = {}

    def keep(self, entries: Iterable[DatasetEntry]) -> None:
        """Keep ``entries`` (all, or none and ``limit``); an id kept is replaced."""
        entries = list(entries)
        if not entries:
            return
        serials, refused = self.rust.keep_datasets(json.dumps([entry_json(e) for e in entries]))
        if refused is not None:
            raise tool_error(refused)
        with self._lock:
            for entry, serial in zip(entries, serials):
                self._values[entry.id] = (serial, entry.dataset)
                self._views.pop(entry.id, None)

    def get(self, address: str, *, field: str | None = None) -> KeptEntry:
        """The dataset ``address`` names (``d:by_region``, with what follows its id, or the
        bare id); ``not_found`` lists the ids there are."""
        found = dataset_id(address) if isinstance(address, str) and address.startswith(DATASET_PREFIX) \
            else address
        view = self.view(found) if isinstance(found, str) else None
        if view is None:
            raise ToolError("not_found", f"no dataset {DATASET_PREFIX}{found}", field=field,
                            valid_options=self.ids())
        return view

    def view(self, id: str) -> KeptEntry | None:
        """The dataset ``id`` (no ``d:``) as the store keeps it now; ``None`` for none."""
        with self._lock:
            have = self._values.get(id)
        found = self.rust.dataset(id, have[0] if have is not None else None)
        with self._lock:
            if found is None:
                self._values.pop(id, None)
                self._views.pop(id, None)
                return None
            serial, meta, values, stale, closed = found
            view = self._views.get(id)
            if view is not None and view.serial == serial:
                view._now = (stale, closed)
                return view
            if values is not None:
                self._values[id] = (serial, dataset_of(json.loads(values)))
            view = KeptEntry(self, serial, json.loads(meta), self._values[id][1], stale, closed)
            self._views[id] = view
            return view

    def state(self, id: str, serial: int) -> tuple[bool, bool] | None:
        """Whether the dataset ``id``'s keeping ``serial`` is stale and closed now; ``None``
        once another keeping has its id."""
        found = self.rust.dataset(id, serial)
        if found is None or found[0] != serial:
            return None
        return found[3], found[4]

    def ids(self) -> list[str]:
        """Every dataset's address, in the order defined."""
        return list(self.rust.dataset_ids())

    def entries(self) -> list[KeptEntry]:
        """Every dataset, in the order defined."""
        return [view for view in (self.view(dataset_id(a) or a) for a in self.ids()) if view is not None]

    def drop(self, address: str) -> None:
        """Forget a dataset; ``not_found`` for none."""
        refused = self.rust.drop_dataset(address)
        if refused is not None:
            raise tool_error(refused)
        found = (dataset_id(address) if address.startswith(DATASET_PREFIX) else None) or address
        with self._lock:
            self._values.pop(found, None)
            self._views.pop(found, None)

    def mapping(self) -> Mapping[str, KeptEntry]:
        """The datasets by id, each read when asked for (:class:`~.session.Session`'s
        ``datasets``)."""
        return _ById(self)


class _ById(Mapping):
    """:meth:`HostDatasets.mapping`."""

    def __init__(self, store: HostDatasets) -> None:
        self._store = store

    def __getitem__(self, id: str) -> KeptEntry:
        found = self._store.view(id)
        if found is None:
            raise KeyError(id)
        return found

    def __iter__(self) -> Iterator[str]:
        return iter([dataset_id(a) or a for a in self._store.ids()])

    def __len__(self) -> int:
        return len(self._store.ids())


__all__ = ["HostDatasets", "KeptEntry", "OtherValue", "dataset_json", "dataset_of", "entry_json"]
