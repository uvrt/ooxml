"""Affinity (round 38): a task goes to the worker that holds what the last task with the
same key left there, waits for it a bounded time when it is busy, then goes elsewhere."""

from __future__ import annotations

import os
import threading
import time

import pytest

from ooxml_edit.tools import InProcess, SubprocessPool, ThreadRunner, WorkerPool
from ooxml_edit.tools import worker as worker_module
from ooxml_edit.tools.worker import takes_affinity

import gates
import tools_toys

POOLS = [pytest.param(lambda: WorkerPool(2), id="processes"),
         pytest.param(lambda: SubprocessPool(2), id="subprocesses")]


def _warm(pool) -> None:
    pool.warm(wait=True)
    assert len(pool.pids()) == 2


@pytest.mark.parametrize("make", POOLS)
def test_a_key_goes_back_to_its_worker_and_finds_what_it_left(make):
    with make() as pool:
        _warm(pool)
        first, before = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        assert before is None
        other, _ = pool.run(tools_toys.keep_value, "b", 1, timeout=60, affinity="b")
        assert other != first                     # a new key goes to the worker holding none
        for value in (2, 3, 4):
            pid, before = pool.run(tools_toys.keep_value, "a", value, timeout=60, affinity="a")
            assert (pid, before) == (first, value - 1)
            pid, _ = pool.run(tools_toys.keep_value, "b", value, timeout=60, affinity="b")
            assert pid == other
        assert pool.holder("a") == first and pool.holder("b") == other
        counts = pool.affinity_counts
        assert counts["hit"] == 6 and counts["unheld"] == 2
        assert counts["elsewhere"] == counts["waited"] == 0


def test_work_without_a_key_goes_to_a_worker_holding_none():
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        for _ in range(4):
            assert pool.run(tools_toys.worker_pid, timeout=60) != holder
        assert pool.affinity_counts["hit"] == 0


def _busy(pool, gate, value: int = 2, timeout: float = 90) -> threading.Thread:
    """The holder of ``a`` kept busy keeping ``value``, held at ``gate`` as ``busy`` until
    the test opens it, and a :class:`gates.Watched` on the pool's condition: who waits."""
    pool._condition = gates.Watched(pool._condition, gate)
    busy = threading.Thread(target=pool.run, args=(tools_toys.keep_value, "a", value, 0, gate.path, "busy"),
                            kwargs={"timeout": timeout, "affinity": "a"}, daemon=True)
    busy.start()
    gate.wait_entered("busy")
    return busy


def _waiting(pool, gate, **options) -> tuple[threading.Thread, dict]:
    """``keep_value("a", 3)`` from a thread named ``waiter``; the gate opened once it waits."""
    out: dict = {}
    thread = threading.Thread(target=lambda: out.setdefault("got", pool.run(
        tools_toys.keep_value, "a", 3, timeout=60, affinity="a", **options)), name="waiter")
    thread.start()
    gate.wait_noted("waiter waits")
    gate.open()
    thread.join()
    return thread, out


def test_a_busy_holder_is_waited_for_within_the_wait(gate):
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        busy = _busy(pool, gate)
        _, out = _waiting(pool, gate, affinity_wait=30)
        pid, before = out["got"]
        busy.join()
        assert (pid, before) == (holder, 2)       # it waited, and found the newest
        assert pool.affinity_counts["waited"] == 1


def test_a_holder_busy_past_the_wait_sends_the_task_elsewhere(gate):
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        busy = _busy(pool, gate)
        began = time.monotonic()
        pid, before = pool.run(tools_toys.keep_value, "a", 3, timeout=60, affinity="a",
                               affinity_wait=0.4)
        took = time.monotonic() - began
        gate.note("elsewhere")
        assert pid != holder and before is None    # the other worker never held it: made afresh
        assert took >= 0.35                        # it waited its wait (a lower bound only)
        assert pool.holder("a") == pid             # the newest state is there now
        gate.open()
        busy.join()
        # Gone elsewhere while the holder was still busy.
        assert [o for o in gate.order() if not o.endswith("waits")] == ["busy in", "elsewhere", "busy out"]
        assert pool.affinity_counts["elsewhere"] == 1


def test_by_default_a_busy_holder_is_waited_for_within_the_deadline(gate):
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        busy = _busy(pool, gate)
        _, out = _waiting(pool, gate)
        pid, before = out["got"]
        busy.join()
        assert (pid, before) == (holder, 2)


def test_with_no_wait_a_busy_holder_is_not_waited_for(gate):
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        busy = _busy(pool, gate)
        pid, _ = pool.run(tools_toys.keep_value, "a", 3, timeout=60, affinity="a", affinity_wait=0)
        gate.note("elsewhere")
        gate.open()
        busy.join()
        # Had it waited, the holder's hold timed out first.
        assert pid != holder
        assert [o for o in gate.order() if not o.endswith("waits")] == ["busy in", "elsewhere", "busy out"]


def test_the_wait_for_a_holder_counts_against_the_deadline(gate):
    from ooxml_edit.tools import ToolError
    with WorkerPool(1) as pool:
        _warm_one = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        assert _warm_one[1] is None
        busy = _busy(pool, gate)                    # until the call below has timed out
        try:
            with pytest.raises(ToolError) as caught:
                pool.run(tools_toys.keep_value, "a", 3, timeout=0.5, affinity="a", affinity_wait=30)
            assert caught.value.code == "timeout"
        finally:
            gate.open()
        busy.join()


