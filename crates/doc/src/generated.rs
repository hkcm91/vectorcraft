//! Images that live effects make as they draw (a smooth-shaded 3D surface), which belong to no
//! document. An effect stores its pixels here and draws an [`ImageObject`] whose key names them
//! ([`put`]); the renderer reads them back ([`get`]), and the copy of a document an exporter
//! writes or Expand Appearance leaves takes them in as embedded images ([`adopt`]).
//!
//! The store keeps the most recently used images up to a budget: an image is made again whenever
//! its effect is drawn, so one dropped meanwhile is only missing until the next redraw.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::{Document, ImageBlob, ImageObject, Node, NodeKind};

/// What every generated image's key starts with.
pub const PREFIX: &str = "fx";
/// The most bytes of pixels the store keeps.
const BUDGET: usize = 384 << 20;
/// The most images the store keeps.
const MAX_IMAGES: usize = 1024;

/// A generated image: `width` × `height` straight RGBA pixels, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

impl Generated {
    /// The image as an embedded PNG. `None` when it can't be encoded.
    pub fn blob(&self) -> Option<ImageBlob> {
        let img = image::RgbaImage::from_raw(self.width, self.height, self.rgba.to_vec())?;
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(ImageBlob::png(png))
    }
}

#[derive(Default)]
struct Store {
    images: HashMap<String, (Generated, u64)>,
    bytes: usize,
    clock: u64,
}

fn store() -> MutexGuard<'static, Store> {
    static STORE: LazyLock<Mutex<Store>> = LazyLock::new(Mutex::default);
    // A panic elsewhere while the store was held leaves it intact (each change is complete).
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Store `width` × `height` straight RGBA pixels and return the key an [`ImageObject`] names them
/// by, the same for the same pixels. `None` when the sizes don't match the pixels or the image is
/// larger than the whole budget.
pub fn put(width: u32, height: u32, rgba: Vec<u8>) -> Option<String> {
    let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() != len || len > BUDGET {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in width.to_le_bytes().iter().chain(&height.to_le_bytes()).chain(&rgba) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let key = format!("{PREFIX}{h:016x}");
    let mut s = store();
    s.clock += 1;
    let now = s.clock;
    if let Some(entry) = s.images.get_mut(&key) {
        entry.1 = now;
        return Some(key);
    }
    while (s.bytes + len > BUDGET || s.images.len() >= MAX_IMAGES) && !s.images.is_empty() {
        let Some(oldest) = s.images.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) else { break };
        if let Some((g, _)) = s.images.remove(&oldest) {
            s.bytes -= g.rgba.len();
        }
    }
    s.bytes += len;
    s.images.insert(key.clone(), (Generated { width, height, rgba: Arc::new(rgba) }, now));
    Some(key)
}

/// The generated image `key` names, if the store still has it.
pub fn get(key: &str) -> Option<Generated> {
    if !key.starts_with(PREFIX) {
        return None;
    }
    let mut s = store();
    s.clock += 1;
    let now = s.clock;
    let (g, t) = s.images.get_mut(key)?;
    *t = now;
    Some(g.clone())
}

/// Embed in `d` every generated image `n` (or anything inside it) shows that `d` doesn't hold yet.
pub fn adopt(d: &mut Document, n: &Node) {
    if let NodeKind::Image(im) = &n.kind {
        adopt_image(d, im);
    }
    for c in n.children().into_iter().flatten() {
        adopt(d, c);
    }
}

fn adopt_image(d: &mut Document, im: &ImageObject) {
    if d.images.contains_key(&im.key) {
        return;
    }
    if let Some(blob) = get(&im.key).and_then(|g| g.blob()) {
        d.images.insert(im.key.clone(), blob);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::Affine;

    #[test]
    fn pixels_round_trip_by_content_and_documents_adopt_them() {
        let px = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let key = put(2, 1, px.clone()).unwrap();
        assert!(key.starts_with(PREFIX));
        assert_eq!(put(2, 1, px.clone()), Some(key.clone()));
        assert_ne!(put(1, 2, px.clone()), Some(key.clone()), "the size is part of the image");
        assert_eq!(*get(&key).unwrap().rgba, px);
        assert!(put(3, 1, px.clone()).is_none(), "sizes that don't match the pixels");
        assert!(get("img0123").is_none());
        let mut d = Document::new(100.0, 100.0);
        let im = ImageObject { key: key.clone(), width: 2, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default() };
        let n = Node::group(d.alloc_id(), vec![Arc::new(Node::new(d.alloc_id(), NodeKind::Image(Box::new(im))))]);
        adopt(&mut d, &n);
        let blob = d.images.get(&key).unwrap();
        assert_eq!(blob.mime, "image/png");
        let back = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
        assert_eq!(back.into_raw(), px);
    }
}
