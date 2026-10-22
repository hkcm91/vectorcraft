//! Arch tool (a VectorCraft addition): drag a box to draw nested arch bands (rainbows, tunnels,
//! gates) with the tool's options; Shift makes the box square, Alt draws it from its centre, ↑/↓
//! during the drag add or remove a band. A click without a drag opens the Arch Tool Options, and
//! OK draws an arch there. The art is `shape.arch`.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use crate::guides::{DrawSnap, Leave};
use crate::shape::drag_rect;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The Arch Tool Options' numbers as `shape.arch` takes them: (key, default, least, most).
pub const ARCH_NUMBERS: [(&str, f64, f64, f64); 5] = [
    ("count", 5.0, 1.0, 64.0),
    ("thickness", 40.0, 1.0, 1000.0),
    ("spacing", 8.0, 0.0, 1000.0),
    ("roundness", 0.0, 0.0, 100.0),
    ("segments", 7.0, 2.0, 200.0),
];

/// The outline shapes (`shape.arch`'s `shape`).
pub const ARCH_SHAPES: [&str; 7] = ["classic", "rounded", "wide", "pointed", "scallop", "wavy", "segmented"];
/// The fill styles (`shape.arch`'s `fillStyle`).
pub const ARCH_FILL_STYLES: [&str; 5] = ["flat", "glossy", "jelly", "matte", "pearlescent"];
/// The palette new arches start with, outermost band first.
pub const ARCH_COLORS: [&str; 6] = ["#FF6FAE", "#FF9A4A", "#FFD43B", "#3ED3A3", "#3BB7F0", "#9C6CF2"];

/// Arch tool: a box drag draws an arch with the options.
pub struct ArchTool {
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    snap: DrawSnap,
    /// [`ARCH_NUMBERS`]' values, in order.
    numbers: [f64; 5],
    shape: &'static str,
    fill_style: &'static str,
    closed: bool,
    colors: Vec<String>,
}

impl Default for ArchTool {
    fn default() -> Self {
        Self {
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            snap: DrawSnap::default(),
            numbers: ARCH_NUMBERS.map(|(_, d, ..)| d),
            shape: "classic",
            fill_style: "flat",
            closed: false,
            colors: ARCH_COLORS.iter().map(|c| c.to_string()).collect(),
        }
    }
}

/// `shape.arch`'s params for an arch in the box `(x, y, width, height)` with the Arch Tool Options
/// `o` (as [`Tool::options`] reports them). Segments count only for the Segmented shape.
pub fn arch_params(o: &Value, x: f64, y: f64, width: f64, height: f64) -> Value {
    let mut v = json!({ "x": x, "y": y, "width": width, "height": height });
    for (key, default, ..) in ARCH_NUMBERS {
        let n = o.get(key).and_then(Value::as_f64).unwrap_or(default);
        v[key] = if matches!(key, "count" | "segments") { json!(n.round() as u64) } else { json!(n) };
    }
    let shape = o.get("shape").and_then(Value::as_str).unwrap_or("classic");
    if shape != "segmented" {
        v["segments"] = json!(1);
    }
    v["shape"] = json!(shape);
    v["fillStyle"] = o.get("fillStyle").cloned().unwrap_or_else(|| json!("flat"));
    v["ends"] = o.get("ends").cloned().unwrap_or_else(|| json!("open"));
    v["colors"] = o.get("colors").cloned().unwrap_or_else(|| json!(ARCH_COLORS));
    v
}

impl ArchTool {
    fn params(&self, start: Point, p: Point, m: Mods) -> Value {
        let r = drag_rect(start, p, m);
        arch_params(&self.options(), r.x0, r.y0, r.width().max(1.0), r.height().max(1.0))
    }
}

