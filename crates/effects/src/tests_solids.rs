use serde_json::json;
use vectorcraft_doc::color::Color;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Affine, BezPath, Point};

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

fn square_node() -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::solid(Color::BLACK), 4.0))
}

fn fx(id: &str, params: Value) -> Effect {
    crate::new_effect(id, &params).unwrap()
}

/// The shaded faces of every surface of `art`, bottom first.
fn faces(art: &Node) -> Vec<&Node> {
    art.children().unwrap().iter().flat_map(|s| s.children().unwrap().iter().map(|f| &**f)).collect()
}

/// A face's colour.
fn fill_rgb(n: &Node) -> [f32; 3] {
    match &n.appearance.items[0] {
        AppearanceItem::Fill(f) => f.paint.color().unwrap().to_rgb(),
        _ => panic!("a fill first"),
    }
}

/// The smooth-shaded image a surface draws (a clip group: the outline, then the image), and its
/// pixels.
fn shading_image(surface: &Node) -> (vectorcraft_doc::ImageObject, vectorcraft_doc::generated::Generated) {
    let group = &surface.children().unwrap()[0];
    let NodeKind::Group { children, clip: true } = &group.kind else { panic!("a clip group: {:?}", group.kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }), "clipped to the faces' outline");
    let NodeKind::Image(im) = &children[1].kind else { panic!("then the image") };
    let pixels = vectorcraft_doc::generated::get(&im.key).expect("its pixels are stored");
    ((**im).clone(), pixels)
}

#[test]
fn every_solid_draws_shaded_faces_and_leaves_the_source_editable() {
    for id in SOLIDS {
        let mut n = square_node();
        let source = n.path_data().unwrap().clone();
        n.appearance.effects.push(fx(id, json!({"smoothShading": false})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(n.path_data(), Some(&source), "{id}");
        assert!(!faces(&art).is_empty(), "{id}");
        // The faces are the fill's red, shaded: never brighter, never another hue.
        for f in faces(&art) {
            let [r, g, b] = fill_rgb(f);
            assert!(r <= 0.8 + 1e-6 && (g * 4.0 - r).abs() < 1e-4 && (b * 8.0 - r).abs() < 1e-4 || id == ROTATE, "{id}: {r} {g} {b}");
        }
        // The art follows the source when it moves.
        let before = art.visual_bounds().unwrap();
        n.transform(Affine::translate((50.0, -30.0)), false);
        let after = solid_art(&n).unwrap().visual_bounds().unwrap();
        assert!((after.x0 - before.x0 - 50.0).abs() < 1e-6 && (after.y0 - before.y0 + 30.0).abs() < 1e-6, "{id}");
        // Hidden, the effect draws nothing of its own.
        n.appearance.effects[0].visible = false;
        assert!(solid_art(&n).is_none(), "{id}");
        // Smooth shading draws the same red, pixel by pixel.
        let mut n = square_node();
        n.appearance.effects.push(fx(id, json!({})));
        let art = solid_art(&n).unwrap();
        let (_, pixels) = shading_image(&art.children().unwrap()[0]);
        let lit: Vec<&[u8]> = pixels.rgba.chunks(4).filter(|p| p[3] == 255 && p[0] > 40).collect();
        assert!(!lit.is_empty(), "{id}");
        for p in lit {
            let [r, g, b] = [p[0], p[1], p[2]].map(f32::from);
            assert!(r <= 205.0 && (g * 4.0 - r).abs() <= 6.0 && (b * 8.0 - r).abs() <= 10.0, "{id}: {p:?}");
        }
    }
}

#[test]
fn rotate_turns_every_fill_and_stroke_and_the_solids_use_the_top_fill() {
    let mut n = square_node();
    n.appearance.effects.push(fx(ROTATE, json!({"smoothShading": false})));
    let art = solid_art(&n).unwrap();
    // The fill's plane and the stroke's outline, each in its own paint.
    let surfaces = art.children().unwrap();
    assert_eq!(surfaces.len(), 2);
    let paint = |s: &Node| fill_rgb(&s.children().unwrap()[0]);
    assert!(paint(&surfaces[0])[0] > paint(&surfaces[0])[1], "the red fill below");
    assert!(paint(&surfaces[1]).iter().all(|c| *c < 0.05), "the black stroke above");
    // Turned 60° about y with no perspective, the stroke's outline spans half its width plus the
    // stroke's own outset, foreshortened as well.
    n.appearance.effects[0].params = json!({"rotationX": 0, "rotationY": 60, "rotationZ": 0, "smoothShading": false});
    let b = solid_art(&n).unwrap().children().unwrap()[1].visual_bounds().unwrap();
    assert!((b.width() - 104.0 * 0.5 - 0.75).abs() < 1e-6, "{b:?}");
    // Extrude & Bevel paints one surface with the top fill.
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 1);
}

#[test]
fn transparency_stays_on_the_whole_surface_and_items_take_their_own_effect() {
    let mut n = square_node();
    n.opacity = 0.5;
    if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
        f.opacity = 0.4;
        f.effects.push(fx(INFLATE, json!({})));
    }
    let art = solid_art(&n).unwrap();
    assert_eq!(art.opacity, 0.5);
    let surfaces = art.children().unwrap();
    // The fill inflated, the stroke drawn flat as it was.
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0].opacity, 0.4);
    assert!(surfaces[0].children().unwrap().iter().all(|c| c.opacity == 1.0));
    assert!(matches!(surfaces[1].kind, NodeKind::Path { .. }));
}

