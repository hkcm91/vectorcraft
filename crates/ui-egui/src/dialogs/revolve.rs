//! Compact controls for the 3D and Materials effects: Extrude & Bevel, Revolve, Inflate and Rotate.
//! The object and the rotation and light gizmos live on the artboard. All document changes run
//! through the effect dialog's existing preview/commit commands.

use egui::{Align2, FontId, Id, Pos2, Sense, Stroke, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{EXTRUDE, INFLATE, REVOLVE, ROTATION_PRESETS};

use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, scrub, widgets};

const LABEL_W: f32 = 102.0;
const VALUE_W: f32 = 74.0;
const SIDE_W: f32 = 354.0;

/// Sized to the screen, with scrolling inside the body so OK and Cancel stay reachable.
pub(super) fn body(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    // Values start in drag mode; clicking or Tab enters their text editor.
    ui.data_mut(|m| m.remove::<bool>(widgets::dialog_focus_flag()));
    let screen = ui.ctx().content_rect();
    let width = (screen.width() - 64.0).clamp(240.0, SIDE_W);
    // Tall enough for Extrude & Bevel's Object tab, the longest, where the screen has the room.
    let height = (screen.height() - 240.0).clamp(120.0, 450.0);
    ui.set_width(width);
    egui::ScrollArea::vertical().id_salt("revolve-body").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        controls(app, ui, d);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.allocate_ui_with_layout(vec2((width - 100.0).max(80.0), 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(super::revolve_gizmo::hint(d)).size(11.0).color(t.text_dim)).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let id = d.str("__effect");
            let defaults = vectorcraft_effects::default_params(&id).unwrap_or(json!({}));
            let changed = defaults.as_object().is_some_and(|m| {
                m.iter().any(|(k, v)| v.as_f64().map_or_else(|| d.fields.get(k) != Some(v), |default| (d.f64(k, default) - default).abs() > 1e-6))
            });
            let (r, _) = ui.allocate_exact_size(vec2(88.0, 24.0), Sense::hover());
            let response = ui.interact(r, Id::new("revolve-reset-all"), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Reset all")));
            if response.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            ui.painter().text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, tl!("Reset all"), FontId::proportional(11.5), t.text);
            dot(ui, r.right_center() - vec2(10.0, 0.0), changed);
            let tip =
                if id == REVOLVE { tl!("Reset all Revolve settings to their defaults.") } else { tl!("Reset all 3D settings to their defaults.") };
            if response.on_hover_text(tip).clicked()
                && let Some(defaults) = defaults.as_object()
            {
                d.fields.extend(defaults.clone());
            }
        });
    });
}

