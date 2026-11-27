- **The process's model cache stays bounded on picture-heavy decks** (production OOM: a
  worker reached 7.4 GB after about 400 versions of a 30-slide deck on a template with 12
  pictures). With pptx2svg 0.3.10, the resolved slides that edits, checks and renders keep
  hold each picture once and stay within 256 MB. A host logs and sets the cache with
  `pptx_agent.edit.resolved.model_cache_stats()` (entries, bytes held by slides and by
  pictures, hits, misses, the budget), `set_model_cache_budget(bytes)` and
  `clear_model_cache()`; each is a worker task, so `toolbox.pool.broadcast(...)` reaches
  worker processes too (`tests/test_model_cache_memory.py`).