#[test]
fn export_and_expand_bake_faces_with_their_own_ids() {
    for id in SOLIDS {
        let mut d = Document::new(400.0, 400.0);
        let mut n = square_node();
        n.id = d.alloc_id();
        n.appearance.effects.push(fx(id, json!({"bevel": "rounded"})));
        assert_eq!(crate::bake_appearance(&n), solid_art(&n), "{id}");
        let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
        let art = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
        let mut ids = std::collections::HashSet::new();
        fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
            assert!(ids.insert(n.id), "{:?} twice", n.id);
            assert!(!has_solid(n));
            for c in n.children().into_iter().flatten() {
                visit(c, ids);
            }
        }
        visit(&art, &mut ids);
        assert!(ids.len() > 2, "{id}");
    }
}

/// A twisted, tapered, hollow Inflate (both sides) exports and expands like the others: faces with
/// their own ids, no 3D effect left on them.
#[test]
fn a_twisted_tapered_hollow_inflate_exports_and_expands() {
    let mut d = Document::new(400.0, 400.0);
    let mut n = square_node();
    n.id = d.alloc_id();
    let params = json!({"twist": 120, "taper": 50, "cap": false, "bothSides": true, "rotationX": -20, "rotationY": 35, "smoothShading": false});
    n.appearance.effects.push(fx(INFLATE, params));
    let art = solid_art(&n).unwrap();
    assert_eq!(crate::bake_appearance(&n), Some(art.clone()));
    assert!(faces(&art).len() > 20, "{} faces", faces(&art).len());
    let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
    let expanded = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
    let mut ids = std::collections::HashSet::new();
    fn visit(n: &Node, ids: &mut std::collections::HashSet<NodeId>) {
        assert!(ids.insert(n.id), "{:?} twice", n.id);
        assert!(!has_solid(n));
        for c in n.children().into_iter().flatten() {
            visit(c, ids);
        }
    }
    visit(&expanded, &mut ids);
    assert!(ids.len() > 20, "{} nodes", ids.len());
}

#[test]
fn effects_before_it_shape_the_solid_and_effects_after_it_bend_the_result() {
    let mut n = square_node();
    n.appearance.effects.push(fx("path.offsetPath", json!({"offset": 20})));
    n.appearance.effects.push(fx(EXTRUDE, json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "perspective": 0, "smoothShading": false})));
    // Straight on, the extrusion shows its front: the offset square.
    let b = solid_art(&n).unwrap().visual_bounds().unwrap();
    assert!((b.width() - 140.0 - 0.75).abs() < 1e-6, "{b:?}");
    n.appearance.effects.push(fx("distort.twist", json!({"angle": 30})));
    let art = solid_art(&n).unwrap();
    assert!(art.appearance.effects.iter().any(|e| e.id == "distort.twist"), "applied to the result as a whole");
    assert!(art.appearance.effects.iter().all(|e| e.id != "path.offsetPath" && !is_three_d(&e.id)));
}

#[test]
fn parameters_are_validated_and_lengths_scale_with_the_object() {
    let n = square_node();
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "rounded", "depth": 80})).is_ok());
    assert!(validate_solid(&n, EXTRUDE, &json!({"bevel": "fancy"})).unwrap_err().contains("bevel must be"));
    let text = Node::group(NodeId(2), vec![]);
    assert!(validate_solid(&text, INFLATE, &json!({})).unwrap_err().contains("requires a path"));
    for (id, key) in [(EXTRUDE, "depth"), (EXTRUDE, "bevelWidth"), (EXTRUDE, "bevelHeight"), (INFLATE, "depth")] {
        assert!(crate::is_length(id, key, false), "{id} {key}");
    }
    assert!(!crate::is_length(EXTRUDE, "twist", false));
    for id in SOLIDS {
        assert!(!crate::is_geometry(id) && is_three_d(id));
        assert!(crate::default_params(id).is_some());
    }
    // Options read from parameters, with the documented ranges' defaults.
    let o = extrude_options(&json!({"taper": 40, "bevel": "step", "bevelSpace": 30, "bevelRepeat": 99}));
    assert_eq!(o.taper, 0.4);
    let b = o.bevel.unwrap();
    assert_eq!((b.shape, b.space, b.repeat), (BevelShape::Step, 0.3, 32));
    assert!(extrude_options(&json!({})).bevel.is_none());
}