fn controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let id = d.str("__effect");
    if id != REVOLVE {
        return solid_controls(app, ui, d, &id);
    }
    let current = if super::revolve_gizmo::lighting(d) { 2 } else { usize::from(d.str("__tab") == "material") };
    if let Some(tab) = widgets::tab_bar(ui, &[tl!("Rotation"), tl!("Material"), tl!("Lighting")], current) {
        d.fields.insert("__gizmo".into(), json!(if tab == 2 { "light" } else { "rotation" }));
        d.fields.insert("__tab".into(), json!(if tab == 1 { "material" } else { "rotation" }));
    }
    ui.add_space(8.0);
    if super::revolve_gizmo::lighting(d) {
        lighting_controls(ui, d, app.session.general_unit());
        return;
    }
    if d.str("__tab") == "material" {
        material_controls(ui, d);
        graphics_controls(app, ui, d);
        return;
    }
    let unit = app.session.general_unit();
    heading(ui, tl!("Shape"));
    number(ui, d, "angle", tl!("Revolve angle"), (0.0, 360.0), "°", 1, None);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Axis side"));
        let edge = d.str("edge");
        let current = if edge == "right" { tl!("Right") } else { tl!("Left") };
        let width = VALUE_W + rail_width(ui) + 6.0;
        if let Some(k) = widgets::dropdown(ui, "revolve-edge", current, &[tl!("Left"), tl!("Right")], width) {
            d.fields.insert("edge".into(), json!(if k == 0 { "left" } else { "right" }));
        }
        if reset_dot(ui, "edge", edge != "left", tl!("Left")) {
            d.fields.insert("edge".into(), json!("left"));
        }
    });
    number(ui, d, "offset", tl!("Offset"), (0.0, 100000.0), "", 1, Some(unit));
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, tl!("View"));
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        number(ui, d, key, name, (-360.0, 360.0), "°", 1, None);
    }
    number(ui, d, "perspective", tl!("Perspective"), (0.0, 100.0), "%", 1, None);
    ui.add_space(6.0);
    egui::CollapsingHeader::new(tl!("Advanced")).id_salt("revolve-advanced").show(ui, |ui| {
        number(ui, d, "segments", tl!("Segments"), (8.0, 128.0), "", 0, None);
        super::form::check(ui, d, "__showAxis", tl!("Show axis"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let visible = d.fields.get("expandVisibleOnly").and_then(Value::as_bool).unwrap_or(true);
            let tip = tl!("Trim covered geometry on Expand Appearance. Live preview keeps the full surface. Transparent paints and surfaces exceeding visibility limits retain all faces.");
            let toggled = ui.allocate_ui_with_layout(
                vec2((ui.available_width() - 30.0).max(100.0), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    widgets::check_tip(ui, tl!("Keep visible surfaces only when expanding"), visible, true, tip)
                },
            ).inner;
            if toggled {
                d.fields.insert("expandVisibleOnly".into(), json!(!visible));
            }
            if reset_dot(ui, "expandVisibleOnly", !visible, "true") {
                d.fields.insert("expandVisibleOnly".into(), json!(true));
            }
        });
    });
}

/// `s` as the 3D effects mean it (a solid's cap, not a stroke's; a bevelled edge, not a join).
fn c3(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "3d", s)
}

/// Extrude & Bevel and Inflate: Object, Rotation and Lighting tabs (Rotate has no Object tab).
/// The rotation gizmo shows on the first two, the light gizmo on Lighting.
fn solid_controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, id: &str) {
    let object = id == EXTRUDE || id == INFLATE;
    let mut tabs = vec![];
    if object {
        tabs.push(tl!("Object"));
    }
    tabs.extend([tl!("Rotation"), tl!("Material"), tl!("Lighting")]);
    let tab_key = d.str("__tab");
    let current = if super::revolve_gizmo::lighting(d) {
        tabs.len() - 1
    } else if tab_key == "material" {
        tabs.len() - 2
    } else if object && tab_key != "rotation" {
        0
    } else {
        tabs.len() - 3
    };
    if let Some(tab) = widgets::tab_bar(ui, &tabs, current) {
        let light = tab + 1 == tabs.len();
        d.fields.insert("__gizmo".into(), json!(if light { "light" } else { "rotation" }));
        let key = if tab + 2 == tabs.len() {
            "material"
        } else if object && tab == 0 {
            "object"
        } else {
            "rotation"
        };
        d.fields.insert("__tab".into(), json!(key));
    }
    ui.add_space(8.0);
    if super::revolve_gizmo::lighting(d) {
        lighting_controls(ui, d, app.session.general_unit());
    } else if d.str("__tab") == "material" {
        material_controls(ui, d);
        graphics_controls(app, ui, d);
    } else if object && d.str("__tab") != "rotation" {
        let unit = app.session.general_unit();
        if id == EXTRUDE {
            let layers = app
                .session
                .active()
                .and_then(|doc| doc.selection.objects.first().and_then(|id| doc.doc.node(*id)))
                .map_or(0, vectorcraft_effects::layer_count);
            extrude_controls(ui, d, unit, layers);
        } else {
            heading(ui, tl!("Object"));
            number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
            number(ui, d, "twist", tl!("Twist"), (0.0, 360.0), "°", 1, None);
            number(ui, d, "taper", tl!("Taper"), (1.0, 100.0), "%", 1, None);
            choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
            number(ui, d, "volume", tl!("Volume"), (0.0, 100.0), "%", 1, None);
            number(ui, d, "pillow", tl!("Pillow"), (0.0, 100.0), "%", 1, None);
            toggle(ui, d, "bothSides", tl!("Both sides"), false);
        }
    } else {
        view_controls(ui, d);
    }
}

