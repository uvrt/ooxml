"""A tool's documents as a declared rule (``Tool.documents``): argument names, and paths into
an object (``reference.doc``) whose ``doc`` defaults to the call's own.  The Rust contract
declares the same rule (``crates/ooxml-tools/src/documents.rs``) and exports it as these
names.  Toy tools only."""

from __future__ import annotations

import pytest

from ooxml_edit.tools.registry import definition_problems, documents_named, obj, string, tool
from ooxml_edit.tools.schema import SubsetError


def _toy(documents):
    @tool("toy_compare", "Compare with a reference block.",
          {"doc": string("Document id."),
           "reference": obj({"doc": string("Its document. Default doc.", optional=True),
                             "block": string("Its block.")}, "The reference.")},
          documents=documents)
    def toy(call, doc, reference):
        return None
    return toy


def test_a_path_names_another_document_only_when_it_differs():
    names = ("doc", "reference.doc")
    assert documents_named(names, {"doc": "d1", "reference": {"block": "p:1"}}) == ["d1"]
    assert documents_named(names, {"doc": "d1", "reference": {"doc": "", "block": "p:1"}}) == ["d1"]
    assert documents_named(names, {"doc": "d1", "reference": {"doc": "d1", "block": "p:1"}}) == ["d1"]
    assert documents_named(names, {"doc": "d1", "reference": {"doc": "d2", "block": "p:1"}}) == ["d1", "d2"]
    assert documents_named(names, {"doc": "d1", "reference": "p:1"}) == ["d1"]


def test_an_argument_name_is_read_as_before():
    assert documents_named(("doc", "source_doc"), {"doc": "d1", "source_doc": "d1"}) == ["d1", "d1"]
    assert documents_named(("a", "b"), {"a": "d1"}) == ["d1"]


def test_the_tool_reads_its_declared_documents():
    toy = _toy(("doc", "reference.doc"))
    assert toy.doc_ids({"doc": "d1", "reference": {"doc": "d3", "block": "t:2"}}) == ["d1", "d3"]
    assert definition_problems(toy) == []


def test_a_path_to_no_string_is_a_definition_problem():
    with pytest.raises(SubsetError, match="document path 'reference.block.doc' names no string"):
        _toy(("doc", "reference.block.doc"))