#[test]
fn inflate_and_extrude_on_a_group_stack_each_layer_in_its_own_paint() {
    let shape = |id: u64, x: f64, s: f64, c: Color| {
        let mut b = BezPath::new();
        b.move_to((x, x));
        b.line_to((x + s, x));
        b.line_to((x + s, x + s));
        b.line_to((x, x + s));
        b.close_path();
        Arc::new(Node::path(NodeId(id), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    };
    let blue = Color::Rgb { r: 0.1, g: 0.2, b: 0.9 };
    let mut g = Node::group(NodeId(9), vec![shape(1, 100.0, 100.0, RED), shape(2, 130.0, 40.0, blue)]);
    let p = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "smoothShading": false, "pillow": 100});
    validate_solid(&g, INFLATE, &p).unwrap();
    assert!(validate_solid(&g, ROTATE, &json!({})).is_err(), "Rotate doesn't take a group");
    g.appearance = Appearance { effects: vec![fx(INFLATE, p)], ..Default::default() }.into();
    let art = solid_art(&g).unwrap();
    let surfaces = art.children().unwrap();
    assert_eq!(surfaces.len(), 2, "one surface per layer, bottom first");
    let mean = |s: &Node| {
        let cs: Vec<[f32; 3]> = s.children().unwrap().iter().map(|f| fill_rgb(f)).collect();
        let n = cs.len() as f32;
        [0, 1, 2].map(|k| cs.iter().map(|c| c[k]).sum::<f32>() / n)
    };
    assert!(mean(&surfaces[0])[0] > mean(&surfaces[0])[2], "red below");
    assert!(mean(&surfaces[1])[2] > mean(&surfaces[1])[0], "blue on top");
    // Straight on, the top layer covers its own square only.
    let b = surfaces[1].visual_bounds().unwrap();
    assert!((b.x0 - 130.0).abs() < 1.0 && (b.x1 - 170.0).abs() < 1.0, "{b:?}");
    assert_eq!(inflate_options(&json!({"pillow": 40})).pillow, 40.0);
    // Extrude & Bevel stacks the same layers, each in its own paint, the top one nearest.
    let extrude = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "smoothShading": false, "depth": 20});
    validate_solid(&g, EXTRUDE, &extrude).unwrap();
    assert!(validate_solid(&g, EXTRUDE, &json!({"bevel": "nope"})).is_err());
    g.appearance = Appearance { effects: vec![fx(EXTRUDE, extrude)], ..Default::default() }.into();
    let art = solid_art(&g).unwrap();
    let surfaces = art.children().unwrap();
    assert_eq!(surfaces.len(), 2);
    assert!(mean(&surfaces[1])[2] > mean(&surfaces[1])[0], "blue on top");
    let b = surfaces[1].visual_bounds().unwrap();
    assert!((b.x0 - 130.0).abs() < 1.0 && (b.x1 - 170.0).abs() < 1.0, "{b:?}");
    assert_eq!(crate::default_params(INFLATE).unwrap()["pillow"], json!(0.0));
}

#[test]
fn inflate_reads_twist_taper_and_cap() {
    let o = inflate_options(&json!({"twist": 90, "taper": 40, "cap": false}));
    assert_eq!((o.twist, o.taper, o.cap), (90.0, 0.4, false));
    let d = inflate_options(&json!({}));
    assert_eq!((d.twist, d.taper, d.cap), (0.0, 1.0, true));
    assert_eq!(crate::default_params(INFLATE).unwrap()["cap"], json!(true));
}

#[test]
fn a_coloured_light_tints_the_faces() {
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({"lightColor": "#0000ff", "ambient": 0.0, "smoothShading": false})));
    let art = solid_art(&n).unwrap();
    for f in faces(&art) {
        let [r, g, _] = fill_rgb(f);
        assert!(r < 1e-6 && g < 1e-6, "a blue light leaves no red or green: {r} {g}");
    }
    // Another light adds to it.
    let mut m = square_node();
    m.appearance.effects.push(fx(
        EXTRUDE,
        json!({"lightColor": "#0000ff", "ambient": 0.0, "lights": [{"azimuth": 0, "elevation": 0, "intensity": 100, "color": "#ff0000"}], "smoothShading": false}),
    ));
    let art = solid_art(&m).unwrap();
    assert!(faces(&art).iter().any(|f| fill_rgb(f)[0] > 0.1), "the red light reaches some face");
}

