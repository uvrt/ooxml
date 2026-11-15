"""Validation per edit: :func:`docx_agent.validate.check`'s problems, found again only where the
document changed.

:func:`docx_agent.validate.check` reads every part whole -- every element of a 3,000-page body,
several times -- so an edit's checks cost the document's size, not the edit's.  This module
finds the same problems (the same sorted list, problem for problem) from **facts** kept per
piece of the document:

* the body (``word/document.xml``) per top-level block -- each paragraph, table, content
  control block -- by the bytes of that block, plus what is in no block (the root, the body
  element and its other children: the last section's properties);
* every other part whole, by its bytes (a part as it was read: by the stored entry).

A block's facts are what each rule needs from it: the problems found in it alone, and its
contribution to every rule that counts across the part or the package (paraIds, bookmarks,
comment marks, note references, ``r:`` ids, list ids, styles named, drawing ids, revision
ids, moves, the complex fields' depth).  Facts are kept by the block's bytes (and the root's
namespaces), so a block an edit did not touch is never read again, wherever it moved; a
version's facts for the whole body are the previous version's, less the blocks that went,
plus those that came.  The bytes of the body are compared with the previous version's from
both ends, so only the stretch between the first and last byte that differ is read again.

Per rule (:func:`validate.check`'s), how it is found:

==================================  ============================================================
rule (codes)                        computed
==================================  ============================================================
child-order, text-in-deletion,      changed blocks: found in each changed block alone; kept per
deleted-text-outside-deletion,      block; the body's own children's order and the root's
hyperlink-in-revision,              from the body element (no descendant read)
cell-not-ending-in-paragraph,
grid-*, vmerge-*, textid-*,
paraid-out-of-range,
numbering-level-out-of-range,
revision-without-author
paraid-repeated, bookmark-*,        incremental: per-block counts, the part's sums kept from
comment-range-unpaired,             version to version (blocks gone subtracted, new added)
comment-reference-without-comment,
*-reference-without-note,
relationship-id-unknown,
numbering-num-missing,
style-missing (blocks' names),
docpr-id-repeated,
revision-id-repeated
field-unbalanced, move-range-       incremental, in order: per-block depth change and lowest
unpaired, move-unpaired             depth (fields), per-block move marks, joined in order
not-well-formed                     changed blocks (each parsed in the root's context), the
                                    rest of a changed part whole; a part as read: once
every other part's rules, the       whole part, again only when its bytes changed
package's (content types,
relationships, orphans),
numbering and style definitions,
comment parts, body-section-*,
ignorable-prefix-undeclared
==================================  ============================================================

Anything else -- a body not written as ``w:`` blocks, a block count that does not match the
live tree, a part that is not well formed, ``target`` given -- falls back to the whole
:func:`validate.check` for that call (counted in :data:`STATS`).

**The oracle.**  ``DOCX_AGENT_FULL_VALIDATE=1`` in the environment runs the whole check too, at
every call, and raises :class:`Mismatch` when the two lists differ (docx-agent's own tests set
it): the whole check stays the definition; this is its cost proportional to the change.

Where it runs: :func:`problems` is the Word format's ``problems`` (``check``'s and the save
gate's ``validation``); :func:`committed` finds a version's problems on the document itself
as the version is committed (:func:`docx_agent.layout.read_copy`, under the document's lock),
so the checks after an edit, which run on the version's read copy, read them without reading
the copy's parts.  :meth:`Document.validate` stays the whole check.
"""

from __future__ import annotations

import hashlib
import json
import os
import threading
from collections import Counter, OrderedDict
from typing import Any

from lxml import etree

from . import validate as _v
from .edit.ids import MC_FALLBACK, PARA_ID, TEXT_ID, valid_long_hex
from .oxml.package import CONTENT_TYPES_PART
from .oxml.xml import CHILD_ORDER, prefixed_name

#: The environment variable that runs the whole check beside this one and compares (``1``).
ENV_FULL = "DOCX_AGENT_FULL_VALIDATE"
#: The environment variable that turns the edit path per change off (``0``): the whole check
#: at every call, as docx-agent 0.1.114 found it.
ENV = "DOCX_AGENT_INCREMENTAL"
#: The environment variable that names a file each process adds its :data:`STATS` to.
ENV_STATS = "DOCX_AGENT_VALIDATE_STATS"

#: What each call did, counted: ``incremental``, ``served`` (a version found at its commit),
#: ``fallback: <why>`` (tests and measurements read it; never in results).
STATS: Counter = Counter()

_W = _v._W
_R = _v._R
_WP = _v._WP
_BLOCKS = frozenset({_W + "p", _W + "tbl", _W + "sdt", _W + "customXml"})
_RECORDS = _v._RECORDS
_MOVE_TAGS = {_W + "moveFromRangeStart": ("from", True), _W + "moveFromRangeEnd": ("from", False),
              _W + "moveToRangeStart": ("to", True), _W + "moveToRangeEnd": ("to", False)}
_RPR_TRPR = (_W + "rPr", _W + "trPr")
_CELL_BLOCKS = (_W + "p", _W + "tbl", _W + "sdt", _W + "customXml")

