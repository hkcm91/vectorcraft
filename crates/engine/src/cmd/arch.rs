//! `shape.arch`: the Arch tool's art. Nested arch bands, the outermost first, make rainbows,
//! tunnels and gates. Each band is a filled path between two arch outlines. The outline shapes are
//! Classic (round), Rounded (squircle), Wide (low), Pointed (gothic), Scallop (bumps) and Wavy.
//! Bands can be cut into blocks (Segmented), have rounded ends, or close into rings. The fill styles
//! add a highlight stripe (Glossy, Jelly, Pearlescent) or soften the colours (Matte, Pearlescent).
//! A VectorCraft addition: Illustrator has no such tool.

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LineCap, Node, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Vec2};

use super::*;

/// The outline shapes `shape.arch` takes (`segmented` is Classic cut into blocks).
pub const SHAPES: [&str; 7] = ["classic", "rounded", "wide", "pointed", "scallop", "wavy", "segmented"];
/// The fill styles `shape.arch` takes.
pub const FILL_STYLES: [&str; 5] = ["flat", "glossy", "jelly", "matte", "pearlescent"];
/// The colours bands take when none are given, outermost first: pink, orange, yellow, mint, sky,
/// violet (original values).
pub const DEFAULT_COLORS: [&str; 6] = ["#FF6FAE", "#FF9A4A", "#FFD43B", "#3ED3A3", "#3BB7F0", "#9C6CF2"];
/// The most bands one arch has.
pub const MAX_BANDS: u64 = 64;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "shape.arch",
        "Arch",
        [],
        None,
        "{x, y, width, height (the outermost band's box; its feet stand on the bottom edge), count=5 (bands, 1–64), thickness=40 (pt, each band), spacing=8 (pt, between bands; bands and gaps shrink together to fill at most 80 % of the half-width, leaving an opening), shape?: classic (default)|rounded|wide|pointed|scallop|wavy|segmented, segments?: n (cut each band into n blocks; segmented: 7), roundness=0 (0–100 %: rounds the band ends), ends?: open (default; feet on the bottom edge)|closed (rings: the bands run along the bottom too), colors?: [\"#rrggbb\", …] (outermost first, repeating; default a six-colour rainbow), fillStyle?: flat (default)|glossy|jelly|matte|pearlescent} nested arch bands as a group named Arch, one path per band (and a highlight stripe for glossy, jelly and pearlescent) → {id, bands, thickness, spacing}",
        has_doc,
        arch
    )]
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Classic,
    Rounded,
    Wide,
    Pointed,
    Scallop,
    Wavy,
}

/// Scallop bumps across the whole arch (odd: one sits on top) and their height.
const SCALLOPS: f64 = 7.0;
const SCALLOP_DEPTH: f64 = 0.12;
const WAVE_DEPTH: f64 = 0.07;

/// One arch outline: the curve from the bottom-left corner of the box `(x0, y0, w, h)` up and
/// over to the bottom-right one, walked by `s` from 0 to 1.
#[derive(Clone, Copy, Debug)]
struct Outline {
    shape: Shape,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
}

impl Outline {
    /// The height of the curved top; straight legs make up the rest.
    fn cap_height(&self) -> f64 {
        let k = match self.shape {
            Shape::Rounded => 0.42,
            Shape::Wide => 0.28,
            Shape::Pointed => 0.85,
            Shape::Classic | Shape::Scallop | Shape::Wavy => 0.5,
        };
        self.h.min(self.w * k).max(0.0)
    }

    /// A Rounded top's corner radius.
    fn corner_radius(&self) -> f64 {
        (self.cap_height().min(self.w / 2.0) * 0.7).max(0.0)
    }

    /// Where the top's left quarter changes curve, in its own `u` (0–1): a Rounded top's corner
    /// ends, between Scallop bumps.
    fn cap_breaks(&self) -> Vec<f64> {
        match self.shape {
            Shape::Rounded => {
                let (ch, r) = (self.cap_height(), self.corner_radius());
                let (l1, l2, l3) = ((ch - r).max(0.0), FRAC_PI_2 * r, (self.w / 2.0 - r).max(0.0));
                let t = l1 + l2 + l3;
                if t > 1e-12 { vec![l1 / t, (l1 + l2) / t] } else { vec![] }
            }
            // Between the bumps: |sin(k φ)| = 0 at φ = jπ/k, u = 2j/k.
            Shape::Scallop => (1..).map(|j| 2.0 * f64::from(j) / SCALLOPS).take_while(|u| *u < 1.0 - 1e-9).collect(),
            _ => vec![],
        }
    }