#[test]
fn a_shadow_is_a_blurred_black_shape_behind_the_faces() {
    for id in SOLIDS {
        let mut n = square_node();
        n.appearance.effects.push(fx(id, json!({"shadow": "below", "shadowDistance": 10, "shadowOpacity": 60, "shadowSoftness": 50})));
        let art = solid_art(&n).unwrap_or_else(|| panic!("{id}"));
        let first = &art.children().unwrap()[0];
        let shadow = if first.path_data().is_some() { first } else { &first.children().unwrap()[0] };
        assert!(shadow.path_data().is_some(), "{id}: the shadow comes first");
        assert_eq!(fill_rgb(shadow), [0.0, 0.0, 0.0], "{id}");
        assert!((shadow.opacity - 0.6).abs() < 1e-6, "{id}");
        assert!(shadow.appearance.effects.iter().any(|e| e.id == "blur.gaussian"), "{id}: softened by a blur");
    }
    let mut n = square_node();
    n.appearance.effects.push(fx(EXTRUDE, json!({})));
    let art = solid_art(&n).unwrap();
    assert!(art.children().unwrap()[0].path_data().is_none(), "no shadow unless asked");
}

/// New effects drop their shadow straight under the object; one saved before the choice existed
/// still casts it along the main light, as it did then.
#[test]
fn a_new_shadow_falls_straight_under_the_object_and_an_old_one_along_the_light() {
    let front = json!({"shadow": "below", "rotationX": 0, "rotationY": 0, "rotationZ": 0, "lightAzimuth": -45, "lightElevation": 45});
    let centre = |e: Effect| {
        let mut n = square_node();
        n.appearance.effects.push(e);
        let art = solid_art(&n).unwrap();
        let b = art.children().unwrap()[0].path_data().unwrap().bounds().unwrap();
        b.center().x
    };
    let new = fx(INFLATE, front.clone());
    assert_eq!(new.params["shadowDirection"], json!("straight"));
    assert!((centre(new) - 150.0).abs() < 1.0, "right under the square's centre");
    let old = Effect { id: INFLATE.into(), params: front.clone(), visible: true };
    assert!(centre(old) - 150.0 > 5.0, "pushed away from a light at the upper left");
    let mut along = front;
    along["shadowDirection"] = json!("light");
    assert!(centre(fx(INFLATE, along)) - 150.0 > 5.0);
    assert!(validate_solid(&square_node(), INFLATE, &json!({"shadowDirection": "down"})).is_err());
}

#[test]
fn bad_lighting_params_are_refused() {
    let n = square_node();
    for p in [
        json!({"shadow": "sideways"}),
        json!({"lightColor": "bright"}),
        json!({"lights": [{}, {}, {}, {}]}),
        json!({"lights": [{"color": 3}]}),
        json!({"lights": [7]}),
    ] {
        assert!(validate_solid(&n, EXTRUDE, &p).is_err(), "{p}");
    }
    assert!(validate_solid(&n, EXTRUDE, &json!({"shadow": "behind", "lights": [{"azimuth": 10, "color": "#ffcc00"}]})).is_ok());
}

/// Every colour a face's first fill paints.
fn face_colours(n: &Node) -> Vec<[f32; 3]> {
    vec![fill_rgb(n)]
}

#[test]
fn smooth_shading_is_on_for_new_effects_and_off_for_saved_ones_without_it() {
    for id in SOLIDS {
        let mut n = square_node();
        n.appearance.effects.push(fx(id, json!({"rotationY": -40})));
        let art = solid_art(&n).unwrap();
        // New effects shade pixel by pixel: an image, clipped to the faces' outline, covering it.
        let (im, pixels) = shading_image(&art.children().unwrap()[0]);
        assert_eq!((pixels.width, pixels.height), (im.width, im.height));
        let outline = art.children().unwrap()[0].children().unwrap()[0].children().unwrap()[0].path_data().unwrap().bounds().unwrap();
        let placed = im.xf.transform_rect_bbox(vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)));
        assert!(placed.contains(outline.origin()) && placed.contains(Point::new(outline.x1, outline.y1)), "{id}: {placed:?} {outline:?}");
        // The same art again shows the same pixels.
        assert_eq!(solid_art(&n).unwrap(), art, "{id}");
        // An effect saved before the option existed has no key: flat faces, as it was drawn then.
        n.appearance.effects[0].params.as_object_mut().unwrap().remove("smoothShading");
        let art = solid_art(&n).unwrap();
        assert!(faces(&art).iter().all(|f| matches!(f.kind, NodeKind::Path { .. }) && face_colours(f).len() == 1), "{id}");
    }
}

