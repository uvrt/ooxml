"""Stable addressing: what every paragraph, table, row, cell and section is called.

pptx-agent's rule carries over: **ids name things, positions name places, and the id says
which it is.**  ROADMAP.md, "Addressing and stable ids", is the specification; in short:

* **A paragraph** is ``p:<paraId>`` -- Word's own ``w14:paraId``, verbatim, so it is the id
  docx2svg puts in ``data-docx-id`` -- prefixed with its story outside the body
  (``header2/p:1A2B3C4D``, ``footnotes/p:...``).
* **Missing:** a paragraph with no paraId is ``p@<story>/<n>`` (its 0-based place among
  the story's paragraphs in document order), honest about being volatile, until the first
  edit **stamps** it with a paraId and a textId.  Reading never stamps.
* **Duplicated** (a paraId is unique in its part by specification, but tools and copying
  repeat them): the first occurrence keeps ``p:<paraId>``, later ones are
  ``p:<paraId>#<k>`` (the k-th repeat).  The first edit re-issues a repeat's paraId, and
  one outside the valid range (0 < id < 0x80000000).
* **Every rename is kept as an alias**, so an id an agent holds still resolves after the
  paragraph is stamped or re-issued -- and, because an alias is consulted only while its
  target exists, after the edit is undone too.

**What the first edit stamps** is the whole document by default (``Document(stamping=
"document")``): every paragraph and table row, in every story, that lacks a valid and
unique paraId, and every drawing that lacks a ``wp14:anchorId`` and ``wp14:editId``.  Measured in Word (ROADMAP.md, "Durable across a Word save -- measured";
``tools/paraid_probe.py``): Word keeps paraIds it did not write -- through saves, edits
elsewhere and edits of the paragraph itself (whose textId it renews) -- only when every
paragraph and row of every story has a valid, unique one and every drawing its two ids;
one paragraph, row or drawing without them, in any story, or one paraId out of range, and
it writes none at all (or, in a document with
comments, new ones throughout).  A repeat it replaces, keeping the first.  ``stamping=
"paragraph"`` stamps only what an edit touches, for callers who want fewer bytes changed and
do not need ids to outlive a Word save.

Generated paraIds are deterministic: values of a sequence seeded by the part's name that no
paraId or textId anywhere in the document uses, so the same edit on the same document writes
the same bytes, and undoing and redoing it writes them again.

**An id is never given to another block** (round 37).  An agent holds ids for many turns, and
another may delete the block meanwhile; were the id free again, the next new block would get
it and the first agent would edit a block it never read.  So:

* a document keeps, beside the ids in use, every id it has had this session -- in any part,
  in any state its history went through (:class:`Ledger`) -- and generates none of them
  again.  Undoing a deletion brings the block back with its id: that is the same block.
* a call naming a block id that is gone fails as *deleted*, saying which change deleted it
  (:class:`DeletedId`), never reaching another block;
* across a save and a reopen nothing is kept but the file, so the sequence goes on after the
  last of its values the file still uses (:func:`generate`, :data:`REACH`): a value of it
  before that one that the file lacks was used and deleted, and is not taken again.

**Opened again with its state** (round 38).  The file alone cannot say which ids were deleted
after the last surviving one of a sequence.  :meth:`Document.id_state` exports the ledger as a
short opaque string (:func:`export_state`) that the application keeps beside the bytes, and
``Document.open(data, ids=state)`` takes it back (:func:`load_state`): then no value the
document ever had is generated again, a deleted id still says by which change, and versions
go on from the saved one.  Word's bytes are not touched: the state is kept apart from them.
"""

from __future__ import annotations

import functools
import hashlib
import operator
import re
import threading
from collections import Counter
from dataclasses import dataclass, field

from lxml import etree
from ooxml_edit.errors import ArgumentError

from ..oxml.xml import MC, W14, Element

_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
W_P = _W + "p"
W_TBL = _W + "tbl"
W_TR = _W + "tr"
W_TC = _W + "tc"
PARA_ID = "{%s}paraId" % W14
TEXT_ID = "{%s}textId" % W14
MC_FALLBACK = "{%s}Fallback" % MC
MC_IGNORABLE = "{%s}Ignorable" % MC
WP14 = "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"
_WP = "{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}"
WP_INLINE = _WP + "inline"
WP_ANCHOR = _WP + "anchor"
ANCHOR_ID = "{%s}anchorId" % WP14
EDIT_ID = "{%s}editId" % WP14
MC_PROBE = "{%s}ProcessContent" % MC

#: ``ST_LongHexNumber`` values Word accepts as a paraId or textId: above 0, below 0x80000000.
PARA_ID_LIMIT = 0x80000000

_HEX = re.compile(r"[0-9A-Fa-f]{1,8}")


@functools.lru_cache(maxsize=65536)
def valid_long_hex(raw: str | None) -> bool:
    return raw is not None and _HEX.fullmatch(raw) is not None and 0 < int(raw, 16) < PARA_ID_LIMIT


#: :func:`valid_long_hex` as one pattern (lane G, for the loops over every paragraph): one to
#: eight hex digits, not all zeros, below 0x80000000 (eight digits: the first 0-7).
_VALID_LONG_HEX = re.compile(r"(?!0+\Z)(?:[0-9A-Fa-f]{1,7}|[0-7][0-9A-Fa-f]{7})")


def live_elements(root: Element, tags: frozenset[str]) -> list[Element]:
    """Elements with one of ``tags`` in document order, skipping ``mc:Fallback`` subtrees: a
    fallback repeats its choice's content (paraIds included, which the specification allows)
    and is not what Word shows.

    lxml's own walk (``iter``) finds them, in C: every index of a part is rebuilt after
    every edit, so this is the walk each edit of a batch repeats over the whole document
    (round 28, W8) -- the elements under a fallback are taken out after, when there is one."""
    if not tags:
        return []
    found = [node for node in root.iter(*tags) if node is not root]
    fallbacks = [node for node in root.iter(MC_FALLBACK) if node is not root]
    if not fallbacks:
        return found
    hidden = {node for fallback in fallbacks for node in fallback.iter(*tags)}
    return [node for node in found if node not in hidden]


@dataclass
class ParagraphEntry:
    element: Element
    id: str
    #: The ``w14:paraId`` as written, or ``None``.
    raw: str | None
    #: ``ok``, ``missing`` (positional id), ``repeat`` (``#k``) or ``invalid`` (out of range).
    state: str
    #: 0-based place among the story's paragraphs.
    index: int

    @property
    def volatile(self) -> bool:
        """Whether the id can change without the paragraph being edited."""
        return self.state in ("missing", "repeat")

    @property
    def needs_stamp(self) -> bool:
        return self.state != "ok"


@dataclass
class TableEntry:
    element: Element
    id: str
    rows: list[tuple[Element, str]] = field(default_factory=list)


