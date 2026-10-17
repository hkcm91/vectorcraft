//! Dimension tool: measure art and leave the measurement on the artboard as a dimension object
//! (`shape.dimension`).
//!
//! - Linear: click a straight side of an object (it lights up under the pointer) or two points,
//!   or drag from one point to the other; then move the pointer to place the dimension line and
//!   click. In Auto orientation, moving above or below the two points draws a horizontal
//!   dimension, beside them a vertical one, and between them an aligned one; Shift keeps it
//!   aligned.
//! - Angular: click a straight side, then another; move the pointer into the angle to measure
//!   (between the lines, or one of the angles beside it) and click.
//! - Radial: click the edge of a circle; move outwards for its radius or inwards for its
//!   diameter, and click.
//!
//! Escape cancels. The options (type, orientation, units, precision, scale, ends, line weight,
//! font size, colour) last and are what the next dimension is drawn with.

use serde_json::{Map, Value, json};
use vectorcraft_geom::{Point, Vec2};

use crate::distort::{BLUE, ellipse_path};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "dimension";
/// The dimension types the tool draws.
pub const TYPES: [&str; 3] = ["linear", "angular", "radial"];
/// A linear dimension's orientations.
pub const ORIENTATIONS: [&str; 4] = ["auto", "aligned", "horizontal", "vertical"];
/// The ends of a dimension line.
pub const ENDS: [&str; 4] = ["arrow", "tick", "dot", "none"];
/// The style options as `shape.dimension` takes them: (key, default, least, most).
pub const NUMBERS: [(&str, f64, f64, f64); 4] =
    [("precision", 2.0, 0.0, 6.0), ("scale", 1.0, 0.000_001, 1_000_000.0), ("strokeWidth", 0.5, 0.0, 100.0), ("fontSize", 9.0, 0.5, 1000.0)];

/// What is being measured once its points are known.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Geom {
    Linear(Point, Point),
    /// Vertex and the two lines' directions.
    Angular(Point, Vec2, Vec2),
    /// Centre and radius.
    Radial(Point, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    /// A linear dimension's first point (clicked, not on a side).
    FirstPoint(Point),
    /// A linear dimension being dragged out: (from, to).
    Dragging(Point, Point),
    /// An angular dimension's first line: its ends.
    FirstLine(Point, Point),
    /// Placing the dimension line; whether the interaction has begun.
    Placing(Geom, bool),
}

pub struct DimensionTool {
    stage: Stage,
    press: Option<Point>,
    hover: Point,
    mods: Mods,
    kind: &'static str,
    orientation: &'static str,
    units: String,
    ends: &'static str,
    color: String,
    show_units: bool,
    /// [`NUMBERS`]' values, in order.
    numbers: [f64; 4],
}

impl Default for DimensionTool {
    fn default() -> Self {
        Self {
            stage: Stage::Idle,
            press: None,
            hover: Point::ZERO,
            mods: Mods::default(),
            kind: "linear",
            orientation: "auto",
            units: "document".into(),
            ends: "arrow",
            color: "#000000".into(),
            show_units: true,
            numbers: NUMBERS.map(|(_, d, ..)| d),
        }
    }
}

fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l.is_finite() && l > 1e-9).then(|| v / l)
}

/// The straight side of an object under `p` (within the pick tolerance): its two ends.
pub fn straight_side_at(cx: &ToolContext, p: Point) -> Option<(Point, Point)> {
    let hit = vectorcraft_doc::hit::hit_test_visible(cx.doc, p, cx.hit_options())?;
    let pd = cx.doc.node(hit.leaf)?.path_data()?;
    let (si, seg, _, _, dist) = pd.nearest(p)?;
    let sp = pd.subpaths.get(si)?;
    if dist > cx.pick_tol() || !sp.segment_is_line(seg) {
        return None;
    }
    let n = sp.anchors.len();
    let (a, b) = (sp.anchors.get(seg % n.max(1))?.p, sp.anchors.get((seg + 1) % n.max(1))?.p);
    (a.distance(b) > 1e-9).then_some((a, b))
}

