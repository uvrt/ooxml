#!/usr/bin/env python3
"""The gates' harness at HEAD uses only what the frozen reference has (``tools/gates.py frozen-api``).

The gates run docx-agent's Python as it was at the commit crates/ooxml-oracle/reference.toml
names (crates/ooxml-oracle/python/reference_packages.py), while the harness that drives it
(crates/ooxml-oracle/python: the edits and reads harnesses, the case makers, the made-up
documents) is the checkout's.  So the harness may name only modules and module-level names
the reference's packages (reference_packages.FROZEN) had at that commit -- private ones such
as ``layout_facts._Facts`` included; a name added to docx-agent after the freeze is not there
when the gates run.  This reads both statically: the harness modules the gates import (from
ENTRIES, following the harness's own imports) and the frozen packages' sources from git at the
commit, with no install.

What it checks of each harness module: every ``import m`` and ``from m import n`` of a frozen
package (``m`` a module there, ``n`` a submodule or a module-level name of ``m``), and every
attribute read off such an imported module (``docx_agent.validate.check``: ``check`` a
module-level name of ``docx_agent.validate``).  Attributes of the names themselves (a class's
methods) are not followed.
"""

from __future__ import annotations

import ast
import subprocess
import sys
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HARNESS = ROOT / "crates" / "ooxml-oracle" / "python"
#: The harness modules the gates run (tools/gates.py harness_command); the rest is what they import.
ENTRIES = ("word_edits_diff.py", "word_reads_diff.py")

sys.path.insert(0, str(HARNESS))
from reference_packages import FROZEN, pinned_sha  # noqa: E402

#: A module whose names cannot be listed (a star import, a module ``__getattr__``): any name.
ANY = "*"


@dataclass(frozen=True)
class Use:
    """A frozen package's module (and a name in it, or None) a harness module names."""

    file: str
    line: int
    module: str
    name: str | None

    def __str__(self) -> str:
        return f"{self.file}:{self.line}: {self.module}{'.' + self.name if self.name else ''}"


def _frozen(module: str) -> bool:
    return module.split(".")[0] in FROZEN.values()


def harness_modules(harness: Path = HARNESS, entries: Iterable[str] = ENTRIES) -> list[Path]:
    """ENTRIES and every harness module they import, directly or not (by name, anywhere)."""
    local = {path.stem: path for path in harness.glob("*.py")}
    todo = [harness / name for name in entries]
    seen: set[Path] = set()
    while todo:
        path = todo.pop()
        if path in seen:
            continue
        seen.add(path)
        for node in ast.walk(ast.parse(path.read_text(), str(path))):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else \
                [node.module] if isinstance(node, ast.ImportFrom) and node.module and not node.level else []
            todo += [local[n.split(".")[0]] for n in names if n.split(".")[0] in local]
    return sorted(seen)


def uses(path: Path, relative: str) -> list[Use]:
    """What ``path``'s code names of the frozen packages (see the module's docstring)."""
    tree = ast.parse(path.read_text(), str(path))
    found: list[Use] = []
    bound: dict[str, str] = {}       # a name the module binds -> the frozen module or name it is
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if _frozen(alias.name):
                    found.append(Use(relative, node.lineno, alias.name, None))
                    if alias.asname:
                        bound[alias.asname] = alias.name
                    else:
                        top = alias.name.split(".")[0]
                        bound[top] = top
        elif isinstance(node, ast.ImportFrom) and node.module and not node.level and _frozen(node.module):
            for alias in node.names:
                found.append(Use(relative, node.lineno, node.module, None if alias.name == "*" else alias.name))
                if alias.name != "*":
                    bound[alias.asname or alias.name] = f"{node.module}.{alias.name}"
    # each outermost attribute read (``a.b.c``, not its ``a.b`` again) off a name bound above
    inner = {id(n.value) for n in ast.walk(tree) if isinstance(n, ast.Attribute)}
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and isinstance(node.ctx, ast.Load) and id(node) not in inner:
            chain = []
            value: ast.expr = node
            while isinstance(value, ast.Attribute):
                chain.append(value.attr)
                value = value.value
            if isinstance(value, ast.Name) and value.id in bound:
                found.append(Use(relative, node.lineno, bound[value.id], ".".join(reversed(chain))))
    return found


