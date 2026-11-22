import traceback, threading
import docx_agent.edit.typography as T
_starts = {}
_rs, _rp = T.start, T.stop
def start():
    tok = _rs(); _starts[id(tok)] = (threading.current_thread().name, "".join(traceback.format_stack(limit=14)))
    return tok
def stop(tok):
    try:
        _rp(tok)
    except ValueError:
        print("=== START in", _starts.get(id(tok), ("?", ""))[0]); print(_starts.get(id(tok), ("", "?"))[1])
        print("=== STOP in", threading.current_thread().name); traceback.print_stack(limit=14)
        raise
T.start, T.stop = start, stop
