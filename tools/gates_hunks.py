"""What a branch's changed lines can change, for tools/gates_fixtures.py's ``select``.

gates_fixtures.py reads a changed file by its path; a path that names no feature, or a
foundation, runs every fixture.  For a few files every branch touches, the changed lines
(the diff's hunks) say more, and ``effect`` reads them -- conservatively: it answers only
when every hunk of the file is of a kind below, else ``None`` (every fixture):

* a comment (in Rust not a ``///`` one: a derive may read it), a blank line or a Python
  docstring: nothing;
* in a ``lib.rs`` or ``mod.rs``: a ``mod`` or ``use`` statement (a module's own files decide);
* in crates/ooxml-session/src/registry.rs: a ``registry.register(...)`` of tool ``word_X``:
  word_X's features;
* in crates/ooxml-oracle/known-differences.toml: the entries added, removed or changed --
  the fixtures their ``file`` names (an entry of the corpus gate: none; one without a file:
  ``None``);
* in a harness case maker (crates/ooxml-oracle/python/word_*_cases.py): the tools whose
  cases changed (the tool names in the changed lines, else in the function around them);
* in python/docx-agent/src/docx_agent/rust/edits.py: an entry of ``SERVED`` -- the served
  tool's features.

A tool's features are those its name's words name (gates_fixtures.FEATURE_WORDS); a tool
whose name names none counts for every fixture.
"""

from __future__ import annotations

import ast
import fnmatch
import json
import re
import subprocess
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
KNOWN = "crates/ooxml-oracle/known-differences.toml"
REGISTRY = "crates/ooxml-session/src/registry.rs"
SERVED = "python/docx-agent/src/docx_agent/rust/edits.py"
CASE_MAKERS = re.compile(r"crates/ooxml-oracle/python/word_\w+_cases\.py")
TOOL = re.compile(r"\bword_[a-z_]+\b")
_HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
_RUST_USE = re.compile(r"^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?(?:use|mod)\b[^;{]*(?:\{[^{}]*\})?[^;{]*;", re.M)
_REGISTER = re.compile(r"^[ \t]*registry\.register\([^;]*?\);[ \t]*$", re.M)
_REGISTERED = re.compile(r"\booxml_tools_word::(word_[a-z_]+)\b")
_SERVED_ENTRY = re.compile(r'^\s*"(\w+)": _\w+,\s*$')


@dataclass
class Hunk:
    old_start: int
    new_start: int
    removed: list[str] = field(default_factory=list)
    added: list[str] = field(default_factory=list)

    def lines(self) -> list[tuple[str, int, str]]:
        """Each changed line: its side (``old`` or ``new``), its number there, its text."""
        return ([("old", self.old_start + k, t) for k, t in enumerate(self.removed)]
                + [("new", self.new_start + k, t) for k, t in enumerate(self.added)])


class Diff:
    """The changes from ``base``'s merge base with ``head`` to ``head``: hunks per path, and
    each path's text on either side."""

    def __init__(self, base: str = "origin/main", head: str = "HEAD", root: Path = ROOT):
        self.root = root
        self.head = head
        self.old_ref = self._git("merge-base", base, head).strip()
        self.hunks: dict[str, list[Hunk]] = {}
        path = old_path = None
        for line in self._git("diff", "-U0", "--no-color", "--no-renames", self.old_ref, head).splitlines():
            if line.startswith("diff --git "):
                path = old_path = None
            elif line.startswith("--- "):
                old_path = None if line[4:] == "/dev/null" else line[6:]
            elif line.startswith("+++ "):
                path = old_path if line[4:] == "/dev/null" else line[6:]
            elif line.startswith("@@") and path is not None:
                m = _HUNK.match(line)
                self.hunks.setdefault(path, []).append(Hunk(int(m.group(1)), int(m.group(3))))
            elif path is not None and path in self.hunks and line[:1] in "+-":
                hunk = self.hunks[path][-1]
                (hunk.added if line[0] == "+" else hunk.removed).append(line[1:])
        self._text: dict[tuple[str, str], str | None] = {}

    def _git(self, *args: str) -> str:
        return subprocess.run(["git", *args], cwd=self.root, capture_output=True, text=True, check=True).stdout

    def text(self, side: str, path: str) -> str | None:
        key = (side, path)
        if key not in self._text:
            ref = self.old_ref if side == "old" else self.head
            done = subprocess.run(["git", "show", f"{ref}:{path}"], cwd=self.root, capture_output=True, text=True)
            self._text[key] = done.stdout if done.returncode == 0 else None
        return self._text[key]


def _quiet_lines(path: str, text: str | None) -> set[int]:
    """The lines of ``text`` no behaviour reads: blank, comments, Python docstrings."""
    if text is None:
        return set()
    lines = text.splitlines()
    out = {k + 1 for k, line in enumerate(lines) if not line.strip()}
    if path.endswith(".py"):
        out |= {k + 1 for k, line in enumerate(lines) if line.strip().startswith("#")}
        try:
            tree = ast.parse(text)
        except SyntaxError:
            return out
        for node in ast.walk(tree):
            body = getattr(node, "body", None)
            if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) and body \
                    and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) \
                    and isinstance(body[0].value.value, str):
                out |= set(range(body[0].lineno, body[0].end_lineno + 1))
    elif path.endswith(".rs"):
        out |= {k + 1 for k, line in enumerate(lines)
                if line.strip().startswith("//") and not line.strip().startswith("///")}
    elif path.endswith(".toml"):
        out |= {k + 1 for k, line in enumerate(lines) if line.strip().startswith("#")}
    return out