/// Extrude & Bevel's depth, twist, taper, cap and bevel.
fn extrude_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, layers: usize) {
    heading(ui, tl!("Object"));
    number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
    layer_depth_controls(ui, d, unit, layers);
    number(ui, d, "twist", tl!("Twist"), (0.0, 360.0), "°", 1, None);
    number(ui, d, "taper", tl!("Taper"), (1.0, 100.0), "%", 1, None);
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, c3("Bevel"));
    let shapes = [
        (json!("none"), tl!("None")),
        (json!("classic"), tl!("Classic")),
        (json!("rounded"), c3("Rounded")),
        (json!("cove"), tl!("Cove")),
        (json!("step"), tl!("Step")),
        (json!("ogee"), tl!("Ogee")),
    ];
    choice(ui, d, "bevel", tl!("Bevel Shape"), &shapes);
    ui.add_enabled_ui(d.str("bevel") != "none", |ui| {
        number(ui, d, "bevelWidth", tl!("Width"), (0.0, 1000.0), "", 1, Some(unit));
        number(ui, d, "bevelHeight", tl!("Height"), (0.0, 1000.0), "", 1, Some(unit));
        number(ui, d, "bevelRepeat", tl!("Repeat"), (1.0, 32.0), "", 0, None);
        number(ui, d, "bevelSpace", c3("Space"), (0.0, 90.0), "%", 1, None);
        toggle(ui, d, "bevelInside", tl!("Bevel Inside"), false);
        toggle(ui, d, "bevelBothSides", tl!("Bevel both sides"), false);
    });
}

/// On a group of `layers` shapes, each one's depth (`layerDepths`, bottom first). Until one is
/// changed every layer follows Depth; Use Depth for All goes back to that.
fn layer_depth_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, layers: usize) {
    if layers < 2 {
        return;
    }
    let depth = d.f64("depth", 50.0);
    let own = vectorcraft_effects::layer_depths(&Value::Object(d.fields.clone()));
    ui.add_space(6.0);
    heading(ui, tl!("Layer Depths"));
    let mut depths = Vec::with_capacity(layers);
    let mut changed = false;
    for k in 0..layers {
        let key = format!("__layerDepth{k}");
        let was = own.get(k).copied().filter(|v| v.is_finite()).unwrap_or(depth);
        d.fields.insert(key.clone(), json!(was));
        let name = crate::i18n::fmt(if k == 0 { tl!("Layer {n} (bottom)") } else { tl!("Layer {n}") }, &[("n", &(k + 1).to_string())]);
        number(ui, d, &key, &name, (0.0, 2000.0), "", 1, Some(unit));
        let now = d.f64(&key, was);
        changed |= (now - was).abs() > 1e-9;
        depths.push(now);
    }
    if changed {
        d.fields.insert("layerDepths".into(), json!(depths));
    }
    if !own.is_empty() && widgets::flat_button(ui, tl!("Use Depth for All"), 130.0).clicked() {
        d.fields.insert("layerDepths".into(), json!([]));
    }
}

