#!/usr/bin/env python3
"""The Markdown parser's differential (test support; phase P2, lane P2b3): texts read by
``ooxml-markdown`` (``markdown-tokens``) and by docx-agent (markdown-it-py), compared.

``--styled N``: N random texts over an alphabet rich in markers, plus fixed cases, read as
``word_set_text`` reads styled text (``docx_agent.markdown.styled.parse``).

``--tokens N``: N random Markdown documents (block and inline constructs, tables,
footnotes, references, HTML, links) plus the Markdown files under ``--tree``, read by
docx-agent's parser (``docx_agent.markdown.parse.parser``): every field of every token.

    python crates/ooxml-oracle/python/markdown_diff.py --rust target/release/markdown-tokens --styled 20000
"""

from __future__ import annotations

import argparse
import json
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "python" / "docx-agent" / "src"))

#: Markers, words, white space and punctuation of several scripts.
ALPHABET = ["*", "**", "***", "_", "\\", "[", "]", "{", "}", "]{", "{style=", "\"", "'", " ", "  ", "a", "word",
            "é", "€", "«", "»", "(", ")", ".", ",", "!", " ", " ", "\t", "\u000b", "x_y", "Name",
            "custom-style=", "=", "　", "日本", "￼", "\\*", "\\[", "\\_", "\\a", "-", "#"]
FIXED = ["", "plain", "a **b** *c*", "5 * 3", "**open", "_x_", "[t]{Strong}", "[a *b*]{style=\"Q\"} c",
         "***x***", "*a **b** c*", "**a *b***", "*a*b*c*", "a*b*", "\\", "[x]{}", "[x]{ y }", "[x]{'a'b}"]


def styled_cases(count: int, seed: int) -> list[dict]:
    rng = random.Random(seed)
    texts = list(FIXED)
    for _ in range(count):
        texts.append("".join(rng.choice(ALPHABET) for _ in range(rng.randint(1, 14))))
    return [{"mode": "styled", "text": t} for t in texts]


BLOCKS = ["# Heading", "## Two *em*", "Setext\n===", "para line\nnext line", "> quote\n> more", "- a\n- b\n  - c",
          "1. one\n2. two", "3) x", "```py\ncode\n```", "    indented", "***", "| a | b |\n|:-|-:|\n| 1 | 2 |",
          "Text[^1] and ^[inline]\n\n[^1]: The note.\n    more", "[ref]: http://x.org \"T\"", "[ref] and [t][ref]",
          "<div>\nhtml\n</div>", "<!-- c -->", "a <b>x</b> &amp; &#35; &copy; &bogus;", "`code` and ``a`b``",
          "~~del~~ ~x~", "[l](http://例え.jp/a b \"t\") ![i](p.png 'x')", "<http://a.b/c> <me@x.org>",
          "line  \nbreak\\\nend", "* [ ] task", "+ loose\n\n+ list", "Term | x\n--- | ---\nv | w",
          "\\*esc\\* \\_", "**strong** __u__ *a*b*", "<pre>\nx\n</pre>", "[a]: <b c> 'd'\n[a]", ""]


def token_cases(count: int, seed: int, tree: list[str]) -> list[dict]:
    rng = random.Random(seed)
    texts = list(BLOCKS)
    for _ in range(count):
        parts = [rng.choice(BLOCKS + ALPHABET) for _ in range(rng.randint(1, 6))]
        texts.append(rng.choice(["\n", "\n\n", " ", ""]).join(parts))
    for folder in tree:
        texts += [p.read_text(encoding="utf-8") for p in sorted(Path(folder).rglob("*.md"))]
    return [{"mode": "tokens", "text": t} for t in texts]


def _token(token) -> dict:
    meta = {k: v for k, v in (token.meta or {}).items() if v is not None}
    return {"type": token.type, "tag": token.tag, "nesting": token.nesting, "level": token.level,
            "content": token.content, "markup": token.markup, "info": token.info,
            "attrs": [[k, v] for k, v in (token.attrs or {}).items()], "map": token.map,
            "children": None if token.children is None else [_token(c) for c in token.children],
            "meta": meta, "block": token.block, "hidden": token.hidden}


def python_tokens(text: str) -> list:
    from docx_agent.markdown.parse import parser

    return [_token(t) for t in parser().parse(text, {})]


def python_styled(text: str) -> dict:
    from docx_agent.markdown.styled import parse

    read = parse(text)
    return {"plain": read.plain,
            "pieces": [[p.start, p.end, p.text, "strong" in p.marks, "em" in p.marks, p.style] for p in read.pieces]}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--rust", required=True, help="the markdown-tokens binary")
    parser.add_argument("--styled", type=int, default=0, help="random styled texts")
    parser.add_argument("--tokens", type=int, default=-1, help="random Markdown documents")
    parser.add_argument("--tree", action="append", default=[], help="Markdown files below this folder too")
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args(argv)
    cases = styled_cases(args.styled, args.seed) if args.styled else []
    if args.tokens >= 0:
        cases += token_cases(args.tokens, args.seed, args.tree)
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(cases, handle)
    lines = subprocess.run([args.rust, handle.name], capture_output=True, text=True, check=True).stdout.splitlines()
    Path(handle.name).unlink()
    differences = 0
    for case, line in zip(cases, lines, strict=True):
        want = python_styled(case["text"]) if case["mode"] == "styled" else python_tokens(case["text"])
        have = json.loads(line)
        if want != have:
            differences += 1
            if differences <= 20:
                print(json.dumps({"text": case["text"], "python": want, "rust": have}, ensure_ascii=False))
    print(f"{len(cases)} cases, {differences} differences")
    return 1 if differences else 0


if __name__ == "__main__":
    sys.exit(main())
