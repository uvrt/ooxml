"""The harness's modules never import a test module or pytest (the gates' environment has
none: twice a case file that took a made-up document from a test module crashed a gate before
its first case), and neither do the Word probes, their documents or the recorded-Word tier's
helpers, which import nothing of docx-agent's library but its tool host."""

from __future__ import annotations

import ast
from pathlib import Path

import pytest

ORACLE = Path(__file__).resolve().parents[2]
HARNESS = sorted((ORACLE / "python").glob("*.py"))
WORD = sorted((ORACLE / "word" / "probes").glob("*.py")) + sorted((ORACLE / "word" / "cases").glob("*.py")) + [
    ORACLE / "word" / "tests" / "recorded.py", ORACLE / "word" / "index.py"]


def imported(path: Path) -> set[str]:
    names = set()
    for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Import):
            names.update(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module and not node.level:
            names.add(node.module)
    return names


def _test_module(name: str) -> bool:
    top = name.split(".")[0]
    return top in ("pytest", "conftest") or top.startswith("test_")


@pytest.mark.parametrize("path", HARNESS + WORD, ids=lambda p: f"{p.parent.name}/{p.name}")
def test_no_test_module_is_imported(path: Path) -> None:
    assert not {name for name in imported(path) if _test_module(name)}


@pytest.mark.parametrize("path", WORD, ids=lambda p: f"{p.parent.name}/{p.name}")
def test_the_word_evidence_uses_only_the_tool_host_of_docx_agent(path: Path) -> None:
    library = {name for name in imported(path) if name.split(".")[0] == "docx_agent"}
    assert library <= {"docx_agent.tools"}, library
