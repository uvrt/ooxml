"""Writes difflib-cases.json for text/diff.rs: random texts with Python difflib's opcodes as
docx-agent folds them (coarse_opcodes and word_opcodes, copied from
python/docx-agent/src/docx_agent/edit/text.py).

    python3 difflib_cases.py 20261121 200 > difflib-cases.json
"""
import difflib
import json
import random
import re
import sys

_MIN_KEPT = 3
_TOKEN = re.compile(r"\w+|\s+|[^\w\s]", re.UNICODE)

def fold(raw, at_a, at_b, fold_equal):
    result = []
    for index, (tag, i1, i2, j1, j2) in enumerate(raw):
        interior = 0 < index < len(raw) - 1
        if tag == "equal" and interior and fold_equal(i1, i2):
            tag = "replace"
        if tag != "equal" and result and result[-1][0] != "equal":
            result[-1][2] = at_a[i2]
            result[-1][4] = at_b[j2]
            continue
        result.append([("equal" if tag == "equal" else "replace"), at_a[i1], at_a[i2], at_b[j1], at_b[j2]])
    return [[op[0] == "equal"] + op[1:] for op in result]

def coarse(old, new):
    raw = difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes()
    ident = range(max(len(old), len(new)) + 1)
    return fold(raw, ident, ident, lambda i1, i2: i2 - i1 < _MIN_KEPT)

def words(old, new):
    a = _TOKEN.findall(old)
    b = _TOKEN.findall(new)
    at_a = [0]
    for t in a:
        at_a.append(at_a[-1] + len(t))
    at_b = [0]
    for t in b:
        at_b.append(at_b[-1] + len(t))
    raw = difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes()
    return fold(raw, at_a, at_b, lambda i1, i2: all(t.isspace() for t in a[i1:i2]))

def text(rng):
    kind = rng.random()
    if kind < 0.4:
        return "".join(rng.choice("ab c") for _ in range(rng.randrange(80)))
    if kind < 0.8:
        vocab = ["the", "report", "text", "before", "after", "a", ",", ".", "  ", "Q3", "é"]
        return " ".join(rng.choice(vocab) for _ in range(rng.randrange(30)))
    return "".join(rng.choice("abcdefgh é中") for _ in range(rng.randrange(120)))

def mutate(rng, s):
    s = list(s)
    for _ in range(rng.randrange(1, 6)):
        at = rng.randrange(len(s) + 1)
        op = rng.randrange(3)
        piece = list(text(rng))[: rng.randrange(1, 10)]
        if op == 0:
            s[at:at] = piece
        elif op == 1:
            del s[at:at + rng.randrange(1, 8)]
        else:
            s[at:at + len(piece)] = piece
    return "".join(s)

rng = random.Random(int(sys.argv[1]))
cases = []
for n in range(int(sys.argv[2])):
    a = text(rng)
    b = mutate(rng, a) if n % 2 else text(rng)
    cases.append([a, b, coarse(a, b), words(a, b)])
json.dump(cases, sys.stdout, ensure_ascii=False, separators=(",", ":"))