# Ancestor flags (what the elements above an element are).
_DEL_OWN = 1        # a w:del that holds runs
_DEL_ANY = 2        # any w:del
_REV_OWN = 4        # a w:ins, w:del, w:moveFrom or w:moveTo that holds runs
_FLD_SKIP = 8       # a w:del or w:moveFrom
_HIDDEN = 16        # an mc:Fallback

#: The counted facts, by key: ``lw`` the problems a word part's rules find in a piece alone
#: (code, detail); ``ls`` those of a story part's rules (code, detail); ``lv`` numbering
#: levels out of range (detail); ``pid`` paraIds (upper case); ``bs``/``be``/``bn`` bookmark
#: starts', ends' ids and names; ``cs``/``ce``/``cr`` comment range starts, ends, references;
#: ``fr``/``er`` note references; ``rv`` ``r:`` attribute values; ``nm`` list ids named;
#: ``st`` styles named ``(tag, id)``; ``dp`` drawing ids; ``ri`` revision ids; ``dl`` the
#: deletion records (``w:del``, ``w:cellDel``, any), for what the body holds (:func:`holds`).
_COUNTED = ("lw", "ls", "lv", "pid", "bs", "be", "bn", "cs", "ce", "cr", "fr", "er", "rv", "nm", "st", "dp", "ri", "dl")

_PNAME: dict[str, str] = {}
_RANKS: dict[str, dict[str, int]] = {}


def _pname(element: Any) -> str:
    tag = element.tag
    found = _PNAME.get(tag)
    if found is None:
        found = _PNAME[tag] = prefixed_name(element)
    return found


def _ranks(parent: str) -> dict[str, int]:
    found = _RANKS.get(parent)
    if found is None:
        found = _RANKS[parent] = _v._ranks(CHILD_ORDER[parent])
    return found


class Mismatch(AssertionError):
    """``DOCX_AGENT_FULL_VALIDATE``: the problems found per change differ from the whole check's."""


class _Fallback(Exception):
    """This call runs the whole check (why: the message)."""


# -- facts of a piece of a part ---------------------------------------------------------------------


class Facts:
    """What the rules need from one piece of a part (a block, the rest of the body, a whole
    part): counted facts (:data:`_COUNTED`, each a ``Counter``, absent when empty), the
    complex fields' depth change and lowest depth (``fld``) and the move marks in order
    (``mv``)."""

    __slots__ = ("c", "fld", "mv")

    def __init__(self) -> None:
        self.c: dict[str, Counter] = {}
        self.fld: tuple[int, int] | None = None
        self.mv: list[tuple[str, bool, str, str]] | None = None

    def add(self, key: str, value: Any, n: int = 1) -> None:
        counter = self.c.get(key)
        if counter is None:
            counter = self.c[key] = Counter()
        counter[value] += n

    @property
    def empty(self) -> bool:
        return not self.c and self.fld is None and self.mv is None