/// Rotation presets, rotation and perspective for Extrude & Bevel, Inflate and Rotate.
fn view_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    heading(ui, tl!("Rotation"));
    let names = [
        tl!("Front"),
        tl!("Back"),
        tl!("Left"),
        tl!("Right"),
        tl!("Top"),
        tl!("Bottom"),
        tl!("Off-Axis Front"),
        tl!("Off-Axis Back"),
        tl!("Off-Axis Left"),
        tl!("Off-Axis Right"),
        tl!("Off-Axis Top"),
        tl!("Off-Axis Bottom"),
        tl!("Isometric Left"),
        tl!("Isometric Right"),
        tl!("Isometric Top"),
        tl!("Isometric Bottom"),
    ];
    let angles = [d.f64("rotationX", 0.0), d.f64("rotationY", 0.0), d.f64("rotationZ", 0.0)];
    let current = ROTATION_PRESETS.iter().position(|(_, a)| a.iter().zip(angles).all(|(a, b)| (a - b).abs() < 0.005));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Presets"));
        let width = VALUE_W + rail_width(ui) + 6.0;
        let shown = current.and_then(|i| names.get(i).copied()).unwrap_or(tl!("Custom"));
        if let Some(k) = widgets::dropdown(ui, "three-d-position", shown, &names, width)
            && let Some((_, [x, y, z])) = ROTATION_PRESETS.get(k)
        {
            d.fields.extend([("rotationX".into(), json!(x)), ("rotationY".into(), json!(y)), ("rotationZ".into(), json!(z))]);
        }
    });
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        number(ui, d, key, name, (-180.0, 180.0), "°", 2, None);
    }
    number(ui, d, "perspective", tl!("Perspective"), (0.0, 100.0), "%", 1, None);
}

/// A dropdown of `options` (value, label) for `key`, with its reset dot.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, options: &[(Value, &str)]) {
    let id = d.str("__effect");
    let default = vectorcraft_effects::default_params(&id).and_then(|p| p.get(key).cloned()).unwrap_or(Value::Null);
    let value = d.fields.get(key).cloned().unwrap_or_else(|| default.clone());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        let current = options.iter().find(|(v, _)| *v == value).map_or("", |(_, l)| *l);
        let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
        let width = VALUE_W + rail_width(ui) + 6.0;
        if let Some(k) = widgets::dropdown(ui, ("three-d-choice", key), current, &labels, width)
            && let Some((v, _)) = options.get(k)
        {
            d.fields.insert(key.into(), v.clone());
        }
        let default_label = options.iter().find(|(v, _)| *v == default).map_or("", |(_, l)| *l);
        if reset_dot(ui, key, value != default, default_label) {
            d.fields.insert(key.into(), default.clone());
        }
    });
}

/// A checkbox for `key`, with its reset dot.
fn toggle(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, default: bool) {
    ui.horizontal(|ui| {
        let on = d.bool(key);
        if widgets::check(ui, name, on, ui.is_enabled()) {
            d.fields.insert(key.into(), json!(!on));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, key, on != default, if default { "true" } else { "false" }) {
            d.fields.insert(key.into(), json!(default));
        }
    });
}

/// The lighting presets' names, in [`vectorcraft_effects::LIGHTING_PRESETS`] order.
fn lighting_preset_names() -> [&'static str; 7] {
    [tl!("Standard"), tl!("Studio"), tl!("Soft"), tl!("Top Light"), tl!("Dramatic"), tl!("Warm & Cool"), tl!("Flat")]
}

/// The material presets' names, in [`vectorcraft_effects::MATERIAL_PRESETS`] order.
fn material_preset_names() -> [&'static str; 8] {
    [c3("Matte"), tl!("Soft Touch"), tl!("Plastic"), tl!("Glossy"), tl!("Metal"), tl!("Chrome"), tl!("Rim Glow"), tl!("Toon")]
}

/// A Presets dropdown showing `names[current]` (Custom when None); the picked index.
fn presets(ui: &mut egui::Ui, salt: &str, names: &[&str], current: Option<usize>) -> Option<usize> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Presets"));
        let width = VALUE_W + rail_width(ui) + 6.0;
        let shown = current.and_then(|i| names.get(i).copied()).unwrap_or(tl!("Custom"));
        picked = widgets::dropdown(ui, ("three-d-presets", salt), shown, names, width);
    });
    picked
}

