"""Lane K: ``ooxml_core.BodySpans`` (a Word body's top-level blocks read from its part's
bytes, and the next version's read from them), ``write_plan``/``write_assemble`` (a part
written from its last bytes), ``index_walks`` and ``body_others`` -- each against a plain
Python reading of the same bytes or tree, on made-up documents changed at random."""
from __future__ import annotations

import random
import re

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
etree = pytest.importorskip("lxml.etree")

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W14 = "http://schemas.microsoft.com/office/word/2010/wordml"
MC = "http://schemas.openxmlformats.org/markup-compatibility/2006"
_TAG = re.compile(rb"<(/?)w:(p|tbl|sdt|customXml|body)(?=[\s/>])")


def _blocks(data: bytes, begin: int, stop: int):
    """The Python reading the Rust one follows (docx-agent's ``tools/touched._blocks``)."""
    depth, start, spans = 0, begin, []
    for match in _TAG.finditer(data, begin, stop):
        if match.group(2) == b"body":
            return None
        if match.group(1):
            depth -= 1
            if depth < 0:
                return None
            if depth == 0:
                spans.append((start, data.index(b">", match.end()) + 1))
            continue
        close = data.index(b">", match.end())
        if data[close - 1] == 0x2F:
            if depth == 0:
                spans.append((match.start(), close + 1))
            continue
        if depth == 0:
            start = match.start()
        depth += 1
    return spans if depth == 0 else None


def _scan(data: bytes):
    root_end = data.find(b">", data.find(b"<w:document"))
    opening = re.compile(rb"<w:body(?=[\s>])").search(data, root_end)
    body_end = data.rfind(b"</w:body>")
    body = (data.index(b">", opening.end()) + 1, body_end)
    return _blocks(data, *body), body


def _block(rng: random.Random, k: int) -> str:
    kind = rng.randrange(6)
    if kind == 0:
        return f'<w:p w14:paraId="{k + 1:08X}"/>'
    if kind == 1:
        return (f'<w:tbl><w:tr w14:paraId="{k + 1:08X}"><w:tc><w:p w14:paraId="{k + 70000:08X}">'
                f'<w:r><w:t>c{k}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>')
    if kind == 2:
        return f'<w:sdt><w:sdtContent><w:p w14:paraId="{k + 1:08X}"/></w:sdtContent></w:sdt>'
    if kind == 3:
        return (f'<w:p w14:paraId="{k + 1:08X}"><w:r><mc:AlternateContent><mc:Choice Requires="w14">'
                f'<w:t>x</w:t></mc:Choice><mc:Fallback><w:p w14:paraId="{k + 90000:08X}"/></mc:Fallback>'
                f'</mc:AlternateContent></w:r></w:p>')
    words = " ".join(rng.choice(["a", "b&amp;c", "&lt;w:p&gt;", "ü"]) for _ in range(rng.randrange(4)))
    return f'<w:p w14:paraId="{k + 1:08X}"><w:pPr><w:pStyle w:val="S{k % 3}"/></w:pPr><w:r><w:t>{words}</w:t></w:r></w:p>'


def _document(blocks: list[str]) -> bytes:
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w:document xmlns:w="{W}" '
            f'xmlns:w14="{W14}" xmlns:mc="{MC}"><w:body>' + "".join(blocks)
            + '<w:sectPr/></w:body></w:document>').encode()