def _walk(top: Any, facts: Facts, *, root: bool = False, alone: bool = False,
          children: list | None = None) -> None:
    """``top``'s facts into ``facts``: ``top`` and its descendants (``alone``: ``top`` only --
    its children's order read, none of them visited; ``children`` its children when the
    caller has them), in document order, as :mod:`docx_agent.validate`'s rules read them.
    ``root``: ``top`` is the part's root (which the rules that skip ``mc:Fallback`` content
    never count)."""
    depth, low = 0, 0
    fields = False
    stack = [(top, 0, root)]
    while stack:
        element, flags, is_root = stack.pop()
        tag = element.tag
        if not isinstance(tag, str):
            continue
        for attribute, value in element.attrib.items():
            if value and attribute.startswith(_R):
                facts.add("rv", value)
        kids = children if (element is top and children is not None) else list(element)
        name = _pname(element)
        if name in CHILD_ORDER:
            ranks = _ranks(name)
            unknown = len(ranks) + 1
            last = -1
            for child in kids:
                if not isinstance(child.tag, str):
                    continue
                rank = ranks.get(_pname(child), unknown)
                if rank < last:
                    facts.add("lw", ("child-order", f"{_pname(child)} in {name}"))
                    break
                last = rank
        live = not is_root and not flags & _HIDDEN
        if tag.startswith(_W):
            local = tag[len(_W):]
            if local in ("p", "tr"):
                if live:
                    raw = element.get(PARA_ID)
                    text = element.get(TEXT_ID)
                    if text is not None and raw is None:
                        facts.add("lw", ("textid-without-paraid", text))
                    if text is not None and not valid_long_hex(text):
                        facts.add("lw", ("textid-out-of-range", text))
                    if raw is not None:
                        if not valid_long_hex(raw):
                            facts.add("lw", ("paraid-out-of-range", raw))
                        facts.add("pid", raw.upper())
            elif local == "t":
                if flags & _DEL_OWN:
                    facts.add("lw", ("text-in-deletion", (element.text or "")[:30]))
            elif local == "delText":
                if not flags & _DEL_ANY:
                    facts.add("lw", ("deleted-text-outside-deletion", (element.text or "")[:30]))
            elif local in ("hyperlink", "fldSimple"):
                if flags & _REV_OWN:
                    facts.add("lw", ("hyperlink-in-revision", local))
            elif local == "fldChar":
                if not flags & _FLD_SKIP:
                    kind = element.get(_W + "fldCharType")
                    if kind == "begin":
                        depth += 1
                        fields = True
                    elif kind == "end":
                        depth -= 1
                        fields = True
                        if depth < low:
                            low = depth
            elif local == "tc":
                blocks = [child for child in kids if child.tag in _CELL_BLOCKS]
                if not blocks or blocks[-1].tag == _W + "tbl":
                    facts.add("lw", ("cell-not-ending-in-paragraph", ""))
            elif local == "tbl":
                if live:
                    for code, detail in _v.grid_problems(element):
                        facts.add("lw", (code, detail))
            elif local == "bookmarkStart":
                facts.add("bs", element.get(_W + "id") or "")
                facts.add("bn", element.get(_W + "name") or "")
            elif local == "bookmarkEnd":
                facts.add("be", element.get(_W + "id") or "")
            elif local == "commentRangeStart":
                facts.add("cs", element.get(_W + "id"))
            elif local == "commentRangeEnd":
                facts.add("ce", element.get(_W + "id"))
            elif local == "commentReference":
                facts.add("cr", element.get(_W + "id"))
            elif local == "footnoteReference":
                facts.add("fr", element.get(_W + "id"))
            elif local == "endnoteReference":
                facts.add("er", element.get(_W + "id"))
            elif local == "numPr":
                num = element.find(_W + "numId")
                level = element.find(_W + "ilvl")
                if num is not None and num.get(_W + "val") not in ("0", None):
                    facts.add("nm", num.get(_W + "val"))
                if level is not None:
                    try:
                        ok = 0 <= int(level.get(_W + "val")) <= 8
                    except (TypeError, ValueError):
                        ok = False
                    if not ok:
                        facts.add("lv", str(level.get(_W + "val")))
            elif local in ("pStyle", "rStyle", "tblStyle"):
                facts.add("st", (local, element.get(_W + "val")))
            if local in ("del", "cellDel"):
                facts.add("dl", local)
            if tag in _RECORDS:
                if live:
                    if element.get(_W + "author") is None:
                        facts.add("ls", ("revision-without-author", local))
                    if element.get(_W + "id") is not None:
                        facts.add("ri", element.get(_W + "id"))
            elif tag in _MOVE_TAGS:
                if live:
                    side, start = _MOVE_TAGS[tag]
                    if facts.mv is None:
                        facts.mv = []
                    facts.mv.append((side, start, element.get(_W + "id") or "",
                                     (element.get(_W + "name") or "") if start else ""))
        elif tag == _WP + "docPr":
            if live:
                facts.add("dp", element.get("id") or "")
        if alone and element is top:
            continue
        # The children's flags: what this element is to them.
        child_flags = flags
        if tag == _W + "del":
            child_flags |= _DEL_ANY | _FLD_SKIP
            if _owns_runs(element):
                child_flags |= _DEL_OWN | _REV_OWN
        elif tag in (_W + "ins", _W + "moveTo"):
            if _owns_runs(element):
                child_flags |= _REV_OWN
        elif tag == _W + "moveFrom":
            child_flags |= _FLD_SKIP
            if _owns_runs(element):
                child_flags |= _REV_OWN
        elif tag == MC_FALLBACK:
            child_flags |= _HIDDEN
        for child in reversed(kids):
            stack.append((child, child_flags, False))
    if fields:
        facts.fld = (depth, low)


def _owns_runs(revision: Any) -> bool:
    parent = revision.getparent()
    return parent is None or parent.tag not in _RPR_TRPR


def _facts_of(element: Any, **how: Any) -> Facts:
    facts = Facts()
    _walk(element, facts, **how)
    return _EMPTY if facts.empty else facts


_EMPTY = Facts()


def _sum(into: dict[str, Counter], facts: Facts, sign: int = 1) -> None:
    for key, counter in facts.c.items():
        total = into.get(key)
        if total is None:
            total = into[key] = Counter()
        if sign > 0:
            total.update(counter)
        else:
            total.subtract(counter)


# -- one document's state ----------------------------------------------------------------------------


class _Body:
    """The body as last read: its bytes, the blocks' spans and keys, their facts, the facts
    of their sum."""

    __slots__ = ("data", "spans", "body", "head", "keys", "facts", "total", "pending", "token")

    def __init__(self, data: bytes, spans: list, body: tuple[int, int], head: bytes, keys: list,
                 facts: list, total: dict[str, Counter]) -> None:
        self.data, self.spans, self.body, self.head = data, spans, body, head
        self.keys, self.facts, self.total = keys, facts, total
        #: The places of new blocks whose facts the sum still lacks (:func:`_fill`).
        self.pending: list[int] | None = None
        #: The main part's stored entry when the bytes are those it was read as.
        self.token: Any = None


class State:
    """What a document keeps for its next validation: the body as last read, and each other
    part's facts by what its bytes were."""

    def __init__(self, cache: "dict | None" = None) -> None:
        self.body: _Body | None = None
        #: Block facts by (root namespaces, block bytes' digest): shared by a document's
        #: copies (the read copy of a version, a draft), whose blocks are the document's.
        self.blocks: dict = cache if cache is not None else {}
        #: Other parts: name -> (token, roots' facts, parsed extras).
        self.parts: dict[str, tuple] = {}
        self.body_problems: list = []
        #: What the body held at the last validation: ``{"deletions": bool}``.
        self.holds: dict[str, bool] = {}
        self.lock = threading.Lock()


