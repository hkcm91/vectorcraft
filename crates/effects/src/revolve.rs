//! Revolve as live, shaded vector art, shared by the canvas and every exporter.
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData, Point, Rect};
use vectorcraft_three_d::{Edge, Revolve};

use crate::util::{flag, num, text};
use crate::{GeomContext, apply_geometry_with, is_geometry};

pub const REVOLVE: &str = "threeD.revolve";

fn visible(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.visible && e.id == REVOLVE)
}

pub fn has_revolve(n: &Node) -> bool {
    visible(&n.appearance.effects) || n.appearance.items.iter().any(|i| i.visible() && visible(i.effects()))
}

/// Shared document-parameter conversion for rendering and interactive preview guides.
pub fn revolve_options(p: &Value) -> Revolve {
    Revolve {
        angle: num(p, "angle", 360.0),
        offset: num(p, "offset", 0.0),
        edge: if text(p, "edge", "left") == "right" { Edge::Right } else { Edge::Left },
        rotation_x: num(p, "rotationX", 0.0),
        rotation_y: num(p, "rotationY", 0.0),
        rotation_z: num(p, "rotationZ", 0.0),
        perspective: num(p, "perspective", 0.0) / 100.0,
        // A draft (a live preview being dragged) goes round in half the steps.
        segments: (num(p, "segments", 64.0).clamp(8.0, 128.0) / if flag(p, "draftPreview", false) { 2.0 } else { 1.0 }) as usize,
        light_azimuth: num(p, "lightAzimuth", -45.0),
        light_elevation: num(p, "lightElevation", 45.0),
        light_intensity: num(p, "lightIntensity", 80.0) / 100.0,
        ambient: num(p, "ambient", 25.0) / 100.0,
        shade: flag(p, "shade", true),
        cap: flag(p, "cap", true),
        ..crate::lighting::lighting(p).into_revolve()
    }
}

/// Validate before a command mutates the document. Loading external files is still bounded
/// by the evaluator; unsupported source kinds remain unchanged when rendered.
pub fn validate_revolve(n: &Node, p: &Value) -> Result<(), String> {
    let path = profile(n).ok_or("Revolve currently requires a path or compound path. Select a profile drawn with the Pen tool.")?;
    let b = path.bounds().ok_or("Revolve requires a profile with at least two anchors")?;
    if !path.subpaths.iter().any(|sp| sp.anchors.len() >= 2) {
        return Err("Revolve requires at least two anchors".into());
    }
    if p.get("edge").is_some_and(|v| v.as_str() != Some("left") && v.as_str() != Some("right")) {
        return Err("Revolve edge must be left or right".into());
    }
    crate::lighting::validate(p)?;
    crate::graphics::validate(p)?;
    vectorcraft_three_d::revolve(&path, b, revolve_options(p)).map(|_| ())
}

fn profile(n: &Node) -> Option<PathData> {
    match &n.kind {
        NodeKind::Path { path, guide: false, clipping: false, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()))
        }
        _ => None,
    }
}

fn geometry(path: &PathData, fx: &[Effect], ctx: &GeomContext) -> PathData {
    apply_geometry_with(fx, path, path.bounds().unwrap_or(Rect::ZERO), ctx)
}