    /// Where the legs end and the top begins, in `s`.
    fn leg_end(&self) -> f64 {
        let ch = self.cap_height();
        let leg = self.h - ch;
        let cap = PI / 4.0 * (self.w / 2.0 + ch);
        if leg + cap > 1e-9 { 0.5 * leg / (leg + cap) } else { 0.0 }
    }

    /// The left quarter of the top at `u` (0: where it springs from the leg, 1: the apex), in box
    /// coordinates (y down from the top of the box).
    fn cap_point(&self, u: f64) -> Point {
        let (w, ch) = (self.w, self.cap_height());
        let phi = u.clamp(0.0, 1.0) * FRAC_PI_2;
        let base = match self.shape {
            Shape::Rounded => {
                // Up the side, round a corner, along the flat top: by length.
                let r = self.corner_radius();
                let (l1, l2, l3) = ((ch - r).max(0.0), FRAC_PI_2 * r, (w / 2.0 - r).max(0.0));
                let t = u.clamp(0.0, 1.0) * (l1 + l2 + l3);
                if t <= l1 {
                    Point::new(0.0, ch - t)
                } else if t <= l1 + l2 && l2 > 0.0 {
                    let a = (t - l1) / r;
                    Point::new(r - r * a.cos(), r - r * a.sin())
                } else {
                    Point::new(r + (t - l1 - l2), 0.0)
                }
            }
            Shape::Pointed => {
                // An equilateral pointed arch (centred on the far springing point), scaled to ch.
                let theta = PI - u.clamp(0.0, 1.0) * PI / 3.0;
                Point::new(w + w * theta.cos(), ch - ch * theta.sin() / (PI / 3.0).sin())
            }
            Shape::Classic | Shape::Wide | Shape::Scallop | Shape::Wavy => Point::new(w / 2.0 - w / 2.0 * phi.cos(), ch - ch * phi.sin()),
        };
        // Scallop and Wavy push the top in and out from its centre (symmetric about the apex).
        let m = match self.shape {
            Shape::Scallop => (1.0 + SCALLOP_DEPTH * (SCALLOPS * phi).sin().abs()) / (1.0 + SCALLOP_DEPTH),
            Shape::Wavy => (1.0 + WAVE_DEPTH * (8.0 * phi).cos()) / (1.0 + WAVE_DEPTH),
            _ => 1.0,
        };
        let c = Point::new(w / 2.0, ch);
        c + (base - c) * m
    }

    /// The outline at `s` in document coordinates.
    fn at(&self, s: f64) -> Point {
        let s = s.clamp(0.0, 1.0);
        let (mirror, s) = if s > 0.5 { (true, 1.0 - s) } else { (false, s) };
        let sl = self.leg_end();
        let ch = self.cap_height();
        let spring = self.cap_point(0.0);
        let local = if s < sl {
            // The left leg, straight up from the bottom edge to where the top springs.
            let t = if sl > 0.0 { s / sl } else { 1.0 };
            Point::new(spring.x, self.h + (ch - self.h) * t)
        } else {
            // The top sits in the box's top `ch`, springing at y = ch.
            let u = if 0.5 - sl > 1e-12 { (s - sl) / (0.5 - sl) } else { 1.0 };
            self.cap_point(u)
        };
        let x = if mirror { self.w - local.x } else { local.x };
        Point::new(self.x0 + x, self.y0 + local.y)
    }

    /// Where the outline has a corner (or changes from leg to top), in `s`, ascending.
    fn breaks(&self) -> Vec<f64> {
        let sl = self.leg_end();
        let mut b = vec![];
        if sl > 0.0 {
            b.push(sl);
        }
        b.extend(self.cap_breaks().into_iter().filter(|u| *u > 1e-9).map(|u| sl + (0.5 - sl) * u));
        if self.shape == Shape::Pointed {
            b.push(0.5);
        }
        let mirrored: Vec<f64> = b.iter().filter(|s| **s < 0.5).map(|s| 1.0 - s).collect();
        b.extend(mirrored);
        b.sort_by(f64::total_cmp);
        b.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        b
    }

