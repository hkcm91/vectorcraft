//! Arch Tool Options (double-click the Arch tool, or click with it): the options the Arch tool
//! keeps, which draw the next arch dragged out. Shape tiles, the band count, thickness, spacing
//! and roundness, Segments for the Segmented shape, Close into Rings, the colour palette (with
//! palettes to start from), the fill style, presets, and a live preview. OK sets them with
//! `tool.setOption {tool: "arch", values}`; opened by a click, it then draws an arch there
//! (`shape.arch`) at the dialog's Width and Height.
//!
//! Fields: `shape`, `count`, `thickness` and `spacing` (points), `roundness` (%), `segments`,
//! `ends` (open|closed), `fillStyle`, `colors` (["#rrggbb", …]), `width` and `height` (points),
//! and `x`, `y` where a click opened it.

use egui::{Rect, Sense, Stroke, StrokeKind, Vec2, vec2};
use serde_json::{Map, Value, json};
use vectorcraft_tools::arch::{ARCH_COLORS, ARCH_NUMBERS, arch_params};

use super::document_setup::check;
use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Arch Tool Options.
pub const KIND: &str = "archOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Arch Tool Options").into(), body, confirm, min_width: 620.0, ..DialogSpec::FORM };

const LABEL_W: f32 = 104.0;
const FIELD_W: f32 = 84.0;

/// The shape tiles: (`shape` value, label).
const SHAPES: [(&str, &str); 7] = [
    ("classic", "Classic"),
    ("rounded", "Rounded"),
    ("wide", "Wide"),
    ("pointed", "Pointed"),
    ("scallop", "Scallop"),
    ("wavy", "Wavy"),
    ("segmented", "Segmented"),
];

/// The fill styles: (`fillStyle` value, label).
const STYLES: [(&str, &str); 5] = [("flat", "Flat"), ("glossy", "Glossy"), ("jelly", "Jelly"), ("matte", "Muted"), ("pearlescent", "Pearlescent")];

/// Palettes to start from (original colours), outermost band first.
const PALETTES: [&[&str]; 6] = [
    &ARCH_COLORS,
    &["#FFB3D1", "#FFD1A8", "#FFF0A8", "#B8F0D8", "#B3DDF7", "#D4C2F7"],
    &["#FF5E7E", "#FF8A5B", "#FFB547", "#FFD86B"],
    &["#0B4F6C", "#1B8AA8", "#3FC1C9", "#A6E3E9"],
    &["#FF7FB8", "#FFB070", "#FFE27A", "#9BE7C4"],
    &["#2E2E38", "#5A5A6B", "#8A8A9C", "#C2C2CF"],
];

/// A preset: its label and the options it sets (colours: an index into [`PALETTES`] or its own).
struct Preset {
    label: &'static str,
    shape: &'static str,
    count: u64,
    thickness: f64,
    spacing: f64,
    roundness: f64,
    closed: bool,
    style: &'static str,
    colors: &'static [&'static str],
}

const PRESETS: [Preset; 6] = [
    Preset {
        label: "Classic Rainbow",
        shape: "classic",
        count: 5,
        thickness: 40.0,
        spacing: 8.0,
        roundness: 0.0,
        closed: false,
        style: "flat",
        colors: &ARCH_COLORS,
    },
    Preset {
        label: "Candy Arch",
        shape: "rounded",
        count: 3,
        thickness: 60.0,
        spacing: 0.0,
        roundness: 100.0,
        closed: false,
        style: "glossy",
        colors: &["#FF7FB8", "#FFB070", "#FFE27A"],
    },
    Preset {
        label: "Double Arch",
        shape: "classic",
        count: 2,
        thickness: 50.0,
        spacing: 30.0,
        roundness: 100.0,
        closed: false,
        style: "jelly",
        colors: &["#FF6FAE", "#3BB7F0"],
    },
    Preset {
        label: "Scallop Gate",
        shape: "scallop",
        count: 4,
        thickness: 36.0,
        spacing: 6.0,
        roundness: 0.0,
        closed: false,
        style: "flat",
        colors: &["#FF9A4A", "#FFD43B", "#3ED3A3", "#3BB7F0"],
    },
    Preset {
        label: "Dream Tunnel",
        shape: "classic",
        count: 7,
        thickness: 22.0,
        spacing: 10.0,
        roundness: 100.0,
        closed: false,
        style: "pearlescent",
        colors: &["#C9A7FF", "#A7C7FF", "#FFC2E2"],
    },
    Preset {
        label: "Wavy Arch",
        shape: "wavy",
        count: 5,
        thickness: 34.0,
        spacing: 6.0,
        roundness: 100.0,
        closed: false,
        style: "glossy",
        colors: &ARCH_COLORS,
    },
];