class PartIndex:
    """Every paragraph and table of one story part, by id and by element.

    Built from the tree on demand and thrown away on any change: rebuilding is one pass,
    and an index that outlived an edit is how ids silently drift.
    """

    def __init__(self, story: str, root: Element) -> None:
        self.story = story
        self.root = root
        prefix = "" if story == "body" else f"{story}/"
        self.paragraphs: list[ParagraphEntry] = []
        self.by_id: dict[str, ParagraphEntry | TableEntry] = {}
        self._by_element: dict[int, ParagraphEntry | TableEntry] = {}

        everything = live_elements(root, frozenset({W_P, W_TR, W_TBL}))
        # Lane G: one pass over the elements' tags and paraIds (read once each), the repeats
        # counted only where some paraId is written twice -- the same entries as reading each
        # element's attributes again in every loop, a fraction of the time on a long body.
        tags = [node.tag for node in everything]
        raws = [None if tag == W_TBL else node.get(PARA_ID) for node, tag in zip(everything, tags)]
        keys = [raw.upper() for raw in raws if raw is not None]
        repeats: list[int] | None = None
        if len(set(keys)) != len(keys):
            seen: dict[str, int] = {}
            repeats = []
            for raw in raws:
                if raw is None:
                    repeats.append(0)
                    continue
                key = raw.upper()
                count = seen.get(key, 0)
                repeats.append(count)
                seen[key] = count + 1
        repeat_of: dict[int, int] = {}
        if repeats is not None:
            repeat_of = {id(node): repeat for node, raw, repeat in zip(everything, raws, repeats) if raw is not None}
        ok = _VALID_LONG_HEX.fullmatch

        paragraphs = self.paragraphs
        by_id = self.by_id
        by_element = self._by_element
        entry_class = ParagraphEntry
        index = 0
        missing_prefix = f"p@{story}/"
        id_prefix = f"{prefix}p:"
        for k, node in enumerate(everything):
            if tags[k] != W_P:
                continue
            raw = raws[k]
            if raw is None:
                entry = entry_class(node, f"{missing_prefix}{index}", None, "missing", index)
            else:
                repeat = repeats[k] if repeats is not None else 0
                if repeat:
                    entry = entry_class(node, f"{id_prefix}{raw}#{repeat}", raw, "repeat", index)
                else:
                    entry = entry_class(node, id_prefix + raw, raw, "ok" if ok(raw) is not None else "invalid", index)
            paragraphs.append(entry)
            by_id[entry.id] = entry
            by_element[id(node)] = entry
            index += 1

        self.tables: list[TableEntry] = []
        for number, node in enumerate(n for n, tag in zip(everything, tags) if tag == W_TBL):
            rows = [row for row in live_elements(node, frozenset({W_TR})) if _owner_table(row) is node]
            first = rows[0].get(PARA_ID) if rows else None
            unique = first is not None and valid_long_hex(first) and repeat_of.get(id(rows[0]), 0) == 0
            table_id = f"{prefix}t:{first}" if unique else f"t@{story}/{number}"
            entry_t = TableEntry(node, table_id)
            for k, row in enumerate(rows):
                raw = row.get(PARA_ID)
                good = raw is not None and valid_long_hex(raw) and repeat_of.get(id(row), 0) == 0
                entry_t.rows.append((row, f"{prefix}tr:{raw}" if good else f"{table_id}/r{k}"))
            self.tables.append(entry_t)
            self.by_id[table_id] = entry_t
            self._by_element[id(node)] = entry_t
        #: 0.1.118: what the index was made from (:func:`walk_of`), for telling an index
        #: still the tree's after a change (:func:`same_walk`).
        self._walk = (everything, raws, _owners(everything, tags))
        #: Lane J: the walk's digest (``ooxml_core.index_walk``), for :func:`same_walk`.
        self._walk_digest = walk_digest(root)

    def entry_for(self, element: Element) -> ParagraphEntry | TableEntry | None:
        return self._by_element.get(id(element))

    def paragraph_ids(self) -> set[str]:
        return {entry.raw.upper() for entry in self.paragraphs if entry.raw}


_BODY_BLOCKS = frozenset({W_P, W_TBL, _W + "sdt", _W + "customXml"})
_P_START = re.compile(rb"<w:p(?=[\s/>])")
_TBL_START = re.compile(rb"<w:tbl(?=[\s/>])")
_TR_START = re.compile(rb"<w:tr(?=[\s/>])")
_FALLBACK_BYTES = re.compile(rb"<mc:Fallback(?=[\s>]).*?</mc:Fallback>", re.S)


def _span_counts(data: bytes, begin: int, end: int) -> tuple[int, int, int]:
    """The paragraphs, tables and rows (not under ``mc:Fallback``) in ``data[begin:end]``."""
    piece = data[begin:end]
    if b"<mc:Fallback" in piece:
        piece = _FALLBACK_BYTES.sub(b"", piece)
    return len(_P_START.findall(piece)), len(_TBL_START.findall(piece)), len(_TR_START.findall(piece))


class IndexBase:
    """What :func:`body_index` builds the body's next index from (0.1.116): an index, the
    main part's bytes it was built from, the body's top-level blocks then (elements, spans,
    their paragraphs' and tables' counts), and the paraIds' counts (upper case)."""

    def __init__(self, index: "PartIndex", data: bytes, tops: list, spans: list, counts: list,
                 keys: Counter, body: tuple[int, int], everything: list) -> None:
        self.index, self.data, self.tops, self.spans, self.counts = index, data, tops, spans, counts
        self.keys, self.body = keys, body
        #: The paragraphs, rows and tables in document order (what PartIndex walks).
        self.everything = everything


def _plain(index: "PartIndex") -> bool:
    """Whether an index's ids depend on no place: every paragraph's, table's and row's id
    its own paraId's, none repeated (what follows the first edit's stamping)."""
    return all(e.state == "ok" for e in index.paragraphs) and all(
        t.id.startswith("t:") and all(rid.startswith("tr:") for _, rid in t.rows) for t in index.tables)