def _spans(pattern: re.Pattern, text: str | None) -> list[tuple[int, int, str]]:
    """The line ranges (first, last) of ``pattern``'s matches in ``text``, with their text."""
    if text is None:
        return []
    out = []
    for m in pattern.finditer(text):
        first = text.count("\n", 0, m.start()) + 1
        out.append((first, first + m.group(0).count("\n"), m.group(0)))
    return out


def tool_features(name: str, words: dict[str, tuple[str, ...]]) -> set[str] | None:
    """The features tool ``name``'s words name, or None (every fixture)."""
    parts = set(name.lower().split("_")) - {"word"}
    found = {feature for feature, names in words.items() if parts & set(names)}
    return found or None


Effect = tuple[set[str], set[str]]          # (features, fixtures)


def _tools_effect(tools: set[str], words) -> Effect | None:
    if not tools:
        return None
    features: set[str] = set()
    for tool in tools:
        found = tool_features(tool, words)
        if found is None:
            return None
        features |= found
    return features, set()


def _in_spans(spans, side_lines) -> tuple[bool, set[str]]:
    """Whether every line lies in one of ``spans``, and the spans' texts that hold them."""
    texts = set()
    for line in side_lines:
        span = next((s for s in spans if s[0] <= line <= s[1]), None)
        if span is None:
            return False, set()
        texts.add(span[2])
    return True, texts


def _known_effect(diff: Diff, fixtures: list[str]) -> Effect | None:
    def entries(side: str) -> set[str]:
        text = diff.text(side, KNOWN)
        return {json.dumps(e, sort_keys=True) for e in tomllib.loads(text).get("difference", [])} if text else set()

    chosen: set[str] = set()
    for entry in map(json.loads, entries("old") ^ entries("new")):
        if entry.get("gate") not in (None, "fixtures", "reads"):
            continue
        if "file" not in entry:
            return None
        chosen |= {name for name in fixtures if fnmatch.fnmatch(name, entry["file"])}
    return set(), chosen


def _function_tools(text: str | None, line: int) -> set[str]:
    """The tool names in the top-level function around ``line``."""
    if text is None:
        return set()
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            first = min([node.lineno, *(d.lineno for d in node.decorator_list)])
            if first <= line <= node.end_lineno:
                return {c.value for c in ast.walk(node) if isinstance(c, ast.Constant) and isinstance(c.value, str)
                        and TOOL.fullmatch(c.value)}
    return set()


def effect(path: str, diff: Diff, words: dict[str, tuple[str, ...]], fixtures: list[str]) -> Effect | None:
    """What the changed lines of ``path`` can change (see the module's docstring), or None."""
    if path == KNOWN:
        return _known_effect(diff, fixtures)
    if not diff.hunks.get(path):
        return None                     # no lines to read (a binary file, a mode change)
    quiet = {side: _quiet_lines(path, diff.text(side, path)) for side in ("old", "new")}
    features: set[str] = set()
    for hunk in diff.hunks.get(path, []):
        loud = [(side, n, t) for side, n, t in hunk.lines() if n not in quiet[side]]
        if not loud:
            continue
        found = _hunk_effect(path, diff, hunk, loud, words)
        if found is None:
            return None
        features |= found
    return features, set()


def _hunk_effect(path: str, diff: Diff, hunk: Hunk, loud, words) -> set[str] | None:
    by_side = {side: [n for s, n, _ in loud if s == side] for side in ("old", "new")}
    if path.endswith(("/lib.rs", "/mod.rs")):
        ok = all(_in_spans(_spans(_RUST_USE, diff.text(side, path)), lines)[0] for side, lines in by_side.items())
        return set() if ok else None
    if path == REGISTRY:
        tools: set[str] = set()
        for side, lines in by_side.items():
            ok, texts = _in_spans(_spans(_REGISTER, diff.text(side, path)), lines)
            if not ok:
                return None
            tools |= {t for text in texts for t in _REGISTERED.findall(text)}
        found = _tools_effect(tools, words)
        return None if found is None else found[0]
    if path == SERVED:
        tools = set()
        for _, _, text in loud:
            m = _SERVED_ENTRY.match(text)
            if m is None:
                return None
            tools.add(m.group(1))
        found = _tools_effect(tools, words)
        return None if found is None else found[0]
    if CASE_MAKERS.fullmatch(path):
        tools = {t for _, _, text in loud for t in TOOL.findall(text)}
        if not tools:
            for side, lines in by_side.items():
                for line in lines:
                    around = _function_tools(diff.text(side, path), line)
                    if not around:
                        return None
                    tools |= around
        found = _tools_effect(tools, words)
        return None if found is None else found[0]
    return None