/// The circle whose edge is under `p`: its centre and radius. Only closed paths whose anchors and
/// segment middles all lie on one circle count.
pub fn circle_at(cx: &ToolContext, p: Point) -> Option<(Point, f64)> {
    let hit = vectorcraft_doc::hit::hit_test_visible(cx.doc, p, cx.hit_options())?;
    let pd = cx.doc.node(hit.leaf)?.path_data()?;
    let [sp] = pd.subpaths.as_slice() else { return None };
    if !sp.closed || sp.anchors.len() < 2 {
        return None;
    }
    let n = sp.anchors.len() as f64;
    let c = sp.anchors.iter().fold(Vec2::ZERO, |acc, a| acc + a.p.to_vec2()) / n;
    let c = c.to_point();
    let r = sp.anchors.iter().map(|a| a.p.distance(c)).sum::<f64>() / n;
    if !(r.is_finite() && r > 1e-6) {
        return None;
    }
    let on = |q: Point| (q.distance(c) - r).abs() <= r * 0.01;
    let round = sp.anchors.iter().all(|a| on(a.p)) && (0..sp.segment_count()).all(|i| on(vectorcraft_geom::ParamCurve::eval(&sp.segment(i), 0.5)));
    (round && (p.distance(c) - r).abs() <= cx.pick_tol()).then_some((c, r))
}

/// The point where the lines through `p1` along `d1` and through `p2` along `d2` meet.
fn intersect(p1: Point, d1: Vec2, p2: Point, d2: Vec2) -> Option<Point> {
    let den = cross(d1, d2);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = cross(p2 - p1, d2) / den;
    let q = p1 + d1 * t;
    (q.x.is_finite() && q.y.is_finite()).then_some(q)
}

/// The two rays, one along each line, that bound the quarter of the plane holding `w` (from
/// the vertex).
fn rays(d1: Vec2, d2: Vec2, w: Vec2) -> (Vec2, Vec2) {
    let sign = |v: f64| if v >= 0.0 { 1.0 } else { -1.0 };
    let r1 = d1 * (sign(cross(d2, d1)) * sign(cross(d2, w)));
    let r2 = d2 * (sign(cross(d1, d2)) * sign(cross(d1, w)));
    (r1, r2)
}

/// A linear dimension's orientation for points `a` and `b` placed at `at`: Auto picks horizontal
/// above or below them, vertical beside them, else aligned.
fn orientation(o: &str, a: Point, b: Point, at: Point, aligned: bool) -> &'static str {
    match o {
        _ if aligned => "aligned",
        "horizontal" => "horizontal",
        "vertical" => "vertical",
        "aligned" => "aligned",
        _ => {
            let (x0, x1, y0, y1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
            let (in_x, in_y) = ((x0..=x1).contains(&at.x), (y0..=y1).contains(&at.y));
            match (in_x, in_y) {
                (true, false) => "horizontal",
                (false, true) => "vertical",
                _ => "aligned",
            }
        }
    }
}

impl DimensionTool {
    /// The style params every dimension takes from the options.
    fn style(&self) -> Map<String, Value> {
        let mut m = Map::new();
        for ((key, ..), v) in NUMBERS.iter().zip(self.numbers) {
            m.insert(key.to_string(), json!(v));
        }
        if self.units != "document" {
            m.insert("units".into(), json!(self.units));
        }
        m.insert("ends".into(), json!(self.ends));
        m.insert("color".into(), json!(self.color));
        m.insert("showUnits".into(), json!(self.show_units));
        m
    }

    /// `shape.dimension`'s params for `g` placed at `at`.
    fn params(&self, g: Geom, at: Point) -> Option<Value> {
        let mut m = self.style();
        m.insert("at".into(), json!([at.x, at.y]));
        let put = |m: &mut Map<String, Value>, pairs: &[(&str, f64)]| {
            for (k, v) in pairs {
                m.insert(k.to_string(), json!(v));
            }
        };
        match g {
            Geom::Linear(a, b) => {
                m.insert("type".into(), json!("linear"));
                m.insert("orientation".into(), json!(orientation(self.orientation, a, b, at, self.mods.shift)));
                put(&mut m, &[("x1", a.x), ("y1", a.y), ("x2", b.x), ("y2", b.y)]);
            }
            Geom::Angular(c, d1, d2) => {
                let w = at - c;
                if w.hypot() < 1e-6 {
                    return None;
                }
                let (r1, r2) = rays(d1, d2, w);
                let (p1, p2) = (c + r1 * 10.0, c + r2 * 10.0);
                m.insert("type".into(), json!("angular"));
                put(&mut m, &[("cx", c.x), ("cy", c.y), ("x1", p1.x), ("y1", p1.y), ("x2", p2.x), ("y2", p2.y)]);
            }
            Geom::Radial(c, r) => {
                m.insert("type".into(), json!(if at.distance(c) < r { "diameter" } else { "radius" }));
                put(&mut m, &[("cx", c.x), ("cy", c.y), ("radius", r)]);
            }
        }
        Some(Value::Object(m))
    }

