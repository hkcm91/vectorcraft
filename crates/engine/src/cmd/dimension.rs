//! `shape.dimension`: the Dimension tool's art. A dimension object is a group named Dimension:
//! extension lines, the dimension line (an arc for an angle), its ends (arrowheads, ticks or
//! dots) and the measurement as point type. Linear dimensions are aligned with the two points,
//! horizontal or vertical; angular ones measure the angle between two rays from a vertex; radial
//! ones a circle's radius or diameter.
//!
//! Behaviour from the public documentation of the Dimension tool (linear, angular and radial
//! dimensions; a dimension line, extension lines and dimension text; units, precision, scale,
//! line weight, ends, font size and colour set in the tool's options):
//! <https://helpx.adobe.com/illustrator/using/measure-and-show-dimensions.html>,
//! <https://helpx.adobe.com/illustrator/desktop/measure-and-align/plot-and-measure/plot-linear-dimensions-of-objects.html>,
//! <https://helpx.adobe.com/illustrator/desktop/measure-and-align/plot-and-measure/plot-angular-dimensions-of-objects.html>,
//! <https://helpx.adobe.com/illustrator/desktop/measure-and-align/plot-and-measure/plot-radial-dimensions-of-objects.html>.
//! The geometry (gaps, overshoot, arrowhead size, label placement) is our own.

use std::f64::consts::{FRAC_PI_2, TAU};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Justify, LineCap, Node, NodeKind, TextObject, Unit};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Vec2};

use super::*;

/// The kinds of dimension `shape.dimension` draws.
pub const KINDS: [&str; 4] = ["linear", "angular", "radius", "diameter"];
/// A linear dimension's orientations.
pub const ORIENTATIONS: [&str; 3] = ["aligned", "horizontal", "vertical"];
/// The ends of a dimension line.
pub const ENDS: [&str; 4] = ["arrow", "tick", "dot", "none"];

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "shape.dimension",
        "Dimension",
        [],
        None,
        "{type: linear|angular|radius|diameter, x1, y1, x2, y2 (linear: the two points measured; angular: a point on each ray), orientation?: aligned (default)|horizontal|vertical (linear), cx, cy (angular: the vertex; radius and diameter: the circle's centre), radius (radius and diameter: the circle's), at: [x, y] (where the dimension line goes: a linear one passes through it, an angle's arc and a radial leader reach it; the angle measured is the one between the rays that holds it), units?: pt|px|in|mm|cm|m|ft|yd|pc (default the General unit), precision=2 (0–6 decimals), scale=1 (the measured length is multiplied by it: 1 pt of art stands for `scale` units), ends?: arrow (default)|tick|dot|none, strokeWidth=0.5 (pt), fontSize=9 (pt), color?: \"#rrggbb\" (lines, ends and text; default black), showUnits=true} a dimension object as a group named Dimension: extension lines, the dimension line, its ends and the measurement as point type → {id, value (in units, or degrees), text}",
        has_doc,
        dimension
    )]
}

/// How a dimension looks.
#[derive(Clone, Debug)]
struct Style {
    unit: Unit,
    precision: usize,
    scale: f64,
    ends: &'static str,
    stroke: f64,
    font: f64,
    color: Color,
    show_units: bool,
}

