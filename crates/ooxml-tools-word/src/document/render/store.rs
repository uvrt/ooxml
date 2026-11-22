//! Owns: what a session keeps of a document's drawn pages between `render` calls
//! ([`PageImages`]): each image by what it draws -- the page's SVG (its `data-docx-*`
//! attributes, which the rasteriser never reads, left out), its width and the faces it is
//! drawn with -- so a page an edit left as it was is not drawn again; a sheet whole, by its
//! tiles and tiling; and the rasters made for a document's faces (the last few). Least
//! recently used go first, within [`MAX_IMAGES`] and the session's memory budget.
//!
//! Must not: draw, or hold anything but bytes and rasters (no document, no layout).

use ooxml_base::{CacheBudget, Digest, DigestKey, Digester};
use ooxml_word_pages::png::PageRaster;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// Images a document keeps at most (pages and sheets together).
pub const MAX_IMAGES: usize = 96;
/// Rasters a document keeps at most (each for one set of faces).
const MAX_RASTERS: usize = 2;
/// What image keys are digested with.
const KEY: DigestKey = DigestKey::new(0x6f6f_786d_6c2d_7069, 0x7865_6c73_2d52_4431);

/// What an image is kept by.
pub type ImageKey = Digest;

/// One image kept.
struct Kept {
    key: ImageKey,
    png: Arc<[u8]>,
}

/// What a document keeps of its drawn pages.
pub struct PageImages {
    budget: Option<Arc<dyn CacheBudget>>,
    images: Mutex<VecDeque<Kept>>,
    rasters: Mutex<VecDeque<(Digest, Arc<PageRaster>)>>,
}

impl std::fmt::Debug for PageImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let images = self.images.lock().unwrap_or_else(PoisonError::into_inner).len();
        f.debug_struct("PageImages")
            .field("images", &images)
            .finish_non_exhaustive()
    }
}

impl Default for PageImages {
    fn default() -> PageImages {
        PageImages::new(None)
    }
}

impl PageImages {
    /// A store whose images count against `budget` (`None`: within [`MAX_IMAGES`] only).
    #[must_use]
    pub fn new(budget: Option<Arc<dyn CacheBudget>>) -> PageImages {
        PageImages {
            budget,
            images: Mutex::new(VecDeque::new()),
            rasters: Mutex::new(VecDeque::new()),
        }
    }

    /// The image kept under `key`, made the most recently used.
    #[must_use]
    pub fn get(&self, key: &ImageKey) -> Option<Arc<[u8]>> {
        let mut images = self.images.lock().unwrap_or_else(PoisonError::into_inner);
        let at = images.iter().position(|k| k.key == *key)?;
        let kept = images.remove(at)?;
        let png = Arc::clone(&kept.png);
        images.push_back(kept);
        Some(png)
    }

    /// Keep `png` under `key`, letting the least recently used go to make room; kept not at
    /// all where the budget cannot give it room.
    pub fn put(&self, key: ImageKey, png: Arc<[u8]>) {
        let mut images = self.images.lock().unwrap_or_else(PoisonError::into_inner);
        if images.iter().any(|k| k.key == key) {
            return;
        }
        while images.len() >= MAX_IMAGES {
            self.let_go(&mut images);
        }
        if let Some(budget) = &self.budget {
            while !budget.reserve(png.len()) {
                if images.is_empty() {
                    return;
                }
                self.let_go(&mut images);
            }
        }
        images.push_back(Kept { key, png });
    }

    fn let_go(&self, images: &mut VecDeque<Kept>) {
        if let Some(gone) = images.pop_front() {
            if let Some(budget) = &self.budget {
                budget.release(gone.png.len());
            }
        }
    }

    /// The raster kept for faces `key`, or the one `make` makes, kept.
    pub fn raster(&self, key: Digest, make: impl FnOnce() -> PageRaster) -> Arc<PageRaster> {
        {
            let rasters = self.rasters.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, raster)) = rasters.iter().find(|(k, _)| *k == key) {
                return Arc::clone(raster);
            }
        }
        // Made without the lock: reading the faces takes a while, and another call may
        // draw meanwhile.
        let made = Arc::new(make());
        let mut rasters = self.rasters.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, raster)) = rasters.iter().find(|(k, _)| *k == key) {
            return Arc::clone(raster);
        }
        if rasters.len() >= MAX_RASTERS {
            rasters.pop_front();
        }
        rasters.push_back((key, Arc::clone(&made)));
        made
    }
}

impl Drop for PageImages {
    fn drop(&mut self) {
        let images = self.images.get_mut().unwrap_or_else(PoisonError::into_inner);
        while !images.is_empty() {
            if let Some(gone) = images.pop_front() {
                if let Some(budget) = &self.budget {
                    budget.release(gone.png.len());
                }
            }
        }
    }
}

/// A page's key: its SVG without the `data-docx-*` attributes, drawn `width` pixels wide
/// with the faces `faces` names.
#[must_use]
pub fn page_key(svg: &str, width: i64, faces: Digest) -> ImageKey {
    let mut digester = Digester::new(KEY);
    digester.update(b"page\0");
    let mut rest = svg;
    while let Some(at) = rest.find(" data-docx-") {
        digester.update(rest.get(..at).unwrap_or_default().as_bytes());
        let after = rest.get(at + 1..).unwrap_or_default();
        // ` data-docx-name="value"`: the attribute ends at its value's closing quote.
        let end = after
            .find("=\"")
            .and_then(|eq| after.get(eq + 2..)?.find('"').map(|q| eq + 2 + q + 1));
        match end {
            Some(end) => rest = after.get(end..).unwrap_or_default(),
            None => {
                digester.update(rest.get(at..).unwrap_or_default().as_bytes());
                rest = "";
            }
        }
    }
    digester.update(rest.as_bytes());
    digester.update(&width.to_le_bytes());
    digester.update(&faces.0);
    digester.finish()
}

/// A sheet's key: its tiles' keys and its tiling.
#[must_use]
pub fn sheet_key(tiles: &[ImageKey], columns: i64, width: i64) -> ImageKey {
    let mut digester = Digester::new(KEY);
    digester.update(b"sheet\0");
    for tile in tiles {
        digester.update(&tile.0);
    }
    digester.update(&columns.to_le_bytes());
    digester.update(&width.to_le_bytes());
    digester.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_attributes_are_not_in_the_key() {
        let faces = Digest::default();
        let a = page_key(
            r#"<g data-docx-path="w:body/w:p[1]" data-docx-id="p:1"><text>A</text></g>"#,
            1000,
            faces,
        );
        let b = page_key(
            r#"<g data-docx-path="w:body/w:p[2]" data-docx-id="p:2"><text>A</text></g>"#,
            1000,
            faces,
        );
        let c = page_key(r#"<g data-docx-path="w:body/w:p[2]"><text>B</text></g>"#, 1000, faces);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, page_key(r#"<g><text>A</text></g>"#, 800, faces));
    }

    #[test]
    fn the_least_recently_used_goes_first() {
        let store = PageImages::default();
        let key = |n: u8| Digest([n; 16]);
        for n in 0..=u8::try_from(MAX_IMAGES).unwrap() {
            store.put(key(n), Arc::from(vec![n]));
            if n == 0 {
                assert!(store.get(&key(0)).is_some());
            }
        }
        assert!(store.get(&key(0)).is_none());
        assert_eq!(store.get(&key(5)).as_deref(), Some(&[5][..]));
    }
}