@pytest.mark.parametrize("seed", range(6))
def test_the_blocks_and_each_step_are_the_python_readings(seed):
    rng = random.Random(seed)
    blocks = [_block(rng, k) for k in range(60)]
    data = _document(blocks)
    reading = ooxml_core.BodySpans.scan(data)
    spans, body = _scan(data)
    assert reading[:] == spans and reading.body == body and len(reading) == len(spans)
    assert reading.counts(0, len(reading)) == [
        (len(re.findall(rb"<w:p(?=[\s/>])", re.sub(rb"<mc:Fallback(?=[\s>]).*?</mc:Fallback>", b"", data[a:b], flags=re.DOTALL))),
         len(re.findall(rb"<w:tbl(?=[\s/>])", data[a:b])), len(re.findall(rb"<w:tr(?=[\s/>])", data[a:b])))
        for a, b in spans]
    for k in range(40):
        at = rng.randrange(len(blocks) + 1)
        change = rng.randrange(3)
        if change == 0 and blocks:
            del blocks[min(at, len(blocks) - 1)]
        elif change == 1:
            blocks.insert(at, _block(rng, 1000 + k))
        elif blocks:
            blocks[min(at, len(blocks) - 1)] = _block(rng, 2000 + k)
        new = _document(blocks)
        found = reading.advance(new)
        assert found is not None
        nxt, before, after, begin, stop = found
        assert reading.advance(new)[0] is nxt            # the step is kept
        spans, body = _scan(new)
        assert nxt[:] == spans and nxt.body == body
        old = reading[:]
        assert old[:before] == spans[:before] and before + after <= len(spans)
        assert [e - s for s, e in old[len(old) - after:]] == [e - s for s, e in spans[len(spans) - after:]]
        assert _blocks(new, begin, stop) == spans[before:len(spans) - after]
        assert nxt.counts(0, len(nxt)) == ooxml_core.BodySpans.scan(new).counts(0, len(nxt))
        reading = nxt


def test_a_change_outside_the_body_or_tags_that_do_not_nest_is_no_step():
    data = _document(['<w:p w14:paraId="00000001"/>', '<w:p w14:paraId="00000002"/>'])
    reading = ooxml_core.BodySpans.scan(data)
    assert reading.advance(data.replace(b"<w:document ", b'<w:document a="1" ')) is None
    assert reading.advance(data.replace(b'<w:p w14:paraId="00000002"/>', b'<w:p w14:paraId="00000002">')) is None
    assert ooxml_core.BodySpans.scan(b"<x/>") is None


@pytest.mark.parametrize("seed", range(4))
def test_common_ends_are_the_bytes_alike(seed):
    rng = random.Random(seed)
    for _ in range(50):
        a = bytes(rng.randrange(3) for _ in range(rng.randrange(9000)))
        b = bytearray(a)
        for _ in range(rng.randrange(3)):
            at = rng.randrange(len(b) + 1)
            b[at:at + rng.randrange(5)] = bytes(rng.randrange(3) for _ in range(rng.randrange(5)))
        b = bytes(b)
        prefix = next((k for k in range(min(len(a), len(b))) if a[k] != b[k]), min(len(a), len(b)))
        limit = min(len(a), len(b)) - prefix
        suffix = next((k for k in range(limit) if a[-1 - k] != b[-1 - k]), limit)
        assert ooxml_core.common_ends(a, b) == (prefix, suffix)
        assert ooxml_core.common_prefix_of(a, b) == prefix
        assert ooxml_core.common_suffix_of(a, b, limit) == suffix


def _tree(rng: random.Random, n: int = 40):
    root = etree.fromstring(_document([_block(rng, k) for k in range(n)]))
    return root, root[0]


def _written(root) -> bytes:
    return etree.tostring(root.getroottree(), xml_declaration=True, encoding="UTF-8", standalone=True)