impl Style {
    fn parse(s: &Session, p: &Value) -> Result<Style> {
        let num = |key: &str, default: f64, lo: f64, hi: f64| -> Result<f64> {
            match p.get(key) {
                None | Some(Value::Null) => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|f| f.is_finite() && (lo..=hi).contains(f))
                    .ok_or_else(|| bad(C, format!("{key} must be a number from {lo} to {hi}"))),
            }
        };
        let unit = match str_param(p, "units") {
            None => s.general_unit(),
            Some(u) => Unit::named(u).ok_or_else(|| bad(C, format!("unknown units `{u}`")))?,
        };
        let ends_name = str_param(p, "ends").unwrap_or("arrow");
        let ends = ENDS.iter().copied().find(|e| *e == ends_name).ok_or_else(|| bad(C, format!("ends must be one of {}", ENDS.join(", "))))?;
        let color = match p.get("color") {
            None | Some(Value::Null) => Color::BLACK,
            Some(c) => {
                c.as_str().and_then(Color::from_hex).or_else(|| color_value(c)).ok_or_else(|| bad(C, "color must be a colour such as \"#202020\""))?
            }
        };
        Ok(Style {
            unit,
            precision: num("precision", 2.0, 0.0, 6.0)?.round() as usize,
            scale: num("scale", 1.0, 1e-9, 1e9)?,
            ends,
            stroke: num("strokeWidth", 0.5, 0.0, 100.0)?,
            font: num("fontSize", 9.0, 0.5, 1000.0)?,
            color,
            show_units: bool_or(p, "showUnits", true),
        })
    }

    /// A length in points as the label shows it.
    fn length(&self, pt: f64) -> (f64, String) {
        let v = self.unit.from_pt(pt * self.scale);
        let mut t = format!("{v:.*}", self.precision);
        if t.starts_with('-') && t.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') {
            t = t.trim_start_matches('-').to_string();
        }
        if self.show_units {
            t = format!("{t} {}", self.unit.suffix());
        }
        (v, t)
    }

    /// How far the ends reach along the line.
    fn end_size(&self) -> f64 {
        4.0 + 6.0 * self.stroke.min(4.0)
    }
    /// The gap between the art and an extension line, and between a line and its label.
    fn gap(&self) -> f64 {
        2.0 + self.stroke
    }
}

const C: &str = "shape.dimension";

fn point(p: &Value, x: &str, y: &str) -> Result<Point> {
    let pt = Point::new(f64_req(p, x, C)?, f64_req(p, y, C)?);
    if pt.x.is_finite() && pt.y.is_finite() { Ok(pt) } else { Err(bad(C, format!("{x} and {y} must be finite"))) }
}

fn unit_vec(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l.is_finite() && l > 1e-9).then(|| v / l)
}

/// The art a dimension is drawn with.
#[derive(Default)]
struct Art {
    extension: BezPath,
    line: BezPath,
    /// (tip, direction it points) per end.
    ends: Vec<(Point, Vec2)>,
    /// (baseline origin, baseline direction, text).
    label: Option<(Point, Vec2, String)>,
}

/// The label for `text` along the segment `a`–`b`, on the side `away` points to (a unit vector),
/// centred and reading left to right.
fn label_along(style: &Style, a: Point, b: Point, away: Vec2, text: String) -> Option<(Point, Vec2, String)> {
    let mut v = unit_vec(b - a)?;
    // Read left to right (and bottom to top on a vertical line).
    if v.x < -1e-9 || (v.x.abs() <= 1e-9 && v.y > 0.0) {
        v = -v;
    }
    // In y-down space, text rotated to run along v has its top towards (v.y, -v.x).
    let up = Vec2::new(v.y, -v.x);
    let mid = a.midpoint(b);
    let cap = style.font * 0.72;
    let origin = if up.dot(away) >= 0.0 { mid + away * style.gap() } else { mid + away * (style.gap() + cap) };
    Some((origin, v, text))
}

fn linear(style: &Style, p: &Value, at: Point) -> Result<(Art, f64, String)> {
    let a = point(p, "x1", "y1")?;
    let b = point(p, "x2", "y2")?;
    let u = match str_param(p, "orientation").unwrap_or("aligned") {
        "aligned" => unit_vec(b - a).ok_or_else(|| bad(C, "the two points must differ"))?,
        "horizontal" => Vec2::new(1.0, 0.0),
        "vertical" => Vec2::new(0.0, 1.0),
        o => return Err(bad(C, format!("orientation must be one of {}, not `{o}`", ORIENTATIONS.join(", ")))),
    };
    let n = Vec2::new(-u.y, u.x);
    let len = (b - a).dot(u).abs();
    // The dimension line passes through `at`, parallel to u.
    let (oa, ob) = ((at - a).dot(n), (at - b).dot(n));
    let (la, lb) = (a + n * oa, b + n * ob);
    let side = if (oa + ob) >= 0.0 { n } else { -n };
    let mut art = Art::default();
    let (gap, over) = (style.gap(), style.gap() + 1.0);
    for (from, to, o) in [(a, la, oa), (b, lb, ob)] {
        if o.abs() > gap {
            let s = if o >= 0.0 { n } else { -n };
            art.extension.move_to(from + s * gap);
            art.extension.line_to(to + s * over);
        }
    }
    art.line.move_to(la);
    art.line.line_to(lb);
    if let Some(d) = unit_vec(lb - la) {
        art.ends.push((la, -d));
        art.ends.push((lb, d));
    }
    let (value, text) = style.length(len);
    art.label = label_along(style, la, lb, side, text.clone());
    Ok((art, value, text))
}

