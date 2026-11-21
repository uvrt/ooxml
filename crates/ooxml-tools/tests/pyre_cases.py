"""Random Python patterns, texts and templates, with what Python's re answers (docx-agent's
semantics: re.finditer, empty matches left out, match.expand on each match).

usage: python pyre_cases.py SEED COUNT OUT.json
"""
import json
import random
import re
import sys
import warnings

warnings.simplefilter("ignore")

ALPHABET = ["a", "b", "A", "B", "é", "É", " ", "\n", "_", "1", "-", ".", "ß", "İ", "ı", "ſ", "K", "K", "\t", "\x1c"]
LITERALS = ["a", "b", "A", "é", " ", "_", "1", "-", "\\.", "\\-", "\\n", "\\t", "\\x41", "\\u00e9", "ß", "ſ", "k", "i", "I"]


def gen_class(r):
    items = []
    for _ in range(r.randint(1, 3)):
        items.append(r.choice(["a", "b", "A", "é", "a-c", "A-Z", "\\w", "\\W", "\\d", "\\s", "\\S", "_", "-", "\\n",
                               " ", "\\]", "ß", "k", "i"]))
    neg = "^" if r.random() < 0.3 else ""
    return "[" + neg + "".join(items) + "]"


def gen(r, depth, groups, verbose):
    out = []
    for _ in range(r.randint(1, 3)):
        out.append(gen_term(r, depth, groups, verbose))
        if verbose and r.random() < 0.3:
            out.append(r.choice([" ", "  ", "\n", " # note\n"]))
    s = "".join(out)
    if depth < 2 and r.random() < 0.2:
        s += "|" + gen(r, depth + 1, groups, verbose)
    return s


def gen_atom(r, depth, groups, verbose):
    k = r.random()
    if k < 0.35 or depth >= 3:
        return r.choice(LITERALS + [".", "\\w", "\\W", "\\d", "\\s", "\\S"])
    if k < 0.45:
        return gen_class(r)
    if k < 0.52:
        return r.choice(["^", "$", "\\b", "\\B", "\\A", "\\Z"])
    if k < 0.70:
        inner = gen(r, depth + 1, groups, verbose)
        kind = r.random()
        if kind < 0.4:
            groups.append(None)
            return "(" + inner + ")"
        if kind < 0.55:
            name = f"n{len(groups)}"
            groups.append(name)
            return f"(?P<{name}>" + inner + ")"
        if kind < 0.7:
            return "(?:" + inner + ")"
        if kind < 0.8:
            return "(?" + r.choice(["i", "s", "m", "-i", "x", "-x", "i-s"]) + ":" + inner + ")"
        if kind < 0.9:
            return "(?>" + inner + ")"
        return r.choice(["(?=", "(?!"]) + inner + ")"
    if k < 0.76:
        return r.choice(["(?<=", "(?<!"]) + r.choice(["a", "b", "\\w", "[ab]", "ab", "a|b", "\\s", "a|bc"]) + ")"
    if k < 0.84 and groups:
        n = r.randint(1, len(groups))
        name = groups[n - 1]
        if name and r.random() < 0.5:
            return f"(?P={name})"
        return f"\\{n}" if r.random() < 0.8 else f"(?:\\{n})"
    if k < 0.88 and groups:
        n = r.randint(1, len(groups))
        return f"(?({n})" + gen(r, 3, groups, verbose) + ("|" + gen(r, 3, groups, verbose) if r.random() < 0.6 else "") + ")"
    if k < 0.95:
        return r.choice(["\\0", "\\01", "\\101", "\\12", "\\x", "\\q", "{", "}", "]", "{1}", "#", " ", "\\777", "[\\1]"])
    if k < 0.97:
        return r.choice(["(", ")", "[", "*", "+", "?", "|", "\\", "(?<n>a)", "(?P<1>a)", "(?(x)a)", "(?Lb)", "(?au)"])
    return r.choice(LITERALS)


def gen_term(r, depth, groups, verbose):
    atom = gen_atom(r, depth, groups, verbose)
    if r.random() < 0.4:
        q = r.choice(["*", "+", "?", "{2}", "{1,2}", "{,2}", "{2,}", "{0}", "{0,1}"])
        q += r.choice(["", "", "?", "+"])
        if verbose and r.random() < 0.3:
            q = " " + q
        atom += q
    return atom


def pattern(r):
    lead = ""
    verbose = False
    if r.random() < 0.35:
        flags = "".join(sorted(set(r.choice("imsxau") for _ in range(r.randint(1, 2)))))
        lead = f"(?{flags})"
        verbose = "x" in flags
    groups = []
    return lead + gen(r, 0, groups, verbose), groups


def template(r, groups):
    pieces = []
    for _ in range(r.randint(0, 3)):
        k = r.random()
        if k < 0.4:
            pieces.append(r.choice(["x", "Y", "é", "-", " "]))
        elif k < 0.6 and groups:
            pieces.append(f"\\{r.randint(1, len(groups))}")
        elif k < 0.7:
            pieces.append("\\g<0>")
        elif k < 0.8 and groups:
            n = r.randint(1, len(groups))
            pieces.append(f"\\g<{groups[n - 1] or n}>")
        else:
            pieces.append(r.choice(["\\n", "\\\\", "\\t", "\\9", "\\q", "\\g<x", "\\0", "\\07", "\\-", "\\"]))
    return "".join(pieces)


def answer(p, ignore, texts, tmpl):
    try:
        rx = re.compile(p, re.IGNORECASE if ignore else 0)
    except (re.error, OverflowError) as e:
        return {"error": str(e), "exception": type(e).__name__}
    out = []
    for t in texts:
        found = []
        try:
            for m in rx.finditer(t):
                if m.end() == m.start():
                    continue
                found.append([m.start(), m.end(), m.expand(tmpl)])
        except re.error as e:
            found = {"template": str(e)}
        except IndexError as e:   # unknown group in a template
            found = {"template": str(e)}
        out.append(found)
    return {"matches": out}


def main():
    seed, count, path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    r = random.Random(seed)
    cases = []
    while len(cases) < count:
        p, groups = pattern(r)
        texts = ["".join(r.choice(ALPHABET) for _ in range(r.randint(0, 10))) for _ in range(3)]
        tmpl = template(r, groups)
        ignore = r.random() < 0.25
        cases.append({"pattern": p, "ignore_case": ignore, "texts": texts, "template": tmpl,
                      "python": answer(p, ignore, texts, tmpl)})
    with open(path, "w", encoding="utf-8") as f:
        json.dump({"python": sys.version.split()[0], "seed": seed, "cases": cases}, f, ensure_ascii=False, indent=0)
    errors = sum("error" in c["python"] for c in cases)
    print(f"{len(cases)} cases, {errors} Python refuses")


main()