#[allow(clippy::too_many_arguments)]
fn surface(
    n: &Node,
    path: &PathData,
    fx: &Effect,
    paint: Paint,
    opacity: f32,
    blend: vectorcraft_doc::color::BlendMode,
    expand: bool,
    symbol: Symbols,
) -> Option<Node> {
    let b = path.bounds()?;
    // A loaded file or a profile edited past the limits keeps drawing its source path instead
    // of crashing the frame. Commands validate the same evaluator and report its errors.
    let params = crate::lighting::drawing_params(REVOLVE, &fx.params);
    let options = revolve_options(&params);
    let pieces = labels(&params, symbol, b, options);
    let art: Vec<Vec<Vec<Point>>> = pieces.iter().map(|piece| piece.rings.clone()).collect();
    let vectorcraft_three_d::Revolved { faces, shadow, shading, contact, mapped } =
        vectorcraft_three_d::revolve_mapped(path, b, options, &art).ok()?;
    let shadow = crate::lighting::shadow_nodes(n, shadow, contact, &params, b.width().max(b.height()));
    let longest = crate::lighting::shading_side(&params);
    // Smooth shading: the surface pixel by pixel.
    if let Some(shading) = &shading {
        let (under, top) = crate::graphics::laid(n.id, &mapped, &pieces, Some(shading), longest);
        if let Some(art) = crate::lighting::shaded_surface(n.id, shading, &paint, longest, under) {
            let mut group = Node::group(n.id, shadow.into_iter().chain([Arc::new(art)]).chain(top).collect());
            group.opacity = opacity;
            group.blend = blend;
            return Some(group);
        }
    }
    // Visibility is an expansion option, never a change to live geometry or export baking.
    // Keep all faces for paints whose coverage we cannot prove opaque. A bounded visibility
    // failure also retains the complete surface rather than losing any of the user's art.
    let visible = (expand
        && flag(&fx.params, "expandVisibleOnly", true)
        && paint.color().is_some()
        && opacity == 1.0
        && blend == Default::default()
        && n.has_default_transparency())
    .then(|| vectorcraft_three_d::visible_faces(&faces).ok())
    .flatten();
    let faces = visible.unwrap_or_else(|| {
        faces
            .into_iter()
            .map(|f| vectorcraft_three_d::VisibleFace {
                path: f.path(),
                brightness: f.brightness,
                depth: f.depth,
                tint: f.tint,
                highlight: f.highlight,
            })
            .collect()
    });
    let children = shadow
        .into_iter()
        .chain(faces.into_iter().map(|face| {
            let items = match paint.color() {
                Some(c) => vec![AppearanceItem::Fill(FillLayer::new(crate::lighting::shaded(c, face.tint, face.highlight)))],
                None => crate::lighting::shading_layers(&paint, face.brightness, face.highlight),
            };
            // Independently antialiased adjacent polygons otherwise leave a hairline grid.
            // Matching narrow edge paint overlaps their coverage in the canvas and exporters.
            // Keep opacity on the surface group so these edges never accumulate transparency.
            let items = items
                .into_iter()
                .flat_map(|item| {
                    let AppearanceItem::Fill(fill) = &item else { return vec![item] };
                    let mut edge = StrokeLayer::new(fill.paint.clone(), 0.75);
                    edge.opacity = fill.opacity;
                    vec![item, AppearanceItem::Stroke(edge)]
                })
                .collect();
            Arc::new(Node::path(n.id, face.path, Appearance { items, ..Default::default() }))
        }))
        // Flat, each graphic takes the shade of the surface under its centre.
        .chain(crate::graphics::laid(n.id, &mapped, &pieces, None, longest).1)
        .collect();
    let mut group = Node::group(n.id, children);
    group.opacity = opacity;
    group.blend = blend;
    Some(group)
}

fn paint_of(item: &AppearanceItem) -> (Paint, f32, vectorcraft_doc::color::BlendMode) {
    match item {
        AppearanceItem::Fill(f) => (f.paint.clone(), f.opacity, f.blend),
        AppearanceItem::Stroke(s) => (s.paint.clone(), s.opacity, s.blend),
    }
}

/// Where Revolve finds the art of a symbol its graphics name.
type Symbols<'a> = &'a dyn Fn(&str) -> Option<Arc<Node>>;

/// The pieces of the graphics in `params` wrapped round a Revolve of profile bounds `b`: placed
/// in a label as wide as the way round at [`vectorcraft_three_d::revolve_radius`] (50 % the
/// front) and as tall as the profile, and clipped to the part of it the turn covers.
fn labels(params: &Value, symbol: Symbols, b: Rect, options: Revolve) -> Vec<crate::graphics::Piece> {
    let graphics = crate::graphics::graphics(params);
    let r = vectorcraft_three_d::revolve_radius(b, options);
    if graphics.is_empty() || r.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || b.height() <= 0.0 {
        return vec![];
    }
    let pi = std::f64::consts::PI;
    let sweep = if options.angle.is_finite() { options.angle.clamp(0.0, 360.0).to_radians() } else { 2.0 * pi };
    // The front is at 0; a partial turn starts a quarter turn to the right of it and runs back
    // round to the left.
    let (x0, x1) = if sweep >= 2.0 * pi - 1e-9 { (-pi * r, pi * r) } else { ((pi / 2.0 - sweep) * r, pi / 2.0 * r) };
    let turned = PathData::single(vectorcraft_geom::SubPath::polyline(
        &[Point::new(x0, b.y0), Point::new(x1, b.y0), Point::new(x1, b.y1), Point::new(x0, b.y1)],
        true,
    ));
    crate::graphics::pieces(&graphics, symbol, &turned, FillRule::NonZero, Rect::new(-pi * r, b.y0, pi * r, b.y1))
}

