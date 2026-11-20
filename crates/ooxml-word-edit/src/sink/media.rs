//! Owns: a picture's bytes put in the package as docx-agent's `_media` puts them
//! ([`EditSink::media`]): a part under `word/media` that holds the same bytes already
//! reused (ooxml-edit's `find_part_with_bytes`, shared with the copy's parts), else a new `imageN.<ext>` (the lowest number free) with its content type
//! declared; a relationship to it from the story part (an existing one of the image type
//! reused).
//!
//! Must not: decide a picture's format (the caller names its extension and type), or touch
//! a story part.

use super::EditSink;
use crate::doc::{Change, Key};
use crate::Result;
use std::sync::Arc;

/// The relationship type of a picture.
pub(crate) const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
/// Where a package keeps its pictures.
const MEDIA: &str = "word/media/";

impl EditSink<'_, '_> {
    /// A relationship from `part` to a media part holding `bytes` (an existing part with
    /// the same bytes, or a new one of `extension` and `content_type`): its id.
    ///
    /// # Errors
    /// When the content types or relationships are not XML.
    pub(crate) fn media(
        &mut self,
        part: &str,
        bytes: &Arc<[u8]>,
        (extension, content_type): (&str, &str),
    ) -> Result<String> {
        let package = self.doc().package().clone();
        let same = crate::new_part::part_with_bytes(&package, bytes, "word/media")?;
        let target = match same {
            Some(name) => {
                // the part reused must still hold those bytes where the edit commits
                self.read(Key::Part(Arc::from(name.as_str())));
                name
            }
            None => {
                let name = (1u32..)
                    .map(|n| format!("{MEDIA}image{n}.{extension}"))
                    .find(|name| !package.has_part(name))
                    .unwrap_or_default();
                self.write(Change::Bytes {
                    part: Arc::from(name.as_str()),
                    bytes: Arc::clone(bytes),
                })?;
                if let Some(made) = crate::new_part::declare_type(self.doc().package(), &name, content_type)? {
                    self.put_from(&made, ooxml_opc::CONTENT_TYPES_PART)?;
                }
                name
            }
        };
        let (made, id) = crate::new_part::add_internal(self.doc().package(), part, &target, REL_IMAGE)?;
        if let Some(made) = made {
            self.put_from(&made, &ooxml_opc::name::rels_for(part))?;
        }
        Ok(id)
    }

    /// Put `made`'s tree of `name` into the edit.
    fn put_from(&mut self, made: &ooxml_opc::Package, name: &str) -> Result<()> {
        match made.tree(name)? {
            Some(tree) => self.put(name, tree),
            None => Ok(()),
        }
    }
}
