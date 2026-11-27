- **`ModelCache` holds each picture once, within a byte budget** (production OOM: a worker
  reached 7.4 GB). Every resolved slide carried its own base64 copy of every picture it
  showed, its layout's and master's included, and the cache kept up to 512 slides by
  count: about 5 MB a version of a slide with one 3.5 MB picture. A picture's base64 text
  is now made once per its bytes (by digest; `pptx2svg.images`) and shared by every slide,
  version and deck that shows it, in one conversion and across the cache; the cache keeps
  its slides and their pictures within `budget` bytes (256 MB by default; at most 512
  slides still), least recently used out first, and a slide that alone exceeds it is drawn
  but not kept. New: `ModelCache(budget=)`, the settable `cache.budget`, and
  `cache.stats()`, a `CacheStats` of entries, entry and image bytes, parts, hits, misses
  and the bounds. 400 edits and renders of a made-up 10-slide deck of 3.5 MB pictures: 5.0
  MB more resident per edit before, 0.08 MB after; a slide edited with its picture
  unchanged resolves without encoding the picture again. Every SVG is the same, byte for
  byte (`tests/test_cache_memory.py`).