#[test]
fn other_paints_are_multiplied_by_the_shading_pixel_by_pixel() {
    let mut n = square_node();
    if let AppearanceItem::Fill(f) = &mut n.appearance.items[0] {
        f.paint = Paint::Pattern { pattern: "dots".into(), xf: Affine::IDENTITY };
    }
    n.appearance.effects.push(fx(INFLATE, json!({"specular": 80})));
    let art = solid_art(&n).unwrap();
    let group = &art.children().unwrap()[0].children().unwrap()[0];
    assert!(group.isolate, "the blends act on the paint alone");
    let NodeKind::Group { children, clip: true } = &group.kind else { panic!("{:?}", group.kind) };
    // The clip, the paint filling the outline, then the shading's tint multiplied and its
    // highlight screened over it.
    assert_eq!(children.len(), 4);
    let AppearanceItem::Fill(fill) = &children[1].appearance.items[0] else { panic!("a fill") };
    assert!(matches!(fill.paint, Paint::Pattern { .. }));
    assert_eq!((children[2].blend, children[3].blend), (BlendMode::Multiply, BlendMode::Screen));
    for (k, c) in children[2..].iter().enumerate() {
        let NodeKind::Image(im) = &c.kind else { panic!("an image") };
        let px = vectorcraft_doc::generated::get(&im.key).unwrap();
        assert!(px.rgba.chunks(4).any(|p| p[3] == 255 && p[0] > 0), "layer {k} paints");
    }
    // Saved flat, it keeps its vector faces.
    n.appearance.effects[0].params["smoothShading"] = json!(false);
    assert!(faces(&solid_art(&n).unwrap()).iter().all(|f| matches!(f.kind, NodeKind::Path { .. })));
}

#[test]
fn options_added_later_keep_saved_effects_as_they_were() {
    // New effects store occlusion, the contact shadow and a studio to reflect…
    let new = fx(EXTRUDE, json!({}));
    assert_eq!(new.params["ambientOcclusion"], json!(60.0));
    assert_eq!(new.params["contactShadow"], json!(60.0));
    assert_eq!(new.params["environment"], json!("studio"));
    let p = crate::lighting::drawing_params(EXTRUDE, &new.params);
    let l = crate::lighting::lighting(&p);
    assert_eq!((l.material.occlusion, l.material.environment), (0.6, vectorcraft_three_d::Environment::Studio));
    // …an effect saved before them draws as it did: none of them.
    let saved = json!({"smoothShading": true, "specular": 50});
    let p = crate::lighting::drawing_params(EXTRUDE, &saved);
    assert_eq!((p["ambientOcclusion"].clone(), p["contactShadow"].clone(), p["environment"].clone()), (json!(0.0), json!(0.0), json!("gradient")));
    assert!(validate_solid(&square_node(), EXTRUDE, &json!({"environment": "moon"})).unwrap_err().contains("environment must be"));
    assert!(validate_solid(&square_node(), EXTRUDE, &json!({"environment": "sunset"})).is_ok());
}

#[test]
fn a_contact_shadow_sits_tight_under_the_object() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({"shadow": "below", "shadowOpacity": 40, "contactShadow": 80})));
    let art = solid_art(&n).unwrap();
    let parts = art.children().unwrap();
    // The cast shadow, then the contact shadow, then the surface.
    let (cast, contact) = (&parts[0], &parts[1]);
    assert!(cast.path_data().is_some() && contact.path_data().is_some());
    assert!(contact.opacity > cast.opacity, "darker: {} vs {}", contact.opacity, cast.opacity);
    let blur = |m: &Node| m.appearance.effects.iter().find(|e| e.id == "blur.gaussian").map(|e| e.params["radius"].as_f64().unwrap()).unwrap();
    assert!(blur(contact) < blur(cast), "and tighter");
    let (cb, sb) = (contact.path_data().unwrap().bounds().unwrap(), cast.path_data().unwrap().bounds().unwrap());
    assert!(cb.height() < sb.height(), "only where the object meets the ground: {cb:?} {sb:?}");
    // Off, or lifted clear of the ground: only the cast shadow.
    n.appearance.effects[0].params["contactShadow"] = json!(0);
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 2);
    n.appearance.effects[0].params["contactShadow"] = json!(80);
    n.appearance.effects[0].params["shadowDistance"] = json!(60);
    assert_eq!(solid_art(&n).unwrap().children().unwrap().len(), 2);
}