/// Open the Arch Tool Options with the tool's values; `at`: where a click opened them, to draw an
/// arch there on OK.
pub fn open(app: &mut VectorcraftApp, at: Option<vectorcraft_geom::Point>) -> Value {
    let mut fields = match app.session.tool_options_of("arch") {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    fields.insert("width".into(), json!(300.0));
    fields.insert("height".into(), json!(220.0));
    if let Some(p) = at {
        fields.insert("x".into(), json!(p.x));
        fields.insert("y".into(), json!(p.y));
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

/// The dialog's colours (the tool's when unset).
fn colors(d: &Dialog) -> Vec<String> {
    let c: Vec<String> =
        d.fields.get("colors").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect()).unwrap_or_default();
    if c.is_empty() { ARCH_COLORS.iter().map(|c| c.to_string()).collect() } else { c }
}

/// The options the dialog sets (what `tool.setOption` takes).
fn options(d: &Dialog) -> Map<String, Value> {
    let mut o: Map<String, Value> = ARCH_NUMBERS
        .iter()
        .map(|(k, default, lo, hi)| {
            let v = d.f64(k, *default).clamp(*lo, *hi);
            (k.to_string(), if matches!(*k, "count" | "segments") { json!(v.round() as u64) } else { json!(v) })
        })
        .collect();
    let str_or = |k: &str, default: &str| d.fields.get(k).and_then(Value::as_str).unwrap_or(default).to_string();
    o.insert("shape".into(), json!(str_or("shape", "classic")));
    o.insert("fillStyle".into(), json!(str_or("fillStyle", "flat")));
    o.insert("ends".into(), json!(if d.bool("rings") || str_or("ends", "open") == "closed" { "closed" } else { "open" }));
    o.insert("colors".into(), json!(colors(d)));
    o
}

/// `shape.arch`'s params for the dialog's arch at `(x, y)`, `w` × `h`.
fn params(d: &Dialog, x: f64, y: f64, w: f64, h: f64) -> Value {
    arch_params(&Value::Object(options(d)), x, y, w, h)
}

/// An arch rendered `size` big (the preview and the tiles), from `p`.
fn arch_image(ui: &mut egui::Ui, p: &Value, size: Vec2) -> Rect {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let key = format!("arch:{p}");
    if let Some(tex) = widgets::doc_preview(ui, &key, size, |w, h| vectorcraft_engine::cmd::arch::preview_document(p, w, h).ok()) {
        ui.painter().image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    r
}

/// A tile's label in the interface language (the labels are listed as literals so the catalogs
/// are checked for them).
fn shown(label: &'static str) -> &'static str {
    match label {
        "Classic" => tl!("Classic"),
        "Rounded" => tl!("Rounded"),
        "Wide" => tl!("Wide"),
        "Pointed" => tl!("Pointed"),
        "Scallop" => tl!("Scallop"),
        "Wavy" => tl!("Wavy"),
        "Segmented" => tl!("Segmented"),
        "Flat" => tl!("Flat"),
        "Glossy" => tl!("Glossy"),
        "Jelly" => tl!("Jelly"),
        "Muted" => tl!("Muted"),
        "Pearlescent" => tl!("Pearlescent"),
        "Classic Rainbow" => tl!("Classic Rainbow"),
        "Candy Arch" => tl!("Candy Arch"),
        "Double Arch" => tl!("Double Arch"),
        "Scallop Gate" => tl!("Scallop Gate"),
        "Dream Tunnel" => tl!("Dream Tunnel"),
        "Wavy Arch" => tl!("Wavy Arch"),
        other => other,
    }
}

/// A selectable tile: `draw` paints its picture, `label` goes under it. Returns true when clicked.
fn tile(ui: &mut egui::Ui, selected: bool, label: &'static str, width: f32, draw: impl FnOnce(&mut egui::Ui)) -> bool {
    let t = Tokens::get(ui.ctx());
    let resp = ui
        .scope(|ui| {
            ui.set_width(width);
            ui.vertical_centered(|ui| {
                draw(ui);
                ui.label(egui::RichText::new(shown(label)).size(11.0).color(if selected { t.text_strong } else { t.text_dim }));
            })
            .response
        })
        .inner;
    let frame = resp.rect.expand(3.0);
    let click = ui.interact(frame, ui.id().with(("arch-tile", label)), Sense::click());
    let stroke = if selected {
        Stroke::new(1.5, t.accent)
    } else if click.hovered() {
        Stroke::new(1.0, t.input_border)
    } else {
        Stroke::new(1.0, t.border)
    };
    ui.painter().rect_stroke(frame, f32::from(t.radius), stroke, StrokeKind::Inside);
    click.clicked()
}

fn number(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit, key: &str, label: &str) {
    let Some(&(_, default, lo, hi)) = ARCH_NUMBERS.iter().find(|(k, ..)| *k == key).or(match key {
        "width" => Some(&("width", 300.0, 1.0, 100_000.0)),
        "height" => Some(&("height", 220.0, 1.0, 100_000.0)),
        _ => None,
    }) else {
        return;
    };
    widgets::label_row(ui, label, LABEL_W, |ui| match key {
        "thickness" | "spacing" | "width" | "height" => {
            if form::length(ui, d, key, unit, FIELD_W) {
                let v = d.f64(key, default).clamp(lo, hi);
                d.fields.insert(key.into(), json!(v));
            }
        }
        _ => {
            let suffix = if key == "roundness" { "%" } else { "" };
            if let Some(v) = widgets::range_field(ui, ("arch", key), d.f64(key, default), lo..=hi, suffix, 0, FIELD_W) {
                d.fields.insert(key.into(), json!(v));
            }
        }
    });
}

fn hex_rgb(hex: &str) -> [u8; 3] {
    vectorcraft_color::Color::from_hex(hex).map(|c| c.to_rgba8(1.0)).map_or([0, 0, 0], |[r, g, b, _]| [r, g, b])
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let t = Tokens::get(ui.ctx());
    let shape = d.fields.get("shape").and_then(Value::as_str).unwrap_or("classic").to_string();
    ui.horizontal_top(|ui| {
        // Left: the options.
        ui.vertical(|ui| {
            ui.set_width(300.0);
            widgets::subheader(ui, tl!("Shape"));
            for row in SHAPES.chunks(4) {
                ui.horizontal(|ui| {
                    for (value, label) in row {
                        let mut p = params(d, 0.0, 0.0, 60.0, 44.0);
                        p["shape"] = json!(value);
                        p["segments"] = json!(if *value == "segmented" { 5 } else { 1 });
                        p["count"] = json!(3);
                        p["thickness"] = json!(8.0);
                        p["spacing"] = json!(2.0);
                        if tile(ui, shape == *value, label, 64.0, |ui| {
                            arch_image(ui, &p, vec2(52.0, 38.0));
                        }) {
                            d.fields.insert("shape".into(), json!(value));
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(6.0);
            }
            number(ui, d, unit, "count", tl!("Arch Count:"));
            number(ui, d, unit, "thickness", tl!("Thickness:"));
            number(ui, d, unit, "spacing", tl!("Spacing:"));
            number(ui, d, unit, "width", tl!("Width:"));
            number(ui, d, unit, "height", tl!("Height:"));
            number(ui, d, unit, "roundness", tl!("Roundness:"));
            if shape == "segmented" {
                number(ui, d, unit, "segments", tl!("Segments:"));
            }
            if !d.fields.contains_key("rings") {
                let closed = d.fields.get("ends").and_then(Value::as_str) == Some("closed");
                d.fields.insert("rings".into(), json!(closed));
            }
            check(ui, d, "rings", tl!("Close into Rings"));
        });
        ui.add_space(16.0);
        // Right: the preview, colours, fill style and presets.
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Preview"));
            let (w, h) = (d.f64("width", 300.0).max(1.0), d.f64("height", 220.0).max(1.0));
            let p = params(d, 0.0, 0.0, w, h);
            let r = arch_image(ui, &p, vec2(280.0, 190.0));
            ui.painter().rect_stroke(r, f32::from(t.radius), Stroke::new(1.0, t.border), StrokeKind::Outside);
            ui.add_space(8.0);

            widgets::subheader(ui, tl!("Color Palette"));
            let mut cs = colors(d);
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                for c in cs.iter_mut() {
                    let mut rgb = hex_rgb(c);
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        *c = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                        changed = true;
                    }
                }
                if cs.len() < 16 && widgets::icon_button(ui, "plus", tl!("Add Color"), false, 22.0).clicked() {
                    cs.push(cs.last().cloned().unwrap_or_else(|| "#FF6FAE".into()));
                    changed = true;
                }
                if cs.len() > 1 && widgets::icon_button(ui, "minus", tl!("Remove Color"), false, 22.0).clicked() {
                    cs.pop();
                    changed = true;
                }
                if cs.len() > 1 && widgets::icon_button(ui, "rotate-cw", tl!("Shuffle Colors"), false, 22.0).clicked() {
                    cs.rotate_left(1);
                    changed = true;
                }
            });
            // Palettes to start from: a strip of each.
            ui.horizontal_wrapped(|ui| {
                for pal in PALETTES {
                    let (r, resp) = ui.allocate_exact_size(vec2(40.0, 14.0), Sense::click());
                    let n = pal.len().max(1) as f32;
                    for (k, c) in pal.iter().enumerate() {
                        let [cr, cg, cb] = hex_rgb(c);
                        let x0 = r.left() + r.width() * k as f32 / n;
                        let x1 = r.left() + r.width() * (k + 1) as f32 / n;
                        ui.painter().rect_filled(Rect::from_x_y_ranges(x0..=x1, r.y_range()), 0.0, egui::Color32::from_rgb(cr, cg, cb));
                    }
                    let border = if resp.hovered() { t.accent } else { t.border };
                    ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, border), StrokeKind::Outside);
                    if resp.clicked() {
                        cs = pal.iter().map(|c| c.to_string()).collect();
                        changed = true;
                    }
                }
            });
            if changed {
                d.fields.insert("colors".into(), json!(cs));
            }
            ui.add_space(8.0);

            widgets::subheader(ui, tl!("Fill Style"));
            let style = d.fields.get("fillStyle").and_then(Value::as_str).unwrap_or("flat").to_string();
            ui.horizontal(|ui| {
                for (value, label) in STYLES {
                    let mut p = params(d, 0.0, 0.0, 40.0, 30.0);
                    p["fillStyle"] = json!(value);
                    p["shape"] = json!("classic");
                    p["segments"] = json!(1);
                    p["count"] = json!(2);
                    p["thickness"] = json!(9.0);
                    p["spacing"] = json!(2.0);
                    if tile(ui, style == value, label, 50.0, |ui| {
                        arch_image(ui, &p, vec2(40.0, 30.0));
                    }) {
                        d.fields.insert("fillStyle".into(), json!(value));
                    }
                    ui.add_space(3.0);
                }
            });
        });
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Presets"));
    ui.horizontal_wrapped(|ui| {
        for preset in &PRESETS {
            let p = json!({
                "x": 0, "y": 0, "width": 60, "height": 44, "shape": preset.shape, "count": preset.count,
                "thickness": preset.thickness / 5.0, "spacing": preset.spacing / 5.0, "roundness": preset.roundness,
                "ends": if preset.closed { "closed" } else { "open" }, "fillStyle": preset.style, "colors": preset.colors,
            });
            if tile(ui, false, preset.label, 72.0, |ui| {
                arch_image(ui, &p, vec2(60.0, 44.0));
            }) {
                apply_preset(d, preset);
            }
            ui.add_space(4.0);
        }
    });
    false
}

fn apply_preset(d: &mut Dialog, p: &Preset) {
    for (k, v) in [
        ("shape", json!(p.shape)),
        ("count", json!(p.count)),
        ("thickness", json!(p.thickness)),
        ("spacing", json!(p.spacing)),
        ("roundness", json!(p.roundness)),
        ("rings", json!(p.closed)),
        ("ends", json!(if p.closed { "closed" } else { "open" })),
        ("fillStyle", json!(p.style)),
        ("colors", json!(p.colors)),
    ] {
        d.fields.insert(k.into(), v);
    }
}

/// OK: set the tool's options; opened by a click, draw an arch there with them.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let values = options(d);
    let at = d.fields.get("x").and_then(Value::as_f64).zip(d.fields.get("y").and_then(Value::as_f64));
    let set = run_and_close(app, "tool.setOption", json!({ "tool": "arch", "values": values }))?;
    let Some((x, y)) = at else { return Ok(set) };
    let (w, h) = (d.f64("width", 300.0).max(1.0), d.f64("height", 220.0).max(1.0));
    app.run("shape.arch", params(d, x, y, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app
    }

    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    fn arches(app: &VectorcraftApp) -> Vec<(u64, usize)> {
        let doc = &app.session.doc().unwrap().doc;
        doc.layers[0]
            .children()
            .unwrap()
            .iter()
            .filter(|n| n.name.as_deref() == Some("Arch"))
            .map(|n| {
                let NodeKind::Group { children, .. } = &n.kind else { panic!("an arch is a group") };
                (n.id.0, children.len())
            })
            .collect()
    }

    #[test]
    fn a_click_with_the_tool_opens_the_options_and_ok_draws_there() {
        let mut app = app();
        app.select_tool("arch");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 100.0, 120.0), view);
        }
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let text = painted(&mut app);
        for label in [
            "Arch Tool Options",
            "Shape",
            "Classic",
            "Segmented",
            "Arch Count:",
            "Color Palette",
            "Fill Style",
            "Pearlescent",
            "Presets",
            "Dream Tunnel",
        ] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        // A preset, then OK: the tool keeps it and an arch is drawn at the click.
        let d = app.ui.dialog.as_mut().unwrap();
        apply_preset(d, &PRESETS[2]);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let a = arches(&app);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].1, 4, "Double Arch: two jelly bands, each with its highlight");
        let doc = &app.session.doc().unwrap().doc;
        let b = doc.bounds_of(&[vectorcraft_engine::doc::NodeId(a[0].0)], false).unwrap();
        assert!((b.x0 - 100.0).abs() < 0.5 && (b.y0 - 120.0).abs() < 0.5, "drawn at the click: {b:?}");
        let o = app.session.tool_options_of("arch");
        assert_eq!((o["shape"].as_str(), o["count"].as_u64(), o["fillStyle"].as_str()), (Some("classic"), Some(2), Some("jelly")));
    }

    #[test]
    fn a_double_click_on_the_tool_sets_the_options_without_drawing() {
        let mut app = app();
        open(&mut app, None);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("shape".into(), json!("pointed"));
        d.fields.insert("count".into(), json!(3));
        d.fields.insert("rings".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(arches(&app).is_empty());
        let o = app.session.tool_options_of("arch");
        assert_eq!((o["shape"].as_str(), o["count"].as_u64(), o["ends"].as_str()), (Some("pointed"), Some(3), Some("closed")));
        // A drag now draws with them.
        app.select_tool("arch");
        let view = app.view_info();
        for (kind, x, y) in [(PointerKind::Down, 100.0, 100.0), (PointerKind::Drag, 300.0, 260.0), (PointerKind::Up, 300.0, 260.0)] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, y), view);
        }
        assert_eq!(arches(&app).first().map(|a| a.1), Some(3));
    }
}
