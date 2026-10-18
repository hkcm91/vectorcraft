//! Graphics on 3D surfaces: a document's symbols laid on the front of an Extrude & Bevel or an
//! Inflate, placed, scaled and turned within the object's bounds. Each fill and stroke of the
//! symbol's art becomes a piece: its outline, clipped to the shape, in its own paint.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect};

use crate::util::num;

/// The most graphics one effect lays on its surface.
pub const MAX_GRAPHICS: usize = 8;
/// The most pieces (fills and strokes) the graphics of one effect make.
const MAX_PIECES: usize = 512;

/// A symbol laid on the surface: its centre at `x`, `y` (percent across and down the object's
/// bounds), at `scale` (percent of its own size) and turned `rotation` degrees anticlockwise.
#[derive(Clone, Debug, PartialEq)]
pub struct Graphic {
    pub symbol: String,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub rotation: f64,
}

/// The graphics in an effect's params (`graphics`), at most [`MAX_GRAPHICS`]; malformed entries
/// are left out (validation reports them).
pub fn graphics(p: &Value) -> Vec<Graphic> {
    let Some(list) = p.get("graphics").and_then(Value::as_array) else { return vec![] };
    list.iter()
        .take(MAX_GRAPHICS)
        .filter_map(|g| {
            let symbol = g.get("symbol")?.as_str()?.to_string();
            let fit = |v: f64, fallback: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { fallback };
            Some(Graphic {
                symbol,
                x: fit(num(g, "x", 50.0), 50.0, -100.0, 200.0),
                y: fit(num(g, "y", 50.0), 50.0, -100.0, 200.0),
                scale: fit(num(g, "scale", 100.0), 100.0, 1.0, 1000.0),
                rotation: fit(num(g, "rotation", 0.0), 0.0, -360.0, 360.0),
            })
        })
        .collect()
}

/// Check `graphics` before a command stores it.
pub fn validate(p: &Value) -> Result<(), String> {
    let Some(v) = p.get("graphics") else { return Ok(()) };
    let list = v.as_array().ok_or("graphics must be a list of {symbol, x, y, scale, rotation}")?;
    if list.len() > MAX_GRAPHICS {
        return Err(format!("At most {MAX_GRAPHICS} graphics can be laid on one surface"));
    }
    for g in list {
        if !g.get("symbol").is_some_and(Value::is_string) {
            return Err("Each graphic needs the name of a symbol".into());
        }
        for key in ["x", "y", "scale", "rotation"] {
            if g.get(key).is_some_and(|v| !v.as_f64().is_some_and(f64::is_finite)) {
                return Err(format!("A graphic's {key} must be a number"));
            }
        }
    }
    Ok(())
}

/// A piece of a placed graphic: its rings (in the art's coordinates, clipped to the shape) and
/// its paint.
#[derive(Clone, Debug)]
pub struct Piece {
    pub rings: Vec<Vec<Point>>,
    pub paint: Paint,
    pub opacity: f32,
}

/// Where `g`'s art goes: its centre to its place in `bounds`, scaled and turned about it.
fn placement(g: &Graphic, art: Rect, bounds: Rect) -> Affine {
    let at = Point::new(bounds.x0 + bounds.width() * g.x / 100.0, bounds.y0 + bounds.height() * g.y / 100.0);
    // Anticlockwise on the page, whose y runs down.
    Affine::translate(at.to_vec2())
        * Affine::rotate(-g.rotation.to_radians())
        * Affine::scale(g.scale / 100.0)
        * Affine::translate(-art.center().to_vec2())
}