def body_index(root: Element, data: bytes, base: "IndexBase | None") -> "tuple[PartIndex, IndexBase | None]":
    """The body's :class:`PartIndex` (story ``body``) of the tree ``root`` whose part's bytes
    are ``data``, made from ``base`` (the index of an earlier state of the same tree) where
    only some top-level blocks changed: the entries of the blocks before and after them
    kept (the later ones' places moved), those of the changed blocks made -- the same index,
    in time proportional to the change.  The whole index where that cannot be said so."""
    from ..revalidate import _scan
    from ..tools.touched import _blocks, _common_prefix, _common_suffix

    body_element = root.find(_W + "body")
    tops = [child for child in body_element if child.tag in _BODY_BLOCKS] if body_element is not None else []
    made = None
    if base is not None and base.data is not data:
        made = _index_from(root, data, base, tops, _blocks, _common_prefix, _common_suffix)
    elif base is not None and base.data is data and base.tops == tops and \
            live_elements(root, frozenset({W_P, W_TR, W_TBL})) == base.everything:
        made = (base.index, base)
    if made is not None:
        from .. import incremental

        incremental.STATS["body index: per change"] += 1
        return made
    index = PartIndex("body", root)
    found = _scan(data) if _plain(index) else None
    if found is None or len(found[0]) != len(tops):
        return index, None
    spans, body, _ = found
    counts = [_span_counts(data, a, b) for a, b in spans]
    everything = live_elements(root, frozenset({W_P, W_TR, W_TBL}))
    if sum(c[0] for c in counts) != len(index.paragraphs) or sum(c[1] for c in counts) != len(index.tables) \
            or sum(sum(c) for c in counts) != len(everything):
        return index, None
    keys = Counter(e.raw.upper() for e in index.paragraphs)
    keys.update(rid.rpartition("tr:")[2].upper() for t in index.tables for _, rid in t.rows)
    return index, IndexBase(index, data, tops, list(spans), counts, keys, body, everything)


_FIRST = operator.itemgetter(0)
_SECOND = operator.itemgetter(1)


def _index_from(root, data, base, tops, _blocks, _common_prefix, _common_suffix):
    from ..tools.touched import _kept_counts

    old = base.data
    prefix = _common_prefix(old, data)
    suffix = _common_suffix(old, data, min(len(old), len(data)) - prefix)
    body_at, body_end = base.body
    if prefix < body_at or len(old) - suffix > body_end:
        return None
    spans = base.spans
    n_old = len(spans)
    before, after = _kept_counts(spans, prefix, len(old) - suffix)
    delta = len(data) - len(old)
    begin = spans[before - 1][1] if before else body_at
    stop = spans[n_old - after][0] + delta if after else body_end + delta
    middle = _blocks(data, begin, stop)
    if middle is None or len(tops) != before + len(middle) + after:
        return None
    if tops[:before] != base.tops[:before] or tops[len(tops) - after:] != base.tops[n_old - after:]:
        return None                        # the kept blocks are not the same elements
    counts = [_span_counts(data, a, b) for a, b in middle]
    region = _span_counts(data, begin, stop)
    if region != tuple(sum(c[k] for c in counts) for k in range(3)):
        return None
    old_index = base.index
    # (lane J: summed in C, not by a generator over every block)
    p_before = sum(map(_FIRST, base.counts[:before]))
    p_gone = sum(map(_FIRST, base.counts[before:n_old - after]))
    t_before = sum(map(_SECOND, base.counts[:before]))
    t_gone = sum(map(_SECOND, base.counts[before:n_old - after]))
    e_before = sum(map(sum, base.counts[:before]))
    e_after = sum(map(sum, base.counts[n_old - after:]))
    # Every paragraph, row and table as PartIndex walks them: those of the kept blocks must be
    # the same elements as before (an element put in place of one written the same is not).
    walked = live_elements(root, frozenset({W_P, W_TR, W_TBL}))
    old_walk = base.everything
    if len(walked) != e_before + e_after + sum(sum(c) for c in counts) \
            or walked[:e_before] != old_walk[:e_before] \
            or walked[len(walked) - e_after:] != old_walk[len(old_walk) - e_after:]:
        return None
    everything = walked[e_before:len(walked) - e_after]
    tags = [node.tag for node in everything]
    raws = [None if tag == W_TBL else node.get(PARA_ID) for node, tag in zip(everything, tags)]
    if any(raw is None for raw, tag in zip(raws, tags) if tag != W_TBL):
        return None                        # a paragraph or row without a paraId: placed ids
    keys = base.keys.copy()
    # (0.1.118: a count gone to 0 taken out here, not every count looked through after)
    _uncount(keys, [entry.raw.upper() for entry in old_index.paragraphs[p_before:p_before + p_gone]])
    for table in old_index.tables[t_before:t_before + t_gone]:
        _uncount(keys, [rid.rpartition("tr:")[2].upper() for _, rid in table.rows])
    new_keys = [raw.upper() for raw in raws if raw is not None]
    keys.update(new_keys)
    if any(keys[k] > 1 for k in new_keys):
        return None                        # a paraId written twice: repeats are placed
    paragraphs: list[ParagraphEntry] = []
    ok = _VALID_LONG_HEX.fullmatch
    for node, tag, raw in zip(everything, tags, raws):
        if tag == W_P:
            if ok(raw) is None:
                return None
            paragraphs.append(ParagraphEntry(node, "p:" + raw, raw, "ok", p_before + len(paragraphs)))
    tables: list[TableEntry] = []
    for node in (n for n, tag in zip(everything, tags) if tag == W_TBL):
        rows = [row for row in live_elements(node, frozenset({W_TR})) if _owner_table(row) is node]
        first = rows[0].get(PARA_ID) if rows else None
        if first is None or not valid_long_hex(first):
            return None
        entry_t = TableEntry(node, f"t:{first}")
        for row in rows:
            raw = row.get(PARA_ID)
            if raw is None or not valid_long_hex(raw):
                return None
            entry_t.rows.append((row, f"tr:{raw}"))
        tables.append(entry_t)
    if len(paragraphs) != sum(c[0] for c in counts) or len(tables) != sum(c[1] for c in counts):
        return None
    index = PartIndex.__new__(PartIndex)
    index.story, index.root = "body", root
    gone_p = old_index.paragraphs[p_before:p_before + p_gone]
    gone_t = old_index.tables[t_before:t_before + t_gone]
    shift = len(paragraphs) - p_gone
    rest = old_index.paragraphs[p_before + p_gone:]
    if shift:
        for entry in rest:
            entry.index += shift
    index.paragraphs = old_index.paragraphs[:p_before] + paragraphs + rest
    index.tables = old_index.tables[:t_before] + tables + old_index.tables[t_before + t_gone:]
    by_id = dict(old_index.by_id)
    by_element = dict(old_index._by_element)
    for entry in gone_p:
        by_id.pop(entry.id, None)
        by_element.pop(id(entry.element), None)
    for entry_t in gone_t:
        by_id.pop(entry_t.id, None)
        by_element.pop(id(entry_t.element), None)
    for entry in paragraphs:
        by_id[entry.id] = entry
        by_element[id(entry.element)] = entry
    for entry_t in tables:
        by_id[entry_t.id] = entry_t
        by_element[id(entry_t.element)] = entry_t
    index.by_id, index._by_element = by_id, by_element
    if old_index.__dict__.get("_stamping_due") is False:
        # (0.1.118: the blocks made here are all stamped, the others as before)
        index.__dict__["_stamping_due"] = False
    old_walk_raws = getattr(old_index, "_walk", None)
    if old_walk_raws is not None:
        _, base_raws, base_owners = old_walk_raws
        index._walk = (walked, base_raws[:e_before] + raws + base_raws[len(base_raws) - e_after:],
                       base_owners[:e_before] + _owners(everything, tags)
                       + base_owners[len(base_owners) - e_after:])
    else:
        index._walk = None
    index._walk_digest = walk_digest(root) if index._walk is not None else None
    new_spans = spans[:before] + middle + [(s + delta, e + delta) for s, e in spans[n_old - after:]]
    new_counts = base.counts[:before] + counts + base.counts[n_old - after:]
    return index, IndexBase(index, data, tops, new_spans, new_counts, keys, (body_at, body_end + delta), walked)