def _state(document: Any) -> State:
    found = document.__dict__.get("_revalidate")
    if found is None:
        live = document.__dict__.get("_live")
        shared = live.__dict__.get("_revalidate") if live is not None else None
        found = document.__dict__["_revalidate"] = State(shared.blocks if shared is not None else None)
    return found


def _digest(data: bytes) -> bytes:
    return hashlib.blake2b(data, digest_size=16).digest()


def _same_token(a: Any, b: Any) -> bool:
    return a[0] == b[0] and (a[1] is b[1] if a[0] == "stored" else a[1] == b[1])


def _well_formed(data: bytes) -> bool:
    try:
        etree.fromstring(data, etree.XMLParser(resolve_entities=False))
    except etree.XMLSyntaxError:
        return False
    return True


# -- the body ------------------------------------------------------------------------------------------


def _scan(data: bytes) -> tuple[list, tuple[int, int], bytes] | None:
    from .tools.touched import _W_DECLARED, _blocks

    root_end = data.find(b">", data.find(b"<w:document"))
    if root_end < 0 or _W_DECLARED not in data[:root_end]:
        return None
    import re

    opening = re.compile(rb"<w:body(?=[\s>])").search(data, root_end)
    body_end = data.rfind(b"</w:body>")
    if opening is None or body_end < 0:
        return None
    body = (data.index(b">", opening.end()) + 1, body_end)
    spans = _blocks(data, *body)
    if spans is None:
        return None
    return spans, body, data[:root_end + 1]


def _body_facts(state: State, package: Any, main: str, root: Any) -> tuple[Facts | None, dict, Any]:
    """The body's facts: the rest's (root, body element, its children not blocks) and the
    blocks' sum; and the body element.  Raises :class:`_Fallback`."""
    from .tools.touched import _blocks, _common_prefix, _common_suffix

    body_element = root.find(_W + "body")
    if body_element is None:
        raise _Fallback("no body")
    old = state.body
    token = None if main in package.dirty_parts else package._parts.stored(main)
    if old is not None and token is not None and old.token is token:
        new = old
    else:
        data = package.read(main) or b""
        new = None
        if old is not None and old.data == data:
            new = old
        elif old is not None:
            prefix = _common_prefix(old.data, data)
            suffix = _common_suffix(old.data, data, min(len(old.data), len(data)) - prefix)
            body_at, body_end = old.body
            if prefix >= body_at and len(old.data) - suffix <= body_end:
                delta = len(data) - len(old.data)
                spans = old.spans
                before = 0
                while before < len(spans) and spans[before][1] <= prefix:
                    before += 1
                after = 0
                while after < len(spans) - before and spans[len(spans) - 1 - after][0] >= len(old.data) - suffix:
                    after += 1
                begin = spans[before - 1][1] if before else body_at
                stop = spans[len(spans) - after][0] + delta if after else body_end + delta
                middle = _blocks(data, begin, stop)
                if middle is not None:
                    new = _splice(state, old, data, before, after, middle, delta, begin, stop)
        if new is None:
            new = _whole(state, data)
        new.token = token
        state.body = new
    blocks = []
    others = []
    for child in body_element:
        (blocks if child.tag in _BLOCKS else others).append(child)
    # validate._body_problems, from the body's children not blocks.
    sections = [child for child in others if child.tag == _W + "sectPr"]
    last = body_element[-1] if len(body_element) else None
    while last is not None and not isinstance(last.tag, str):
        last = last.getprevious()
    if sections and last is not sections[-1]:
        state.body_problems = [_v.Problem("body-section-not-last", main)]
    elif len(sections) > 1:
        state.body_problems = [_v.Problem("body-has-two-sections", main)]
    else:
        state.body_problems = []
    if len(blocks) != len(new.spans):
        state.body = None
        raise _Fallback("the body's blocks and its bytes' differ")
    _fill(state, new, blocks)
    rest = Facts()
    _walk(root, rest, root=True, alone=True)
    for child in root:
        if child is not body_element:
            _walk(child, rest)
    _walk(body_element, rest, alone=True, children=list(body_element))
    for child in others:
        _walk(child, rest)
    if rest.fld is not None:
        raise _Fallback("complex fields outside the blocks")
    return rest, new.total, body_element


def _whole(state: State, data: bytes) -> _Body:
    found = _scan(data)
    if found is None:
        raise _Fallback("the body's blocks are not read from its bytes")
    spans, body, head = found
    if not _well_formed(data):
        raise _Fallback("not well formed")
    hk = _digest(head)
    keys = [(hk, _digest(data[a:b])) for a, b in spans]
    return _Body(data, spans, body, head, keys, [None] * len(keys), {})