/// Evaluated art keeps the source's identity and transparency. Each item's opacity belongs
/// to its entire surface, not to each face, so overlapping faces don't accumulate alpha.
/// Graphics need their symbols ([`revolve_art_in`]); without them the surface is drawn bare.
pub fn revolve_art(n: &Node) -> Option<Node> {
    revolve_art_in(n, None)
}

/// [`revolve_art`], the symbols its graphics name from `doc`.
pub fn revolve_art_in(n: &Node, doc: Option<&Document>) -> Option<Node> {
    if !has_revolve(n) {
        return None;
    }
    let used = crate::graphics::used(n, &[REVOLVE], doc);
    crate::memo::remember_with(1, n, &used, || art(n, false, &|name| crate::graphics::symbol_art(doc, name)))
}

/// Revolve's own Expand Appearance path. The shared command and UI remain unchanged.
pub(crate) fn expand_revolve_art(n: &Node, doc: Option<&Document>) -> Option<Node> {
    art(n, true, &|name| crate::graphics::symbol_art(doc, name))
}

fn art(n: &Node, expand: bool, symbol: Symbols) -> Option<Node> {
    if !has_revolve(n) {
        return None;
    }
    let path = profile(n)?;
    let ctx = GeomContext::of(n);
    let mut children = vec![];
    let mut remaining = n.appearance.effects.clone();
    if let Some(index) = n.appearance.effects.iter().position(|e| e.visible && e.id == REVOLVE) {
        let fx = n.appearance.effects.get(index)?;
        let path = geometry(&path, n.appearance.effects.get(..index)?, &ctx);
        // An object's surface uses its top visible fill, falling back to its stroke.
        let item = n
            .appearance
            .items
            .iter()
            .rev()
            .find(|i| matches!(i, AppearanceItem::Fill(_)) && crate::paints(i))
            .or_else(|| n.appearance.items.iter().rev().find(|i| crate::paints(i)))?;
        let (paint, opacity, blend) = paint_of(item);
        let expand = expand && !n.appearance.effects.iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
        children.push(Arc::new(surface(n, &path, fx, paint, opacity, blend, expand, symbol)?));
        // Later geometry acts on the projected art as a whole through container evaluation.
        remaining = n
            .appearance
            .effects
            .iter()
            .enumerate()
            .filter(|(i, e)| e.id != REVOLVE && (!is_geometry(&e.id) || *i > index))
            .map(|(_, e)| e.clone())
            .collect();
    } else {
        let path = geometry(&path, &n.appearance.effects, &ctx);
        remaining.retain(|e| !is_geometry(&e.id));
        for item in n.appearance.items.iter().filter(|i| crate::paints(i)) {
            if let Some(index) = item.effects().iter().position(|e| e.visible && e.id == REVOLVE) {
                let fx = item.effects().get(index)?;
                let p = geometry(&path, item.effects().get(..index)?, &ctx.item(item));
                let (paint, opacity, blend) = paint_of(item);
                let expand = expand && !item.effects().iter().skip(index + 1).any(|e| e.visible && is_geometry(&e.id));
                let mut art = surface(n, &p, fx, paint, opacity, blend, expand, symbol)?;
                art.appearance.effects = item.effects().iter().skip(index + 1).filter(|e| e.id != REVOLVE).cloned().collect();
                children.push(Arc::new(art));
            } else {
                children.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: vec![item.clone()], ..Default::default() })));
            }
        }
    }
    let mut art = n.clone();
    art.kind = NodeKind::Group { children, clip: false };
    art.appearance = Appearance { effects: remaining, ..Default::default() }.into();
    Some(art)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_doc::color::Color;
    use vectorcraft_doc::{Document, NodeId};
    use vectorcraft_geom::{Affine, BezPath};
    fn profile_node() -> Node {
        let mut b = BezPath::new();
        b.move_to((100.0, 40.0));
        b.line_to((130.0, 60.0));
        b.line_to((130.0, 140.0));
        b.line_to((100.0, 160.0));
        Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.8, 0.2, 0.1)), Paint::None, 0.0))
    }
    /// Revolve, shaded flat: its faces as vector paths.
    fn fx() -> Effect {
        crate::new_effect(REVOLVE, &json!({"smoothShading": false})).unwrap()
    }
    #[test]
    fn source_stays_editable_surface_moves_and_hidden_effect_restores_it() {
        let mut n = profile_node();
        let before = n.path_data().unwrap().clone();
        n.appearance.effects.push(fx());
        let a = revolve_art(&n).unwrap();
        assert!(!a.children().unwrap().is_empty());
        assert_eq!(n.path_data().unwrap(), &before);
        let b = a.visual_bounds().unwrap();
        n.transform(Affine::translate((80.0, 20.0)), false);
        let moved = revolve_art(&n).unwrap().visual_bounds().unwrap();
        assert!((moved.x0 - b.x0 - 80.0).abs() < 1e-6 && (moved.y0 - b.y0 - 20.0).abs() < 1e-6);
        n.appearance.effects[0].visible = false;
        assert!(revolve_art(&n).is_none());
    }
    #[test]
    fn item_effect_and_transparency_are_scoped_to_whole_surface() {
        let mut n = profile_node();
        n.opacity = 0.6;
        n.appearance.items[0].effects_mut().push(fx());
        if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
            f.opacity = 0.4;
        }
        let a = revolve_art(&n).unwrap();
        assert_eq!(a.opacity, 0.6);
        let surface = &a.children().unwrap()[0];
        assert_eq!(surface.opacity, 0.4);
        assert!(surface.children().unwrap().iter().all(|c| c.opacity == 1.0 && !has_revolve(c)));
    }
    #[test]
    fn export_bakes_faces_with_unique_ids_and_no_revolve_effects() {
        let mut d = Document::new(300.0, 300.0);
        let mut n = profile_node();
        n.id = d.alloc_id();
        n.appearance.effects.push(fx());
        let mut stroke = |_: &mut Document, _: &PathData, _: vectorcraft_geom::FillRule, _: &vectorcraft_doc::StrokeLayer| None;
        let art = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
        let mut ids = std::collections::HashSet::new();
        fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
            assert!(ids.insert(n.id));
            assert!(!has_revolve(n));
            for c in n.children().into_iter().flatten() {
                visit(c, ids);
            }
        }
        visit(&art, &mut ids);
        assert!(ids.len() > 50);
    }

    #[test]
    fn visible_surface_option_only_affects_expansion_and_can_keep_all_faces() {
        let mut n = profile_node();
        n.appearance.effects.push(fx());
        let source = n.path_data().unwrap().clone();
        let live = revolve_art(&n).unwrap();
        let expanded = expand_revolve_art(&n, None).unwrap();
        let count = |n: &Node| n.children().unwrap()[0].children().unwrap().len();
        assert!(count(&expanded) < count(&live) * 3 / 4, "hidden back surfaces are removed");
        assert_eq!(crate::bake_appearance(&n), Some(live.clone()), "other baking retains the full surface");
        n.appearance.effects[0].params["expandVisibleOnly"] = json!(false);
        assert_eq!(revolve_art(&n), Some(live.clone()), "the live view ignores the expansion setting");
        assert_eq!(expand_revolve_art(&n, None), Some(live));
        assert_eq!(n.path_data(), Some(&source));
        n.appearance.effects[0].params["expandVisibleOnly"] = json!(true);
        assert_eq!(expand_revolve_art(&n, None), Some(expanded.clone()));
        // Per-item Revolve uses the same option, without trimming another item's artwork.
        let effect = n.appearance.effects.remove(0);
        n.appearance.items[0].effects_mut().push(effect);
        assert_eq!(expand_revolve_art(&n, None).unwrap().children().unwrap()[0], expanded.children().unwrap()[0]);
    }

    #[test]
    fn new_revolves_are_capped_and_ones_saved_before_the_cap_stay_hollow() {
        let faces = |params: Value| {
            let mut n = profile_node();
            n.appearance.effects.push(Effect { params, ..fx() });
            revolve_art(&n).unwrap().children().unwrap()[0].children().unwrap().len()
        };
        // Off the axis and seen from above, the profile's ends turn into lids.
        let mut legacy = fx().params;
        legacy.as_object_mut().unwrap().remove("cap");
        legacy["offset"] = json!(20.0);
        legacy["rotationX"] = json!(-30.0);
        let solid = crate::new_effect(REVOLVE, &json!({"smoothShading": false, "offset": 20.0, "rotationX": -30.0})).unwrap();
        assert_eq!(solid.params["cap"], json!(true));
        let hollow = faces(json!({"smoothShading": false, "offset": 20.0, "rotationX": -30.0, "cap": false}));
        assert_eq!(faces(legacy.clone()), hollow, "saved without a cap: hollow, as drawn then");
        assert!(faces(solid.params) > hollow);
        assert_eq!(crate::drawing_params(REVOLVE, &legacy)["cap"], json!(false), "the dialog shows it hollow too");
    }
    #[test]
    fn graphics_wrap_round_the_surface_like_a_label() {
        // A 20 pt blue square as the symbol.
        let mut b = BezPath::new();
        b.move_to((0.0, 0.0));
        b.line_to((20.0, 0.0));
        b.line_to((20.0, 20.0));
        b.line_to((0.0, 20.0));
        b.close_path();
        let logo = Node::path(NodeId(50), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0));
        let mut doc = Document::new(400.0, 400.0);
        doc.symbols.push(vectorcraft_doc::Symbol { name: "Logo".into(), art: Arc::new(logo) });
        let laid = |extra: Value| {
            let mut n = profile_node();
            let mut p = json!({"smoothShading": false, "graphics": [{"symbol": "Logo"}]});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            n.appearance.effects.push(crate::new_effect(REVOLVE, &p).unwrap());
            let bare = revolve_art(&n).unwrap().children().unwrap()[0].children().unwrap().len();
            let art = revolve_art_in(&n, Some(&doc)).unwrap();
            let pieces = art.children().unwrap()[0].children().unwrap().clone();
            (pieces.len() - bare, pieces.last().and_then(|p| p.geometric_bounds()))
        };
        // At the front of a radius-30 surface, straight on: 2·30·sin(10/30) across, centred on
        // the axis, at the profile's middle height.
        let (count, b) = laid(json!({}));
        let b = b.unwrap();
        assert_eq!(count, 1);
        assert!((b.width() - 60.0 * (10.0f64 / 30.0).sin()).abs() < 1e-6 && (b.center().x - 100.0).abs() < 1e-6, "{b:?}");
        assert!((b.center().y - 100.0).abs() < 1e-6);
        // At the back (0 %), hidden; a quarter turn ending at the front shows its right half only.
        assert_eq!(laid(json!({"graphics": [{"symbol": "Logo", "x": 0}]})).0, 0);
        let (count, half) = laid(json!({"angle": 90}));
        assert_eq!(count, 1);
        assert!((half.unwrap().x0 - 100.0).abs() < 1e-6, "{half:?}");
    }
    #[test]
    fn smooth_shading_draws_the_surface_pixel_by_pixel() {
        let mut n = profile_node();
        n.appearance.effects.push(crate::new_effect(REVOLVE, &json!({})).unwrap());
        let art = revolve_art(&n).unwrap();
        let surface = &art.children().unwrap()[0];
        let clip = &surface.children().unwrap()[0];
        let NodeKind::Group { children, clip: true } = &clip.kind else { panic!("{:?}", clip.kind) };
        let NodeKind::Image(im) = &children[1].kind else { panic!("an image") };
        let pixels = vectorcraft_doc::generated::get(&im.key).unwrap();
        assert!(pixels.rgba.chunks(4).any(|p| p[3] == 255));
        assert_eq!(revolve_art(&n), Some(art), "the same pixels again");
    }

    #[test]
    fn transparency_unknown_paints_and_later_distortion_keep_complete_surfaces() {
        let mut n = profile_node();
        n.appearance.effects.push(fx());
        for change in 0..4 {
            let mut sample = n.clone();
            match change {
                0 => sample.opacity = 0.5,
                1 => {
                    if let AppearanceItem::Fill(f) = &mut sample.appearance.items[0] {
                        f.opacity = 0.4;
                    }
                }
                2 => sample.blend = vectorcraft_doc::color::BlendMode::Multiply,
                _ => sample.appearance.effects.push(crate::new_effect("distort.twist", &json!({"angle":45})).unwrap()),
            }
            assert_eq!(expand_revolve_art(&sample, None), revolve_art(&sample), "preserve surface for case {change}");
        }
        let mut sample = n;
        if let AppearanceItem::Fill(f) = &mut sample.appearance.items[0] {
            f.paint = Paint::Pattern { pattern: "unknown transparency".into(), xf: Affine::IDENTITY };
        }
        assert_eq!(expand_revolve_art(&sample, None), revolve_art(&sample));
    }
}
