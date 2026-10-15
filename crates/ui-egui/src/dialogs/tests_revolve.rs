//! Real pointer interaction with Revolve's compact controls.
use super::*;
use crate::{dialogs, theme};
use egui::{Event, PointerButton, Rect, Vec2, pos2};
use vectorcraft_engine::Session;

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    screen: Vec2,
    texts: Vec<(String, Rect)>,
}

impl Harness {
    fn new(screen: Vec2) -> Self {
        Self::with("M150 40 L180 60 L180 140 L150 160", REVOLVE, screen)
    }

    /// The dialog of 3D effect `effect` on a path drawn from SVG path data `d`.
    fn with(d: &str, effect: &str, screen: Vec2) -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.run("path.create", json!({"d": d})).unwrap();
        app.run("paint.setFill", json!({"color":"#338ad6"})).unwrap();
        app.run("effect.dialog", json!({"effect":effect,"item":null})).unwrap();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        Self { app, ctx, screen, texts: vec![] }
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| dialogs::show(&mut self.app, ui.ctx()));
        out.textures_delta.clear();
        fn collect(shape: &egui::Shape, clip: Rect, texts: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(t) if clip.contains_rect(t.visual_bounding_rect()) => {
                    texts.push((t.galley.text().to_string(), t.visual_bounding_rect()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, clip, texts)),
                _ => {}
            }
        }
        self.texts.clear();
        out.shapes.iter().for_each(|s| collect(&s.shape, s.clip_rect, &mut self.texts));
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.frame(vec![]);
        }
    }

    fn rect(&self, text: &str) -> Rect {
        self.texts.iter().find(|(s, _)| s == text).map(|(_, r)| *r).unwrap_or_else(|| panic!("missing text {text:?}"))
    }

    fn response(&self, id: impl std::hash::Hash + std::fmt::Debug) -> Rect {
        self.ctx.read_response(Id::new(id)).unwrap().rect
    }

    fn click(&mut self, p: Pos2) {
        self.frame(vec![Event::PointerMoved(p), button(p, true)]);
        self.frame(vec![button(p, false)]);
        self.settle();
    }

    fn drag(&mut self, p: Pos2, delta: Vec2) {
        self.frame(vec![Event::PointerMoved(p), button(p, true)]);
        for i in 1..=5 {
            self.frame(vec![Event::PointerMoved(p + delta * (i as f32 / 5.0))]);
        }
        self.frame(vec![button(p + delta, false)]);
        self.settle();
    }

    fn value(&self, key: &str) -> f64 {
        self.app.ui.dialog.as_ref().unwrap().f64(key, 0.0)
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn values_labels_sliders_and_reset_dots_change_the_live_effect_in_one_undo_step() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    assert!(!h.rect("Advanced").is_negative(), "Advanced is visible without scrolling in a normal window");
    let panel = h.ctx.memory(|m| m.area_rect(Id::new(("dialog", "effect")).with("revolve"))).unwrap();
    assert!(panel.width() < 420.0 && panel.height() < 540.0, "compact controls: {panel:?}");
    let label = h.rect("Rotation X");
    h.drag(pos2(label.left() + LABEL_W + 6.0 + VALUE_W / 2.0, label.center().y), vec2(40.0, 0.0));
    assert!(h.value("rotationX") > 0.0, "the value itself drags");
    let label = h.rect("Rotation Y");
    h.drag(label.center(), vec2(40.0, 0.0));
    assert!(h.value("rotationY") > 0.0, "the label scrubs");
    let label = h.rect("Revolve angle");
    h.click(pos2(label.left() + LABEL_W + VALUE_W + 24.0, label.center().y));
    assert!(h.value("angle") < 180.0, "the slider changes the sweep");
    h.click(h.response(("revolve-reset", "rotationX")).center());
    assert_eq!(h.value("rotationX"), 0.0);
    assert!(h.value("angle") < 180.0, "a dot resets only its own setting");
    let node = h.app.session.doc().unwrap().doc.node(h.app.session.doc().unwrap().selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["angle"], json!(h.value("angle")));
    h.click(h.response("revolve-reset-all").center());
    let defaults = vectorcraft_effects::default_params(REVOLVE).unwrap();
    let dialog = h.app.ui.dialog.as_ref().unwrap();
    for (key, value) in defaults.as_object().unwrap() {
        assert_eq!(dialog.fields.get(key), Some(value), "reset all: {key}");
    }
    dialogs::confirm(&mut h.app).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn preview_off_changes_fields_without_changing_the_document() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    h.app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    let before = h.app.session.doc().unwrap().doc.clone();
    h.settle();
    h.drag(h.rect("Rotation Y").center(), vec2(40.0, 0.0));
    assert!(h.value("rotationY") > 0.0);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
    dialogs::cancel(&mut h.app);
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn command_driven_fields_preview_and_small_windows_keep_the_buttons_reachable() {
    let mut h = Harness::new(vec2(640.0, 480.0));
    h.settle();
    let dialog = h.app.ui.dialog.as_mut().unwrap();
    dialog.fields.insert("angle".into(), json!(180));
    h.settle();
    let state = h.app.session.doc().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["angle"], json!(180));
    let screen = Rect::from_min_size(Pos2::ZERO, h.screen);
    assert!(screen.contains_rect(h.rect("OK")) && screen.contains_rect(h.rect("Cancel")));
    dialogs::cancel(&mut h.app);
    assert!(!h.app.session.in_interaction());
}

