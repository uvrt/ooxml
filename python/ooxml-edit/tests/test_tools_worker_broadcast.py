"""Round 58 (docx-agent's live view, Q2): a note to every worker of a pool
(``broadcast``) -- the idle ones at once, the busy ones as their task ends and before their
next, ``later`` ones queued without a lock and run as the workers come back.  Toy tasks
(tools_toys.py) keep and forget values in the worker that ran them."""

from __future__ import annotations

import logging
import threading

import pytest

from ooxml_edit.errors import ArgumentError
from ooxml_edit.tools import InProcess, SubprocessPool, ThreadRunner, WorkerPool

import tools_toys

POOLS = [pytest.param(lambda: WorkerPool(2), id="processes"),
         pytest.param(lambda: SubprocessPool(2), id="subprocesses")]


def _two_holders(pool) -> tuple[int, int]:
    """``a`` kept in one worker and ``b`` in the other; their pids."""
    pool.warm(wait=True)
    first, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
    other, _ = pool.run(tools_toys.keep_value, "b", 1, timeout=60, affinity="b")
    assert first != other
    return first, other


@pytest.mark.parametrize("make", POOLS)
def test_a_note_reaches_every_idle_worker_at_once(make):
    with make() as pool:
        first, other = _two_holders(pool)
        pool.broadcast(tools_toys.forget_value, "a")
        pool.broadcast(tools_toys.forget_value, "b")
        assert pool.run(tools_toys.keep_value, "a", 2, timeout=60, affinity="a") == (first, None)
        assert pool.run(tools_toys.keep_value, "b", 2, timeout=60, affinity="b") == (other, None)
        assert sorted(pool.pids()) == sorted([first, other])       # no worker was replaced


@pytest.mark.parametrize("make", POOLS)
def test_a_busy_worker_runs_the_note_when_its_task_ends(make, gate):
    with make() as pool:
        first, _ = _two_holders(pool)
        busy = threading.Thread(target=lambda: pool.run(tools_toys.keep_value, "a", 2, timeout=60, gate=gate.path,
                                                        name="busy", affinity="a"), daemon=True)
        busy.start()
        gate.wait_entered("busy")
        pool.broadcast(tools_toys.forget_value, "a", gate.path)   # the idle one now, the busy one after
        gate.note("told")
        gate.open()
        busy.join(60)
        assert not busy.is_alive()
        assert pool.run(tools_toys.keep_value, "a", 3, timeout=60, affinity="a") == (first, None)
        assert gate.order() == ["busy in", "forget", "told", "busy out", "forget"]


@pytest.mark.parametrize("make", POOLS)
def test_a_note_for_later_runs_as_the_workers_come_back(make):
    with make() as pool:
        first, _ = _two_holders(pool)
        pool.broadcast(tools_toys.forget_value, "a", later=True)
        # Not told yet: the task runs, then the note before the worker is idle again.
        assert pool.run(tools_toys.keep_value, "a", 2, timeout=60, affinity="a") == (first, 1)
        assert pool.run(tools_toys.keep_value, "a", 3, timeout=60, affinity="a") == (first, None)


@pytest.mark.parametrize("make", POOLS)
def test_a_note_that_fails_is_logged_and_the_worker_serves_on(make, caplog):
    with make() as pool:
        first, other = _two_holders(pool)
        with caplog.at_level(logging.WARNING, logger="ooxml_edit.tools.worker"):
            pool.broadcast(tools_toys.failing_note)
        assert "a note that fails" in caplog.text
        assert pool.run(tools_toys.keep_value, "a", 2, timeout=60, affinity="a") == (first, 1)
        assert sorted(pool.pids()) == sorted([first, other])


def test_a_subprocess_pool_refuses_a_note_it_cannot_run():
    def local(key):
        return key

    with SubprocessPool(1) as pool:
        with pytest.raises(ArgumentError, match="not a registered worker task"):
            pool.broadcast(local, "a")


@pytest.mark.parametrize("runner", [lambda: InProcess(2), lambda: ThreadRunner(2), lambda: WorkerPool(0)],
                         ids=["in-process", "threads", "none"])
def test_runners_of_one_process_run_the_note_here(runner):
    seen: list = []
    with runner() as pool:
        pool.broadcast(seen.append, "now")
        assert seen == ["now"]
        pool.broadcast(seen.append, "later", later=True)
        assert seen == ["now"]
        pool.run(len, "x", timeout=60)
        assert seen == ["now", "later"]