def _owners(everything: list, tags: list) -> list:
    """Each row's table (:func:`_owner_table`), ``None`` for a paragraph or table."""
    return [_owner_table(node) if tag == W_TR else None for node, tag in zip(everything, tags)]


def same_walk(index: "PartIndex") -> bool:
    """Whether the tree ``index`` was made from still has its paragraphs, rows and tables:
    the same elements in the same order, the same paraIds, each row in the same table --
    all a :class:`PartIndex` reads, so the index is still the tree's (0.1.118: an index
    kept through a change inside a table or a paragraph's text, not made again)."""
    walk = getattr(index, "_walk", None)
    if walk is None:
        return False
    digest = getattr(index, "_walk_digest", None)
    if digest is not None:
        # Lane J (0.1.120): the walk read in Rust, as a digest of the elements (the nodes:
        # the index holds each, so none is freed and another made where it was), their
        # kinds, paraIds and rows' tables -- what is compared below, milliseconds instead of
        # a walk in Python.
        now_digest = walk_digest(index.root)
        if now_digest is not None:
            same = now_digest == digest
            from .. import incremental

            if incremental.verifying() and same != _same_walk(index, walk):
                raise incremental.Mismatch("same walk: the walk's digest differs from the walk")
            return same
    return _same_walk(index, walk)


def _same_walk(index: "PartIndex", walk: tuple) -> bool:
    everything, raws, owners = walk
    now = live_elements(index.root, frozenset({W_P, W_TR, W_TBL}))
    if len(now) != len(everything) or now != everything:
        return False
    tags = [node.tag for node in now]
    if [None if tag == W_TBL else node.get(PARA_ID) for node, tag in zip(now, tags)] != raws:
        return False
    return _owners(now, tags) == owners


def walk_digest(root: Element) -> bytes | None:
    """``ooxml_core.index_walk(root)`` (lane J), or ``None`` without the core, with the
    per-change passes off, or where the walk cannot be read so."""
    from .. import incremental

    if not incremental.enabled():
        return None
    from ..oxml.package import _core

    core = _core()
    walk = getattr(core, "index_walk", None)
    return walk(root) if walk is not None else None


def same_index(a: "PartIndex", b: "PartIndex") -> bool:
    """Whether two indexes name the same elements the same way (the oracle's test)."""
    return ([(e.element, e.id, e.raw, e.state, e.index) for e in a.paragraphs]
            == [(e.element, e.id, e.raw, e.state, e.index) for e in b.paragraphs]
            and [(t.element, t.id, t.rows) for t in a.tables] == [(t.element, t.id, t.rows) for t in b.tables]
            and {k: v.element for k, v in a.by_id.items()} == {k: v.element for k, v in b.by_id.items()}
            and {k: v.element for k, v in a._by_element.items()} == {k: v.element for k, v in b._by_element.items()})


def _owner_table(row: Element) -> Element | None:
    node = row.getparent()
    while node is not None and node.tag != W_TBL:
        node = node.getparent()
    return node


# -- generating and stamping -----------------------------------------------------------------


def used_long_hex(roots: list[Element]) -> set[int]:
    """Every paraId and textId, and every drawing's anchorId and editId, written anywhere in
    ``roots``.

    lxml's walk over the four elements (``iter``, in C), the attributes read by name: a
    quarter of the time of the XPath it replaced (round 37; that one walked the tree once
    per element and attribute), read before every edit that renews a textId (round 28, W8)."""
    used: set[int] = set()
    for root in roots:
        for node in root.iter(W_P, W_TR, WP_INLINE, WP_ANCHOR):
            for name in _USED_NAMES:
                raw = node.get(name)
                if raw is not None and _HEX.fullmatch(raw):
                    used.add(int(raw, 16))
    return used


#: The attributes :func:`used_long_hex` reads.
_USED_NAMES = (PARA_ID, TEXT_ID, ANCHOR_ID, EDIT_ID)


#: A drawing's number, its ``d:`` id, is its ``wp:docPr``'s ``id``.
WP_DOC_PR = _WP + "docPr"


class DeletedId(KeyError):
    """An id that named a block (or a drawing) this session and names nothing now: deleted,
    or replaced.  A ``KeyError``, as any id the document lacks; ``version`` is the version of
    the change that removed it (``None`` when not known), ``replaced_by`` the id of the one
    block that took its place, when one did, ``earlier`` whether that change was made before
    the document was last opened (with its id state, round 38)."""

    def __init__(self, message: str, identifier: str, version: int | None = None,
                 replaced_by: str | None = None, *, earlier: bool = False) -> None:
        super().__init__(message)
        self.identifier = identifier
        self.version = version
        self.replaced_by = replaced_by
        #: Whether the change was made before the document was saved and opened again
        #: (with its id state, round 38): no undo of this session reaches it.
        self.earlier = earlier

    def __str__(self) -> str:
        return str(self.args[0])