def _splice(state: State, old: _Body, data: bytes, before: int, after: int, middle: list, delta: int,
            begin: int, stop: int) -> _Body:
    """The body from ``old``'s: ``before`` blocks before and ``after`` after kept, ``middle``
    (spans in ``data``) read again."""
    n = len(old.spans)
    # What is between the blocks of the stretch read again (and the head and tail around it):
    # well formed.
    gaps = []
    previous = begin
    for a, b in middle:
        gaps.append(data[previous:a])
        previous = b
    gaps.append(data[previous:stop])
    head, tail = data[:old.body[0]], data[old.body[1] + delta:]
    if not _well_formed(head + b"".join(gaps) + tail):
        raise _Fallback("not well formed")
    hk = old.keys[0][0] if old.keys else _digest(old.head)
    new_keys = [(hk, _digest(data[a:b])) for a, b in middle]
    for (a, b), key in zip(middle, new_keys):
        if key not in state.blocks and not _well_formed(head + data[a:b] + tail):
            raise _Fallback("not well formed")
    old_keys = old.keys[before:n - after]
    old_facts = old.facts[before:n - after]
    total = old.total
    # The blocks that went less those that came (by key: equal bytes, equal facts).
    went = Counter(old_keys)
    went.subtract(new_keys)
    by_key = dict(zip(old_keys, old_facts))
    for key, count in went.items():
        if count > 0:
            facts = by_key.get(key)
            if facts is None:
                # A block whose facts were never read (none of its state's were needed): its
                # sum is not kept either -- all made again.
                total = None
                break
            for _ in range(count):
                _sum(total, facts, -1)
    spans = old.spans[:before] + middle + [(s + delta, e + delta) for s, e in old.spans[n - after:]]
    keys = old.keys[:before] + new_keys + old.keys[n - after:]
    facts = old.facts[:before] + [None] * len(middle) + old.facts[n - after:]
    found = _Body(data, spans, (old.body[0], old.body[1] + delta), old.head, keys, facts,
                  total if total is not None else {})
    if total is None:
        found.facts = [None] * len(keys)
        return found
    # The blocks that came less those that went: added once their facts are read (_fill).
    found.total = total
    came = Counter(new_keys)
    came.subtract(old_keys)
    pending = []
    for j, key in enumerate(new_keys):
        if came.get(key, 0) > 0:
            came[key] -= 1
            pending.append(before + j)
        else:
            known = by_key.get(key)
            facts[before + j] = known
    found.facts = facts
    found.pending = pending
    return found


def _fill(state: State, body: _Body, blocks: list) -> None:
    """Every block's facts read (from the live element, by key when known), and the sum
    complete."""
    cache = state.blocks
    if body.pending is not None:
        for j in body.pending:
            facts = cache.get(body.keys[j])
            if facts is None:
                facts = cache[body.keys[j]] = _facts_of(blocks[j])
            body.facts[j] = facts
            _sum(body.total, facts)
        body.pending = None
    if any(facts is None for facts in body.facts):
        total: dict[str, Counter] = {}
        for j, key in enumerate(body.keys):
            facts = body.facts[j]
            if facts is None:
                facts = cache.get(key)
                if facts is None:
                    facts = cache[key] = _facts_of(blocks[j])
                body.facts[j] = facts
            _sum(total, facts)
        body.total = total
    # Keep the cache to about what the body holds (and the last few versions' blocks).
    if len(cache) > 2 * len(body.keys) + 4096:
        keep = dict(zip(body.keys, body.facts))
        cache.clear()
        cache.update(keep)


# -- the problems ---------------------------------------------------------------------------------------


def _part_facts(state: State, package: Any, name: str) -> tuple[Facts, Any] | None:
    """A part other than the body: its facts (whole) and root; ``None`` when not well formed."""
    data = None
    if name in package.dirty_parts:
        data = package.read(name) or b""
        token: Any = ("bytes", _digest(data))
    else:
        token = ("stored", package._parts.stored(name))
    kept = state.parts.get(name)
    if kept is not None and _same_token(kept[0], token):
        return (kept[1], package.tree(name)) if kept[1] is not None else None
    if data is None:
        data = package.read(name)
    if not _well_formed(data):
        state.parts[name] = (token, None)
        return None
    root = package.tree(name)
    facts = Facts()
    _walk(root, facts, root=True)
    state.parts[name] = (token, facts)
    return facts, root


