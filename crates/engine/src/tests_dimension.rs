//! `shape.dimension`: linear, angular and radial dimension objects (the Dimension tool's art).

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    s
}

fn dim(s: &mut Session, p: Value) -> Value {
    s.execute("shape.dimension", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// The dimension group's children by name, and the label's text.
fn parts(s: &Session, id: u64) -> (Vec<String>, String) {
    let doc = &s.doc().unwrap().doc;
    let g = doc.node(NodeId(id)).unwrap();
    assert_eq!(g.name.as_deref(), Some("Dimension"));
    let NodeKind::Group { children, .. } = &g.kind else { panic!("a dimension is a group") };
    let label = children
        .iter()
        .find_map(|c| match &c.kind {
            NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        })
        .unwrap_or_default();
    (children.iter().filter_map(|c| c.name.clone()).collect(), label)
}

#[test]
fn a_linear_dimension_measures_aligned_horizontal_and_vertical() {
    let mut s = session();
    let base = json!({"type": "linear", "x1": 100, "y1": 100, "x2": 130, "y2": 140, "units": "pt"});
    let mut p = base.clone();
    p["at"] = json!([200, 100]);
    let r = dim(&mut s, p);
    assert!((r["value"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert_eq!(r["text"], "50.00 pt");
    let (names, label) = parts(&s, r["id"].as_u64().unwrap());
    assert_eq!(names, ["Extension Lines", "Dimension Line", "Ends", "Label"]);
    assert_eq!(label, "50.00 pt");
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Dimension");
    for (orientation, expect) in [("horizontal", 30.0), ("vertical", 40.0)] {
        let mut p = base.clone();
        p["orientation"] = json!(orientation);
        p["at"] = json!([115, 60]);
        let r = dim(&mut s, p);
        assert!((r["value"].as_f64().unwrap() - expect).abs() < 1e-9, "{orientation}: {r}");
    }
}

#[test]
fn units_precision_scale_and_ends() {
    let mut s = session();
    let r = dim(
        &mut s,
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 72, "y2": 0, "at": [36, -20], "units": "in", "precision": 0, "scale": 12, "showUnits": false, "ends": "tick"}),
    );
    assert_eq!(r["text"], "12");
    let r =
        dim(&mut s, json!({"type": "linear", "x1": 0, "y1": 0, "x2": 72, "y2": 0, "at": [36, -20], "units": "mm", "precision": 1, "ends": "none"}));
    assert_eq!(r["text"], "25.4 mm");
    let (names, _) = parts(&s, r["id"].as_u64().unwrap());
    assert!(!names.iter().any(|n| n == "Ends"), "no ends: {names:?}");
}

#[test]
fn an_angle_is_the_one_between_the_rays_that_holds_at() {
    let mut s = session();
    let base = json!({"type": "angular", "cx": 300, "cy": 300, "x1": 400, "y1": 300, "x2": 300, "y2": 200, "precision": 1});
    let mut p = base.clone();
    p["at"] = json!([360, 240]);
    assert_eq!(dim(&mut s, p)["text"], "90.0°");
    // On the other side of the rays: the reflex angle.
    let mut p = base.clone();
    p["at"] = json!([240, 360]);
    assert_eq!(dim(&mut s, p)["text"], "270.0°");
}

#[test]
fn radius_and_diameter() {
    let mut s = session();
    let r = dim(&mut s, json!({"type": "radius", "cx": 300, "cy": 300, "radius": 50, "at": [400, 300], "units": "pt", "precision": 0}));
    assert_eq!(r["text"], "R 50 pt");
    let r = dim(&mut s, json!({"type": "diameter", "cx": 300, "cy": 300, "radius": 50, "at": [310, 300], "units": "pt", "precision": 0}));
    assert_eq!(r["text"], "Ø 100 pt");
}

#[test]
fn bad_params_are_refused() {
    let mut s = session();
    for bad in [
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 0, "y2": 0, "at": [1, 1]}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 5, "y2": 0}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 5, "y2": 0, "at": [1, 1], "orientation": "diagonal"}),
        json!({"type": "angular", "cx": 0, "cy": 0, "x1": 0, "y1": 0, "x2": 5, "y2": 0, "at": [1, 1]}),
        json!({"type": "angular", "cx": 0, "cy": 0, "x1": 5, "y1": 0, "x2": 9, "y2": 0, "at": [0, 0]}),
        json!({"type": "radius", "cx": 0, "cy": 0, "radius": -1, "at": [1, 1]}),
        json!({"type": "spiral", "at": [1, 1]}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 5, "y2": 0, "at": [1, 1], "units": "furlongs"}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 5, "y2": 0, "at": [1, 1], "precision": 40}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 5, "y2": 0, "at": [1, 1], "ends": "flag"}),
        json!({"type": "linear", "x1": 0, "y1": 0, "x2": 1e308, "y2": 0, "at": [1, f64::MAX]}),
    ] {
        let r = s.execute("shape.dimension", &bad);
        if bad["x2"] == json!(1e308) {
            // Huge but finite: it may draw or refuse, but never panics.
            continue;
        }
        assert!(r.is_err(), "{bad}");
    }
}