class Ledger:
    """What a document's ids have been this session, beside what they are now.

    ``pool`` holds every ``ST_LongHexNumber`` value any state had -- paraIds, textIds,
    drawing ids -- and every one :func:`generate` gave, so none is generated again.
    ``gone`` maps each block id value no longer in the document to the version of the change
    that removed it, ``drawings_gone`` the same for drawing numbers, for the error that says
    so.  :meth:`note` reads a state: once per version, at the edges of edits, undo and redo.

    Memory: one int per value ever written (a paragraph's paraId and textId, each textId an
    edit of its text renews), about 60 bytes each in a set, plus a dict entry for each
    deleted block -- some hundred kilobytes for a long report written and rewritten all day.
    """

    def __init__(self) -> None:
        self.pool: IdPool | None = None
        self.version: int | None = None
        self.blocks: set[int] = set()
        self.gone: dict[int, int] = {}
        self.drawings: set[int] = set()
        self.drawings_gone: dict[int, int] = {}
        #: The highest drawing number any state had or an insertion took.
        self.highest_drawing = 0
        #: Block id value -> the id of the one block that replaced it, in its change.
        self.replaced: dict[int, str] = {}
        #: The changes made before the document was saved and opened again with its id
        #: state (round 38), by version: who made each, in words (``None``: not known).
        self.earlier: dict[int, str | None] = {}

    def restore(self, loaded: "LoadedState") -> None:
        """Take back what a state saved beside the bytes held (:func:`load_state`), before
        any state of this session is read."""
        self.pool = loaded.pool
        self.gone = dict(loaded.gone)
        self.drawings_gone = dict(loaded.drawings_gone)
        self.highest_drawing = max(self.highest_drawing, loaded.highest_drawing)
        self.replaced = dict(loaded.replaced)
        self.earlier = dict(loaded.steps)

    def note(self, roots: list[Element], version: int,
             found: "tuple[set[int], set[int]] | None" = None) -> None:
        """Read the state ``roots`` are, as version ``version``: what is gone since the last
        state read went in ``version``; what is back (an undone deletion) is no longer gone.
        ``found``: the state's block ids and drawing numbers, read already (:class:`IdScan`,
        per change); else read here from ``roots``."""
        if self.version == version and self.pool is not None:
            return
        if found is not None and self.pool is not None:
            blocks, drawings = found
        else:
            blocks, drawings = block_and_drawing_ids(roots)
        if self.pool is None:
            # The state as opened: every value in it.  Later states add only what generate
            # gave (into the pool itself), what a copy kept (Document._remember_ids) and
            # what Document._used finds in use.
            self.pool = IdPool(used_long_hex(roots))
        else:
            for value in self.blocks - blocks:
                self.gone[value] = version
            for value in self.drawings - drawings:
                self.drawings_gone[value] = version
        for value in blocks & self.gone.keys():
            del self.gone[value]
        for value in drawings & self.drawings_gone.keys():
            del self.drawings_gone[value]
        self.pool |= blocks
        self.blocks = blocks
        self.drawings = drawings
        self.highest_drawing = max(self.highest_drawing, max(drawings, default=0))
        self.version = version


def block_and_drawing_ids(roots: list[Element]) -> tuple[set[int], set[int]]:
    """Every paragraph's and row's paraId (as a value) and every drawing number in
    ``roots``: what :meth:`Ledger.note` reads of a state."""
    blocks: set[int] = set()
    drawings: set[int] = set()
    for root in roots:
        for node in root.iter(W_P, W_TR, WP_DOC_PR):
            if node.tag == WP_DOC_PR:
                raw = node.get("id")
                if raw is not None and raw.isdigit():
                    drawings.add(int(raw))
            else:
                raw = node.get(PARA_ID)
                if raw is not None and _HEX.fullmatch(raw):
                    blocks.add(int(raw, 16))
    return blocks, drawings


_W_URI = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
_WP_URI = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
#: The prefixes :class:`IdScan` reads the body's bytes by, and the namespaces they must name.
_SCAN_PREFIXES = {b"w": _W_URI.encode(), b"w14": W14.encode(), b"wp": _WP_URI.encode(), b"wp14": WP14.encode()}
#: The start tags of the elements :func:`block_and_drawing_ids` reads, in such bytes.
_SCANNED = re.compile(rb"<(w:p|w:tr|wp:docPr)(?=[\s/>])([^>]*)>")
_SCANNED_ATTRIBUTE = re.compile(rb'\s([\w.:-]+)="([^"]*)"')
_DECLARATION = re.compile(rb'\sxmlns(?::([\w.-]+))?="([^"]*)"')
_REFERENCE = re.compile(r"&(#x[0-9A-Fa-f]+|#[0-9]+|amp|lt|gt|quot|apos);")
_NAMED = {"amp": "&", "lt": "<", "gt": ">", "quot": '"', "apos": "'"}


def _attribute_text(raw: bytes) -> str:
    """An attribute's value as the parser reads it, from its bytes as lxml writes them."""
    text = raw.decode("utf-8")
    if "&" in text:
        text = _REFERENCE.sub(lambda m: _NAMED.get(m.group(1)) or chr(
            int(m.group(1)[2:], 16) if m.group(1)[1] == "x" else int(m.group(1)[1:])), text)
    return text


def _scanned_ids(data: bytes, begin: int, end: int) -> tuple[tuple[int, ...], tuple[int, ...]]:
    """:func:`block_and_drawing_ids` of ``data[begin:end]`` (bytes as lxml writes a part
    whose root declares :data:`_SCAN_PREFIXES` and nothing below it declares any)."""
    blocks: list[int] = []
    drawings: list[int] = []
    for match in _SCANNED.finditer(data, begin, end):
        docpr = match.group(1) == b"wp:docPr"
        for attribute in _SCANNED_ATTRIBUTE.finditer(match.group(2)):
            name = attribute.group(1)
            if docpr and name == b"id":
                raw = _attribute_text(attribute.group(2))
                if raw.isdigit():
                    drawings.append(int(raw))
            elif not docpr and name == b"w14:paraId":
                raw = _attribute_text(attribute.group(2))
                if _HEX.fullmatch(raw):
                    blocks.append(int(raw, 16))
    return tuple(blocks), tuple(drawings)


def _uncount(counts: Counter, keys: "tuple[int, ...]") -> None:
    """``counts`` less one of each of ``keys`` (counted there), a key at 0 taken out."""
    for key in keys:
        value = counts[key] - 1
        if value > 0:
            counts[key] = value
        else:
            del counts[key]


