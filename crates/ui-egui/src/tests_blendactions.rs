//! The Properties panel's Quick Actions for a selected blend: Expand and Release (Object › Blend).

use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::tests_removeanchors::{at, has, properties_frame};

fn blend_app() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = app.run("shape.ellipse", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.ellipse", json!({"x": 200, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let id = app.run("object.blend.make", json!({"steps": 3})).unwrap()["id"].as_u64().unwrap();
    (app, id)
}

fn kind_of(app: &VectorcraftApp, id: u64) -> NodeKind {
    app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind.clone()
}

/// Click the Properties panel at `p` (two frames: press, then release).
fn click_properties(app: &mut VectorcraftApp, p: Pos2) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(260.0, 900.0));
    for events in [
        vec![Event::PointerMoved(p)],
        vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }],
        vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }],
    ] {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            crate::panels::properties::show(app, ui);
        });
        out.textures_delta.clear();
    }
}

#[test]
fn a_selected_blend_offers_expand_and_release() {
    let (mut app, id) = blend_app();
    assert!(matches!(kind_of(&app, id), NodeKind::Blend { .. }));
    let texts = properties_frame(&mut app, 260.0);
    assert!(has(&texts, "Expand") && has(&texts, "Release"), "{texts:?}");
    click_properties(&mut app, at(&texts, "Expand"));
    assert!(matches!(kind_of(&app, id), NodeKind::Group { .. }), "Expand turns the blend into a group of its steps");
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.last().unwrap().label, "Expand Blend");
    // A group isn't a blend: the buttons go.
    let texts = properties_frame(&mut app, 260.0);
    assert!(!has(&texts, "Release"), "{texts:?}");
}

#[test]
fn the_release_button_releases_the_blend() {
    let (mut app, id) = blend_app();
    let texts = properties_frame(&mut app, 260.0);
    click_properties(&mut app, at(&texts, "Release"));
    assert!(app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_none_or(|n| !matches!(n.kind, NodeKind::Blend { .. })));
}