/// The Material tab: how the surface takes the light.
fn material_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    let id = d.str("__effect");
    let defaults = vectorcraft_effects::default_params(&id).unwrap_or(Value::Null);
    let value = |d: &Dialog, k: &str| d.f64(k, defaults.get(k).and_then(Value::as_f64).unwrap_or(0.0));
    let current = vectorcraft_effects::MATERIAL_PRESETS.iter().position(|p| p.matches(&d.fields, &defaults));
    heading(ui, tl!("Material"));
    if let Some(k) = presets(ui, "material", &material_preset_names(), current)
        && let Some(p) = vectorcraft_effects::MATERIAL_PRESETS.get(k)
    {
        d.fields.extend(p.params());
    }
    number(ui, d, "specular", tl!("Highlight"), (0.0, 100.0), "%", 1, None);
    ui.add_enabled_ui(value(d, "specular") > 0.0, |ui| {
        number(ui, d, "roughness", tl!("Roughness"), (0.0, 100.0), "%", 1, None);
    });
    number(ui, d, "metallic", tl!("Metallic"), (0.0, 100.0), "%", 1, None);
    let backdrops = [
        (json!("gradient"), tl!("Gradient")),
        (json!("studio"), tl!("Studio")),
        (json!("outdoor"), tl!("Outdoor")),
        (json!("sunset"), tl!("Sunset")),
        (json!("dark"), tl!("Dark")),
    ];
    // What metal and glossy surfaces mirror.
    ui.add_enabled_ui(value(d, "specular") > 0.0 || value(d, "metallic") > 0.0, |ui| choice(ui, d, "environment", tl!("Reflections"), &backdrops));
    ui.add_space(8.0);
    heading(ui, tl!("Rim Light"));
    number(ui, d, "rimLight", tl!("Intensity"), (0.0, 100.0), "%", 1, None);
    ui.add_enabled_ui(value(d, "rimLight") > 0.0, |ui| colour(ui, d, "rimColor", tl!("Color")));
    ui.add_space(8.0);
    heading(ui, tl!("Shading"));
    number(ui, d, "shadingSteps", tl!("Cel Bands"), (0.0, f64::from(vectorcraft_three_d::MAX_SHADING_STEPS)), "", 0, None);
    toggle(ui, d, "smoothShading", tl!("Smooth shading"), true);
    // Ambient occlusion needs the per-pixel shading's depth.
    ui.add_enabled_ui(d.bool("smoothShading"), |ui| {
        number(ui, d, "ambientOcclusion", tl!("Ambient Occlusion"), (0.0, 100.0), "%", 1, None);
    });
}

fn lighting_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    let defaults = vectorcraft_effects::default_params(&d.str("__effect")).unwrap_or(Value::Null);
    let current = vectorcraft_effects::LIGHTING_PRESETS.iter().position(|p| p.matches(&d.fields, &defaults));
    if let Some(k) = presets(ui, "lighting", &lighting_preset_names(), current)
        && let Some(p) = vectorcraft_effects::LIGHTING_PRESETS.get(k)
    {
        d.fields.extend(p.params());
        d.fields.insert("shade".into(), json!(true));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let shade = d.bool("shade");
        if widgets::check(ui, tl!("Shading"), shade, true) {
            d.fields.insert("shade".into(), json!(!shade));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shade", !shade, "true") {
            d.fields.insert("shade".into(), json!(true));
        }
    });
    ui.add_enabled_ui(d.bool("shade"), |ui| {
        number(ui, d, "lightAzimuth", tl!("Direction"), (-360.0, 360.0), "°", 1, None);
        number(ui, d, "lightElevation", tl!("Elevation"), (-90.0, 90.0), "°", 1, None);
        number(ui, d, "lightIntensity", tl!("Intensity"), (0.0, 100.0), "%", 1, None);
        colour(ui, d, "lightColor", tl!("Color"));
        number(ui, d, "lightSoftness", tl!("Softness"), (0.0, 100.0), "%", 1, None);
        number(ui, d, "ambient", tl!("Ambient"), (0.0, 100.0), "%", 1, None);
        colour(ui, d, "ambientColor", tl!("Ambient Color"));
        // More lights and the shadow fold away until they are used, keeping the tab compact.
        let more = d.fields.get("lights").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
        egui::CollapsingHeader::new(tl!("More Lights")).id_salt("three-d-more-lights").default_open(more).show(ui, |ui| extra_lights(ui, d));
    });
    let on = d.fields.get("shadow").and_then(Value::as_str).is_some_and(|s| s != "none");
    egui::CollapsingHeader::new(tl!("Shadows")).id_salt("three-d-shadows").default_open(on).show(ui, |ui| {
        let positions = [(json!("none"), tl!("None")), (json!("below"), tl!("Below Object")), (json!("behind"), tl!("Behind Object"))];
        choice(ui, d, "shadow", tl!("Position"), &positions);
        let on = d.fields.get("shadow").and_then(Value::as_str).is_some_and(|s| s != "none");
        ui.add_enabled_ui(on, |ui| {
            // Straight down onto the ground (or straight back onto the wall), or along the light.
            let directions = [(json!("straight"), tl!("Straight")), (json!("light"), tl!("From Light"))];
            choice(ui, d, "shadowDirection", tl!("Direction"), &directions);
            number(ui, d, "shadowDistance", tl!("Distance"), (0.0, 100000.0), "", 1, Some(unit));
            number(ui, d, "shadowOpacity", tl!("Opacity"), (0.0, 100.0), "%", 1, None);
            number(ui, d, "shadowSoftness", tl!("Softness"), (0.0, 100.0), "%", 1, None);
            number(ui, d, "contactShadow", tl!("Contact Shadow"), (0.0, 100.0), "%", 1, None);
        });
    });
}