class IdScan:
    """:meth:`Ledger.note`'s reading of a state (every paraId and drawing number), per
    change (0.1.116): the body's top-level blocks read from the main part's bytes, each
    block's ids kept, and only the blocks between the first and the last byte that differ
    from the state read before read again (``docx_agent.tools.touched``'s rule); the other
    parts read whole when their bytes changed.  ``None`` from :meth:`read` where the bytes
    cannot be read so (a part not written by lxml with the usual prefixes): the whole
    reading then, as before."""

    def __init__(self) -> None:
        #: The main part as last read: its bytes, blocks' spans, body span, each block's
        #: ids, the ids outside blocks, and the counts of each.
        self.data: bytes | None = None
        self.spans: list[tuple[int, int]] = []
        self.body: tuple[int, int] = (0, 0)
        self.units: list[tuple[tuple[int, ...], tuple[int, ...]]] = []
        self.blocks: Counter = Counter()
        self.drawings: Counter = Counter()
        #: Other parts: name -> (their bytes or stored token, block ids, drawing numbers).
        self.parts: dict[str, tuple[object, set[int], set[int]]] = {}

    def read(self, document) -> "tuple[set[int], set[int]] | None":
        package = document.package
        main = package.document_part()
        blocks: set[int] = set()
        drawings: set[int] = set()
        seen: set[str] = set()
        for part in document._parts():
            root = package.tree(part)
            if root is None:
                continue
            seen.add(part)
            if part == main and part in package.dirty_parts:
                found = self._main(package.read(part) or b"")
                if found is None:
                    return None
                blocks |= found[0]
                drawings |= found[1]
                continue
            token = package.read(part) if part in package.dirty_parts else package._parts.stored(part)
            kept = self.parts.get(part)
            if kept is None or not (kept[0] is token or (isinstance(token, bytes) and kept[0] == token)):
                kept = (token, *block_and_drawing_ids([root]))
                self.parts[part] = kept
            blocks |= kept[1]
            drawings |= kept[2]
        for part in list(self.parts):
            if part not in seen:
                del self.parts[part]
        return blocks, drawings

    def _main(self, data: bytes) -> "tuple[set[int], set[int]] | None":
        from ..tools.touched import _blocks, _common_prefix, _common_suffix, _kept_counts

        old = self.data
        if old is not None and (old is data or old == data):
            self.data = data
            return set(self.blocks), set(self.drawings)
        if old is not None:
            prefix = _common_prefix(old, data)
            suffix = _common_suffix(old, data, min(len(old), len(data)) - prefix)
            body_at, body_end = self.body
            if prefix >= body_at and len(old) - suffix <= body_end:
                delta = len(data) - len(old)
                spans = self.spans
                before, after = _kept_counts(spans, prefix, len(old) - suffix)
                begin = spans[before - 1][1] if before else body_at
                stop = spans[len(spans) - after][0] + delta if after else body_end + delta
                middle = _blocks(data, begin, stop)
                if middle is not None and b"xmlns" not in data[begin:stop]:
                    units = [_scanned_ids(data, a, b) for a, b in middle]
                    outside = sum(len(u[0]) + len(u[1]) for u in units)
                    found = _scanned_ids(data, begin, stop)
                    if len(found[0]) + len(found[1]) == outside:
                        gone = self.units[before:len(spans) - after]
                        # (0.1.118: the counts changed in place, a count gone to 0 taken
                        # out -- not every count looked through again)
                        for unit in gone:
                            _uncount(self.blocks, unit[0])
                            _uncount(self.drawings, unit[1])
                        for unit in units:
                            self.blocks.update(unit[0])
                            self.drawings.update(unit[1])
                        self.units[before:len(spans) - after] = units
                        self.spans = (spans[:before] + middle
                                      + [(s + delta, e + delta) for s, e in spans[len(spans) - after:]])
                        self.body = (body_at, body_end + delta)
                        self.data = data
                        return set(self.blocks), set(self.drawings)
        return self._whole(data)

    def _whole(self, data: bytes) -> "tuple[set[int], set[int]] | None":
        from ..revalidate import _scan

        self.data = None
        root_end = data.find(b">", data.find(b"<w:document"))
        found = _scan(data)
        if root_end < 0 or found is None:
            return None
        declared: dict[bytes, bytes] = {}
        for match in _DECLARATION.finditer(data, 0, root_end):
            if match.group(1) is None:
                return None                  # a default namespace: elements unprefixed
            declared[match.group(1)] = match.group(2)
        for prefix, uri in declared.items():
            if uri in _SCAN_PREFIXES.values() and _SCAN_PREFIXES.get(prefix) != uri:
                return None                  # one of them under another prefix
            if prefix in _SCAN_PREFIXES and _SCAN_PREFIXES[prefix] != uri:
                return None
        if data.find(b"xmlns", root_end) >= 0:
            return None
        spans, body, _head = found
        units = [_scanned_ids(data, a, b) for a, b in spans]
        everything = _scanned_ids(data, 0, len(data))
        if sum(len(u[0]) + len(u[1]) for u in units) != len(everything[0]) + len(everything[1]):
            return None                      # ids outside the body's blocks
        self.spans, self.body, self.units = list(spans), body, units
        self.blocks = Counter()
        self.drawings = Counter()
        for unit in units:
            self.blocks.update(unit[0])
            self.drawings.update(unit[1])
        self.data = data
        return set(self.blocks), set(self.drawings)


def drawings_needing_ids(root: Element) -> list[Element]:
    """The drawings (``wp:inline``, ``wp:anchor``) without a valid ``wp14:anchorId`` and
    ``wp14:editId``: Word keeps no paraId in a document that has one (measured)."""
    return [node for node in live_elements(root, frozenset({WP_INLINE, WP_ANCHOR}))
            if not (valid_long_hex(node.get(ANCHOR_ID)) and valid_long_hex(node.get(EDIT_ID)))]


#: How far :func:`generate` looks along a seed's sequence past the last value in use before it
#: takes new ones: a run of values the document lacks shorter than this, followed by one it
#: has, is one this sequence wrote and an edit deleted (a deleted table of a thousand cells is
#: two thousand values: two per paragraph or row).  Found once per seed and document
#: (:class:`IdPool`), from cached values: a set lookup each.
REACH = 4096

#: Each seed's sequence as far as it has been needed, shared by every document (the seeds are
#: part names, so few): ``seed -> values``.  Replaced, never changed in place, under the lock.
_SEQUENCES: dict[str, list[int]] = {}
_SEQUENCES_LOCK = threading.Lock()


def _sequence(seed: str, length: int) -> list[int]:
    """The first ``length`` (or more) values of ``seed``'s sequence, in ``0 .. 0x7FFFFFFF``."""
    found = _SEQUENCES.get(seed)
    if found is not None and len(found) >= length:
        return found
    with _SEQUENCES_LOCK:
        found = _SEQUENCES.get(seed) or []
        if len(found) < length:
            more = [int.from_bytes(hashlib.sha256(f"{seed}\0{counter}".encode()).digest()[:4], "big")
                    & 0x7FFFFFFF for counter in range(len(found), max(length, 2 * len(found)))]
            found = found + more
            _SEQUENCES[seed] = found
        return found


def copy_ledger(ledger: Ledger) -> Ledger:
    """A copy of ``ledger`` that changes apart from it (round 45: a read copy of a document's
    version, ``docx_agent.layout.read_copy``): every set and map copied, the pool with its
    cursors."""
    out = Ledger()
    if ledger.pool is not None:
        out.pool = IdPool(ledger.pool)
        out.pool.cursors.update(ledger.pool.cursors)
    out.version = ledger.version
    out.blocks = set(ledger.blocks)
    out.gone = dict(ledger.gone)
    out.drawings = set(ledger.drawings)
    out.drawings_gone = dict(ledger.drawings_gone)
    out.highest_drawing = ledger.highest_drawing
    out.replaced = dict(ledger.replaced)
    out.earlier = dict(ledger.earlier)
    return out


class IdPool(set):
    """A document's ids, in use and retired (:meth:`Ledger.pool`): a set of ints that
    :func:`generate` adds to, and where along each seed's sequence it got to -- every value
    before that point is in the set, so the next call starts there."""

    __slots__ = ("cursors",)

    def __init__(self, values=()) -> None:
        super().__init__(values)
        self.cursors: dict[str, int] = {}


