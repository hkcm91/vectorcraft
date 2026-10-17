//! Dimension Tool Options (double-click the Dimension tool): the options the next dimension is
//! drawn with — its type and a linear dimension's orientation, the units, precision and scale of
//! the measurement, the ends, line weight, font size and colour, and whether the label shows the
//! units. OK sets them with `tool.setOption {tool: "dimension", values}`; dimensions already drawn
//! keep the look they were drawn with.
//!
//! Fields: `type` (linear|angular|radial), `orientation` (auto|aligned|horizontal|vertical),
//! `units` (document or a unit key), `precision`, `scale`, `ends` (arrow|tick|dot|none),
//! `strokeWidth` and `fontSize` (points), `color` ("#rrggbb"), `showUnits`.

use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_tools::dimension::{ENDS, NUMBERS, ORIENTATIONS, TYPES};

use super::document_setup::check;
use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Dimension Tool Options.
pub const KIND: &str = "dimensionOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Dimension Tool Options").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

const LABEL_W: f32 = 110.0;
const FIELD_W: f32 = 150.0;

/// Open the Dimension Tool Options with the tool's values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let fields = match app.session.tool_options_of(vectorcraft_tools::dimension::ID) {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

/// A choice's label (listed as literals so the catalogs are checked for them).
fn shown(value: &str) -> &'static str {
    match value {
        "linear" => tl!("Linear"),
        "angular" => tl!("Angular"),
        "radial" => tl!("Radial"),
        "auto" => tl!("Auto"),
        "aligned" => tl!("Aligned"),
        "horizontal" => tl!("Horizontal"),
        "vertical" => tl!("Vertical"),
        "arrow" => tl!("Arrow"),
        "tick" => tl!("Tick"),
        "dot" => tl!("Dot"),
        "none" => tl!("None"),
        "document" => tl!("Document"),
        _ => "",
    }
}

/// A dropdown row choosing one of `values` for `key`.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, values: &[&str]) {
    let cur = d.fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    widgets::label_row(ui, label, LABEL_W, |ui| {
        let labels: Vec<&str> = values.iter().map(|v| shown(v)).collect();
        if let Some(v) = widgets::dropdown_names(ui, ("dim", key), shown(&cur), &labels, FIELD_W).and_then(|i| values.get(i)) {
            d.fields.insert(key.into(), json!(v));
        }
    });
}

fn number(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, key: &str, label: &str) {
    let Some(&(_, default, lo, hi)) = NUMBERS.iter().find(|(k, ..)| *k == key) else { return };
    widgets::label_row(ui, label, LABEL_W, |ui| match key {
        "strokeWidth" | "fontSize" => {
            if form::length(ui, d, key, unit, 90.0) {
                let v = d.f64(key, default).clamp(lo, hi);
                d.fields.insert(key.into(), json!(v));
            }
        }
        _ => {
            let (prefix, decimals) = if key == "scale" { ("1 : ", 4) } else { ("", 0) };
            if !prefix.is_empty() {
                ui.label(prefix);
            }
            if let Some(v) = widgets::range_field(ui, ("dim", key), d.f64(key, default), lo..=hi, "", decimals, 90.0) {
                d.fields.insert(key.into(), json!(v));
            }
        }
    });
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    choice(ui, d, "type", tl!("Type:"), &TYPES);
    let linear = d.fields.get("type").and_then(Value::as_str) == Some("linear");
    ui.add_enabled_ui(linear, |ui| choice(ui, d, "orientation", tl!("Orientation:"), &ORIENTATIONS));
    ui.add_space(6.0);
    // Units: the document's General unit, or a fixed one.
    let cur = d.fields.get("units").and_then(Value::as_str).unwrap_or("document").to_string();
    widgets::label_row(ui, tl!("Units:"), LABEL_W, |ui| {
        let mut labels = vec![shown("document")];
        labels.extend(Unit::ALL.iter().map(|u| tl!(u.label())));
        let current = Unit::named(&cur).map_or(shown("document"), |u| tl!(u.label()));
        if let Some(i) = widgets::dropdown_names(ui, ("dim", "units"), current, &labels, FIELD_W) {
            let v = if i == 0 { "document" } else { Unit::ALL.get(i - 1).map_or("document", |u| u.key()) };
            d.fields.insert("units".into(), json!(v));
        }
    });
    let unit = Unit::Points;
    number(ui, d, unit, "precision", tl!("Precision:"));
    number(ui, d, unit, "scale", tl!("Scale:"));
    ui.add_space(6.0);
    choice(ui, d, "ends", tl!("Ends:"), &ENDS);
    number(ui, d, unit, "strokeWidth", tl!("Line Weight:"));
    number(ui, d, unit, "fontSize", tl!("Font Size:"));
    widgets::label_row(ui, tl!("Color:"), LABEL_W, |ui| {
        let hex = d.fields.get("color").and_then(Value::as_str).unwrap_or("#000000").to_string();
        let mut rgb = vectorcraft_color::Color::from_hex(&hex).map(|c| c.to_rgba8(1.0)).map_or([0, 0, 0], |[r, g, b, _]| [r, g, b]);
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            d.fields.insert("color".into(), json!(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])));
        }
    });
    if !d.fields.contains_key("showUnits") {
        d.fields.insert("showUnits".into(), json!(true));
    }
    check(ui, d, "showUnits", tl!("Show Units"));
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let keys = ["type", "orientation", "units", "ends", "color", "showUnits"].into_iter().chain(NUMBERS.iter().map(|(k, ..)| *k));
    let values: Map<String, Value> = keys.filter_map(|k| d.fields.get(k).map(|v| (k.to_string(), v.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": vectorcraft_tools::dimension::ID, "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 600})).unwrap();
        app
    }

    #[test]
    fn the_options_set_what_the_next_dimension_is_drawn_with() {
        let mut app = app();
        crate::toolbar::open_options(&mut app, "dimension").unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in [
            "Dimension Tool Options",
            "Type:",
            "Linear",
            "Orientation:",
            "Units:",
            "Precision:",
            "Scale:",
            "Ends:",
            "Line Weight:",
            "Font Size:",
            "Show Units",
        ] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        for (k, v) in [("units", json!("mm")), ("precision", json!(1)), ("showUnits", json!(true))] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        let o = app.session.tool_options_of("dimension");
        assert_eq!((o["units"].as_str(), o["precision"].as_f64()), (Some("mm"), Some(1.0)));
        // Two clicks, then a click to place: a dimension of 72 pt reads 25.4 mm.
        app.select_tool("dimension");
        let view = app.view_info();
        for (x, y) in [(100.0, 100.0), (172.0, 100.0), (136.0, 60.0)] {
            for kind in [PointerKind::Down, PointerKind::Up] {
                crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, y), view);
            }
        }
        let doc = &app.session.doc().unwrap().doc;
        let dim = doc.layers[0].children().unwrap().iter().find(|n| n.name.as_deref() == Some("Dimension")).expect("a dimension");
        let NodeKind::Group { children, .. } = &dim.kind else { panic!("a group") };
        let label = children.iter().find_map(|c| if let NodeKind::Text(t) = &c.kind { Some(t.plain_text()) } else { None });
        assert_eq!(label.as_deref(), Some("25.4 mm"));
    }
}
