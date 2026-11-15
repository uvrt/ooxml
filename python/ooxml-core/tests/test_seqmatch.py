"""``ooxml_core.opcodes`` against its oracle, ``difflib.SequenceMatcher(None, a, b,
autojunk=False).get_opcodes()``: random sequences of 16-byte keys from a small alphabet (so
many keys repeat, where difflib's choice among equal matches decides), edited by inserts,
deletes and replacements; and long ones, as a version's blocks are."""
from __future__ import annotations

import difflib
import random

import pytest

ooxml_core = pytest.importorskip("ooxml_core")


def _key(n: int) -> bytes:
    return n.to_bytes(16, "big")


def _edited(rng: random.Random, a: list[bytes], alphabet: int, edits: int) -> list[bytes]:
    b = list(a)
    for _ in range(edits):
        kind = rng.randrange(4)
        if kind == 0 and b:
            del b[rng.randrange(len(b))]
        elif kind == 1:
            b.insert(rng.randint(0, len(b)), _key(rng.randrange(alphabet)))
        elif kind == 2 and b:
            b[rng.randrange(len(b))] = _key(rng.randrange(alphabet))
        elif b:
            at = rng.randrange(len(b))
            b[at:at] = b[rng.randrange(len(b)):][:rng.randint(1, 5)]       # a stretch repeated
    return b


def _oracle(a: list[bytes], b: list[bytes]) -> list[tuple]:
    return difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()


@pytest.mark.parametrize("seed", range(8))
def test_opcodes_are_difflibs(seed):
    rng = random.Random(seed)
    for _ in range(400):
        alphabet = rng.choice([2, 3, 5, 20, 1000])
        a = [_key(rng.randrange(alphabet)) for _ in range(rng.randint(0, 60))]
        b = _edited(rng, a, alphabet, rng.randint(0, 6)) if rng.random() < 0.8 else \
            [_key(rng.randrange(alphabet)) for _ in range(rng.randint(0, 60))]
        assert ooxml_core.opcodes(b"".join(a), b"".join(b), 16) == _oracle(a, b), (a, b)


def test_opcodes_on_long_sequences():
    rng = random.Random(46)
    chapter = [_key(rng.randrange(10 ** 9)) for _ in range(300)]
    a = [k for _ in range(20) for k in chapter] + [_key(rng.randrange(10 ** 9)) for _ in range(2_000)]
    for edits in (1, 3, 12):
        b = _edited(rng, a, 50, edits)
        assert ooxml_core.opcodes(b"".join(a), b"".join(b), 16) == _oracle(a, b)


def test_opcodes_refuses_keys_of_another_width():
    with pytest.raises(ValueError):
        ooxml_core.opcodes(b"abc", b"", 16)
    with pytest.raises(ValueError):
        ooxml_core.opcodes(b"", b"", 0)