def _after_last(seed: str, used: set[int]) -> int:
    """Where along ``seed``'s sequence new values start: after the last one ``used`` holds
    with no later one within :data:`REACH` -- 0 for a document none of whose ids it wrote."""
    last = -1
    k = 0
    values = _sequence(seed, REACH + 1)
    while k - last <= REACH:
        if k >= len(values):
            values = _sequence(seed, 2 * k)
        if values[k] in used:
            last = k
        k += 1
    return last + 1


def generate(seed: str, used: set[int], count: int = 1) -> list[str]:
    """``count`` new ``ST_LongHexNumber`` values, deterministic in ``seed`` and ``used``:
    the next members of the seed's sequence that ``used`` does not hold, in ``1 ..
    0x7FFFFFFF``, upper-case and eight digits wide as Word writes them.  ``used`` gains them.

    *Next* is after the last member ``used`` holds (:func:`_after_last`), not the first free
    one: a member before it that ``used`` lacks was written once and deleted -- in a document
    saved and opened again, whose session no longer knows it -- and a block id is never
    given to another block.  An :class:`IdPool` remembers where each seed got to."""
    cursors = getattr(used, "cursors", None)
    k = cursors.get(seed) if cursors is not None else None
    if k is None:
        k = _after_last(seed, used)
    values = _sequence(seed, k + 2 * count + 16)
    out: list[str] = []
    while len(out) < count:
        if k >= len(values):
            values = _sequence(seed, 2 * k + 16)
        value = values[k]
        k += 1
        if value == 0 or value in used:
            continue
        used.add(value)
        out.append(f"{value:08X}")
    if cursors is not None:
        cursors[seed] = k
    drew = getattr(used, "drew", None)
    if drew is not None:
        # Round 45 (lane B): a draft's pool records what it drew, in order, so the same
        # draws can be made again where the document's sequence is when its change is
        # applied (docx_agent.blocks).
        drew(seed, out)
    return out


def ensure_w14(root: Element, *, drawings: bool = False, extra: dict[str, str] | None = None) -> None:
    """Declare ``w14`` (and ``wp14``, with ``drawings``) on the part's root and list them in
    ``mc:Ignorable``, once attributes in them have been written somewhere in the tree.

    lxml declares a namespace where it is first used, under a made-up prefix; Word wants
    them declared on the root, and a consumer that predates Word 2010 must be told it may
    ignore them.  A root that already declares and ignores them is left alone.
    Declarations the document has, used or not, are kept: ``mc:Ignorable`` names prefixes.
    ``extra`` names further ignorable namespaces by prefix (``w16du``, for a revision's
    ``w16du:dateUtc``).
    """
    wanted = {"w14": W14, **({"wp14": WP14} if drawings else {}), **(extra or {})}
    found = {name: next((p for p, uri in root.nsmap.items() if uri == want and p), None)
             for name, want in wanted.items()}
    mc = next((p for p, uri in root.nsmap.items() if uri == MC and p), None)
    if mc is None or None in found.values():
        # Keep every declaration the document made; not the ones lxml just made up for the
        # attributes written before this call, which the top declarations replace.
        ours = set(wanted.values()) | {MC}
        prefixes: set[str] = set()
        for node in root.iter():
            if isinstance(node.tag, str):
                prefixes.update(p for p, uri in node.nsmap.items()
                                if p and not (uri in ours and p not in (*wanted, "mc")))
        top = {name: wanted[name] for name, prefix in found.items() if prefix is None}
        # cleanup_namespaces declares at the top only what is used below it, so mc is used
        # for a moment on a child; the attribute goes, the declaration stays.
        probe = next((child for child in root if isinstance(child.tag, str)), None)
        if mc is None and probe is not None:
            top["mc"] = MC
            probe.set(MC_PROBE, "")
        etree.cleanup_namespaces(root, top_nsmap=top, keep_ns_prefixes=sorted(prefixes))
        if probe is not None:
            probe.attrib.pop(MC_PROBE, None)
        found = {name: next((p for p, uri in root.nsmap.items() if uri == want and p), name)
                 for name, want in wanted.items()}
    ignorable = (root.get(MC_IGNORABLE) or "").split()
    missing = [prefix for prefix in found.values() if prefix not in ignorable]
    if missing:
        root.set(MC_IGNORABLE, " ".join(ignorable + missing))


def stamp(element: Element, para_id: str, text_id: str) -> None:
    element.set(PARA_ID, para_id)
    element.set(TEXT_ID, text_id)


# -- the state kept beside the bytes (round 38) ------------------------------------------------

#: What every id state starts with: the format's name and number.  A later docx-agent that
#: writes another form changes the number and still reads this one.
STATE_PREFIX = "docx-ids.1."


class IdStateError(ArgumentError):
    """``Document.open(..., ids=state)`` refused the state: not one docx-agent wrote, or not
    the state of these bytes.  An :class:`~ooxml_edit.errors.ArgumentError` (so a
    ``ValueError``); the message says what to pass instead."""


#: The way out every refusal of a state ends with.
_WAY_OUT = ("Pass the state saved together with these bytes (Session.save gives both at one "
            "version; a save_document output carries its own, Output.state), or open the bytes "
            "without ids=: the ids are then read from the file alone, and one deleted after the "
            "last surviving id of its sequence may be given again")


def drawing_numbers(roots: list[Element]) -> set[int]:
    """Every drawing number (``wp:docPr/@id``) written in ``roots``."""
    out: set[int] = set()
    for root in roots:
        for node in root.iter(WP_DOC_PR):
            raw = node.get("id")
            if raw is not None and raw.isdigit():
                out.add(int(raw))
    return out


def footprint(used: set[int], drawings: set[int]) -> str:
    """The digest of a document's ids -- every paraId, textId and drawing id
    (:func:`used_long_hex`) and drawing number -- that a state names the bytes it belongs
    to by: the same for the same ids however the package is zipped, and different after any
    edit that writes one (an edit of a paragraph's text renews its textId)."""
    digest = hashlib.sha256()
    digest.update(b"".join(value.to_bytes(4, "big") for value in sorted(used)))
    digest.update(b"|" + ",".join(str(number) for number in sorted(drawings)).encode())
    return digest.hexdigest()[:24]


def _deltas(values) -> list[int]:
    """Sorted values as their differences: small numbers, which compress well."""
    out: list[int] = []
    previous = 0
    for value in sorted(values):
        out.append(value - previous)
        previous = value
    return out


def _steps(values: list[int]) -> list[int]:
    """Values in the order given as their differences (which may be negative)."""
    out: list[int] = []
    previous = 0
    for value in values:
        out.append(value - previous)
        previous = value
    return out


def _sums(deltas) -> list[int]:
    out: list[int] = []
    running = 0
    for delta in deltas:
        running += int(delta)
        out.append(running)
    return out