    /// The direction the outline runs at `s`, looking into `[s, s + ds]` (`ds` > 0) or back into
    /// `[s + ds, s]` (`ds` < 0) so a corner at `s` doesn't blur it.
    fn derivative(&self, s: f64, ds: f64) -> Vec2 {
        let a = self.at(s);
        let b = self.at(s + ds);
        (b - a) / ds
    }

    /// Append the outline from `s0` to `s1` (either way round) to `bp` as cubics: each piece
    /// between corners is sampled and joined with the curve's own tangents (Hermite to Bézier).
    fn append(&self, bp: &mut BezPath, s0: f64, s1: f64) {
        let (lo, hi) = (s0.min(s1), s0.max(s1));
        let mut cuts = vec![lo];
        cuts.extend(self.breaks().into_iter().filter(|b| *b > lo + 1e-9 && *b < hi - 1e-9));
        cuts.push(hi);
        if s1 < s0 {
            cuts.reverse();
        }
        for pair in cuts.windows(2) {
            let &[a, b] = pair else { continue };
            let n = 8;
            let eps = (b - a) * 1e-4;
            for i in 0..n {
                let (u0, u1) = (a + (b - a) * i as f64 / n as f64, a + (b - a) * (i + 1) as f64 / n as f64);
                // One-sided at the piece's ends, so corners stay sharp.
                let d0 = if i == 0 { self.derivative(u0, eps) } else { self.derivative(u0 - eps, 2.0 * eps) };
                let d1 = if i + 1 == n { self.derivative(u1, -eps) } else { self.derivative(u1 - eps, 2.0 * eps) };
                let du = u1 - u0;
                let (p0, p1) = (self.at(u0), self.at(u1));
                bp.curve_to(p0 + d0 * (du / 3.0), p1 - d1 * (du / 3.0), p1);
            }
        }
    }

    /// The outline `d` further in: the box shrinks by `d` at the sides and top (and at the bottom
    /// too for a ring).
    fn inset(&self, d: f64, closed: bool) -> Outline {
        Outline { x0: self.x0 + d, y0: self.y0 + d, w: self.w - 2.0 * d, h: self.h - if closed { 2.0 * d } else { d }, ..*self }
    }

    /// About how long the outline is (for gaps given in points).
    fn length(&self) -> f64 {
        (0..64).map(|i| self.at(i as f64 / 64.0).distance(self.at((i + 1) as f64 / 64.0))).sum()
    }
}

/// A rounded band end from `a` to `b`, bulging `depth` along `dir` (a half circle at depth =
/// half the width).
fn end_cap(bp: &mut BezPath, a: Point, b: Point, dir: Vec2, depth: f64) {
    if depth <= 1e-9 || !(dir.x.is_finite() && dir.y.is_finite()) {
        bp.line_to(b);
        return;
    }
    let k = dir * (depth * 4.0 / 3.0);
    bp.curve_to(a + k, b + k, b);
}

fn unit(v: Vec2) -> Vec2 {
    let l = v.hypot();
    if l > 1e-12 && l.is_finite() { v / l } else { Vec2::ZERO }
}

/// A band piece between outlines `outer` and `inner` from `a` to `b` in `s`, its ends rounded by
/// `round` (0–1).
fn band_piece(outer: &Outline, inner: &Outline, a: f64, b: f64, round: f64) -> BezPath {
    let mut bp = BezPath::new();
    let eps = ((b - a) * 1e-4).max(1e-9);
    bp.move_to(outer.at(a));
    outer.append(&mut bp, a, b);
    let (ob, ib) = (outer.at(b), inner.at(b));
    end_cap(&mut bp, ob, ib, unit(outer.derivative(b, -eps)), ob.distance(ib) / 2.0 * round);
    inner.append(&mut bp, b, a);
    let (ia, oa) = (inner.at(a), outer.at(a));
    end_cap(&mut bp, ia, oa, -unit(outer.derivative(a, eps)), ia.distance(oa) / 2.0 * round);
    bp.close_path();
    bp
}

/// A closed loop along `o`: the arch and the bottom edge back.
fn ring_loop(bp: &mut BezPath, o: &Outline, reverse: bool) {
    let (a, b) = if reverse { (1.0, 0.0) } else { (0.0, 1.0) };
    bp.move_to(o.at(a));
    o.append(bp, a, b);
    bp.close_path();
}

/// `shape.arch`'s params, checked, with the bands and gaps fitted into the box.
struct Spec {
    outer: Outline,
    count: usize,
    thickness: f64,
    spacing: f64,
    round: f64,
    segments: usize,
    closed: bool,
    style: &'static str,
    colors: Vec<Color>,
}