def _incremental(document: Any) -> list:
    package = document.package
    state = _state(document)
    main = package.document_part()
    word_parts: list[str] = []
    roots: dict[str, Any] = {}
    facts_of: dict[str, Facts | dict] = {}
    for name in package.part_names:
        if not name.endswith((".xml", ".rels")):
            continue
        if name == main:
            continue
        found = _part_facts(state, package, name)
        if found is None:
            raise _Fallback("not well formed")
        facts_of[name], roots[name] = found
    main_root = package.tree(main)
    if main_root is None:
        raise _Fallback("no main part")
    rest, total, body = _body_facts(state, package, main, main_root)
    roots[main] = main_root
    main_total: dict[str, Counter] = dict(total)
    for key, counter in rest.c.items():
        main_total[key] = main_total.get(key, Counter()) + counter
    for name in package.part_names:
        if name in roots and name.startswith("word/") and name.endswith(".xml"):
            word_parts.append(name)

    def counted(name: str, key: str) -> Counter:
        if name == main:
            return main_total.get(key) or Counter()
        return facts_of[name].c.get(key) or Counter()

    problems: list = []
    Problem = _v.Problem
    problems += _v._package_problems(package)
    for name in word_parts:
        for (code, detail), n in counted(name, "lw").items():
            problems += [Problem(code, name, detail)] * n
        problems += [Problem("paraid-repeated", name, raw) for raw, n in counted(name, "pid").items() if n > 1]
        problems += _fields(name, state.body, rest) if name == main else _fields(name, None, facts_of[name])
        problems += _v._ignorable_problems(name, roots[name])
    story = [p for p in [main] + package.story_parts() if p in roots]
    problems += _ranges(package, roots, story, counted)
    problems += _references(package, roots, counted)
    problems += _numbering(package, roots, story, counted)
    problems += _styles(package, roots, story, counted)
    docpr: Counter = Counter()
    for part in story:
        docpr.update(counted(part, "dp"))
    problems += [Problem("docpr-id-repeated", main, raw) for raw, n in docpr.items() if n > 0 and n > 1]
    problems += _tracking(main, story, counted, state.body, rest, facts_of)
    problems += _comment_parts(package, roots, story, counted)
    problems += state.body_problems
    state.holds = {"deletions": any(n > 0 for n in counted(main, "dl").values())}
    return sorted(problems)


def _fields(name: str, body: _Body | None, facts: Facts) -> list:
    pieces = (list(body.facts) + [facts]) if body is not None else [facts]
    depth = 0
    for piece in pieces:
        if piece.fld is None:
            continue
        change, low = piece.fld
        if depth + low < 0:
            return [_v.Problem("field-unbalanced", name, "end without begin")]
        depth += change
    return [_v.Problem("field-unbalanced", name, "begin without end")] if depth else []


def _ranges(package: Any, roots: dict, story: list, counted: Any) -> list:
    Problem = _v.Problem
    out: list = []
    starts: Counter = Counter()
    ends: Counter = Counter()
    names: Counter = Counter()
    comment_starts, comment_ends, comment_refs = set(), set(), set()
    note_refs: dict[str, set] = {"footnote": set(), "endnote": set()}
    for part in story:
        starts.update(counted(part, "bs"))
        ends.update(counted(part, "be"))
        names.update(counted(part, "bn"))
        comment_starts |= {k for k, n in counted(part, "cs").items() if n > 0}
        comment_ends |= {k for k, n in counted(part, "ce").items() if n > 0}
        comment_refs |= {k for k, n in counted(part, "cr").items() if n > 0}
        note_refs["footnote"] |= {k for k, n in counted(part, "fr").items() if n > 0}
        note_refs["endnote"] |= {k for k, n in counted(part, "er").items() if n > 0}
    starts, ends, names = +starts, +ends, +names
    main = package.document_part()
    for mark in sorted(set(starts) | set(ends)):
        if starts[mark] != ends[mark]:
            out.append(Problem("bookmark-unpaired", main, mark))
    folded: Counter = Counter()
    for name, count in names.items():
        folded[name.casefold()] += count
        if not _v._BOOKMARK_NAME.fullmatch(name):
            out.append(Problem("bookmark-name-invalid", main, name))
    for name, count in folded.items():
        if count > 1:
            out.append(Problem("bookmark-name-repeated", main, name))
    for mark in sorted(comment_starts ^ comment_ends):
        out.append(Problem("comment-range-unpaired", main, str(mark)))
    comments = set()
    for part in package.story_parts():
        root = roots.get(part)
        if root is not None and root.tag == _W + "comments":
            comments |= {n.get(_W + "id") for n in root.iter(_W + "comment")}
    for mark in sorted(comment_refs - comments):
        out.append(Problem("comment-reference-without-comment", main, str(mark)))
    for kind in ("footnote", "endnote"):
        notes = set()
        for part in package.story_parts():
            root = roots.get(part)
            if root is not None and root.tag == _W + f"{kind}s":
                notes |= {n.get(_W + "id") for n in root.iter(_W + kind)}
        for mark in sorted(note_refs[kind] - notes):
            out.append(Problem(f"{kind}-reference-without-note", main, str(mark)))
    return out


def _references(package: Any, roots: dict, counted: Any) -> list:
    Problem = _v.Problem
    out: list = []
    for name in roots:
        if not name.endswith(".xml") or name.endswith(".rels") or name == CONTENT_TYPES_PART:
            continue
        values = counted(name, "rv")
        if not values:
            continue
        relationships = package.relationships(name)
        for value, n in values.items():
            if n > 0 and value not in relationships:
                out += [Problem("relationship-id-unknown", name, value)] * n
    return out