impl Tool for ArchTool {
    fn id(&self) -> &'static str {
        "arch"
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.snap.hover(cx, ev.pos, &[], None);
                vec![]
            }
            PointerKind::Down => {
                self.start = Some(self.snap.press(cx, ev.pos, &[], None));
                self.last = ev.pos;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(s) = self.start else { return vec![] };
                let pos = self.snap.drag(cx, ev.pos, Some(&Leave::diagonal(s, ev.mods.shift)));
                self.last = pos;
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin("Arch".into()));
                }
                out.push(Action::Preview("shape.arch".into(), self.params(s, pos, ev.mods)));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                if std::mem::take(&mut self.began) {
                    vec![Action::Commit]
                } else {
                    vec![Action::Dialog("arch".into(), json!({ "x": s.x, "y": s.y }))]
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        match key {
            ToolKey::Up | ToolKey::Down => {
                let n = &mut self.numbers[0];
                *n = if key == ToolKey::Up { (*n + 1.0).min(64.0) } else { (*n - 1.0).max(1.0) };
                vec![Action::Preview("shape.arch".into(), self.params(s, self.last, self.mods))]
            }
            ToolKey::Escape => {
                self.start = None;
                self.began = false;
                self.snap.clear();
                vec![Action::Cancel]
            }
            _ => vec![],
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            let r = drag_rect(s, self.last, self.mods);
            o.push(Overlay::Measure { p: self.last, text: cx.size_label(r.width(), r.height()) });
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        let mut v = json!({
            "shape": self.shape,
            "fillStyle": self.fill_style,
            "ends": if self.closed { "closed" } else { "open" },
            "colors": self.colors,
        });
        for ((key, ..), x) in ARCH_NUMBERS.iter().zip(self.numbers) {
            v[*key] = if matches!(*key, "count" | "segments") { json!(x.round() as u64) } else { json!(x) };
        }
        v
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "shape" => {
                if let Some(s) = v.as_str().and_then(|s| ARCH_SHAPES.iter().find(|k| **k == s)) {
                    self.shape = s;
                }
            }
            "fillStyle" => {
                if let Some(s) = v.as_str().and_then(|s| ARCH_FILL_STYLES.iter().find(|k| **k == s)) {
                    self.fill_style = s;
                }
            }
            "ends" => self.closed = v.as_str() == Some("closed"),
            "colors" => {
                let colors: Vec<String> = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .filter(|c| vectorcraft_color::Color::from_hex(c).is_some())
                            .take(256)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                if !colors.is_empty() {
                    self.colors = colors;
                }
            }
            _ => {
                let Some((slot, (_, default, lo, hi))) = self.numbers.iter_mut().zip(ARCH_NUMBERS).find(|(_, (k, ..))| *k == key) else { return };
                let x = v.as_f64().filter(|x| x.is_finite()).unwrap_or(default).clamp(lo, hi);
                *slot = if matches!(key, "count" | "segments") { x.round() } else { x };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn a_drag_draws_an_arch_and_a_click_asks_for_options() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = ArchTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 420.0, 380.0));
        assert_eq!(a[0], Action::Begin("Arch".into()));
        let Action::Preview(c, v) = &a[1] else { panic!("{a:?}") };
        assert_eq!(c, "shape.arch");
        assert_eq!((v["x"].as_f64(), v["width"].as_f64(), v["height"].as_f64()), (Some(300.0), Some(120.0), Some(80.0)));
        assert_eq!((v["count"].as_u64(), v["segments"].as_u64(), v["shape"].as_str()), (Some(5), Some(1), Some("classic")));
        // ↑ adds a band while drawing.
        let a = t.key(&cx, ToolKey::Up, Mods::default());
        assert!(matches!(&a[0], Action::Preview(_, v) if v["count"] == 6), "{a:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 420.0, 380.0)), vec![Action::Commit]);

        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 60.0));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 50.0, 60.0)),
            vec![Action::Dialog("arch".into(), json!({"x": 50.0, "y": 60.0}))]
        );
    }

    #[test]
    fn options_stay_valid() {
        let mut t = ArchTool::default();
        t.set_option("count", &json!(500));
        t.set_option("shape", &json!("segmented"));
        t.set_option("shape", &json!("nonsense"));
        t.set_option("thickness", &json!(f64::NAN));
        t.set_option("colors", &json!(["#123456", "not a colour"]));
        t.set_option("ends", &json!("closed"));
        let o = t.options();
        assert_eq!((o["count"].as_u64(), o["shape"].as_str(), o["thickness"].as_f64()), (Some(64), Some("segmented"), Some(40.0)));
        assert_eq!(o["colors"], json!(["#123456"]));
        let p = arch_params(&o, 0.0, 0.0, 10.0, 10.0);
        assert_eq!((p["segments"].as_u64(), p["ends"].as_str()), (Some(7), Some("closed")));
    }
}