impl Spec {
    fn parse(p: &Value) -> Result<Spec> {
        const C: &str = "shape.arch";
        let x = f64_req(p, "x", C)?;
        let y = f64_req(p, "y", C)?;
        let w = f64_req(p, "width", C)?;
        let h = f64_req(p, "height", C)?;
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
            return Err(bad(C, "x, y, width and height must be finite, with a positive width and height"));
        }
        let shape_name = str_param(p, "shape").unwrap_or("classic");
        let shape = match shape_name {
            "classic" | "segmented" => Shape::Classic,
            "rounded" => Shape::Rounded,
            "wide" => Shape::Wide,
            "pointed" => Shape::Pointed,
            "scallop" => Shape::Scallop,
            "wavy" => Shape::Wavy,
            o => return Err(bad(C, format!("unknown shape `{o}` (one of {})", SHAPES.join(", ")))),
        };
        let count = match p.get("count") {
            None | Some(Value::Null) => 5,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=MAX_BANDS).contains(n))
                .ok_or_else(|| bad(C, format!("count must be a whole number from 1 to {MAX_BANDS}")))?,
        } as usize;
        let num = |key: &str, default: f64, lo: f64, hi: f64| -> Result<f64> {
            match p.get(key) {
                None | Some(Value::Null) => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|f| f.is_finite() && (lo..=hi).contains(f))
                    .ok_or_else(|| bad(C, format!("{key} must be a number from {lo} to {hi}"))),
            }
        };
        let mut thickness = num("thickness", 40.0, 0.01, 1e6)?;
        let mut spacing = num("spacing", 8.0, 0.0, 1e6)?;
        let round = num("roundness", 0.0, 0.0, 100.0)? / 100.0;
        let default_segments = if shape_name == "segmented" { 7 } else { 1 };
        let segments = match p.get("segments") {
            None | Some(Value::Null) => default_segments,
            Some(v) => v.as_u64().filter(|n| *n <= 200).ok_or_else(|| bad(C, "segments must be a whole number from 0 to 200"))?.max(1),
        } as usize;
        let closed = match str_param(p, "ends") {
            None | Some("open") => false,
            Some("closed") => true,
            Some(o) => return Err(bad(C, format!("ends must be open or closed, not `{o}`"))),
        };
        let style_name = str_param(p, "fillStyle").unwrap_or("flat");
        let style = FILL_STYLES
            .iter()
            .copied()
            .find(|s| *s == style_name)
            .ok_or_else(|| bad(C, format!("unknown fillStyle `{style_name}` (one of {})", FILL_STYLES.join(", "))))?;
        let colors: Vec<Color> = match p.get("colors") {
            None | Some(Value::Null) => DEFAULT_COLORS.iter().filter_map(|c| Color::from_hex(c)).collect(),
            Some(Value::Array(a)) if !a.is_empty() && a.len() <= 256 => a
                .iter()
                .map(|c| c.as_str().and_then(Color::from_hex).or_else(|| color_value(c)))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad(C, "colors must be colours such as \"#ff6fae\""))?,
            Some(_) => return Err(bad(C, "colors must be a list of 1 to 256 colours")),
        };
        // Bands and gaps shrink together until they fill at most 80 % of the half-width (or the
        // height), leaving an opening in the middle.
        let room = if closed { (w / 2.0).min(h / 2.0) } else { (w / 2.0).min(h) } * 0.8;
        let need = count as f64 * thickness + (count as f64 - 1.0) * spacing;
        if need > room && need > 0.0 {
            let f = room / need;
            thickness *= f;
            spacing *= f;
        }
        // Rounded feet bulge down by up to half the thickness: the legs stop that much higher so the
        // feet still end on the box's bottom edge.
        let lift = if closed { 0.0 } else { (thickness / 2.0 * round).min(h * 0.5) };
        Ok(Spec { outer: Outline { shape, x0: x, y0: y, w, h: h - lift }, count, thickness, spacing, round, segments, closed, style, colors })
    }

    /// Band `i`'s outer and inner outlines.
    fn band(&self, i: usize) -> (Outline, Outline) {
        let d = i as f64 * (self.thickness + self.spacing);
        (self.outer.inset(d, self.closed), self.outer.inset(d + self.thickness, self.closed))
    }

    /// Band `i`'s colour (after the fill style) and opacity.
    fn paint(&self, i: usize) -> (Color, f32) {
        let base = self.colors.get(i % self.colors.len().max(1)).copied().unwrap_or(Color::BLACK);
        let fill = match self.style {
            "matte" => base.lerp(&Color::gray(0.5), 0.18),
            "pearlescent" => base.lerp(&Color::WHITE, 0.45),
            _ => base,
        };
        (fill, if self.style == "jelly" { 0.82 } else { 1.0 })
    }

    /// The blocks of band `outer`, in `s` (one for a whole band), with a gap between them along
    /// the band's middle of 40 % of the thickness, or the spacing between bands when that is wider.
    fn blocks(&self, outer: &Outline) -> Vec<(f64, f64)> {
        let n = self.segments.max(1);
        if self.closed || n == 1 {
            return vec![(0.0, 1.0)];
        }
        // Rounded ends bulge into the gap from both sides: widen it by as much.
        let gap = ((self.thickness * 0.4).max(self.spacing.min(self.thickness)) + self.thickness * self.round)
            / outer.inset(self.thickness / 2.0, false).length().max(1e-9);
        let step = 1.0 / n as f64;
        (0..n)
            .map(|j| (j as f64 * step + if j > 0 { gap / 2.0 } else { 0.0 }, (j + 1) as f64 * step - if j + 1 < n { gap / 2.0 } else { 0.0 }))
            .filter(|(a, b)| b > a)
            .collect()
    }

    fn glossy(&self) -> bool {
        matches!(self.style, "glossy" | "jelly" | "pearlescent")
    }
}