    /// Start placing `g` with the pointer at `at`.
    fn place(&mut self, g: Geom, at: Point) -> Vec<Action> {
        self.stage = Stage::Placing(g, true);
        let mut out = vec![Action::Begin("Dimension".into())];
        out.extend(self.params(g, at).map(|p| Action::Preview("shape.dimension".into(), p)));
        out
    }

    /// A click (no drag) at `p`.
    fn click(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        match (self.kind, self.stage) {
            ("linear", Stage::FirstPoint(a)) => {
                let b = crate::guides::snap_draw(cx, p, &[]).0;
                if b.distance(a) < 1e-6 {
                    return vec![];
                }
                self.place(Geom::Linear(a, b), p)
            }
            ("linear", _) => match straight_side_at(cx, p) {
                Some((a, b)) => self.place(Geom::Linear(a, b), p),
                None => {
                    self.stage = Stage::FirstPoint(crate::guides::snap_draw(cx, p, &[]).0);
                    vec![]
                }
            },
            ("angular", stage) => {
                // Only a straight side starts or finishes an angle (it lights up under the pointer).
                let Some((a, b)) = straight_side_at(cx, p) else { return vec![] };
                match stage {
                    Stage::FirstLine(p1, q1) => {
                        let (Some(d1), Some(d2)) = (unit(q1 - p1), unit(b - a)) else { return vec![] };
                        match intersect(p1, d1, a, d2) {
                            Some(c) => self.place(Geom::Angular(c, d1, d2), p),
                            // Parallel lines make no angle: wait for another side.
                            None => vec![],
                        }
                    }
                    _ => {
                        self.stage = Stage::FirstLine(a, b);
                        vec![]
                    }
                }
            }
            _ => match circle_at(cx, p) {
                Some((c, r)) => self.place(Geom::Radial(c, r), p),
                None => vec![],
            },
        }
    }

    fn reset(&mut self) -> Vec<Action> {
        let began = matches!(self.stage, Stage::Placing(_, true));
        self.stage = Stage::Idle;
        self.press = None;
        if began { vec![Action::Cancel] } else { vec![] }
    }
}

