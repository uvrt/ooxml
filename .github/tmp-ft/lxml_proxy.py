"""TEMPORARY (lane L): does lxml hand out an invalid Element proxy on a free-threaded
Python when a proxy is finalized on another thread than the one looking its node up?

One tree; thread A makes a proxy of every element and hands the list to thread B; B drops
it (so, with biased reference counting, each proxy goes back to A, its owner, which
finalizes it at its next eval breaker) and reads the tree again, element by element.  The
two threads never touch the tree at the same time on purpose (a lock is held around every
read and every list made); only the finalizers run outside it.  Counts the invalid proxies
B meets.  ``--no-lock``: without the lock (both threads read side by side)."""
import sys
import threading
import time

import lxml.etree as etree

seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 60
locked = "--no-lock" not in sys.argv
root = etree.Element("root")
for k in range(3000):
    p = etree.SubElement(root, "p", n=str(k))
    for r in range(4):
        etree.SubElement(p, "r", a=str(r)).text = "x"
lock = threading.Lock() if locked else None
handed = []
cond = threading.Condition()
bad = [0]
reads = [0]
stop = time.time() + seconds


def held():
    return lock if lock is not None else _Null()


class _Null:
    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def maker():
    while time.time() < stop:
        with held():
            made = list(root.iter())
        with cond:
            handed.append(made)
            cond.notify()
        del made
        # Python work: eval breakers, where proxies handed back are finalized
        sum(range(2000))


def reader():
    while time.time() < stop:
        with cond:
            while not handed and time.time() < stop:
                cond.wait(0.1)
            got = handed.pop() if handed else None
        del got
        with held():
            for element in root.iter():
                try:
                    element.get("a")
                    element.tag
                except AssertionError:
                    bad[0] += 1
            reads[0] += 1


threads = [threading.Thread(target=maker), threading.Thread(target=reader), threading.Thread(target=reader)]
for t in threads:
    t.start()
for t in threads:
    t.join()
print({"lxml": etree.LXML_VERSION, "gil": getattr(sys, "_is_gil_enabled", lambda: True)(), "locked": locked, "reads": reads[0],
       "invalid_proxies": bad[0]})
sys.exit(1 if bad[0] else 0)