/// A circular arc of radius `r` about `c` from angle `a0` sweeping `sweep` radians (y-down: a
/// positive sweep turns clockwise on screen), as cubics.
fn arc(bp: &mut BezPath, c: Point, r: f64, a0: f64, sweep: f64) {
    let n = (sweep.abs() / FRAC_PI_2).ceil().clamp(1.0, 8.0) as usize;
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |a: f64| c + Vec2::new(a.cos(), a.sin()) * r;
    let tan = |a: f64| Vec2::new(-a.sin(), a.cos()) * r;
    bp.move_to(at(a0));
    for i in 0..n {
        let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        bp.curve_to(at(s) + tan(s) * k, at(e) - tan(e) * k, at(e));
    }
}

fn angular(style: &Style, p: &Value, at: Point) -> Result<(Art, f64, String)> {
    let c = point(p, "cx", "cy")?;
    let p1 = point(p, "x1", "y1")?;
    let p2 = point(p, "x2", "y2")?;
    let (r1, r2) = (
        unit_vec(p1 - c).ok_or_else(|| bad(C, "x1, y1 must differ from the vertex"))?,
        unit_vec(p2 - c).ok_or_else(|| bad(C, "x2, y2 must differ from the vertex"))?,
    );
    let r = (at - c).hypot();
    if !(r.is_finite() && r > 1e-6) {
        return Err(bad(C, "`at` must differ from the vertex"));
    }
    let (a1, a2) = (r1.y.atan2(r1.x), r2.y.atan2(r2.x));
    let beta = (at.y - c.y).atan2(at.x - c.x);
    let s = (a2 - a1).rem_euclid(TAU);
    // The sector from ray 1 to ray 2 (increasing angle) or the other one, whichever holds `at`.
    let (start, sweep) = if (beta - a1).rem_euclid(TAU) <= s { (a1, s) } else { (a2, TAU - s) };
    if sweep < 1e-9 {
        return Err(bad(C, "the rays must not lie on one another"));
    }
    let mut art = Art::default();
    arc(&mut art.line, c, r, start, sweep);
    let (gap, over) = (style.gap(), style.gap() + 1.0);
    for (q, dir) in [(p1, r1), (p2, r2)] {
        let d = (q - c).hypot();
        if r > d + gap {
            art.extension.move_to(q + dir * gap);
            art.extension.line_to(c + dir * (r + over));
        }
    }
    let end = start + sweep;
    art.ends.push((c + Vec2::new(start.cos(), start.sin()) * r, Vec2::new(start.sin(), -start.cos())));
    art.ends.push((c + Vec2::new(end.cos(), end.sin()) * r, Vec2::new(-end.sin(), end.cos())));
    let degrees = sweep.to_degrees();
    let text = format!("{degrees:.*}°", style.precision);
    // The label sits outside the arc's middle, horizontal.
    let mid = start + sweep / 2.0;
    let dir = Vec2::new(mid.cos(), mid.sin());
    // Horizontal centred type: pushed out by about half its width sideways and its cap height up
    // or down, so it clears the arc whichever way the middle of the angle faces.
    let cap = style.font * 0.72;
    let half_width = text.chars().count() as f64 * style.font * 0.28;
    let edge = c + dir * (r + style.gap());
    let origin = edge + Vec2::new(dir.x * half_width, (dir.y + 1.0) * cap * 0.5);
    art.label = Some((origin, Vec2::new(1.0, 0.0), text.clone()));
    Ok((art, degrees, text))
}

