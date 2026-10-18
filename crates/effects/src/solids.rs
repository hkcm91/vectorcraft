//! Extrude & Bevel, Inflate and Rotate as live, shaded vector art, shared by the canvas and every
//! exporter. Like Revolve, the source path stays editable and the effect draws its 3D faces.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Point, Rect};
use vectorcraft_three_d::{Bevel, BevelShape, Extrude, Inflate, Shaded, View};

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

pub const EXTRUDE: &str = "threeD.extrude";
pub const INFLATE: &str = "threeD.inflate";
pub const ROTATE: &str = "threeD.rotate";
/// The 3D effects this module draws (Revolve has its own).
pub const SOLIDS: [&str; 3] = [EXTRUDE, INFLATE, ROTATE];

/// Is `id` one of the 3D and Materials effects (Revolve included)?
pub fn is_three_d(id: &str) -> bool {
    id == crate::REVOLVE || SOLIDS.contains(&id)
}

fn solid(fx: &[Effect]) -> Option<(usize, &Effect)> {
    fx.iter().enumerate().find(|(_, e)| e.visible && SOLIDS.contains(&e.id.as_str()))
}

/// Does `n` carry a visible Extrude & Bevel, Inflate or Rotate effect?
pub fn has_solid(n: &Node) -> bool {
    solid(&n.appearance.effects).is_some() || n.appearance.items.iter().any(|i| i.visible() && solid(i.effects()).is_some())
}

/// The Position presets: (id, [rotation X, Y, Z]). Front, Back, Left, Right, Top and Bottom face
/// the art that way; Off-Axis tilts each 20° down and 25° aside (showing the top and right);
/// Isometric lays the art on the left, right, top or bottom plane of an isometric view (the
/// visible axes foreshortened equally: atan(1/√2) ≈ 35.26° and 45°). Our own values, from the
/// geometry of each view.
pub const ROTATION_PRESETS: [(&str, [f64; 3]); 16] = [
    ("front", [0.0, 0.0, 0.0]),
    ("back", [0.0, 180.0, 0.0]),
    ("left", [0.0, 90.0, 0.0]),
    ("right", [0.0, -90.0, 0.0]),
    ("top", [-90.0, 0.0, 0.0]),
    ("bottom", [90.0, 0.0, 0.0]),
    ("offAxisFront", [-21.88, -23.4, 9.06]),
    ("offAxisBack", [-158.12, 23.4, -170.94]),
    ("offAxisLeft", [-40.74, 58.39, -36.26]),
    ("offAxisRight", [-139.26, -58.39, 143.74]),
    ("offAxisTop", [-111.88, -23.4, 9.06]),
    ("offAxisBottom", [68.12, -23.4, 9.06]),
    ("isometricLeft", [-45.0, -35.26, 30.0]),
    ("isometricRight", [-45.0, 35.26, -30.0]),
    ("isometricTop", [45.0, -35.26, 30.0]),
    ("isometricBottom", [-45.0, -35.26, -30.0]),
];

/// The view parameters every 3D effect shares, named as Revolve's.
pub fn view_options(p: &Value) -> View {
    View {
        rotation_x: num(p, "rotationX", ROTATION_PRESETS[6].1[0]),
        rotation_y: num(p, "rotationY", ROTATION_PRESETS[6].1[1]),
        rotation_z: num(p, "rotationZ", ROTATION_PRESETS[6].1[2]),
        perspective: num(p, "perspective", 0.0) / 100.0,
        light_azimuth: num(p, "lightAzimuth", -45.0),
        light_elevation: num(p, "lightElevation", 45.0),
        light_intensity: num(p, "lightIntensity", 80.0) / 100.0,
        ambient: num(p, "ambient", 25.0) / 100.0,
        shade: flag(p, "shade", true),
        ..crate::lighting::lighting(p).into_view()
    }
}