#[test]
fn exporting_and_expanding_smooth_shading_embeds_its_image() {
    let mut d = Document::new(400.0, 400.0);
    let mut n = square_node();
    n.id = d.alloc_id();
    n.appearance.effects.push(fx(INFLATE, json!({})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n.clone()).unwrap();
    let baked = crate::bake_document(&d).unwrap();
    let mut keys = vec![];
    for layer in &baked.layers {
        layer.walk(&mut |m: &Node| {
            if let NodeKind::Image(im) = &m.kind {
                keys.push(im.key.clone());
            }
        });
    }
    assert_eq!(keys.len(), 1);
    let blob = baked.images.get(&keys[0]).expect("the export copy holds the image");
    assert_eq!(blob.mime, "image/png");
    assert!(d.images.is_empty(), "the document itself is unchanged");
    // Expand Appearance leaves an image of the document's own.
    let mut stroke = |_: &mut Document, _: &PathData, _: FillRule, _: &StrokeLayer| None;
    let expanded = crate::expand_leaf(&mut d, &n, &mut stroke).unwrap();
    let mut found = 0;
    expanded.walk(&mut |m: &Node| {
        if let NodeKind::Image(im) = &m.kind {
            assert!(d.images.contains_key(&im.key));
            found += 1;
        }
    });
    assert_eq!(found, 1);
}

#[test]
fn a_glossy_highlight_lifts_a_face_above_its_paint_and_matte_never_does() {
    // The light straight ahead of a face turned to it: the highlight's peak.
    let lit = json!({"rotationX": 0, "rotationY": 0, "rotationZ": 0, "lightAzimuth": 0, "lightElevation": 0, "smoothShading": false});
    let mut matte = square_node();
    matte.appearance.effects.push(fx(EXTRUDE, lit.clone()));
    let mut glossy = square_node();
    let mut p = lit.clone();
    p.as_object_mut().unwrap().extend([("specular".into(), json!(100)), ("roughness".into(), json!(20))]);
    glossy.appearance.effects.push(fx(EXTRUDE, p));
    let brightest = |n: &Node| faces(&solid_art(n).unwrap()).iter().flat_map(|f| face_colours(f)).map(|c| c[1]).fold(0.0f32, f32::max);
    // The paint's green is 0.2: matte light never brightens it, a highlight adds white.
    assert!(brightest(&matte) <= 0.2 + 1e-5);
    assert!(brightest(&glossy) > 0.8, "{}", brightest(&glossy));
}

#[test]
fn every_preset_validates_and_bad_material_params_are_refused() {
    let n = square_node();
    let defaults = crate::default_params(EXTRUDE).unwrap();
    for preset in crate::MATERIAL_PRESETS {
        assert!(validate_solid(&n, EXTRUDE, &Value::Object(preset.params())).is_ok(), "{}", preset.id);
        assert!(preset.matches(&preset.params(), &defaults), "{}", preset.id);
    }
    // The defaults are Matte; Metal is told from Chrome, and from a metal reflecting the studio.
    let [matte, metal, chrome] = [0, 4, 5].map(|i| &crate::MATERIAL_PRESETS[i]);
    assert!(matte.matches(&serde_json::Map::new(), &defaults));
    assert!(!chrome.matches(&metal.params(), &defaults));
    let mut studio_metal = metal.params();
    studio_metal.insert("environment".into(), json!("studio"));
    assert!(!metal.matches(&studio_metal, &defaults));
    for preset in crate::LIGHTING_PRESETS {
        assert!(validate_solid(&n, INFLATE, &Value::Object(preset.params())).is_ok(), "{}", preset.id);
    }
    for p in [json!({"shadingSteps": 9}), json!({"shadingSteps": -1}), json!({"rimColor": "glow"}), json!({"ambientColor": 4})] {
        assert!(validate_solid(&n, EXTRUDE, &p).is_err(), "{p}");
    }
}

#[test]
fn a_lighting_preset_matches_its_own_params_and_not_another() {
    let defaults = crate::default_params(EXTRUDE).unwrap();
    let standard = &crate::LIGHTING_PRESETS[0];
    // The defaults are the Standard preset.
    assert!(standard.matches(&serde_json::Map::new(), &defaults));
    let studio = &crate::LIGHTING_PRESETS[1];
    assert!(studio.matches(&studio.params(), &defaults));
    assert!(!standard.matches(&studio.params(), &defaults));
    let mut tweaked = studio.params();
    tweaked.insert("lightColor".into(), json!("#FFFFFF"));
    assert!(studio.matches(&tweaked, &defaults), "colours compare ignoring case");
    tweaked.insert("ambient".into(), json!(30));
    assert!(!studio.matches(&tweaked, &defaults));
}

#[test]
fn the_dramatic_preset_keeps_its_shadow_side_readable() {
    // A sphere's far side, turned away from the key light: still clearly the paint, not black.
    let preset = crate::lighting::LIGHTING_PRESETS.iter().find(|p| p.id == "dramatic").unwrap();
    let mut params = preset.params();
    params.insert("volume".into(), json!(50));
    params.insert("rotationX".into(), json!(0));
    params.insert("rotationY".into(), json!(0));
    params.insert("rotationZ".into(), json!(0));
    let mut b = BezPath::new();
    b.extend(vectorcraft_geom::Shape::path_elements(&kurbo::Circle::new((150.0, 150.0), 50.0), 0.01));
    let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    n.appearance.effects.push(fx(INFLATE, Value::Object(params)));
    let (_, pixels) = shading_image(&solid_art(&n).unwrap().children().unwrap()[0]);
    let w = pixels.width as usize;
    let h = pixels.height as usize;
    // Three quarters of the way across, away from the key light on the left.
    let p = &pixels.rgba[((h / 2) * w + w * 3 / 4) * 4..][..4];
    assert!(p[0] >= 40, "the dark side shows the red: {p:?}");
    let lit = &pixels.rgba[((h / 2) * w + w / 4) * 4..][..4];
    assert!(lit[0] > p[0] + 60, "and stays clearly darker than the lit side: {lit:?} {p:?}");
}

#[test]
fn a_draft_preview_shades_coarser_and_the_same_art_is_made_once() {
    let mut n = square_node();
    n.appearance.effects.push(fx(INFLATE, json!({"specular": 60})));
    let full = solid_art(&n).unwrap();
    // The same object again: the remembered art, the same image.
    assert_eq!(solid_art(&n).unwrap(), full);
    let (im, _) = shading_image(&full.children().unwrap()[0]);
    n.appearance.effects[0].params["draftPreview"] = json!(true);
    let draft = solid_art(&n).unwrap();
    let (dm, _) = shading_image(&draft.children().unwrap()[0]);
    assert!(f64::from(dm.width) <= vectorcraft_three_d::DRAFT_SIDE + 6.0 && im.width > 2 * dm.width, "{} {}", im.width, dm.width);
    // The validated parameters take the flag.
    assert!(validate_solid(&n, INFLATE, &n.appearance.effects[0].params).is_ok());
}

/// A document with a symbol "Logo": a 20 pt blue square.
fn logo_document(blue: f32) -> Document {
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.line_to((20.0, 0.0));
    b.line_to((20.0, 20.0));
    b.line_to((0.0, 20.0));
    b.close_path();
    let art = Node::path(NodeId(50), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, blue)), Paint::None, 0.0));
    let mut d = Document::new(400.0, 400.0);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Logo".into(), art: Arc::new(art) });
    d
}