def _numbering(package: Any, roots: dict, story: list, counted: Any) -> list:
    Problem = _v.Problem
    numbering_part = package.numbering_part()
    numbering = roots.get(numbering_part) if numbering_part else None
    nums: dict = {}
    abstracts: set = set()
    if numbering is not None:
        abstracts = {n.get(_W + "abstractNumId") for n in numbering.findall(_W + "abstractNum")}
        for num in numbering.findall(_W + "num"):
            ref = num.find(_W + "abstractNumId")
            nums[num.get(_W + "numId")] = ref.get(_W + "val") if ref is not None else None
    out: list = []
    styles_part = package.styles_part()
    owners = list(story) + ([styles_part] if styles_part in roots else [])
    for part in owners:
        for value, n in counted(part, "nm").items():
            if n > 0 and value not in nums:
                out += [Problem("numbering-num-missing", part, value)] * n
        for value, n in counted(part, "lv").items():
            if n > 0:
                out += [Problem("numbering-level-out-of-range", part, value)] * n
    for num_id, abstract in nums.items():
        if abstract not in abstracts:
            out.append(Problem("numbering-abstract-missing", numbering_part, f"{num_id} -> {abstract}"))
    if numbering is not None:
        nsids = Counter((n.get(_W + "val") or "").upper() for n in numbering.iter(_W + "nsid"))
        out += [Problem("numbering-nsid-repeated", numbering_part, nsid) for nsid, n in nsids.items() if n > 1]
    return out


def _styles(package: Any, roots: dict, story: list, counted: Any) -> list:
    Problem = _v.Problem
    styles_part = package.styles_part()
    styles = roots.get(styles_part) if styles_part else None
    if styles is None:
        return []
    by_id = {s.get(_W + "styleId"): s for s in styles.findall(_W + "style")}
    out: list = []
    for part in story:
        for (tag, value), n in counted(part, "st").items():
            if n > 0 and value not in by_id:
                out += [Problem("style-missing", part, f"{tag} {value}")] * n
    for style_id, node in by_id.items():
        for tag in ("basedOn", "next", "link"):
            ref = node.find(_W + tag)
            if ref is not None and ref.get(_W + "val") not in by_id:
                out.append(Problem("style-missing", styles_part, f"{style_id} {tag} {ref.get(_W + 'val')}"))
        seen = set()
        current = style_id
        while current in by_id and current not in seen:
            seen.add(current)
            ref = by_id[current].find(_W + "basedOn")
            current = ref.get(_W + "val") if ref is not None else None
        if current in seen:
            out.append(Problem("style-based-on-cycle", styles_part, style_id))
    return out


def _tracking(main: str, story: list, counted: Any, body: _Body | None, rest: Facts, facts_of: dict) -> list:
    Problem = _v.Problem
    out: list = []
    ids: Counter = Counter()
    for part in story:
        for (code, detail), n in counted(part, "ls").items():
            if n > 0:
                out += [Problem(code, part, detail)] * n
        ids.update(counted(part, "ri"))
        pieces = (list(body.facts) + [rest]) if part == main else [facts_of[part]]
        starts: dict[str, dict[str, str]] = {"from": {}, "to": {}}
        ends: dict[str, set[str]] = {"from": set(), "to": set()}
        moved = False
        for piece in pieces:
            if piece.mv is None:
                continue
            moved = True
            for side, start, mark, name in piece.mv:
                if start:
                    if mark in starts[side] and starts[side][mark] != name and part == main:
                        # Which name is kept is the later's in document order, which the
                        # body's pieces apart (blocks, then the rest) do not say.
                        raise _Fallback("a move range id started twice")
                    starts[side][mark] = name
                else:
                    ends[side].add(mark)
        if not moved:
            continue
        for side in ("from", "to"):
            for mark in sorted(set(starts[side]) ^ ends[side]):
                out.append(Problem("move-range-unpaired", part, f"{side} {mark}"))
        names_from, names_to = set(starts["from"].values()), set(starts["to"].values())
        for name in sorted(names_from ^ names_to):
            out.append(Problem("move-unpaired", part, name))
    out += [Problem("revision-id-repeated", main, raw) for raw, n in ids.items() if n > 1]
    return out


def _comment_parts(package: Any, roots: dict, story: list, counted: Any) -> list:
    """:func:`validate._comment_part_problems`, the references counted."""
    Problem = _v.Problem
    out: list = []
    main = package.document_part()

    def related(kind: str) -> Any:
        found = package.related_parts_of_type(main, kind)
        return roots.get(found[0]) if found else None

    comments = related("http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments")
    if comments is None:
        return out
    references: set = set()
    for part in story:
        references |= {k for k, n in counted(part, "cr").items() if n > 0}
    last_ids = []
    for comment in comments.findall(_W + "comment"):
        if comment.get(_W + "id") not in references:
            out.append(Problem("comment-without-reference", main, comment.get(_W + "id") or ""))
        paragraphs = comment.findall(".//" + _W + "p")
        last_ids.append((paragraphs[-1].get(PARA_ID) or "").upper() if paragraphs else "")
    extended = related("http://schemas.microsoft.com/office/2011/relationships/commentsExtended")
    if extended is not None:
        entries = {(n.get(_v._W15 + "paraId") or "").upper(): n for n in extended.findall(_v._W15 + "commentEx")}
        for para in last_ids:
            if para and para not in entries:
                out.append(Problem("comment-without-extended-entry", main, para))
        for node in entries.values():
            parent = (node.get(_v._W15 + "paraIdParent") or "").upper()
            if parent and parent not in entries:
                out.append(Problem("comment-parent-unknown", main, parent))
    durable_ids: Counter = Counter()
    ids_root = related("http://schemas.microsoft.com/office/2016/09/relationships/commentsIds")
    if ids_root is not None:
        known = {(n.get(_v._W16CID + "paraId") or "").upper() for n in ids_root.findall(_v._W16CID + "commentId")}
        for node in ids_root.findall(_v._W16CID + "commentId"):
            durable_ids[(node.get(_v._W16CID + "durableId") or "").upper()] += 1
        for para in last_ids:
            if para and para not in known:
                out.append(Problem("comment-without-durable-id", main, para))
    out += [Problem("comment-durable-id-repeated", main, raw) for raw, n in durable_ids.items() if n > 1]
    extensible = related("http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible")
    if extensible is not None and ids_root is not None:
        for node in extensible.findall(_v._W16CEX + "commentExtensible"):
            raw = (node.get(_v._W16CEX + "durableId") or "").upper()
            if raw not in durable_ids:
                out.append(Problem("comment-extensible-unknown", main, raw))
    return out