fn radial(style: &Style, p: &Value, at: Point, diameter: bool) -> Result<(Art, f64, String)> {
    let c = point(p, "cx", "cy")?;
    let r = f64_req(p, "radius", C)?;
    if !(r.is_finite() && r > 0.0) {
        return Err(bad(C, "radius must be a positive number"));
    }
    let w = unit_vec(at - c).unwrap_or(Vec2::new(1.0, 0.0));
    let edge = c + w * r;
    let mut art = Art::default();
    let (value, number) = style.length(if diameter { 2.0 * r } else { r });
    let (text, a, b) = if diameter {
        let far = c - w * r;
        art.line.move_to(far);
        art.line.line_to(edge);
        art.ends.push((far, -w));
        art.ends.push((edge, w));
        (format!("Ø {number}"), far, edge)
    } else {
        // From the centre to the edge, and on out to `at` when it lies outside the circle.
        let out = if (at - c).hypot() > r { at } else { edge };
        art.line.move_to(c);
        art.line.line_to(out);
        art.ends.push((edge, w));
        // The label goes along the leader outside the circle when there is one, clear of the
        // arrowhead at the edge.
        let (a, b) = if out.distance(edge) > style.end_size() * 3.0 { (edge, out) } else { (c, edge) };
        (format!("R {number}"), a, b)
    };
    let away = Vec2::new(w.y, -w.x);
    let away = if away.y > 0.0 { -away } else { away };
    art.label = label_along(style, a, b, away, text.clone());
    Ok((art, value, text))
}

/// The ends' art: filled arrowheads or dots, or stroked ticks.
fn ends(style: &Style, at: &[(Point, Vec2)]) -> (BezPath, bool) {
    let mut bp = BezPath::new();
    let len = style.end_size();
    for &(tip, dir) in at {
        let Some(d) = unit_vec(dir) else { continue };
        let n = Vec2::new(-d.y, d.x);
        match style.ends {
            "arrow" => {
                let base = tip - d * len;
                bp.move_to(tip);
                bp.line_to(base + n * (len * 0.35));
                bp.line_to(base - n * (len * 0.35));
                bp.close_path();
            }
            "tick" => {
                let k = (d + n) * (len * 0.4);
                bp.move_to(tip - k);
                bp.line_to(tip + k);
            }
            "dot" => {
                let r = len * 0.22;
                arc(&mut bp, tip, r, 0.0, TAU);
                bp.close_path();
            }
            _ => {}
        }
    }
    (bp, style.ends != "tick")
}