#[test]
fn a_graphic_is_laid_on_the_front_in_its_own_paint_and_shaded_with_it() {
    let doc = logo_document(1.0);
    for id in [EXTRUDE, INFLATE] {
        let mut n = square_node();
        let straight = json!({"smoothShading": false, "rotationX": 0, "rotationY": 0, "rotationZ": 0, "graphics": [{"symbol": "Logo"}]});
        n.appearance.effects.push(fx(id, straight.clone()));
        let bare = solid_art(&n).unwrap();
        let art = solid_art_in(&n, Some(&doc)).unwrap();
        assert_eq!(faces(&art).len(), faces(&bare).len() + 1, "{id}: one more piece, on top");
        let logo = *faces(&art).last().unwrap();
        // Straight on, it sits 20 pt across in the middle of the object, blue, lit as the front is.
        let b = logo.geometric_bounds().unwrap();
        assert!((b.x0 - 140.0).abs() < 1e-6 && (b.x1 - 160.0).abs() < 1e-6 && (b.y0 - 140.0).abs() < 1e-6, "{id}: {b:?}");
        let [r, g, bl] = fill_rgb(logo);
        assert!(r < 0.05 && g < 0.05 && bl > 0.3, "{id}: {r} {g} {bl}");
        // Turned away, the front and its graphic are hidden.
        let mut away = square_node();
        away.appearance.effects.push(fx(id, json!({"smoothShading": false, "rotationY": 180, "graphics": [{"symbol": "Logo"}]})));
        let away_bare = solid_art(&away).unwrap();
        assert_eq!(faces(&solid_art_in(&away, Some(&doc)).unwrap()).len(), faces(&away_bare).len(), "{id}");
        // A symbol edited draws anew; one not in the document adds nothing.
        assert_ne!(solid_art_in(&n, Some(&logo_document(0.5))).unwrap(), art, "{id}");
        assert_eq!(solid_art_in(&n, Some(&Document::new(400.0, 400.0))).unwrap(), bare, "{id}");
    }
}