#[test]
fn lighting_tab_shows_light_fields_and_keeps_the_dialog_compact() {
    let mut h = Harness::new(vec2(1280.0, 900.0));
    h.settle();
    h.click(h.rect("Lighting").center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("__gizmo"), "light");
    for label in ["Shading", "Direction", "Elevation", "Intensity", "Ambient"] {
        assert!(Rect::from_min_size(Pos2::ZERO, h.screen).contains_rect(h.rect(label)));
    }
    let panel = h.ctx.memory(|m| m.area_rect(Id::new(("dialog", "effect")).with("revolve"))).unwrap();
    // Presets, Colour, Softness and Ambient Color add four rows; More Lights and Shadows stay
    // folded until used.
    assert!(panel.width() < 420.0 && panel.height() < 580.0, "compact lighting controls: {panel:?}");
    h.drag(h.rect("Direction").center(), vec2(24.0, 0.0));
    assert_ne!(h.value("lightAzimuth"), -45.0);
    h.click(h.response(("revolve-reset", "lightAzimuth")).center());
    assert_eq!(h.value("lightAzimuth"), -45.0);
    h.click(h.rect("Rotation").center());
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("__gizmo"), "rotation");
    assert!(!h.rect("Rotation X").is_negative());
}

#[test]
fn advanced_visibility_option_is_saved_resettable_and_leaves_preview_geometry_complete() {
    let mut h = Harness::new(vec2(1280.0, 1000.0));
    h.settle();
    h.click(h.rect("Advanced").center());
    let panel = h.ctx.memory(|m| m.area_rect(Id::new(("dialog", "effect")).with("revolve"))).unwrap();
    h.frame(vec![
        Event::PointerMoved(panel.center()),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -220.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        },
    ]);
    h.settle();
    let before = vectorcraft_effects::revolve_art({
        let state = h.app.session.doc().unwrap();
        state.doc.node(state.selection.objects[0]).unwrap()
    })
    .unwrap();
    let checkbox = h.rect("Keep visible surfaces only when expanding");
    h.click(checkbox.center());
    assert!(!h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    let state = h.app.session.doc().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["expandVisibleOnly"], json!(false));
    assert_eq!(vectorcraft_effects::revolve_art(node), Some(before));
    h.click(h.response(("revolve-reset", "expandVisibleOnly")).center());
    assert!(h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    h.click(h.rect("Keep visible surfaces only when expanding").center());
    h.click(h.response("revolve-reset-all").center());
    assert!(h.app.ui.dialog.as_ref().unwrap().bool("expandVisibleOnly"));
    dialogs::confirm(&mut h.app).unwrap();
    let state = h.app.session.doc().unwrap();
    let loaded = vectorcraft_format::load(&vectorcraft_format::save(&state.doc, false)).unwrap();
    assert_eq!(loaded.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["expandVisibleOnly"], json!(true));
}

/// Extrude & Bevel, Inflate and Rotate open the same compact controls, with their own options, the
/// Position presets and the artboard gizmo turning about the centre of the art.
#[test]
fn the_other_3d_effects_show_their_options_and_position_presets() {
    use vectorcraft_effects::{EXTRUDE, INFLATE, ROTATE, ROTATION_PRESETS};
    let square = "M100 100 L200 100 L200 200 L100 200 Z";
    for (effect, own) in [
        (EXTRUDE, &["Depth", "Twist", "Taper", "Cap", "Bevel", "Bevel Shape", "Space", "Bevel Inside", "Bevel both sides"][..]),
        (INFLATE, &["Depth", "Twist", "Taper", "Cap", "Volume", "Both sides"][..]),
        (ROTATE, &[][..]),
    ] {
        let mut h = Harness::with(square, effect, vec2(1400.0, 1000.0));
        h.settle();
        // Extrude & Bevel and Inflate open on their Object tab; Rotate on Rotation.
        for text in own.iter() {
            h.rect(text);
        }
        if !own.is_empty() {
            h.app.ui.dialog.as_mut().unwrap().fields.insert("__tab".into(), json!("rotation"));
            h.settle();
        }
        for text in ["Presets", "Rotation X", "Perspective"] {
            h.rect(text);
        }
        // The defaults are the Off-Axis Front position, shown by name.
        h.rect("Off-Axis Front");
        assert!(!h.texts.iter().any(|(t, _)| t == "Revolve angle"), "{effect}: Revolve's own options stay out");
        let d = h.app.ui.dialog.as_ref().unwrap();
        assert!(dialogs::revolve_gizmo::active(d), "{effect}: the gizmo is on");
        // A preset's angles show its name; other angles are Custom.
        let [x, y, z] = ROTATION_PRESETS[14].1;
        let d = h.app.ui.dialog.as_mut().unwrap();
        d.fields.extend([("rotationX".into(), json!(x)), ("rotationY".into(), json!(y)), ("rotationZ".into(), json!(z))]);
        h.settle();
        h.rect("Isometric Top");
        h.app.ui.dialog.as_mut().unwrap().fields.insert("rotationZ".into(), json!(12.5));
        h.settle();
        h.rect("Custom");
    }
}