type Part = (String, PathData, FillRule, Appearance, f32);

/// The paths of the arch `spec` describes: each band, followed by its highlight stripe for the
/// glossy styles.
fn parts(spec: &Spec) -> Vec<Part> {
    let mut parts: Vec<Part> = vec![];
    for i in 0..spec.count {
        let (o, inn) = spec.band(i);
        let (fill, opacity) = spec.paint(i);
        let mut bp = BezPath::new();
        let blocks = spec.blocks(&o);
        if spec.closed {
            ring_loop(&mut bp, &o, false);
            ring_loop(&mut bp, &inn, true);
        } else {
            for (a, b) in &blocks {
                bp.extend(band_piece(&o, &inn, *a, *b, spec.round));
            }
        }
        let rule = if spec.closed { FillRule::EvenOdd } else { FillRule::NonZero };
        parts.push((format!("Band {}", i + 1), PathData::from_bezpath(&bp), rule, Appearance::basic(Paint::solid(fill), Paint::None, 0.0), opacity));
        if spec.glossy() {
            // A soft white stripe along the band, a third of the way in.
            let d = i as f64 * (spec.thickness + spec.spacing);
            let mid = spec.outer.inset(d + spec.thickness * 0.3, spec.closed);
            let mut hl = BezPath::new();
            if spec.closed {
                ring_loop(&mut hl, &mid, false);
            } else {
                for (a, b) in &blocks {
                    let trim = (b - a) * 0.04;
                    hl.move_to(mid.at(a + trim));
                    mid.append(&mut hl, a + trim, b - trim);
                }
            }
            let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::WHITE), spec.thickness * 0.16);
            if let Some(st) = ap.stroke_mut() {
                st.cap = LineCap::Round;
            }
            let strength = if spec.style == "jelly" { 0.6 } else { 0.45 };
            parts.push((format!("Highlight {}", i + 1), PathData::from_bezpath(&hl), FillRule::NonZero, ap, strength));
        }
    }
    parts
}

/// The arch group, with ids from `d`.
fn group(d: &mut Document, parts: Vec<Part>) -> Node {
    let children = parts
        .into_iter()
        .map(|(name, path, rule, appearance, opacity)| {
            let mut n = Node::new(d.alloc_id(), NodeKind::Path { path, rule, live: None, clipping: false, guide: false });
            n.appearance = appearance.into();
            n.name = Some(name);
            n.opacity = opacity;
            Arc::new(n)
        })
        .collect();
    let mut g = Node::group(d.alloc_id(), children);
    g.name = Some("Arch".into());
    g
}