/// A colour value (`#rrggbb`) for `key`, picked with the colour button, and its reset dot.
fn colour(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str) {
    let default = vectorcraft_effects::default_params(&d.str("__effect")).and_then(|p| p.get(key).and_then(Value::as_str).map(String::from));
    let default = default.unwrap_or_else(|| "#ffffff".into());
    let hex = d.fields.get(key).and_then(Value::as_str).map_or_else(|| default.clone(), String::from);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        if let Some(v) = colour_button(ui, &hex) {
            d.fields.insert(key.into(), json!(v));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, key, !hex.eq_ignore_ascii_case(&default), &default) {
            d.fields.insert(key.into(), json!(default));
        }
    });
}

/// egui's colour button for a `#rrggbb` value: the new value when it changed.
fn colour_button(ui: &mut egui::Ui, hex: &str) -> Option<String> {
    let mut rgb = vectorcraft_color::Color::from_hex(hex).map(|c| c.to_rgba8(1.0)).map_or([255, 255, 255], |[r, g, b, _]| [r, g, b]);
    ui.color_edit_button_srgb(&mut rgb).changed().then(|| format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

/// The lights besides the main one (`lights`, up to three): each with its direction, elevation,
/// intensity, colour and softness, and a button to delete it; Add Light below.
fn extra_lights(ui: &mut egui::Ui, d: &mut Dialog) {
    let mut lights: Vec<Value> = d.fields.get("lights").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut changed = false;
    let mut delete = None;
    for (i, light) in lights.iter_mut().enumerate() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            heading(ui, &format!("{} {}", tl!("Light Source"), i + 2));
            let hex = light.get("color").and_then(Value::as_str).unwrap_or("#ffffff").to_string();
            if let Some(v) = colour_button(ui, &hex) {
                light["color"] = json!(v);
                changed = true;
            }
            if widgets::icon_button(ui, "trash-2", tl!("Delete Light"), false, 22.0).clicked() {
                delete = Some(i);
            }
        });
        for (key, name, lo, hi, default, suffix) in [
            ("azimuth", tl!("Direction"), -360.0, 360.0, 45.0, "°"),
            ("elevation", tl!("Elevation"), -90.0, 90.0, 30.0, "°"),
            ("intensity", tl!("Intensity"), 0.0, 100.0, 50.0, "%"),
            ("softness", tl!("Softness"), 0.0, 100.0, 0.0, "%"),
        ] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                label(ui, name);
                let v = light.get(key).and_then(Value::as_f64).unwrap_or(default);
                if let Some(n) = widgets::range_field(ui, ("three-d-light", i, key), v, lo..=hi, suffix, 1, VALUE_W) {
                    light[key] = json!(n);
                    changed = true;
                }
            });
        }
    }
    if let Some(i) = delete {
        lights.remove(i);
        changed = true;
    }
    if lights.len() < vectorcraft_three_d::MAX_EXTRA_LIGHTS && widgets::flat_button(ui, tl!("Add Light"), 110.0).clicked() {
        lights.push(json!({"azimuth": 45.0, "elevation": 30.0, "intensity": 50.0, "color": "#ffffff", "softness": 0.0}));
        changed = true;
    }
    if changed {
        d.fields.insert("lights".into(), Value::Array(lights));
    }
}

