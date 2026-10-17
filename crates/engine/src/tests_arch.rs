//! `shape.arch`: nested arch bands as a group named Arch (the Arch tool's art).

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::FillRule;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// The arch group's children: (name, subpath count, fill rule, opacity, fill hex).
fn parts(s: &Session, id: u64) -> Vec<(String, usize, FillRule, f32, String)> {
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(NodeId(id)).unwrap();
    assert_eq!(g.name.as_deref(), Some("Arch"));
    let NodeKind::Group { children, .. } = &g.kind else { panic!("an arch is a group") };
    children
        .iter()
        .map(|c| {
            let NodeKind::Path { path, rule, .. } = &c.kind else { panic!("bands are paths") };
            let fill = c.appearance.fill_paint().color().map(|c| c.to_hex()).unwrap_or_default();
            (c.name.clone().unwrap(), path.subpaths.len(), *rule, c.opacity, fill)
        })
        .collect()
}

fn arch(s: &mut Session, p: Value) -> Value {
    s.execute("shape.arch", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn draws_one_band_per_count_in_the_palette_order_in_one_undo_step() {
    let mut s = session();
    let r = arch(&mut s, json!({"x": 40, "y": 40, "width": 400, "height": 300}));
    let id = r["id"].as_u64().unwrap();
    let p = parts(&s, id);
    assert_eq!(p.len(), 5);
    assert_eq!(p[0].0, "Band 1");
    assert!(p.iter().all(|(_, subpaths, rule, op, _)| *subpaths == 1 && *rule == FillRule::NonZero && *op == 1.0));
    assert_eq!(p[0].4.to_lowercase(), "#ff6fae");
    assert_eq!(p[5 - 1].4.to_lowercase(), "#3bb7f0");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.last().unwrap().label, "Arch");
    assert_eq!(st.selection.objects, vec![NodeId(id)]);
    // The outer band fills its box and stands on its bottom edge.
    let b = st.doc.bounds_of(&[NodeId(id)], false).unwrap();
    assert!((b.x0 - 40.0).abs() < 0.5 && (b.y0 - 40.0).abs() < 0.5 && (b.x1 - 440.0).abs() < 0.5 && (b.y1 - 340.0).abs() < 0.5, "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(NodeId(id)).is_none());
}

#[test]
fn bands_shrink_to_fit_a_small_box() {
    let mut s = session();
    let r = arch(&mut s, json!({"x": 0, "y": 0, "width": 100, "height": 100, "count": 5, "thickness": 120, "spacing": 40}));
    assert_eq!(r["bands"], 5);
    let (t, g) = (r["thickness"].as_f64().unwrap(), r["spacing"].as_f64().unwrap());
    assert!((5.0 * t + 4.0 * g - 40.0).abs() < 1e-9, "fills 80 % of half the width: {t} {g}");
    assert!((t / g - 3.0).abs() < 1e-9, "in proportion");
}

#[test]
fn every_shape_style_and_end_draws() {
    let mut s = session();
    for shape in super::cmd::arch::SHAPES {
        for style in super::cmd::arch::FILL_STYLES {
            for ends in ["open", "closed"] {
                let p = json!({"x": 10, "y": 10, "width": 300, "height": 200, "shape": shape, "fillStyle": style, "ends": ends, "roundness": 60, "count": 3});
                let id = arch(&mut s, p.clone())["id"].as_u64().unwrap();
                let parts = parts(&s, id);
                let highlights = matches!(style, "glossy" | "jelly" | "pearlescent");
                assert_eq!(parts.len(), if highlights { 6 } else { 3 }, "{p}");
                let band = &parts[0];
                match (ends, shape) {
                    ("closed", _) => assert_eq!((band.1, band.2), (2, FillRule::EvenOdd), "a ring: {p}"),
                    (_, "segmented") => assert_eq!(band.1, 7, "seven blocks: {p}"),
                    _ => assert_eq!(band.1, 1, "{p}"),
                }
                if style == "jelly" {
                    assert!(band.3 < 1.0);
                }
                let preview = super::cmd::arch::preview_document(&p, 120.0, 80.0).unwrap();
                let b = preview.bounds_of(&[preview.layers[0].children().unwrap()[0].id], false).unwrap();
                assert!(b.x0 >= 0.0 && b.y0 >= 0.0 && b.x1 <= 120.0 && b.y1 <= 80.0, "the preview fits: {b:?}");
            }
        }
    }
}

#[test]
fn colours_repeat_and_bad_params_are_refused() {
    let mut s = session();
    let id = arch(&mut s, json!({"x": 0, "y": 0, "width": 300, "height": 200, "count": 3, "colors": ["#112233", "#445566"]}))["id"].as_u64().unwrap();
    let fills: Vec<String> = parts(&s, id).into_iter().map(|p| p.4.to_lowercase()).collect();
    assert_eq!(fills, ["#112233", "#445566", "#112233"]);
    for bad in [
        json!({"x": 0, "y": 0, "width": 0, "height": 10}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "count": 0}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "count": 1000}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "shape": "heart"}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "colors": []}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "colors": ["no"]}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "ends": "both"}),
        json!({"x": 0, "y": 0, "width": 10, "height": 10, "thickness": -1}),
        json!({"x": 0, "y": 0, "width": 1e308, "height": 1e308, "roundness": 1000}),
    ] {
        assert!(s.execute("shape.arch", &bad).is_err(), "{bad}");
    }
}