def _top_names(source: str) -> set[str]:
    """A module's module-level names (in its top-level blocks too); ANY if they cannot be listed."""
    names: set[str] = set()

    def visit(body: list[ast.stmt]) -> None:
        for node in body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names.add(node.name)
                if node.name == "__getattr__":
                    names.add(ANY)
            elif isinstance(node, (ast.Import, ast.ImportFrom)):
                for alias in node.names:
                    names.add(ANY if alias.name == "*" else alias.asname or alias.name.split(".")[0])
            elif isinstance(node, (ast.Assign, ast.AnnAssign, ast.AugAssign)):
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                for target in targets:
                    names.update(n.id for n in ast.walk(target) if isinstance(n, ast.Name))
            elif isinstance(node, (ast.If, ast.Try, ast.With, ast.For, ast.While)):
                for field in ("body", "orelse", "finalbody"):
                    visit(getattr(node, field, []))
                for handler in getattr(node, "handlers", []):
                    visit(handler.body)
    visit(ast.parse(source).body)
    return names


class Reference:
    """The frozen packages' modules: ``modules`` (dotted name -> its path), ``read`` (a path's source)."""

    def __init__(self, modules: dict[str, str], read: Callable[[str], str]):
        self.modules, self.read = modules, read
        self._names: dict[str, set[str]] = {}

    @classmethod
    def at(cls, sha: str, root: Path = ROOT) -> Reference:
        """The reference's packages as git has them at ``sha``."""
        def git(*args: str) -> str:
            done = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
            if done.returncode:
                raise SystemExit(f"git {args[0]}: {done.stderr.strip()} (the frozen reference's commit must be "
                                 f"fetched: git fetch origin {sha})")
            return done.stdout

        listed = git("ls-tree", "-r", "--name-only", sha, "--", *(f"python/{f}/src/" for f in FROZEN))
        modules = {}
        for path in listed.splitlines():
            if path.endswith(".py"):
                dotted = path.split("/src/", 1)[1].removesuffix(".py").replace("/", ".")
                modules[dotted.removesuffix(".__init__")] = path
        return cls(modules, lambda path: git("show", f"{sha}:{path}"))

    def names(self, module: str) -> set[str]:
        if module not in self._names:
            self._names[module] = _top_names(self.read(self.modules[module]))
        return self._names[module]

    def _named(self, module: str, name: str) -> bool:
        names = self.names(module)
        return ANY in names or name in names

    def has(self, module: str, name: str | None) -> bool:
        """Whether ``module`` (and ``name``: a dotted chain of attributes read off it) is there:
        each leading part of ``name`` that is a submodule followed, the next one a module-level
        name of the last module (the rest, attributes of that name, not followed)."""
        if module not in self.modules:          # a name ``from m import name`` bound: m.name
            head, _, last = module.rpartition(".")
            return head in self.modules and self._named(head, last)
        for part in name.split(".") if name else []:
            if f"{module}.{part}" not in self.modules:
                return self._named(module, part)
            module = f"{module}.{part}"
        return True


def missing(reference: Reference, harness: Path = HARNESS, root: Path = ROOT) -> list[Use]:
    """The uses of the frozen packages in the gates' harness that ``reference`` lacks."""
    return [use for path in harness_modules(harness) for use in uses(path, str(path.relative_to(root)))
            if not reference.has(use.module, use.name)]


def main() -> int:
    sha = pinned_sha()
    if sha is None:
        print("no frozen reference (crates/ooxml-oracle/reference.toml): nothing to check")
        return 0
    found = missing(Reference.at(sha))
    modules = harness_modules()
    if found:
        for use in found:
            print(f"::error::{use}: not in the frozen reference ({sha[:12]}); the harness may use only what "
                  "the reference has (tools/gates_frozen.py)")
        return 1
    print(f"frozen reference {sha[:12]}: the {len(modules)} harness modules name only what it has")
    return 0


if __name__ == "__main__":
    sys.exit(main())