@pytest.mark.parametrize("seed", range(5))
def test_a_part_written_from_its_last_bytes_is_the_part_written_whole(seed):
    rng = random.Random(seed)
    root, body = _tree(rng)
    data = _written(root)
    _, units, _ = ooxml_core.tree_digests(root, body)
    start = data.index(b">", data.index(b"<w:body")) + 1
    end = data.rindex(b"</w:body>")
    starts = ooxml_core.Starts(ooxml_core.unit_starts(data, start, end))
    for k in range(25):
        change = rng.randrange(3)
        at = rng.randrange(len(body))
        if change == 0:
            body.insert(at, etree.fromstring(_document([_block(rng, 3000 + k)]))[0][0])
        elif change == 1 and len(body) > 2:
            body.remove(body[min(at, len(body) - 2)])
        else:
            body[at].set(f"{{{W}}}rsidR", f"{k:08X}")
        _, new_units, _ = ooxml_core.tree_digests(root, body)
        _head, _tail, missing = ooxml_core.write_plan(units, new_units)
        whole = _written(root)
        # (each child as written inside the part: the root's declarations not repeated)
        pieces = [re.sub(rb' xmlns:\w+="[^"]*"', b"", etree.tostring(body[m], with_tail=True, encoding="UTF-8")) for m in missing]
        found = ooxml_core.write_assemble(data, starts, end, units, new_units, pieces)
        assert found is not None
        data, starts, end = found
        assert data == whole
        assert starts.list() == ooxml_core.unit_starts(whole, start, whole.rindex(b"</w:body>"))
        assert end == whole.rindex(b"</w:body>")
        units = new_units
    assert ooxml_core.write_assemble(data, starts, end, units, units, [b"x"]) is None


def _records(root):
    """What an index reads of a tree, per body child (elements as nodes, kinds, paraIds)."""
    out = []
    for child in root[0]:
        nodes = [] if child.tag == f"{{{MC}}}Fallback" else [child] + [
            n for n in child.iter(f"{{{W}}}p", f"{{{W}}}tr", f"{{{W}}}tbl") if n is not child
            and not any(a.tag == f"{{{MC}}}Fallback" for a in n.iterancestors())]
        out.append([(id(n), n.tag, n.get(f"{{{W14}}}paraId")) for n in nodes
                    if n.tag in (f"{{{W}}}p", f"{{{W}}}tr", f"{{{W}}}tbl")])
    return out


@pytest.mark.parametrize("seed", range(4))
def test_the_walks_of_the_blocks_change_with_the_blocks_only(seed):
    rng = random.Random(seed)
    root, body = _tree(rng)
    whole, blocks, places, elsewhere = ooxml_core.index_walks(root)
    assert whole == ooxml_core.index_walk(root) and not elsewhere
    assert len(blocks) == 16 * (len(body) - 1) and len(places) == 4 * (len(body) - 1)
    assert [int.from_bytes(places[4 * k:4 * k + 4], "little") for k in range(len(body) - 1)] == list(range(len(body) - 1))
    held = list(root.iter())                     # (every node kept: none freed and made again)
    for k in range(20):
        at = rng.randrange(len(body) - 1)
        node = body[at]
        p = node if node.tag == f"{{{W}}}p" else next(node.iter(f"{{{W}}}p"))
        p.set(f"{{{W14}}}paraId", f"{4000 + k:08X}")
        held += list(root.iter())
        now = ooxml_core.index_walks(root)
        assert now[0] == ooxml_core.index_walk(root) != whole
        changed = [j for j in range(len(body) - 1) if now[1][16 * j:16 * j + 16] != blocks[16 * j:16 * j + 16]]
        assert changed == [at]
        whole, blocks = now[0], now[1]
    # a body child that is no block, with a paragraph in it, is said so
    extra = etree.SubElement(body, f"{{{W}}}customXmlInsRangeStart")
    etree.SubElement(extra, f"{{{W}}}p")
    assert ooxml_core.index_walks(root)[3]


def test_the_body_s_other_children_by_their_places():
    rng = random.Random(1)
    _root, body = _tree(rng, 10)
    body.insert(3, etree.Element(f"{{{W}}}bookmarkStart"))
    places, blocks, plain = ooxml_core.body_others(body)
    assert places == [k for k, c in enumerate(body) if c.tag not in (f"{{{W}}}p", f"{{{W}}}tbl", f"{{{W}}}sdt")]
    assert blocks == 10 and plain
    body.insert(5, etree.Comment("c"))
    assert ooxml_core.body_others(body)[2] is False
