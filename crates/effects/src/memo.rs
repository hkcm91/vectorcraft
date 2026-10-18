//! The last few 3D arts evaluated, by the content of their object: the renderer asks for an
//! object's bounds and then for its art, and both are the same per-pixel shading, which is the
//! slow part. A remembered art is used again only while the shading images it shows are still
//! stored ([`vectorcraft_doc::generated`]).

use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};

use vectorcraft_doc::{Node, NodeKind};

/// How many arts are remembered.
const SIZE: usize = 8;

type Memo = VecDeque<(u64, Option<Node>)>;

static MEMO: LazyLock<Mutex<Memo>> = LazyLock::new(Mutex::default);

/// FNV-1a of `kind`, the object as saved and the art it draws from elsewhere (`extra`: the
/// symbols of its graphics): the same for the same content.
fn key(kind: u8, n: &Node, extra: &[Arc<Node>]) -> Option<u64> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    add(kind);
    for b in serde_json::to_vec(n).ok()? {
        add(b);
    }
    for e in extra {
        for b in serde_json::to_vec(&**e).ok()? {
            add(b);
        }
    }
    Some(h)
}

/// Are the generated images `art` shows still stored?
fn complete(art: &Node) -> bool {
    let mut ok = true;
    art.walk(&mut |m: &Node| {
        if let NodeKind::Image(im) = &m.kind
            && im.key.starts_with(vectorcraft_doc::generated::PREFIX)
            && vectorcraft_doc::generated::get(&im.key).is_none()
        {
            ok = false;
        }
    });
    ok
}

/// `make()`'s art for `n` (of kind `kind`: each evaluator its own), remembered by `n`'s content.
pub(crate) fn remember(kind: u8, n: &Node, make: impl FnOnce() -> Option<Node>) -> Option<Node> {
    remember_with(kind, n, &[], make)
}

/// [`remember`], the art also depending on `extra`.
pub(crate) fn remember_with(kind: u8, n: &Node, extra: &[Arc<Node>], make: impl FnOnce() -> Option<Node>) -> Option<Node> {
    let Some(k) = key(kind, n, extra) else { return make() };
    let lock = || MEMO.lock().unwrap_or_else(|e| e.into_inner());
    let hit = lock().iter().find(|(h, _)| *h == k).map(|(_, art)| art.clone());
    if let Some(art) = hit
        && art.as_ref().is_none_or(complete)
    {
        return art;
    }
    // Made outside the lock: other threads carry on meanwhile.
    let art = make();
    let mut memo = lock();
    memo.retain(|(h, _)| *h != k);
    memo.push_front((k, art.clone()));
    memo.truncate(SIZE);
    art
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vectorcraft_doc::{ImageObject, NodeId};
    use vectorcraft_geom::Affine;

    fn showing(key: String) -> Node {
        let im = ImageObject { key, width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default() };
        Node::group(NodeId(1), vec![Arc::new(Node::new(NodeId(2), NodeKind::Image(Box::new(im))))])
    }

    #[test]
    fn an_art_is_made_once_while_its_images_are_stored() {
        let key = vectorcraft_doc::generated::put(1, 1, vec![1, 2, 3, 255]).unwrap();
        let source = Node::group(NodeId(9), vec![]);
        let mut made = 0;
        for _ in 0..3 {
            remember(7, &source, || {
                made += 1;
                Some(showing(key.clone()))
            });
        }
        assert_eq!(made, 1);
        // An image no longer stored (or never): made again.
        assert!(!complete(&showing(format!("{}0000", vectorcraft_doc::generated::PREFIX))));
        assert!(complete(&showing(key)));
        // Another kind of art of the same object is its own.
        remember(8, &source, || {
            made += 1;
            None
        });
        assert_eq!(made, 2);
    }
}