def export_state(ledger: Ledger, roots: list[Element], *, version: int, last: int,
                 aliases: dict[str, str], describe=None) -> str:
    """The ledger of a document whose parts are ``roots``, at ``version`` (``last`` the
    highest version its history gave), as one short string: :data:`STATE_PREFIX` and the
    url-safe base64 of compressed JSON.  Read the ledger's state first (``Ledger.note``).

    Compact: a seed's generated values are the members of its sequence up to where it got
    (one number per seed); values in the file now are not written (opening reads them);
    a deleted block's id is its place along its seed's sequence where it has one, in order,
    with its change's version; who made a change (``describe(version)``) once per wording."""
    import base64
    import json
    import zlib

    pool = ledger.pool if ledger.pool is not None else IdPool()
    used = used_long_hex(roots)
    drawings = drawing_numbers(roots)
    seeds = sorted((seed, k) for seed, k in pool.cursors.items() if k > 0)
    place: dict[int, tuple[int, int]] = {}
    covered: set[int] = set()
    for number, (seed, k) in enumerate(seeds):
        members = _sequence(seed, k)[:k]
        for index, value in enumerate(members):
            place.setdefault(value, (number, index))
        covered.update(members)
    extra = set(pool) - used - covered

    # Each deleted id: its seed's number (-1: none) and place, and the version that deleted
    # it; written per seed in order of place, places and versions as differences.
    groups: dict[int, list[tuple[int, int]]] = {}
    for value, step in ledger.gone.items():
        number, index = place.get(value, (-1, value))
        groups.setdefault(number, []).append((index, step))
    gone = []
    for number, entries in sorted(groups.items()):
        entries.sort()
        gone.append([number, _deltas(index for index, _ in entries), _steps([step for _, step in entries])])
    drawn = sorted(ledger.drawings_gone.items())
    # Who made each change that deleted something, once per distinct wording.
    words: list[str] = []
    by: list[tuple[int, int]] = []
    for step in sorted({*ledger.gone.values(), *ledger.drawings_gone.values()}):
        text = ledger.earlier[step] if step in ledger.earlier else (describe(step) if describe is not None else None)
        if text is None:
            continue
        if text not in words:
            words.append(text)
        by.append((step, words.index(text)))
    record = {
        "digest": footprint(used, drawings), "version": version, "last": last,
        "seeds": [[seed, k] for seed, k in seeds], "extra": _deltas(extra), "gone": gone,
        "drawings": [_deltas(value for value, _ in drawn), _steps([step for _, step in drawn])],
        "who": words, "by": [_deltas(step for step, _ in by), [word for _, word in by]],
        "highest": ledger.highest_drawing,
        "replaced": {f"{value:08X}": new for value, new in sorted(ledger.replaced.items())
                     if value in ledger.gone},
        "aliases": dict(sorted(aliases.items())),
    }
    packed = zlib.compress(json.dumps(record, separators=(",", ":")).encode(), 9)
    return STATE_PREFIX + base64.urlsafe_b64encode(packed).decode("ascii").rstrip("=")


@dataclass
class LoadedState:
    """An id state read back (:func:`load_state`): what :meth:`Ledger.restore` takes."""

    version: int
    last: int
    pool: IdPool
    gone: dict[int, int]
    drawings_gone: dict[int, int]
    steps: dict[int, str | None]
    highest_drawing: int
    replaced: dict[int, str]
    aliases: dict[str, str]


def load_state(state, roots: list[Element]) -> LoadedState:
    """Read ``state`` (:func:`export_state`'s string; its bytes too) for the document whose
    parts are ``roots``: an :class:`IdStateError` when it is not a state or not these ids'."""
    import base64
    import binascii
    import json
    import zlib

    if isinstance(state, (bytes, bytearray)):
        try:
            state = bytes(state).decode("ascii")
        except UnicodeDecodeError:
            state = None
    if not isinstance(state, str) or not state.startswith("docx-ids."):
        shown = type(state).__name__ if not isinstance(state, str) else repr(state[:24])
        raise IdStateError(f"ids= takes the string Document.id_state() gave (it starts {STATE_PREFIX!r}), "
                           f"not {shown}. {_WAY_OUT}")
    if not state.startswith(STATE_PREFIX):
        raise IdStateError(f"this id state is of another form ({state.split('.', 2)[1]!r}) than this "
                           f"docx-agent reads (1): a later docx-agent wrote it -- upgrade docx-agent. {_WAY_OUT}")
    body = state[len(STATE_PREFIX):]
    try:
        record = json.loads(zlib.decompress(base64.urlsafe_b64decode(body + "=" * (-len(body) % 4))))
        version, last = int(record["version"]), int(record["last"])
        seeds = [(str(seed), int(k)) for seed, k in record["seeds"]]
        digest = str(record["digest"])
    except (binascii.Error, zlib.error, ValueError, KeyError, TypeError) as error:
        raise _damaged(error) from None
    used = used_long_hex(roots)
    drawings = drawing_numbers(roots)
    found = footprint(used, drawings)
    if found != digest:
        raise IdStateError(f"this id state is of other bytes: it was saved with a document whose ids' "
                           f"digest is {digest}, and these bytes' is {found} -- the state of another "
                           f"version of this document, or of another document. {_WAY_OUT}")
    try:
        pool = IdPool(used)
        members: list[list[int]] = []
        for seed, k in seeds:
            values = _sequence(seed, k)[:k]
            members.append(values)
            pool.update(values)
            pool.cursors[seed] = k
        pool.update(_sums(record.get("extra", ())))
        gone: dict[int, int] = {}
        for number, deltas, steps in record.get("gone", ()):
            for index, step in zip(_sums(deltas), _sums(steps), strict=True):
                gone[members[number][index] if number >= 0 else index] = step
        values, steps = record.get("drawings", ((), ()))
        drawings_gone = dict(zip(_sums(values), _sums(steps), strict=True))
        words = [str(word) for word in record.get("who", ())]
        versions, said = record.get("by", ((), ()))
        known = {step: words[word] for step, word in zip(_sums(versions), said, strict=True)}
        earlier = {step: known.get(step) for step in {*gone.values(), *drawings_gone.values()}}
        pool.update(gone)
        return LoadedState(
            version=version, last=last, pool=pool, gone=gone, drawings_gone=drawings_gone,
            steps=earlier,
            highest_drawing=max(int(record.get("highest", 0)), max(drawings, default=0)),
            replaced={int(value, 16): str(new) for value, new in record.get("replaced", {}).items()},
            aliases={str(old): str(new) for old, new in record.get("aliases", {}).items()})
    except (ValueError, KeyError, TypeError, IndexError, AttributeError) as error:
        raise _damaged(error) from None


def _damaged(error: BaseException) -> IdStateError:
    return IdStateError(f"this id state is damaged ({type(error).__name__}): it is not the whole string "
                        f"Document.id_state() gave. {_WAY_OUT}")