impl Tool for DimensionTool {
    fn id(&self) -> &'static str {
        ID
    }
    fn busy(&self) -> bool {
        self.stage != Stage::Idle || self.press.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.hover = ev.pos;
        self.mods = ev.mods;
        match ev.kind {
            PointerKind::Move => match self.stage {
                Stage::Placing(g, _) => self.params(g, ev.pos).map(|p| vec![Action::Preview("shape.dimension".into(), p)]).unwrap_or_default(),
                _ => vec![],
            },
            PointerKind::Down => {
                self.press = Some(ev.pos);
                vec![]
            }
            PointerKind::Drag => {
                match self.stage {
                    Stage::Placing(g, _) => {
                        return self.params(g, ev.pos).map(|p| vec![Action::Preview("shape.dimension".into(), p)]).unwrap_or_default();
                    }
                    Stage::Dragging(a, _) => {
                        let b = if ev.mods.shift {
                            a + vectorcraft_geom::constrain_angle(ev.pos - a, 45.0)
                        } else {
                            crate::guides::snap_draw(cx, ev.pos, &[]).0
                        };
                        self.stage = Stage::Dragging(a, b);
                    }
                    Stage::Idle if self.kind == "linear" => {
                        if let Some(s) = self.press.filter(|s| s.distance(ev.pos) >= cx.tol(3.0)) {
                            let a = crate::guides::snap_draw(cx, s, &[]).0;
                            self.stage = Stage::Dragging(a, crate::guides::snap_draw(cx, ev.pos, &[]).0);
                        }
                    }
                    _ => {}
                }
                vec![]
            }
            PointerKind::Up => {
                let Some(_) = self.press.take() else { return vec![] };
                match self.stage {
                    Stage::Placing(g, _) => {
                        self.stage = Stage::Idle;
                        let mut out: Vec<Action> = self.params(g, ev.pos).map(|p| Action::Preview("shape.dimension".into(), p)).into_iter().collect();
                        out.push(Action::Commit);
                        out
                    }
                    Stage::Dragging(a, b) if a.distance(b) > 1e-6 => self.place(Geom::Linear(a, b), ev.pos),
                    Stage::Dragging(..) => {
                        self.stage = Stage::Idle;
                        vec![]
                    }
                    _ => self.click(cx, ev.pos),
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.busy() { self.reset() } else { vec![] }
    }
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.busy()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.reset()
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let lit = cx.smart_guide_color;
        let mut o = vec![];
        match self.stage {
            Stage::Idle | Stage::FirstLine(..) if self.kind != "radial" => {
                if let Some((a, b)) = straight_side_at(cx, self.hover) {
                    o.push(Overlay::Line { a, b, color: lit, dashed: false });
                }
            }
            Stage::Idle => {
                if let Some((c, r)) = circle_at(cx, self.hover) {
                    o.push(Overlay::Path { path: ellipse_path(c, r, r, 0.0), color: lit, width: 1.5, dashed: false });
                }
            }
            _ => {}
        }
        match self.stage {
            Stage::FirstLine(a, b) => o.push(Overlay::Line { a, b, color: BLUE, dashed: false }),
            Stage::FirstPoint(a) => o.push(Overlay::Line { a, b: self.hover, color: BLUE, dashed: true }),
            Stage::Dragging(a, b) => {
                o.push(Overlay::Line { a, b, color: BLUE, dashed: true });
                if cx.measurement_labels {
                    o.push(Overlay::Measure { p: b, text: format!("D: {}", cx.len(a.distance(b))) });
                }
            }
            _ => {}
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        let mut m = self.style();
        m.insert("type".into(), json!(self.kind));
        m.insert("orientation".into(), json!(self.orientation));
        m.insert("units".into(), json!(self.units));
        Value::Object(m)
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        let pick = |list: &[&'static str], v: &Value| v.as_str().and_then(|s| list.iter().copied().find(|k| *k == s));
        match key {
            "type" => {
                if let Some(t) = pick(&TYPES, v) {
                    self.kind = t;
                    self.stage = Stage::Idle;
                }
            }
            "orientation" => self.orientation = pick(&ORIENTATIONS, v).unwrap_or(self.orientation),
            "ends" => self.ends = pick(&ENDS, v).unwrap_or(self.ends),
            "units" => {
                if let Some(u) = v.as_str().filter(|u| *u == "document" || vectorcraft_doc::Unit::named(u).is_some()) {
                    self.units = u.to_string();
                }
            }
            "color" => {
                if let Some(c) = v.as_str().filter(|c| vectorcraft_color::Color::from_hex(c).is_some()) {
                    self.color = c.to_string();
                }
            }
            "showUnits" => self.show_units = v.as_bool().unwrap_or(true),
            _ => {
                let Some((slot, (_, default, lo, hi))) = self.numbers.iter_mut().zip(NUMBERS).find(|(_, (k, ..))| *k == key) else { return };
                let x = v.as_f64().filter(|x| x.is_finite()).unwrap_or(default).clamp(lo, hi);
                *slot = if key == "precision" { x.round() } else { x };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Node, Selection};
    use vectorcraft_geom::{Rect, shapes};

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    fn click(t: &mut DimensionTool, cx: &ToolContext, x: f64, y: f64) -> Vec<Action> {
        t.pointer(cx, &ev(PointerKind::Down, x, y));
        t.pointer(cx, &ev(PointerKind::Up, x, y))
    }

    fn preview(a: &[Action]) -> Value {
        a.iter()
            .rev()
            .find_map(|a| match a {
                Action::Preview(c, v) if c == "shape.dimension" => Some(v.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no preview in {a:?}"))
    }

    #[test]
    fn a_side_then_a_click_places_a_linear_dimension() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        // The rectangle's top side (100,100)–(200,100) lights up and is picked.
        let mut t = DimensionTool { hover: Point::new(150.0, 100.5), ..DimensionTool::default() };
        assert!(matches!(t.overlays(&cx).first(), Some(Overlay::Line { .. })));
        let a = click(&mut t, &cx, 150.0, 100.5);
        assert_eq!(a.first(), Some(&Action::Begin("Dimension".into())));
        // Above it: a horizontal dimension of the side.
        let a = t.pointer(&cx, &ev(PointerKind::Move, 150.0, 60.0));
        let v = preview(&a);
        assert_eq!((v["type"].as_str(), v["orientation"].as_str()), (Some("linear"), Some("horizontal")));
        assert_eq!((v["x1"].as_f64(), v["x2"].as_f64()), (Some(100.0), Some(200.0)));
        assert_eq!(v["at"], json!([150.0, 60.0]));
        let a = click(&mut t, &cx, 150.0, 60.0);
        assert_eq!(a.last(), Some(&Action::Commit));
        assert!(!t.busy());
    }

    #[test]
    fn two_points_or_a_drag_and_escape_cancels() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        cx.snap_to_point = false;
        let mut t = DimensionTool::default();
        assert!(click(&mut t, &cx, 300.0, 300.0).is_empty());
        let a = click(&mut t, &cx, 400.0, 350.0);
        let v = preview(&a);
        assert_eq!((v["x1"].as_f64(), v["y2"].as_f64(), v["orientation"].as_str()), (Some(300.0), Some(350.0), Some("aligned")));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        // A drag from one point to the other.
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 400.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 350.0, 400.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 380.0, 400.0));
        let v = preview(&t.pointer(&cx, &ev(PointerKind::Up, 380.0, 400.0)));
        assert_eq!((v["x2"].as_f64(), v["y2"].as_f64()), (Some(380.0), Some(400.0)));
        // Beside the points, Auto turns vertical; Shift keeps it aligned.
        assert_eq!(preview(&t.pointer(&cx, &ev(PointerKind::Move, 450.0, 400.0)))["orientation"], "vertical");
        let shift = Mods { shift: true, ..Default::default() };
        assert_eq!(preview(&t.pointer(&cx, &ev(PointerKind::Move, 450.0, 400.0).with_mods(shift)))["orientation"], "aligned");
    }

    #[test]
    fn two_sides_make_an_angle_and_a_circle_a_radius_or_diameter() {
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 1, Node::path(id, shapes::ellipse(Rect::new(300.0, 300.0, 400.0, 400.0)), Appearance::default_art())).unwrap();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = DimensionTool::default();
        t.set_option("type", &json!("angular"));
        // The top side, then the right side: the corner at (200, 100) is the vertex.
        assert!(click(&mut t, &cx, 150.0, 100.5).is_empty());
        let v = preview(&click(&mut t, &cx, 199.5, 150.0));
        assert_eq!((v["cx"].as_f64(), v["cy"].as_f64()), (Some(200.0), Some(100.0)));
        // Inside the rectangle: the rays run left and down.
        let v = preview(&t.pointer(&cx, &ev(PointerKind::Move, 180.0, 120.0)));
        let (r1, r2) =
            (Point::new(v["x1"].as_f64().unwrap(), v["y1"].as_f64().unwrap()), Point::new(v["x2"].as_f64().unwrap(), v["y2"].as_f64().unwrap()));
        assert!(r1.x < 200.0 && (r1.y - 100.0).abs() < 1e-9, "{r1:?}");
        assert!(r2.y > 100.0 && (r2.x - 200.0).abs() < 1e-9, "{r2:?}");
        t.reset();

        t.set_option("type", &json!("radial"));
        assert!(click(&mut t, &cx, 150.0, 150.0).is_empty(), "not a circle");
        assert!(!t.busy());
        let a = click(&mut t, &cx, 400.0, 350.0);
        let v = preview(&a);
        assert_eq!((v["cx"].as_f64(), v["radius"].as_f64()), (Some(350.0), Some(50.0)));
        assert_eq!(preview(&t.pointer(&cx, &ev(PointerKind::Move, 450.0, 350.0)))["type"], "radius");
        assert_eq!(preview(&t.pointer(&cx, &ev(PointerKind::Move, 360.0, 350.0)))["type"], "diameter");
    }

    #[test]
    fn options_stay_valid() {
        let mut t = DimensionTool::default();
        t.set_option("type", &json!("nonsense"));
        t.set_option("units", &json!("mm"));
        t.set_option("units", &json!("furlongs"));
        t.set_option("precision", &json!(99));
        t.set_option("color", &json!("red-ish"));
        t.set_option("ends", &json!("tick"));
        let o = t.options();
        assert_eq!((o["type"].as_str(), o["units"].as_str(), o["precision"].as_f64()), (Some("linear"), Some("mm"), Some(6.0)));
        assert_eq!((o["color"].as_str(), o["ends"].as_str()), (Some("#000000"), Some("tick")));
    }
}