/// A `w` × `h` pt document holding the arch `p` describes, scaled to fit with a small margin and
/// centred (`x` and `y` don't matter): what the Arch Tool Options preview.
pub fn preview_document(p: &Value, w: f64, h: f64) -> Result<Document> {
    let spec = Spec::parse(p)?;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(bad("shape.arch", "the preview needs a positive size"));
    }
    // The box asked for (the outline's own height is shorter when rounded feet lift it).
    let (aw, ah) = (f64_req(p, "width", "shape.arch")?, f64_req(p, "height", "shape.arch")?);
    let k = ((w * 0.9) / aw).min((h * 0.9) / ah);
    let mut fitted = p.clone();
    for (key, v) in [
        ("x", (w - aw * k) / 2.0),
        ("y", (h - ah * k) / 2.0),
        ("width", aw * k),
        ("height", ah * k),
        ("thickness", (spec.thickness * k).max(0.01)),
        ("spacing", spec.spacing * k),
    ] {
        fitted[key] = json!(v);
    }
    let spec = Spec::parse(&fitted)?;
    let mut d = Document::new(w, h);
    let layer = d.layers.first().map(|l| l.id);
    let g = group(&mut d, parts(&spec));
    d.insert(layer, usize::MAX, g)?;
    Ok(d)
}

fn arch(s: &mut Session, p: &Value) -> Result<Value> {
    let spec = Spec::parse(p)?;
    let parts = parts(&spec);
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Arch", |d, sel| {
        let g = group(d, parts);
        let gid = g.id;
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0, "bands": spec.count, "thickness": spec.thickness, "spacing": spec.spacing }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(shape: Shape) -> Outline {
        Outline { shape, x0: 0.0, y0: 0.0, w: 200.0, h: 150.0 }
    }

    #[test]
    fn every_outline_runs_from_foot_to_foot_over_its_box() {
        for shape in [Shape::Classic, Shape::Rounded, Shape::Wide, Shape::Pointed, Shape::Scallop, Shape::Wavy] {
            let o = outline(shape);
            let (a, b, top) = (o.at(0.0), o.at(1.0), o.at(0.5));
            assert!((a.y - 150.0).abs() < 1e-9 && (b.y - 150.0).abs() < 1e-9, "{shape:?}: feet on the bottom edge");
            assert!((a.x + b.x - 200.0).abs() < 1e-9, "{shape:?}: symmetric feet");
            assert!((top.x - 100.0).abs() < 1e-6 && top.y.abs() < 1e-6, "{shape:?}: apex at the top centre: {top:?}");
            for i in 0..=100 {
                let p = o.at(i as f64 / 100.0);
                assert!((-1e-6..=200.0 + 1e-6).contains(&p.x) && (-1e-6..=150.0 + 1e-6).contains(&p.y), "{shape:?} leaves its box at {p:?}");
                let q = o.at(1.0 - i as f64 / 100.0);
                assert!((p.x + q.x - 200.0).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6, "{shape:?}: mirror symmetric");
            }
        }
    }

    #[test]
    fn a_classic_arch_is_a_half_circle_on_straight_legs() {
        let o = outline(Shape::Classic);
        let sl = o.leg_end();
        assert!(sl > 0.0);
        assert!((o.at(sl / 2.0).x).abs() < 1e-9, "the leg is straight up");
        for i in 0..=20 {
            let p = o.at(sl + (0.5 - sl) * i as f64 / 20.0);
            assert!((p.distance(Point::new(100.0, 100.0)) - 100.0).abs() < 1e-6);
        }
        // The cubics stay on the circle.
        let mut bp = BezPath::new();
        bp.move_to(o.at(0.0));
        o.append(&mut bp, 0.0, 1.0);
        let mut prev = o.at(0.0);
        for el in bp.elements() {
            let vectorcraft_geom::PathEl::CurveTo(c1, c2, p) = *el else { continue };
            // The cubic's middle: (p0 + 3 c1 + 3 c2 + p3) / 8.
            let m = Point::new((prev.x + 3.0 * c1.x + 3.0 * c2.x + p.x) / 8.0, (prev.y + 3.0 * c1.y + 3.0 * c2.y + p.y) / 8.0);
            prev = p;
            if m.y < 100.0 {
                assert!((m.distance(Point::new(100.0, 100.0)) - 100.0).abs() < 0.05, "{m:?}");
            }
        }
    }

    #[test]
    fn insets_shrink_the_box() {
        let o = outline(Shape::Classic).inset(10.0, false);
        assert_eq!((o.x0, o.y0, o.w, o.h), (10.0, 10.0, 180.0, 140.0));
        let r = outline(Shape::Classic).inset(10.0, true);
        assert_eq!(r.h, 130.0);
    }
}
