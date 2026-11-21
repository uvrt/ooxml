**New ids drawn where the change lands (lane D1, part a).** Writers that each make new
paragraphs or rows from one version (`word_insert_markdown`, new paragraphs, rows,
sections, notes) no longer conflict over the ids they draw: an edit's new ids are
stand-ins drawn again where its commit lands (`Change::Draw`, `ooxml_word_edit::Drawn`),
so each gets the ids the commit order gives, byte for byte as docx-agent in that order.
The store keeps keys created under stand-ins apart and resolves them at commit
(`DocModel::created`, `resolve`, `begin`); replies name the ids as drawn
(`ooxml_tools_word::resolve_drawn`). A stand-in written where no draw reaches is pinned
(its sequence read: a conflict, as before).