/// The painted regions of `n` (placed art), bottom to top: each fill its path, each stroke its
/// outline. Type is outlined; images and other kinds add nothing.
fn regions(n: &Node, out: &mut Vec<(PathData, FillRule, Paint, f32)>) {
    if out.len() >= MAX_PIECES || !n.visible {
        return;
    }
    let leaf = match &n.kind {
        NodeKind::Path { path, rule, guide: false, clipping: false, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        NodeKind::Text(_) => {
            if let Some(outlined) = crate::outline_text(n) {
                regions(&outlined, out);
            }
            None
        }
        _ => None,
    };
    if let Some((path, rule)) = leaf {
        for item in n.appearance.items.iter().filter(|i| crate::paints(i)) {
            let (region, paint, opacity) = match item {
                AppearanceItem::Fill(f) => (path.clone(), f.paint.clone(), f.opacity),
                AppearanceItem::Stroke(s) => (crate::stroke::outline_region(&path, rule, s), s.paint.clone(), s.opacity),
            };
            let rule = if matches!(item, AppearanceItem::Fill(_)) { rule } else { FillRule::NonZero };
            out.push((region, rule, paint, opacity * n.opacity));
        }
    }
    for c in n.children().into_iter().flatten() {
        regions(c, out);
    }
}

/// The pieces of `graphics` laid on `shape` (fill rule `rule`, bounds `bounds`), their symbols'
/// art from `symbol`. A symbol that isn't there adds nothing.
pub fn pieces(graphics: &[Graphic], symbol: &dyn Fn(&str) -> Option<Arc<Node>>, shape: &PathData, rule: FillRule, bounds: Rect) -> Vec<Piece> {
    let tolerance = (bounds.width().max(bounds.height()) / 2000.0).max(0.05);
    let mut out = vec![];
    for g in graphics {
        let Some(art) = symbol(&g.symbol) else { continue };
        let Some(art_bounds) = art.geometric_bounds() else { continue };
        let mut placed = (*art).clone();
        // Scaled as a picture is: strokes too.
        placed.transform(placement(g, art_bounds, bounds), true);
        let mut found = vec![];
        regions(&placed, &mut found);
        for (region, region_rule, paint, opacity) in found {
            if out.len() >= MAX_PIECES {
                return out;
            }
            let Ok(clipped) = vectorcraft_pathops::try_boolean(
                &region,
                region_rule,
                shape,
                rule,
                vectorcraft_pathops::BoolOp::Intersect,
                vectorcraft_pathops::DEFAULT_PRECISION,
            ) else {
                continue;
            };
            let rings = flatten(&clipped, tolerance);
            if !rings.is_empty() {
                out.push(Piece { rings, paint, opacity });
            }
        }
    }
    out
}

/// `path` as closed polygons, its curves within `tolerance`.
fn flatten(path: &PathData, tolerance: f64) -> Vec<Vec<Point>> {
    let mut rings: Vec<Vec<Point>> = vec![];
    vectorcraft_geom::kurbo::flatten(path.to_bezpath(), tolerance, |el| {
        use vectorcraft_geom::kurbo::PathEl;
        match el {
            PathEl::MoveTo(p) => rings.push(vec![p]),
            PathEl::LineTo(p) => {
                if let Some(r) = rings.last_mut() {
                    r.push(p);
                }
            }
            _ => {}
        }
    });
    rings.retain(|r| r.len() >= 3);
    rings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::color::Color;
    use vectorcraft_doc::{Appearance, NodeId};
    use vectorcraft_geom::BezPath;

    fn square(x: f64, y: f64, s: f64) -> PathData {
        let mut b = BezPath::new();
        b.move_to((x, y));
        b.line_to((x + s, y));
        b.line_to((x + s, y + s));
        b.line_to((x, y + s));
        b.close_path();
        PathData::from_bezpath(&b)
    }

    /// A 10 pt red square with a 2 pt blue stroke, far from the origin.
    fn logo() -> Arc<Node> {
        let mut n = Node::path(NodeId(9), square(500.0, 500.0, 10.0), Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0));
        n.appearance.items.push(AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 2.0)));
        Arc::new(n)
    }

    #[test]
    fn params_are_read_bounded_and_checked() {
        let p = json!({"graphics": [{"symbol": "Logo", "x": 25, "scale": 1e9}, {"x": 3}, {"symbol": "B", "rotation": f64::NAN.to_string()}]});
        let g = graphics(&p);
        assert_eq!(g.len(), 2, "the entry without a symbol is left out");
        assert_eq!((g[0].x, g[0].y, g[0].scale, g[0].rotation), (25.0, 50.0, 1000.0, 0.0));
        assert!(validate(&p).is_err());
        assert!(validate(&json!({"graphics": [{"symbol": "Logo", "x": 10}]})).is_ok());
        assert!(validate(&json!({"graphics": "Logo"})).is_err());
        assert!(validate(&json!({"graphics": vec![json!({"symbol": "a"}); 9]})).is_err());
        assert!(graphics(&json!({})).is_empty());
    }

    #[test]
    fn a_symbol_is_placed_scaled_and_clipped_to_the_shape() {
        let shape = square(0.0, 0.0, 100.0);
        let b = shape.bounds().unwrap();
        let symbol = |name: &str| (name == "Logo").then(logo);
        let g = Graphic { symbol: "Logo".into(), x: 50.0, y: 50.0, scale: 200.0, rotation: 0.0 };
        let p = pieces(std::slice::from_ref(&g), &symbol, &shape, FillRule::NonZero, b);
        assert_eq!(p.len(), 2, "the fill and the stroke");
        let extent = |piece: &Piece| {
            let pts: Vec<Point> = piece.rings.iter().flatten().copied().collect();
            pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, q| r.union_pt(*q))
        };
        // Twice the size, centred: the fill spans 40 to 60, the stroke 2 pt further each way.
        let fill = extent(&p[0]);
        assert!((fill.x0 - 40.0).abs() < 1e-6 && (fill.x1 - 60.0).abs() < 1e-6, "{fill:?}");
        let stroke = extent(&p[1]);
        assert!((stroke.x0 - 38.0).abs() < 1e-3 && (stroke.x1 - 62.0).abs() < 1e-3, "{stroke:?}");
        assert_eq!(p[1].paint, Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
        // At the corner, only the part over the shape is kept.
        let corner = pieces(&[Graphic { x: 0.0, y: 0.0, ..g.clone() }], &symbol, &shape, FillRule::NonZero, b);
        let clipped = extent(&corner[0]);
        assert!(clipped.x0 >= -1e-6 && clipped.y0 >= -1e-6 && (clipped.x1 - 10.0).abs() < 1e-6, "{clipped:?}");
        // Turned a quarter, a square is still a square; off the shape or unknown, nothing.
        assert_eq!(pieces(&[Graphic { rotation: 90.0, ..g.clone() }], &symbol, &shape, FillRule::NonZero, b).len(), 2);
        assert!(pieces(&[Graphic { x: 180.0, ..g.clone() }], &symbol, &shape, FillRule::NonZero, b).is_empty());
        assert!(pieces(&[Graphic { symbol: "Gone".into(), ..g }], &symbol, &shape, FillRule::NonZero, b).is_empty());
    }
}