/// Extrude & Bevel's parameters. `bevel` is `"none"` or a [`BevelShape`] id.
pub fn extrude_options(p: &Value) -> Extrude {
    Extrude {
        depth: num(p, "depth", 50.0),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
        bevel: BevelShape::parse(text(p, "bevel", "none")).map(|shape| Bevel {
            shape,
            width: num(p, "bevelWidth", 4.0),
            height: num(p, "bevelHeight", 4.0),
            repeat: num(p, "bevelRepeat", 1.0).clamp(1.0, 32.0) as usize,
            space: num(p, "bevelSpace", 0.0) / 100.0,
            inside: flag(p, "bevelInside", false),
            both_sides: flag(p, "bevelBothSides", false),
        }),
    }
}

pub fn inflate_options(p: &Value) -> Inflate {
    Inflate {
        depth: num(p, "depth", 0.0),
        volume: num(p, "volume", 50.0),
        both_sides: flag(p, "bothSides", false),
        twist: num(p, "twist", 0.0),
        taper: num(p, "taper", 100.0) / 100.0,
        cap: flag(p, "cap", true),
        draft: flag(p, "draftPreview", false),
    }
}

/// The shape a solid is made from: a path's or a compound path's outline.
fn profile(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, guide: false, clipping: false, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        _ => None,
    }
}

/// The solid's facets, with `art` (pieces of rings) laid on its front.
fn facets(id: &str, path: &PathData, bounds: Rect, p: &Value, art: &[Vec<Vec<Point>>]) -> Result<Shaded, String> {
    let view = view_options(p);
    match id {
        EXTRUDE => vectorcraft_three_d::extrude_mapped(path, bounds, extrude_options(p), view, art),
        INFLATE => vectorcraft_three_d::inflate_mapped(path, bounds, inflate_options(p), view, art),
        _ => vectorcraft_three_d::plane_mapped(path, bounds, view, art),
    }
}

/// Validate before a command changes the document; the evaluator bounds loaded files too.
pub fn validate_solid(n: &Node, id: &str, p: &Value) -> Result<(), String> {
    let label = crate::effect_info(id).map_or("This effect", |e| e.label.trim_end_matches('…'));
    let (path, _) =
        profile(n).ok_or_else(|| format!("{label} currently requires a path or compound path. Convert type, images and groups to paths first."))?;
    let b = path.bounds().ok_or_else(|| format!("{label} requires a shape with at least two anchors"))?;
    if let Some(v) = p.get("bevel")
        && !v.as_str().is_some_and(|s| s == "none" || BevelShape::parse(s).is_some())
    {
        let names: Vec<_> = BevelShape::ALL.iter().map(|b| b.id()).collect();
        return Err(format!("bevel must be none or one of {}", names.join(", ")));
    }
    crate::lighting::validate(p)?;
    crate::graphics::validate(p)?;
    facets(id, &path, b, p, &[]).map(|_| ())
}