/// The graphics laid on the surface (`graphics`, up to eight): each a symbol of the document with
/// its place across and down the object (round a Revolve, across is the way round), its scale,
/// turn and finish, and a button to delete it; Add Graphic below (the first symbol, in the middle).
fn graphics_controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let names: Vec<String> = app.session.active().map(|doc| doc.doc.symbols.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    let mut graphics: Vec<Value> = d.fields.get("graphics").and_then(Value::as_array).cloned().unwrap_or_default();
    ui.add_space(8.0);
    heading(ui, tl!("Graphics"));
    if names.is_empty() && graphics.is_empty() {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Make a symbol to lay it on the surface.")).color(t.text_dim));
        return;
    }
    let options: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut changed = false;
    let mut delete = None;
    for (i, g) in graphics.iter_mut().enumerate() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let current = g.get("symbol").and_then(Value::as_str).unwrap_or_default().to_string();
            let width = VALUE_W + rail_width(ui) + 6.0;
            if let Some(k) = widgets::dropdown_names(ui, ("three-d-graphic", i), &current, &options, width)
                && let Some(name) = options.get(k)
            {
                g["symbol"] = json!(name);
                changed = true;
            }
            if widgets::icon_button(ui, "trash-2", tl!("Delete Graphic"), false, 22.0).clicked() {
                delete = Some(i);
            }
        });
        // Matte keeps a logo as printed; Surface takes the surface's gloss or metal.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            label(ui, tl!("Finish"));
            let surface = g.get("finish").and_then(Value::as_str) == Some("surface");
            let current = if surface { c3("Surface") } else { c3("Matte") };
            let width = VALUE_W + rail_width(ui) + 6.0;
            if let Some(k) = widgets::dropdown(ui, ("three-d-graphic-finish", i), current, &[c3("Matte"), c3("Surface")], width) {
                g["finish"] = json!(if k == 0 { "matte" } else { "surface" });
                changed = true;
            }
        });
        for (key, name, lo, hi, default, suffix) in [
            ("x", tl!("Horizontal"), -100.0, 200.0, 50.0, "%"),
            ("y", tl!("Vertical"), -100.0, 200.0, 50.0, "%"),
            ("scale", tl!("Scale"), 1.0, 1000.0, 100.0, "%"),
            ("rotation", tl!("Rotation"), -360.0, 360.0, 0.0, "°"),
        ] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                label(ui, name);
                let v = g.get(key).and_then(Value::as_f64).unwrap_or(default);
                if let Some(n) = widgets::range_field(ui, ("three-d-graphic", i, key), v, lo..=hi, suffix, 1, VALUE_W) {
                    g[key] = json!(n);
                    changed = true;
                }
            });
        }
    }
    if let Some(i) = delete {
        graphics.remove(i);
        changed = true;
    }
    if let Some(first) = names.first()
        && graphics.len() < vectorcraft_effects::graphics::MAX_GRAPHICS
        && widgets::flat_button(ui, tl!("Add Graphic"), 110.0).clicked()
    {
        graphics.push(json!({"symbol": first, "x": 50.0, "y": 50.0, "scale": 100.0, "rotation": 0.0, "finish": "matte"}));
        changed = true;
    }
    if changed {
        d.fields.insert("graphics".into(), Value::Array(graphics));
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).font(crate::theme::semibold(13.0)).color(t.text));
}

fn label(ui: &mut egui::Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 24.0), Sense::hover());
    ui.put(r, egui::Label::new(egui::RichText::new(name).size(12.5).color(t.text)).halign(egui::Align::Min).truncate()).on_hover_text(name);
    scrub::note_label(ui, r);
}

