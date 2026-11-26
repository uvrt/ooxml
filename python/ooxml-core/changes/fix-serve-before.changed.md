- `Session.edit`'s fifth value is now the version the call's first edit was applied to, held (an
  `ooxml_core.Version`; its `state` the number it was), instead of that number: a host reads what
  the step changed from it, however many steps other writers commit meanwhile. The number alone
  could name a state the session's undo history had let go by then.