fn dimension(s: &mut Session, p: &Value) -> Result<Value> {
    let style = Style::parse(s, p)?;
    let at = p.get("at").and_then(finite_numbers::<2>).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(C, "`at` must be [x, y]"))?;
    let (art, value, text) = match str_param(p, "type").unwrap_or("linear") {
        "linear" => linear(&style, p, at)?,
        "angular" => angular(&style, p, at)?,
        "radius" => radial(&style, p, at, false)?,
        "diameter" => radial(&style, p, at, true)?,
        o => return Err(bad(C, format!("type must be one of {}, not `{o}`", KINDS.join(", ")))),
    };
    let paint = Paint::solid(style.color);
    let stroked = |path: &BezPath| {
        let mut ap = Appearance::basic(Paint::None, paint.clone(), style.stroke);
        if let Some(st) = ap.stroke_mut() {
            st.cap = LineCap::Butt;
        }
        (PathData::from_bezpath(path), ap)
    };
    let mut parts: Vec<(&str, NodeKind, Appearance)> = vec![];
    let path_kind = |path: PathData| NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false };
    if !art.extension.elements().is_empty() {
        let (pd, ap) = stroked(&art.extension);
        parts.push(("Extension Lines", path_kind(pd), ap));
    }
    let (pd, ap) = stroked(&art.line);
    parts.push(("Dimension Line", path_kind(pd), ap));
    let (end_path, filled) = ends(&style, &art.ends);
    if !end_path.elements().is_empty() {
        let (pd, ap) =
            if filled { (PathData::from_bezpath(&end_path), Appearance::basic(paint.clone(), Paint::None, 0.0)) } else { stroked(&end_path) };
        parts.push(("Ends", path_kind(pd), ap));
    }
    if let Some((origin, dir, label)) = art.label {
        let cs = CharStyle { size: style.font, fill: paint.clone(), ..CharStyle::default() };
        let mut t = TextObject::point(origin, &label, cs);
        t.para = super::create::new_type_para();
        t.para.justify = Justify::Center;
        let angle = dir.y.atan2(dir.x);
        t.xf = Affine::translate(origin.to_vec2()) * Affine::rotate(if angle.abs() < 1e-12 { 0.0 } else { angle });
        super::typecmd::refresh_bounds(&mut t);
        parts.push(("Label", NodeKind::Text(Box::new(t)), Appearance::default()));
    }
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Dimension", |d, sel| {
        let children = parts
            .into_iter()
            .map(|(name, kind, appearance)| {
                let mut n = Node::new(d.alloc_id(), kind);
                n.appearance = appearance.into();
                n.name = Some(name.into());
                Arc::new(n)
            })
            .collect();
        let mut g = Node::group(d.alloc_id(), children);
        g.name = Some("Dimension".into());
        let gid = g.id;
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "value": value, "text": text }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style { unit: Unit::Points, precision: 2, scale: 1.0, ends: "arrow", stroke: 0.5, font: 9.0, color: Color::BLACK, show_units: true }
    }

    #[test]
    fn labels_format_units_precision_and_scale() {
        let mut s = style();
        assert_eq!(s.length(72.0).1, "72.00 pt");
        s.unit = Unit::Inches;
        s.precision = 3;
        assert_eq!(s.length(72.0).1, "1.000 in");
        s.scale = 48.0;
        s.show_units = false;
        assert_eq!(s.length(72.0), (48.0, "48.000".into()));
    }

    #[test]
    fn labels_read_left_to_right_and_stay_off_the_line() {
        let s = style();
        // A line drawn right to left still reads left to right; the label goes above it.
        let (o, v, _) = label_along(&s, Point::new(100.0, 50.0), Point::new(0.0, 50.0), Vec2::new(0.0, -1.0), "x".into()).unwrap();
        assert_eq!(v, Vec2::new(1.0, 0.0));
        assert!(o.y < 50.0 && (o.x - 50.0).abs() < 1e-9);
        // Below the line, the baseline drops by the cap height so the type clears the line.
        let (o, ..) = label_along(&s, Point::new(0.0, 50.0), Point::new(100.0, 50.0), Vec2::new(0.0, 1.0), "x".into()).unwrap();
        assert!(o.y > 50.0 + s.font * 0.7);
        // A vertical line reads bottom to top.
        let (_, v, _) = label_along(&s, Point::new(0.0, 0.0), Point::new(0.0, 100.0), Vec2::new(-1.0, 0.0), "x".into()).unwrap();
        assert_eq!(v, Vec2::new(0.0, -1.0));
    }

    #[test]
    fn arcs_stay_on_their_circle() {
        let mut bp = BezPath::new();
        arc(&mut bp, Point::new(10.0, 10.0), 50.0, 0.3, 4.0);
        let mut prev = Point::new(10.0 + 50.0 * 0.3f64.cos(), 10.0 + 50.0 * 0.3f64.sin());
        for el in bp.elements() {
            if let vectorcraft_geom::PathEl::CurveTo(c1, c2, q) = *el {
                let m = Point::new((prev.x + 3.0 * c1.x + 3.0 * c2.x + q.x) / 8.0, (prev.y + 3.0 * c1.y + 3.0 * c2.y + q.y) / 8.0);
                assert!((m.distance(Point::new(10.0, 10.0)) - 50.0).abs() < 0.05);
                prev = q;
            }
        }
    }
}