def test_a_killed_holder_takes_its_keys_with_it():
    from ooxml_edit.tools import ToolError
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        with pytest.raises(ToolError) as caught:          # past its deadline: killed
            pool.run(tools_toys.keep_value, "a", 2, 30, timeout=1.0, affinity="a")
        assert caught.value.code == "timeout" and pool.killed[-1] == holder
        assert pool.holder("a") is None
        pid, before = pool.run(tools_toys.keep_value, "a", 3, timeout=60, affinity="a")
        assert pid != holder and before is None           # made afresh, correctly
        assert pool.affinity_counts["unheld"] == 2


def test_a_holder_that_died_while_idle_is_replaced():
    import signal
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a")
        os.kill(holder, signal.SIGKILL if hasattr(signal, "SIGKILL") else signal.SIGTERM)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and holder in pool.pids():
            if not any(w.pid == holder and w.alive() for w in pool._idle):
                break
            time.sleep(0.05)
        pid, before = pool.run(tools_toys.keep_value, "a", 2, timeout=60, affinity="a")
        assert pid != holder and before is None
        assert pool.holder("a") == pid


def test_the_keys_remembered_are_bounded(monkeypatch):
    monkeypatch.setattr(worker_module, "AFFINITY_KEYS", 3)
    with WorkerPool(1) as pool:
        for key in "abcde":
            pool.run(tools_toys.worker_pid, timeout=60, affinity=key)
        assert [pool.holder(key) is not None for key in "abcde"] == [False, False, True, True, True]


def test_many_threads_with_keys_all_finish():
    with WorkerPool(2) as pool:
        _warm(pool)
        errors: list = []
        seen: dict = {}

        def one(n: int) -> None:
            key = "k%d" % (n % 3)
            try:
                for value in range(4):
                    pid, _ = pool.run(tools_toys.keep_value, key, value, 0.02, timeout=120,
                                      affinity=key, affinity_wait=0.2)
                    seen.setdefault(key, set()).add(pid)
            except BaseException as exc:  # noqa: BLE001
                errors.append(exc)

        threads = [threading.Thread(target=one, args=(n,)) for n in range(11)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(120)
        assert not any(thread.is_alive() for thread in threads)
        assert not errors
        assert sum(pool.affinity_counts.values()) == 44


@pytest.mark.parametrize("make", [lambda: InProcess(2), lambda: ThreadRunner(2)])
def test_runners_of_one_process_take_a_key_and_need_it_not(make):
    with make() as runner:
        assert takes_affinity(runner)
        pid, _ = runner.run(tools_toys.keep_value, "a", 1, timeout=60, affinity="a",
                            affinity_wait=1)
        assert pid == os.getpid()


def test_an_application_runner_is_handed_a_key_only_when_it_says_it_takes_one():
    from ooxml_edit.tools import Toolbox

    class Plain:
        size = 1

        def __init__(self) -> None:
            self.kwargs: list = []

        def run(self, fn, *args, timeout, **kwargs):
            self.kwargs.append(kwargs)
            return fn(*args)

        def close(self) -> None:
            pass

    class Keyed(Plain):
        takes_affinity = True

    from ooxml_edit.tools.dispatch import Call

    for runner, handed in ((Plain(), {}), (Keyed(), {"affinity": "a", "affinity_wait": 2})):
        assert takes_affinity(runner) is (handed != {})
        with Toolbox(tools_toys.TOOLS, formats=tools_toys.FORMATS, runner=runner) as box:
            call = Call.__new__(Call)            # only what run reads (a timeout given): the toolbox
            call.toolbox = box
            assert call.run(tools_toys.worker_pid, timeout=5, affinity="a",
                            affinity_wait=2) == os.getpid()
        assert runner.kwargs[-1] == handed


def test_tasks_waiting_for_a_holder_take_it_in_turn_and_others_leave_it_to_them(gate):
    """Three tasks wait for their busy holder; work without a key meanwhile goes to the other
    worker, never to the holder they wait for; they take the holder in the order they came."""
    with WorkerPool(2) as pool:
        _warm(pool)
        holder, _ = pool.run(tools_toys.keep_value, "a", 0, timeout=60, affinity="a")
        order: list = []
        lock = threading.Lock()

        def keyed(value: int) -> None:
            pid, before = pool.run(tools_toys.keep_value, "a", value, 0, gate.path, f"keep {value}",
                                   timeout=90, affinity="a", affinity_wait=90)
            with lock:
                order.append((value, pid, before))

        pool._condition = gates.Watched(pool._condition, gate)
        busy = threading.Thread(target=keyed, args=(1,))
        busy.start()
        gate.wait_entered("keep 1")               # the holder busy until the gate opens
        waiters = []
        for value in (2, 3, 4):                   # each comes once the one before waits
            waiters.append(threading.Thread(target=keyed, args=(value,), name=f"keyed {value}"))
            waiters[-1].start()
            gate.wait_noted(f"keyed {value} waits")
        others = [pool.run(tools_toys.worker_pid, timeout=60) for _ in range(5)]
        gate.open()
        busy.join()
        for thread in waiters:
            thread.join()
        assert holder not in others
        assert [value for value, _, _ in order] == [1, 2, 3, 4]
        assert all(pid == holder for _, pid, _ in order)
        assert [before for _, _, before in order] == [0, 1, 2, 3]
