- `ooxml_core.LayoutFallback` is gone: nothing lays a version out another way. `WordPages.lay_out` and
  `update` raise `NotImplementedError` (naming why and where) for a version the layout does not lay out yet.

  Instead: catch `NotImplementedError`; the tools refuse such a version as not supported yet, with a way out.