/// One surface as a group with the paint's opacity and blend mode (so overlapping facets don't
/// add up their transparency): with smooth shading the surface pixel by pixel, otherwise each
/// facet a path shaded from `paint`. `pieces` are the graphics laid on it ([`Shaded::mapped`]).
fn surface(n: &Node, mut shaded: Shaded, paint: &Paint, opacity: f32, blend: BlendMode, longest: f64, pieces: &[crate::graphics::Piece]) -> Node {
    let mapped = std::mem::take(&mut shaded.mapped);
    if let Some(shading) = &shaded.shading {
        // Pixel by pixel, graphics in the surface's finish go under its shading; matte ones over it.
        let (under, top) = crate::graphics::laid(n.id, &mapped, pieces, Some(shading), longest);
        if let Some(art) = crate::lighting::shaded_surface(n.id, shading, paint, longest, under) {
            let mut group = Node::group(n.id, std::iter::once(Arc::new(art)).chain(top).collect());
            group.opacity = opacity;
            group.blend = blend;
            return group;
        }
    }
    let children = shaded
        .facets
        .into_iter()
        .map(|face| {
            let items = match paint.color() {
                Some(c) => vec![AppearanceItem::Fill(FillLayer::new(crate::lighting::shaded(c, face.tint, face.highlight)))],
                // Gradients and patterns keep their paint, darkened by black and lit by the highlight.
                None => crate::lighting::shading_layers(paint, face.brightness, face.highlight),
            };
            // Adjacent facets, antialiased on their own, would leave hairline seams: a narrow
            // stroke of the same paint closes them. No wider than the facet is thick, though: seen
            // at a grazing angle a facet can be thinner than the stroke, and its stroke would then
            // paint over its neighbours.
            let seam = (face.thickness() / 2.0).clamp(0.05, 0.75);
            let items = items
                .into_iter()
                .flat_map(|item| {
                    let AppearanceItem::Fill(fill) = &item else { return vec![item] };
                    let mut edge = StrokeLayer::new(fill.paint.clone(), seam);
                    edge.opacity = fill.opacity;
                    vec![item, AppearanceItem::Stroke(edge)]
                })
                .collect();
            Arc::new(Node::path(n.id, face.path(), Appearance { items, ..Default::default() }))
        })
        // Flat, each graphic takes the shade of the surface under its centre.
        .chain(crate::graphics::laid(n.id, &mapped, pieces, None, longest).1)
        .collect();
    let mut group = Node::group(n.id, children);
    group.opacity = opacity;
    group.blend = blend;
    group
}

fn paint_of(item: &AppearanceItem) -> (Paint, f32, BlendMode) {
    match item {
        AppearanceItem::Fill(f) => (f.paint.clone(), f.opacity, f.blend),
        AppearanceItem::Stroke(s) => (s.paint.clone(), s.opacity, s.blend),
    }
}

/// The area `item` paints on `path`: the path itself for a fill, the stroke's outline for a stroke.
fn region(path: &PathData, rule: FillRule, item: &AppearanceItem) -> PathData {
    match item {
        AppearanceItem::Stroke(s) if s.width > 0.0 && s.width.is_finite() => crate::stroke::outline_region(path, rule, s),
        _ => path.clone(),
    }
}