fn rail_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - VALUE_W - 24.0 - 18.0).clamp(40.0, 134.0)
}

/// A value can be typed, dragged, scrubbed by its label, or moved on its slider.
#[allow(clippy::too_many_arguments)]
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, range: (f64, f64), suffix: &str, decimals: usize, unit: Option<Unit>) {
    let default = vectorcraft_effects::default_params(&d.str("__effect")).and_then(|p| p.get(key).and_then(Value::as_f64)).unwrap_or(0.0);
    let from = |n| unit.map_or(n, |u| u.from_pt(n));
    let to = |n| unit.map_or(n, |u| u.to_pt(n));
    let typed = unit.map(|u| widgets::typed_unit(ui.ctx(), u));
    let mut value = from(d.f64(key, default)).clamp(from(range.0), from(range.1));
    let suffix = unit.map_or_else(|| suffix.to_string(), |u| format!(" {}", u.suffix()));
    let mut next = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        let t = Tokens::get(ui.ctx());
        let response = ui
            .scope(|ui| {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = t.input;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
                ui.add_sized(
                    [VALUE_W, 24.0],
                    egui::DragValue::new(&mut value)
                        .range(from(range.0)..=from(range.1))
                        .speed(if decimals == 0 { 1.0 } else { 0.25 })
                        .max_decimals(decimals)
                        .suffix(&suffix)
                        .update_while_editing(false)
                        .custom_parser(|s| {
                            unit.map_or_else(
                                || vectorcraft_doc::parse_number(&s.replace(['°', '%'], "")),
                                |u| typed.unwrap_or(u).parse(s).map(|v| u.from_pt(v)),
                            )
                        }),
                )
            })
            .inner;
        response.clone().on_hover_text(tl!("Drag values or labels. Double-click a value to type."));
        if response.changed() {
            next = Some(value);
        }
        if let Some(n) = scrub::field(ui, response.id, response.rect, &value.to_string(), Some(value), decimals as i32) {
            next = Some(n);
        }
        // Lengths' rails cover a useful working range; larger values can still be typed.
        let rail_max = match key {
            "offset" | "depth" => from(500.0),
            "bevelWidth" | "bevelHeight" => from(100.0),
            _ => from(range.1),
        };
        let rail_min = from(range.0);
        let rail = rail_width(ui) + VALUE_W;
        let slider =
            widgets::color_slider(ui, ("revolve-slider", key), ((value - rail_min) / (rail_max - rail_min)).clamp(0.0, 1.0) as f32, rail, &|_| {
                t.input_border
            });
        if let (Some(n), _) = slider {
            next = Some(rail_min + f64::from(n) * (rail_max - rail_min));
        }
        if reset_dot(ui, key, (to(value) - default).abs() > 1e-6, &format!("{}{}", from(default), suffix)) {
            next = Some(from(default));
        }
    });
    if let Some(n) = next.filter(|n| n.is_finite()) {
        let factor = 10f64.powi(decimals as i32);
        let n = (n * factor).round() / factor;
        d.fields.insert(key.into(), json!(to(n.clamp(from(range.0), from(range.1)))));
    }
}

fn dot(ui: &egui::Ui, p: Pos2, changed: bool) {
    let t = Tokens::get(ui.ctx());
    if changed {
        ui.painter().circle_filled(p, 3.0, if ui.is_enabled() { t.accent } else { t.text_disabled });
    } else {
        ui.painter().circle_stroke(p, 3.0, Stroke::new(1.0, t.text_disabled));
    }
}

fn reset_dot(ui: &mut egui::Ui, key: &str, changed: bool, default: &str) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let response = ui.interact(r, Id::new(("revolve-reset", key)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tl!("Reset to default")));
    if response.hovered() {
        ui.painter().rect_filled(r, 3.0, Tokens::get(ui.ctx()).hover);
    }
    dot(ui, r.center(), changed);
    response.on_hover_text(format!("{}: {default}", tl!("Reset to default"))).clicked()
}

#[cfg(test)]
#[path = "tests_revolve.rs"]
mod tests;
