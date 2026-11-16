"""``ooxml_core.tree_digests`` and ``unit_starts`` (lane J): a digest per child of a part's
body that changes exactly when what libxml2 writes of the child (its tail included) changes,
one for the rest of the part, and where each child begins in the bytes libxml2 writes."""
from __future__ import annotations

import random

import pytest

ooxml_core = pytest.importorskip("ooxml_core")
etree = pytest.importorskip("lxml.etree")

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def _tree(rng: random.Random, blocks: int = 40):
    root = etree.Element(f"{{{W}}}document", nsmap={"w": W, "x": "urn:x"})
    body = etree.SubElement(root, f"{{{W}}}body")
    body.text = "\n"
    for k in range(blocks):
        p = etree.SubElement(body, f"{{{W}}}p")
        p.set(f"{{{W}}}rsidR", f"{k:08X}")
        for _ in range(rng.randrange(3)):
            r = etree.SubElement(p, f"{{{W}}}r")
            t = etree.SubElement(r, f"{{{W}}}t")
            t.text = rng.choice(["a", "b & c", "<d>", " e ", "ü"])
        p.tail = rng.choice(["", "\n", " "])
    return root, body


def _units(found: bytes) -> list[bytes]:
    return [found[k:k + 16] for k in range(0, len(found), 16)]


def _written(root) -> bytes:
    return etree.tostring(root.getroottree(), xml_declaration=True, encoding="UTF-8", standalone=True)


@pytest.mark.parametrize("seed", range(5))
def test_a_digest_changes_with_its_child_only(seed):
    rng = random.Random(seed)
    root, body = _tree(rng)
    skeleton, units, elements = ooxml_core.tree_digests(root, body)
    assert elements and len(units) == 16 * len(body)
    for _ in range(30):
        before = _units(units)
        k = rng.randrange(len(body))
        kind = rng.randrange(5)
        node = body[k]
        was = etree.tostring(node, with_tail=True)
        if kind == 0:
            node.set(f"{{{W}}}rsidR", f"{rng.randrange(1 << 30):08X}")
        elif kind == 1:
            node.tail = (node.tail or "") + "\n"
        elif kind == 2:
            etree.SubElement(node, f"{{{W}}}r")
        elif kind == 3 and len(node):
            node.remove(node[0])
        else:
            node.set("{urn:x}mark", "1")
        new_skeleton, units, _ = ooxml_core.tree_digests(root, body)
        after = _units(units)
        assert new_skeleton == skeleton
        changed = [j for j, (a, b) in enumerate(zip(before, after)) if a != b]
        assert changed == ([] if etree.tostring(node, with_tail=True) == was else [k])


def test_equal_children_have_equal_digests_and_the_rest_its_own():
    rng = random.Random(7)
    root, body = _tree(rng, 3)
    body.append(etree.fromstring(etree.tostring(body[0])))
    body[3].tail = body[0].tail
    skeleton, units, _ = ooxml_core.tree_digests(root, body)
    assert _units(units)[0] == _units(units)[3]
    root.set("{urn:x}other", "1")
    assert ooxml_core.tree_digests(root, body)[0] != skeleton
    body.text = " "
    body.insert(1, etree.Comment(" note "))
    found = ooxml_core.tree_digests(root, body)
    assert not found[2] and len(found[1]) == 16 * len(body)


def test_where_each_child_begins():
    rng = random.Random(3)
    root, body = _tree(rng, 25)
    data = _written(root)
    start = data.index(b">", data.index(b"<w:body")) + 1
    end = data.rindex(b"</w:body>")
    starts = ooxml_core.unit_starts(data, start, end)
    assert len(starts) == len(body)
    for k, at in enumerate(starts):
        assert data[at:at + 4] == b"<w:p"
    # A comment or a processing instruction at the body's level: not read.
    assert ooxml_core.unit_starts(b"<!-- x --><a/>", 0, 14) is None
    # Within a child they are passed over.
    assert ooxml_core.unit_starts(b"<a><!-- <b> --></a><c/>", 0, 23) == [0, 19]


def test_not_an_element():
    with pytest.raises(TypeError):
        ooxml_core.tree_digests("no", "no")
    root, body = _tree(random.Random(1), 2)
    with pytest.raises(ValueError):
        ooxml_core.tree_digests(body, root)


def test_the_walk_reads_paragraphs_rows_and_tables_only():
    """``index_walk``: the paragraphs, rows and tables in order, with their paraIds and rows'
    tables -- another element put between blocks (a comment's range marks) leaves it; a
    paraId changed, or a paragraph put in, does not."""
    w14 = "http://schemas.microsoft.com/office/word/2010/wordml"
    root = etree.fromstring(
        f'<w:document xmlns:w="{W}" xmlns:w14="{w14}"><w:body><w:p w14:paraId="1"/>'
        f'<w:tbl><w:tr w14:paraId="2"><w:tc><w:p w14:paraId="3"/></w:tc></w:tr></w:tbl>'
        f'<w:p w14:paraId="4"/></w:body></w:document>'.encode())
    body = root[0]
    walk = ooxml_core.index_walk(root)
    body.insert(1, etree.Element(f"{{{W}}}commentRangeStart"))
    body.insert(3, etree.Element(f"{{{W}}}commentRangeEnd"))
    assert ooxml_core.index_walk(root) == walk
    body[0].set(f"{{{w14}}}paraId", "5")
    assert ooxml_core.index_walk(root) != walk
    body[0].set(f"{{{w14}}}paraId", "1")
    assert ooxml_core.index_walk(root) == walk
    body.append(etree.Element(f"{{{W}}}p"))
    assert ooxml_core.index_walk(root) != walk
