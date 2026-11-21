Lane P4f8/P4f9: `copy_from` reads what its destination's paragraphs use (`Key::StyleUse`: each
paragraph's style, list and whether it is blank) for the body style `unmapped: body` maps to
and the list `lists: continue` joins. A concurrent `set_text` that gave a blank paragraph text,
or a list edit that took the items before the place out of their list, wrote block keys only,
so a copy prepared before it could commit a body style or list a serial run would not give; it
is now prepared again.
