from __future__ import annotations

import os

import pytest

from ooxml_edit.opc import OpcPackage
from ooxml_edit.xml import register_namespaces

import synthetic

# 0.117.0 (lane G): every kept serialization of an edited part is compared with the tree
# written out again, at every use (ooxml_edit.opc.OpcPackage.keep_serialized).
os.environ.setdefault("OOXML_EDIT_VERIFY_SERIALIZED", "1")

# The test vocabulary, registered once the way a format layer registers its own.
register_namespaces({"tst": synthetic.NS})


@pytest.fixture
def data() -> bytes:
    """The synthetic package's bytes, as written by an ordinary zip writer."""
    return synthetic.outer_package()


@pytest.fixture
def package(data) -> OpcPackage:
    return OpcPackage.open(data)


@pytest.fixture
def gate(tmp_path):
    """A :class:`gates.Gate` of this test's own (tests/gates.py)."""
    import gates

    return gates.Gate(tmp_path)


def pytest_addoption(parser):
    parser.addoption("--run-slow", action="store_true", default=False,
                     help="also run the timing measurements marked slow")


def pytest_collection_modifyitems(config, items):
    """The timing measurements marked slow run with ``--run-slow``: they hold on an idle
    machine, and the PR suites hold the order of events they measure instead."""
    if config.getoption("--run-slow"):
        return
    later = pytest.mark.skip(reason="a timing measurement: run with --run-slow")
    for item in items:
        if "slow" in item.keywords:
            item.add_marker(later)