#[test]
fn a_smooth_shaded_graphic_is_lit_on_a_matte_finish_or_in_the_surface() {
    let doc = logo_document(1.0);
    let art_with = |finish: &str| {
        let mut n = square_node();
        n.appearance.effects.push(fx(
            INFLATE,
            json!({"metallic": 100, "specular": 100, "graphics": [{"symbol": "Logo", "scale": 200, "rotation": 30, "finish": finish}]}),
        ));
        solid_art_in(&n, Some(&doc)).unwrap()
    };
    // In the surface's finish: under its shading. The outline, the paint, the graphic, then the
    // shading's tint (multiplied) and highlight.
    let art = art_with("surface");
    let surface = art.children().unwrap().last().unwrap();
    let group = &surface.children().unwrap()[0];
    let NodeKind::Group { children, clip: true } = &group.kind else { panic!("a clip group") };
    let blends: Vec<BlendMode> = children.iter().map(|c| c.blend).collect();
    assert_eq!(children.len(), 5);
    assert_eq!(blends[3..], [BlendMode::Multiply, BlendMode::Screen]);
    let [_, _, b] = fill_rgb(&children[2]);
    assert_eq!(b, 1.0, "unshaded: the shading lights it");
    // Matte (the default): over the surface, in a clip group of its own, multiplied by a matte
    // shading's tint, which on metal is far brighter than the metal's.
    let art = art_with("matte");
    let surface = art.children().unwrap().last().unwrap();
    let parts = surface.children().unwrap();
    assert_eq!(parts.len(), 2, "the surface, then the graphic");
    let NodeKind::Group { children, clip: true } = &parts[1].kind else { panic!("a clip group") };
    assert!(parts[1].isolate);
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    let NodeKind::Image(matte) = &children.last().unwrap().kind else { panic!("the matte tint") };
    assert_eq!(children.last().unwrap().blend, BlendMode::Multiply);
    let mean = |key: &str| {
        let px = vectorcraft_doc::generated::get(key).unwrap();
        let lit: Vec<f64> = px.rgba.chunks(4).filter(|p| p[3] == 255).map(|p| f64::from(p[0])).collect();
        lit.iter().sum::<f64>() / lit.len() as f64
    };
    // Lit by the lights as plain paint is, not darkened as metal's diffuse light is.
    assert!(mean(&matte.key) > 120.0, "{}", mean(&matte.key));
    // Without graphics, a flat colour shades straight into one image.
    let mut plain = square_node();
    plain.appearance.effects.push(fx(INFLATE, json!({})));
    let plain = solid_art(&plain).unwrap();
    assert!(shading_image(plain.children().unwrap().last().unwrap()).1.width > 0);
}

#[test]
fn flat_a_matte_graphic_takes_no_highlight() {
    // A blue logo on a broad white gloss: in the surface's finish the highlight adds to it;
    // matte, it stays pure blue, lit.
    let doc = logo_document(1.0);
    let logo = |finish: &str| {
        let mut n = square_node();
        let p = json!({"smoothShading": false, "rotationX": 0, "rotationY": 0, "rotationZ": 0, "specular": 100, "roughness": 100, "graphics": [{"symbol": "Logo", "finish": finish}]});
        n.appearance.effects.push(fx(EXTRUDE, p));
        fill_rgb(faces(&solid_art_in(&n, Some(&doc)).unwrap()).last().unwrap())
    };
    let [r, g, b] = logo("matte");
    assert!(r == 0.0 && g == 0.0 && b > 0.3, "{r} {g} {b}");
    let [r, g, _] = logo("surface");
    assert!(r > 0.05 && g > 0.05, "{r} {g}");
}

#[test]
fn rotate_lays_graphics_on_its_top_fill() {
    let doc = logo_document(1.0);
    let mut n = square_node();
    n.appearance
        .effects
        .push(fx(ROTATE, json!({"smoothShading": false, "rotationX": 0, "rotationY": 0, "rotationZ": 0, "graphics": [{"symbol": "Logo"}]})));
    let bare = faces(&solid_art(&n).unwrap()).len();
    let art = solid_art_in(&n, Some(&doc)).unwrap();
    // The fill's surface takes the graphic, last; the stroke's doesn't.
    let surfaces = art.children().unwrap();
    let fill_surface = surfaces.iter().find(|s| s.children().unwrap().len() > 1).unwrap();
    assert_eq!(faces(&art).len(), bare + 1);
    let logo = fill_surface.children().unwrap().last().unwrap();
    let b = logo.geometric_bounds().unwrap();
    assert!((b.x0 - 140.0).abs() < 1e-6 && (b.width() - 20.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn bad_graphics_are_refused() {
    let n = square_node();
    for bad in [
        json!({"graphics": "Logo"}),
        json!({"graphics": [{"x": 5}]}),
        json!({"graphics": [{"symbol": "Logo", "scale": "big"}]}),
        json!({"graphics": [{"symbol": "Logo", "finish": "gold"}]}),
    ] {
        assert!(validate_solid(&n, EXTRUDE, &bad).is_err(), "{bad}");
    }
    assert!(validate_solid(&n, INFLATE, &json!({"graphics": [{"symbol": "Logo", "x": 20, "y": 80, "scale": 50, "rotation": -15}]})).is_ok());
}