# -- entry points -------------------------------------------------------------------------------------


def full_check_on() -> bool:
    return os.environ.get(ENV_FULL, "").strip() not in ("", "0")


def enabled() -> bool:
    """Whether the edit path per change is on (``DOCX_AGENT_INCREMENTAL``, on unless ``0``)."""
    return os.environ.get(ENV, "1").strip() != "0"


def find(document: Any) -> list:
    """``validate.check(document.package)``, found per change (see the module doc): the
    whole check when the per-change path cannot say (counted); with :data:`ENV_FULL`, both,
    compared."""
    return _find(document)[0]


def _find(document: Any) -> tuple[list, dict[str, bool] | None]:
    """:func:`find`, and what the body holds (``None`` after the whole check)."""
    state = _state(document)
    with state.lock:
        try:
            found = _incremental(document)
            STATS["incremental"] += 1
            held = dict(state.holds)
        except _Fallback as why:
            STATS[f"fallback: {why}"] += 1
            state.body = None
            return _v.check(document.package), None
    if full_check_on():
        whole = _v.check(document.package)
        if whole != found:
            raise Mismatch(_mismatch(whole, found))
    return found, held


def _mismatch(whole: list, found: list) -> str:
    a, b = Counter(whole), Counter(found)
    return (f"validation per change differs from the whole check: only in the whole "
            f"{sorted((a - b).elements())[:10]}, only per change {sorted((b - a).elements())[:10]}")


#: Versions' problems kept per document (found at commit), the latest few.
_KEPT = 16


def committed(document: Any) -> None:
    """A version committed (under the document's lock): its problems found now, on the document
    itself, and kept by version for its read copy's checks (:func:`problems`)."""
    if document.__dict__.get("_live") is not None or not enabled():
        return
    version = document.history.version
    kept: OrderedDict = document.__dict__.setdefault("_problems_at", OrderedDict())
    if version in kept:
        return
    kept[version] = _find(document)
    while len(kept) > _KEPT:
        kept.popitem(last=False)


def body_size(document: Any) -> int | None:
    """The body part's size in bytes as last read here (its validation at the last commit),
    else as stored when it is as read; ``None`` when neither says without writing it out."""
    state = document.__dict__.get("_revalidate")
    if state is not None and state.body is not None:
        return len(state.body.data)
    package = document.package
    main = package.document_part()
    if main in package.dirty_parts:
        return None
    return package.part_size(main)


def holds(document: Any, what: str) -> bool | None:
    """Whether the body of a read copy's version held ``what`` (``"deletions"``: any
    ``w:del`` or ``w:cellDel``) when it was committed, as its validation found it; ``None``
    when that is not known (then read the document)."""
    if not enabled():
        return None
    live = document.__dict__.get("_live")
    frozen = document.__dict__.get("_frozen")
    if live is None or frozen is None or frozen[0] != document.history.version or frozen[1] != document._state:
        return None
    kept = live.__dict__.get("_problems_at")
    found = kept.get(document.history.version) if kept is not None else None
    if found is None or found[1] is None:
        return None
    return found[1].get(what)


def problems(document: Any) -> list:
    """The Word format's ``problems``: a read copy's version's as found at its commit (on the
    document), else found per change on ``document`` itself; ``DOCX_AGENT_INCREMENTAL=0``: the
    whole check."""
    if not enabled():
        return _v.check(document.package)
    live = document.__dict__.get("_live")
    frozen = document.__dict__.get("_frozen")
    if live is not None and frozen is not None and frozen[0] == document.history.version \
            and frozen[1] == document._state:
        kept = live.__dict__.get("_problems_at")
        found = kept.get(document.history.version) if kept is not None else None
        if found is not None:
            found = found[0]
            STATS["served"] += 1
            if full_check_on():
                whole = _v.check(document.package)
                if whole != found:
                    raise Mismatch(_mismatch(whole, found))
            return list(found)
    return find(document)


def _stats_at_exit() -> None:
    path = os.environ.get(ENV_STATS)
    if path and STATS:
        try:
            with open(path, "a", encoding="utf-8") as out:
                out.write(json.dumps(dict(STATS)) + "\n")
        except OSError:
            pass


if os.environ.get(ENV_STATS):
    import atexit

    atexit.register(_stats_at_exit)