/// The 3D art of one effect `fx` on `path` (already through the effects before it).
/// Extrude & Bevel and Inflate build a solid from the outline, painted with the top visible fill
/// (or, without one, the top stroke); Rotate turns every fill and stroke, each in its own paint.
fn art_for(n: &Node, fx: &Effect, path: &PathData, rule: FillRule, items: &[&AppearanceItem], symbol: Symbols) -> Option<Vec<Arc<Node>>> {
    let b = path.bounds()?;
    let p = crate::lighting::drawing_params(&fx.id, &fx.params);
    let size = b.width().max(b.height());
    if fx.id == ROTATE {
        // Every part turns about the centre of the object's own bounds, so they stay together;
        // the shadow is the whole object's, behind them all.
        let mut out = vec![];
        let mut shadow = vec![];
        let mut contact = vec![];
        let mut strength: f64 = 0.0;
        // Graphics go on the top fill (else the top stroke), over the whole object.
        let top = items.iter().rposition(|i| matches!(i, AppearanceItem::Fill(_))).or(items.len().checked_sub(1));
        let pieces = crate::graphics::pieces(&crate::graphics::graphics(&p), symbol, path, rule, b);
        let art: Vec<Vec<Vec<Point>>> = pieces.iter().map(|piece| piece.rings.clone()).collect();
        for (k, item) in items.iter().enumerate() {
            let (paint, opacity, blend) = paint_of(item);
            let (laid, art): (&[crate::graphics::Piece], &[Vec<Vec<Point>>]) = if Some(k) == top { (&pieces, &art) } else { (&[], &[]) };
            let mut s = facets(ROTATE, &region(path, rule, item), b, &p, art).ok()?;
            shadow.extend(s.shadow.take().into_iter().flat_map(|s| s.subpaths));
            if let Some((c, k)) = s.contact.take() {
                contact.extend(c.subpaths);
                strength = strength.max(k);
            }
            out.push(Arc::new(surface(n, s, &paint, opacity, blend, crate::lighting::shading_side(&p), laid)));
        }
        let shadow = (!shadow.is_empty()).then(|| PathData::new(shadow));
        let contact = (!contact.is_empty()).then(|| (PathData::new(contact), strength));
        let mut all = crate::lighting::shadow_nodes(n, shadow, contact, &p, size);
        all.extend(out);
        return Some(all);
    }
    let item = items.iter().rev().find(|i| matches!(i, AppearanceItem::Fill(_))).or_else(|| items.last())?;
    let (paint, opacity, blend) = paint_of(item);
    // A loaded file or a shape edited past the limits keeps drawing its source path instead of
    // failing the frame; commands validate the same evaluator and report its errors.
    let pieces = crate::graphics::pieces(&crate::graphics::graphics(&p), symbol, path, rule, b);
    let art: Vec<Vec<Vec<Point>>> = pieces.iter().map(|piece| piece.rings.clone()).collect();
    let mut s = facets(&fx.id, path, b, &p, &art).ok()?;
    let mut all = crate::lighting::shadow_nodes(n, s.shadow.take(), s.contact.take(), &p, size);
    all.push(Arc::new(surface(n, s, &paint, opacity, blend, crate::lighting::shading_side(&p), &pieces)));
    Some(all)
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

/// Where a 3D effect finds the art of a symbol its graphics name.
type Symbols<'a> = &'a dyn Fn(&str) -> Option<Arc<Node>>;

/// `n` drawn with its Extrude & Bevel, Inflate or Rotate effect: a group of shaded faces keeping
/// the object's id and transparency, the source path untouched. `None` without such an effect.
/// Graphics need their symbols ([`solid_art_in`]); without them the surface is drawn bare.
pub fn solid_art(n: &Node) -> Option<Node> {
    solid_art_in(n, None)
}

/// [`solid_art`], the symbols its graphics name from `doc`.
pub fn solid_art_in(n: &Node, doc: Option<&Document>) -> Option<Node> {
    if !has_solid(n) {
        return None;
    }
    let symbol = |name: &str| crate::graphics::symbol_art(doc, name);
    let used = crate::graphics::used(n, &SOLIDS, doc);
    crate::memo::remember_with(2, n, &used, || evaluate(n, &symbol))
}

fn evaluate(n: &Node, symbol: Symbols) -> Option<Node> {
    let (path, rule) = profile(n)?;
    let ctx = GeomContext::of(n);
    let painted: Vec<&AppearanceItem> = n.appearance.items.iter().filter(|i| crate::paints(i)).collect();
    let mut children = vec![];
    let remaining: Vec<Effect>;
    if let Some((index, fx)) = solid(&n.appearance.effects) {
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        children.extend(art_for(n, fx, &path, rule, &painted, symbol)?);
        // Later geometry effects act on the projected art as a whole.
        remaining = n
            .appearance
            .effects
            .iter()
            .enumerate()
            .filter(|(i, e)| *i != index && (!is_geometry(&e.id) || *i > index))
            .map(|(_, e)| e.clone())
            .collect();
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining = n.appearance.effects.iter().filter(|e| !is_geometry(&e.id)).cloned().collect();
        for item in &painted {
            if let Some((index, fx)) = solid(item.effects()) {
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                for mut art in art_for(n, fx, &p, rule, &[*item], symbol)? {
                    Arc::make_mut(&mut art).appearance.effects =
                        item.effects().iter().skip(index + 1).filter(|e| !is_three_d(&e.id)).cloned().collect();
                    children.push(art);
                }
            } else {
                children.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: vec![(*item).clone()], ..Default::default() })));
            }
        }
    }
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

#[cfg(test)]
#[path = "tests_solids.rs"]
mod tests;