#[test]
fn extrude_bevel_options_enable_with_a_bevel_shape() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", vectorcraft_effects::EXTRUDE, vec2(1400.0, 1000.0));
    h.settle();
    let width = h.response(("revolve-reset", "bevelWidth"));
    assert!(!h.ctx.read_response(Id::new(("revolve-reset", "bevelWidth"))).unwrap().enabled(), "no bevel: its sizes are off");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("bevel".into(), json!("rounded"));
    h.settle();
    assert!(h.ctx.read_response(Id::new(("revolve-reset", "bevelWidth"))).unwrap().enabled());
    assert_eq!(h.response(("revolve-reset", "bevelWidth")), width);
}

#[test]
fn lighting_tab_adds_lights_and_a_shadow_to_the_live_effect() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", vectorcraft_effects::EXTRUDE, vec2(1400.0, 1100.0));
    h.settle();
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("__gizmo".into(), json!("light"));
    h.settle();
    for label in ["Color", "Softness", "More Lights", "Shadows"] {
        h.rect(label);
    }
    // Shadows opens on a click; a position enables its distance.
    h.click(h.rect("Shadows").center());
    h.rect("Position");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("shadow".into(), json!("below"));
    h.settle();
    h.rect("Distance");
    // More Lights opens on a click; Add Light adds one, shown as Light Source 2.
    h.click(h.rect("More Lights").center());
    h.click(h.rect("Add Light").center());
    let lights = h.app.ui.dialog.as_ref().unwrap().fields.get("lights").and_then(Value::as_array).map_or(0, Vec::len);
    assert_eq!(lights, 1, "Add Light adds a light");
    h.rect("Light Source 2");
    // The live preview draws the shadow first, under the faces.
    let st = h.app.session.doc().unwrap();
    let id = st.selection.objects[0];
    let art = vectorcraft_effects::solid_art(st.doc.node(id).unwrap()).unwrap();
    let first = &art.children().unwrap()[0];
    assert!(first.appearance.effects.iter().any(|e| e.id == "blur.gaussian"), "a blurred shadow at the back");
}

#[test]
fn material_tab_presets_set_the_material_and_lighting_presets_the_lights() {
    let mut h = Harness::with("M100 100 L200 100 L200 200 L100 200 Z", vectorcraft_effects::EXTRUDE, vec2(1400.0, 1100.0));
    h.settle();
    h.click(h.rect("Material").center());
    for label in ["Presets", "Highlight", "Roughness", "Metallic", "Rim Light", "Cel Bands", "Smooth shading"] {
        h.rect(label);
    }
    // The defaults are the Matte preset, shown by name.
    h.rect("Matte");
    let d = h.app.ui.dialog.as_mut().unwrap();
    for (k, v) in vectorcraft_effects::MATERIAL_PRESETS[4].1 {
        d.fields.insert((*k).into(), json!(v));
    }
    h.settle();
    h.rect("Metal");
    h.app.ui.dialog.as_mut().unwrap().fields.insert("roughness".into(), json!(12.5));
    h.settle();
    h.rect("Custom");
    // The live preview takes the material: a metal's faces are gradients with smooth shading.
    let st = h.app.session.doc().unwrap();
    let node = st.doc.node(st.selection.objects[0]).unwrap();
    let e = node.appearance.effects.last().unwrap();
    assert_eq!(e.params["metallic"], json!(100.0));
    // Lighting presets: Studio sets two more lights; the tab then shows its name.
    h.click(h.rect("Lighting").center());
    h.rect("Standard");
    let defaults = vectorcraft_effects::default_params(vectorcraft_effects::EXTRUDE).unwrap();
    let studio = &vectorcraft_effects::LIGHTING_PRESETS[1];
    h.app.ui.dialog.as_mut().unwrap().fields.extend(studio.params());
    h.settle();
    h.rect("Studio");
    let d = h.app.ui.dialog.as_ref().unwrap();
    assert!(studio.matches(&d.fields, &defaults));
    assert_eq!(d.fields["lights"].as_array().map(Vec::len), Some(2));
}
