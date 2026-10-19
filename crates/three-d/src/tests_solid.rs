use vectorcraft_geom::{BezPath, PathData, Point, Rect};

use super::*;

fn square(x: f64, y: f64, s: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + s, y));
    b.line_to((x + s, y + s));
    b.line_to((x, y + s));
    b.close_path();
    b
}

fn shape(b: &BezPath) -> (PathData, Rect) {
    let p = PathData::from_bezpath(b);
    let r = p.bounds().unwrap();
    (p, r)
}

/// Straight on, without perspective.
fn front() -> View {
    View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, perspective: 0.0, ..View::default() }
}

fn extent(f: &[Facet]) -> Rect {
    let pts: Vec<Point> = f.iter().flat_map(|f| f.contours.iter().flatten().copied()).collect();
    pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

/// The area the facets cover: each facet's contours counted with their winding (holes
/// subtract), whichever way round the facet runs.
fn area(f: &[Facet]) -> f64 {
    f.iter().map(|f| f.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs()).sum()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn seen_straight_on_an_extrusion_is_its_front_face_exactly_where_the_art_is() {
    let (p, b) = shape(&square(10.0, 20.0, 100.0));
    let f = extrude(&p, b, Extrude::default(), front()).unwrap();
    assert_eq!(f.len(), 1, "the walls and back face away: only the front cap shows");
    assert_eq!(f[0].contours.len(), 1);
    assert_eq!(extent(&f), Rect::new(10.0, 20.0, 110.0, 120.0));
}

#[test]
fn turned_about_y_the_front_foreshortens_and_one_side_wall_shows() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Extrude { depth: 40.0, ..Extrude::default() };
    let f = extrude(&p, b, o, View { rotation_y: 30.0, ..front() }).unwrap();
    // Orthographic: the silhouette is the front's width times cos 30° plus the depth times sin 30°.
    let w = extent(&f).width();
    assert!(close(w, 100.0 * 30f64.to_radians().cos() + 40.0 * 30f64.to_radians().sin(), 1e-6), "{w}");
    assert_eq!(f.len(), 2, "the front cap and one side wall");
    // Lit from the upper left by default: the cap and the wall are shaded differently.
    assert!(!close(f[0].brightness, f[1].brightness, 1e-6));
    // The nearest face is drawn last.
    assert!(f.windows(2).all(|w| w[0].depth <= w[1].depth));
}

#[test]
fn no_depth_is_the_flat_shape_and_no_cap_shows_the_inside() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: 20.0, rotation_y: 30.0, ..front() };
    let flat = extrude(&p, b, Extrude { depth: 0.0, ..Extrude::default() }, v).unwrap();
    assert_eq!(flat, plane(&p, b, v).unwrap());
    let solid = extrude(&p, b, Extrude::default(), v).unwrap();
    let hollow = extrude(&p, b, Extrude { cap: false, ..Extrude::default() }, v).unwrap();
    // Solid: front, right and top faces. Hollow: all four walls (two seen from inside), no caps.
    assert_eq!(solid.len(), 3);
    assert_eq!(hollow.len(), 4);
    assert!(hollow.iter().all(|f| f.contours.len() == 1 && f.contours[0].len() == 4));
}

#[test]
fn taper_shrinks_the_back_and_twist_turns_it() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let side = View { rotation_y: 90.0, ..front() };
    let o = Extrude { depth: 60.0, cap: false, ..Extrude::default() };
    let straight = extent(&extrude(&p, b, o, side).unwrap());
    assert!(close(straight.height(), 100.0, 1e-6) && close(straight.width(), 60.0, 1e-6));
    // Seen from the side, a 50 % taper keeps the front's full height and halves the back's.
    let tapered = extrude(&p, b, Extrude { taper: 0.5, ..o }, side).unwrap();
    let back: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, straight.x0, 1e-6)).collect();
    let (lo, hi) = back.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    let at_front: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, straight.x1, 1e-6)).collect();
    let (flo, fhi) = at_front.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    let (back_h, front_h) = (hi - lo, fhi - flo);
    assert!(close(front_h, 100.0, 1e-6) && close(back_h, 50.0, 1e-6), "front {front_h}, back {back_h}");
    // Seen from behind, a 45° twist shows the back cap turned into a diamond.
    let back = View { rotation_y: 180.0, ..front() };
    let twisted = extrude(&p, b, Extrude { twist: 45.0, cap: true, ..o }, back).unwrap();
    assert_eq!(twisted.len(), 1, "only the back cap faces the viewer");
    assert!(close(extent(&twisted).width(), 100.0 * 2f64.sqrt(), 1e-6), "{:?}", extent(&twisted));
    // Its walls are cut into slices that follow the turn.
    let open = extrude(&p, b, Extrude { twist: 45.0, ..o }, View { rotation_x: -20.0, rotation_y: 30.0, ..front() }).unwrap();
    assert!(open.len() > 2 * 8, "{}", open.len());
}

#[test]
fn a_bevel_inside_shrinks_the_cap_and_outside_grows_the_walls() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let bevel = Bevel { width: 10.0, height: 10.0, ..Bevel::default() };
    let inside = extrude(&p, b, Extrude { bevel: Some(Bevel { inside: true, ..bevel }), ..Extrude::default() }, front()).unwrap();
    // Straight on: the cap inset by the width, and the chamfer around it filling the outline.
    let cap = inside.iter().find(|f| close(signed_area(&f.contours[0]).abs(), 80.0 * 80.0, 1e-6));
    assert!(cap.is_some(), "an 80 × 80 cap");
    assert_eq!(extent(&inside), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(close(area(&inside), 100.0 * 100.0, 1e-6));
    let outside = extrude(&p, b, Extrude { bevel: Some(bevel), ..Extrude::default() }, front()).unwrap();
    assert_eq!(extent(&outside), Rect::new(-10.0, -10.0, 110.0, 110.0), "the walls stand 10 pt outside the art");
    assert!(outside.iter().any(|f| close(signed_area(&f.contours[0]).abs(), 100.0 * 100.0, 1e-6)), "the cap is the art");
    // The chamfer's faces lean towards the light differently from the cap.
    assert!(inside.iter().any(|f| !close(f.brightness, inside[inside.len() - 1].brightness, 1e-3)));
}

#[test]
fn bevel_shapes_repeats_and_both_sides_change_the_surface() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: -20.0, rotation_y: 30.0, ..front() };
    let base = Bevel { width: 12.0, height: 12.0, ..Bevel::default() };
    let classic = extrude(&p, b, Extrude { bevel: Some(base), ..Extrude::default() }, v).unwrap();
    for shape in BevelShape::ALL.into_iter().skip(1) {
        assert_ne!(classic, extrude(&p, b, Extrude { bevel: Some(Bevel { shape, ..base }), ..Extrude::default() }, v).unwrap(), "{shape:?}");
    }
    for bevel in [Bevel { repeat: 3, ..base }, Bevel { repeat: 3, space: 0.3, ..base }, Bevel { both_sides: true, ..base }] {
        assert_ne!(classic, extrude(&p, b, Extrude { bevel: Some(bevel), ..Extrude::default() }, v).unwrap(), "{bevel:?}");
    }
    // The step shape keeps its corners: half the width at half the height.
    assert_eq!(BevelShape::Step.at(0.5), (0.5, 0.5));
    assert_eq!(BevelShape::Step.at(0.25), (0.0, 0.5));
    for shape in BevelShape::ALL {
        assert_eq!(shape.at(0.0), (0.0, 0.0), "{shape:?}");
        let (x, y) = shape.at(1.0);
        assert!(close(x, 1.0, 1e-12) && close(y, 1.0, 1e-12), "{shape:?}");
        assert_eq!(BevelShape::parse(shape.id()), Some(shape));
    }
}

#[test]
fn holes_stay_holes_in_the_caps_and_get_walls_of_their_own() {
    let mut b = square(0.0, 0.0, 100.0);
    // The hole drawn the same way round as the outline: even-odd nesting still makes it a hole.
    b.extend(square(30.0, 30.0, 40.0));
    let (p, r) = shape(&b);
    let f = extrude(&p, r, Extrude::default(), front()).unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].contours.len(), 2);
    assert!(close(area(&f), 100.0 * 100.0 - 40.0 * 40.0, 1e-6));
    let turned = extrude(&p, r, Extrude::default(), View { rotation_y: 40.0, ..front() }).unwrap();
    // Front cap, the outline's right wall and the hole's left wall (seen through the hole).
    assert_eq!(turned.len(), 3);
}

#[test]
fn a_plane_turns_about_the_given_centre() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let f = plane(&p, b, View { rotation_y: 60.0, ..front() }).unwrap();
    assert_eq!(f.len(), 1);
    let e = extent(&f);
    assert!(close(e.width(), 50.0, 1e-6) && close(e.center().x, 50.0, 1e-9));
    // Turned about the centre of a larger box, the square moves as well as narrowing.
    let wide = plane(&p, Rect::new(0.0, 0.0, 300.0, 100.0), View { rotation_y: 60.0, ..front() }).unwrap();
    assert!(close(extent(&wide).x0, 150.0 - 150.0 * 0.5, 1e-6));
    // Its back shows when turned past 90°: a plane is two-sided.
    assert_eq!(plane(&p, b, View { rotation_y: 150.0, ..front() }).unwrap().len(), 1);
}

#[test]
fn inflate_covers_the_shape_and_puffs_by_the_volume() {
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((100.0, 100.0), 50.0), 0.01));
    let (p, b) = shape(&c);
    let f = inflate(&p, b, Inflate::default(), front()).unwrap();
    // Straight on, the cushion covers the circle (sampled as a polygon) and nothing else.
    assert!(close(area(&f), std::f64::consts::PI * 50.0 * 50.0, 60.0), "{}", area(&f));
    assert!(extent(&f).x0 >= 50.0 - 1e-6 && extent(&f).x1 <= 150.0 + 1e-6);
    // Its slopes catch the light differently.
    let (lo, hi) = f.iter().fold((1.0f64, 0.0f64), |(lo, hi), f| (lo.min(f.brightness), hi.max(f.brightness)));
    assert!(hi - lo > 0.3, "{lo}..{hi}");
    // From the side: 50 % volume rounds a disc into a half sphere on a flat back.
    let side = View { rotation_y: 90.0, ..front() };
    let thick = |o: Inflate| extent(&inflate(&p, b, o, side).unwrap()).width();
    assert!(close(thick(Inflate::default()), 50.0, 2.0), "{}", thick(Inflate::default()));
    assert!(close(thick(Inflate { both_sides: true, ..Inflate::default() }), 100.0, 4.0));
    assert!(close(thick(Inflate { depth: 30.0, ..Inflate::default() }), 80.0, 2.0));
    assert!(close(thick(Inflate { volume: 25.0, ..Inflate::default() }), 25.0, 2.0));
    assert!(thick(Inflate { volume: 0.0, depth: 10.0, ..Inflate::default() }) <= 10.0 + 1e-6);
}

fn disc(cx: f64, cy: f64, r: f64) -> BezPath {
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((cx, cy), r), 0.01));
    c
}

#[test]
fn a_pillow_rises_from_a_seam_at_an_angle_and_is_plump_and_level_on_top() {
    let (p, b) = shape(&disc(100.0, 100.0, 50.0));
    let puff = |pillow: f64| Puff::solve(&p, b, Inflate { pillow, ..Inflate::default() }).unwrap().unwrap();
    let (dome, pillow) = (puff(0.0), puff(100.0));
    let at = |f: &Puff, rho: f64| f.lift(Point::new(100.0 + 50.0 * rho, 100.0));
    // The same height at the middle, the half sphere's radius.
    assert!(close(at(&dome, 0.0), 50.0, 1.0) && close(at(&pillow, 0.0), 50.0, 1.0), "{} {}", at(&dome, 0.0), at(&pillow, 0.0));
    // Plumper across the top, lower towards the seam.
    assert!(at(&pillow, 0.4) > at(&dome, 0.4) + 1.0, "{} {}", at(&pillow, 0.4), at(&dome, 0.4));
    assert!(at(&pillow, 0.97) < at(&dome, 0.97) - 3.0, "{} {}", at(&pillow, 0.97), at(&dome, 0.97));
    // At the seam a dome stands straight up (its height grows as the root of the way in), a
    // pillow leans out at an angle (its height grows with the way in): twice as far in, a dome is
    // √2 as high, a pillow twice.
    let growth = |f: &Puff| at(f, 1.0 - 1.0 / 50.0) / at(f, 1.0 - 0.5 / 50.0);
    assert!(close(growth(&dome), 2f64.sqrt(), 0.1), "{}", growth(&dome));
    assert!(close(growth(&pillow), 2.0, 0.1), "{}", growth(&pillow));
    // A square pillow's corners are pinched: lower there than along its sides at the same inset.
    let (sq, r) = shape(&square(0.0, 0.0, 100.0));
    let s = Puff::solve(&sq, r, Inflate { pillow: 100.0, ..Inflate::default() }).unwrap().unwrap();
    assert!(s.lift(Point::new(12.0, 12.0)) < 0.8 * s.lift(Point::new(50.0, 12.0)));
}

#[test]
fn stacked_layers_rise_from_the_surface_below_them() {
    let (base, _) = shape(&disc(100.0, 100.0, 50.0));
    let (upper, _) = shape(&disc(100.0, 100.0, 20.0));
    let b = Rect::new(50.0, 50.0, 150.0, 150.0);
    let side = View { rotation_y: 90.0, ..front() };
    let o = Inflate::default();
    let alone = inflate(&base, b, o, side).unwrap();
    let stack = inflate_stack(&[base.clone(), upper.clone()], b, o, side, &[]).unwrap();
    assert_eq!(stack.len(), 2);
    // The bottom layer is drawn as on its own (only its depth order, framed for the whole stack,
    // differs).
    let outlines = |f: &[Facet]| f.iter().map(|f| f.contours.clone()).collect::<Vec<_>>();
    assert_eq!(outlines(&stack[0].facets), outlines(&alone));
    // From the side the top layer stands on the bottom's peak: their heights add.
    let top = extent(&stack[1].facets);
    let small = extent(&inflate(&upper, b, o, side).unwrap()).width();
    // It rises from the bottom's surface under its rim (0.4 of the way out on a half sphere of
    // 50) to the bottom's peak plus its own height.
    let rim = 50.0 * (1.0 - 0.4f64 * 0.4).sqrt();
    assert!(close(top.width(), 50.0 + small - rim, 2.0), "{} vs {}", top.width(), 50.0 + small - rim);
    let (lo, hi) = (extent(&alone).x0.min(extent(&alone).x1), top.x0.max(top.x1));
    assert!(close(hi - lo, 50.0 + small, 3.0), "{} vs {}", hi - lo, 50.0 + small);
    // Straight on, the top layer covers its own disc only, over the bottom.
    let f = inflate_stack(&[base, upper], b, o, front(), &[]).unwrap();
    assert!(close(area(&f[1].facets), std::f64::consts::PI * 400.0, 20.0), "{}", area(&f[1].facets));
    assert!(inflate_stack(&vec![PathData::default(); MAX_LAYERS + 1], b, o, front(), &[]).is_err());
}

#[test]
fn inflate_leaves_holes_open() {
    let mut b = square(0.0, 0.0, 100.0);
    b.extend(square(30.0, 30.0, 40.0));
    let (p, r) = shape(&b);
    let f = inflate(&p, r, Inflate::default(), front()).unwrap();
    assert!(close(area(&f), 100.0 * 100.0 - 40.0 * 40.0, 1e-3), "{}", area(&f));
}

#[test]
fn malformed_and_degenerate_input_is_bounded() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    for r in [Rect::new(f64::NAN, 0.0, 1.0, 1.0), Rect::new(0.0, 0.0, 1e9, 1.0)] {
        assert!(extrude(&p, r, Extrude::default(), front()).is_err());
        assert!(inflate(&p, r, Inflate::default(), front()).is_err());
        assert!(plane(&p, r, front()).is_err());
    }
    let wild = Extrude {
        depth: f64::INFINITY,
        twist: f64::NAN,
        taper: -3.0,
        cap: true,
        bevel: Some(Bevel { width: f64::NAN, height: 1e12, repeat: usize::MAX, space: 7.0, ..Bevel::default() }),
    };
    let f = extrude(&p, b, wild, View { rotation_x: f64::NAN, perspective: 9.0, ..front() }).unwrap();
    assert!(f.iter().flat_map(|f| f.contours.iter().flatten()).all(|q| q.x.is_finite() && q.y.is_finite()));
    assert!(
        inflate(
            &p,
            b,
            Inflate {
                depth: -5.0,
                volume: f64::NAN,
                pillow: f64::NAN,
                both_sides: true,
                twist: f64::INFINITY,
                taper: f64::NAN,
                cap: true,
                draft: false
            },
            front()
        )
        .is_ok()
    );
    assert!(extrude(&PathData::default(), b, Extrude::default(), front()).unwrap().is_empty());
    // A line has no area to extrude.
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((100.0, 0.0));
    assert!(extrude(&PathData::from_bezpath(&line), b, Extrude::default(), front()).unwrap().is_empty());
    // Too many anchors: refused, not truncated.
    let mut many = BezPath::new();
    many.move_to((0.0, 0.0));
    for i in 0..3000 {
        many.line_to((f64::from(i), f64::from(i % 7)));
    }
    many.close_path();
    assert!(extrude(&PathData::from_bezpath(&many), b, Extrude::default(), front()).is_err());
}

#[test]
fn perspective_makes_the_near_face_larger() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let o = Extrude { depth: 100.0, ..Extrude::default() };
    let flat = extent(&extrude(&p, b, o, front()).unwrap());
    let deep = extent(&extrude(&p, b, o, View { perspective: 0.6, ..front() }).unwrap());
    assert!(deep.width() > flat.width() + 1.0);
    assert!(close(deep.center().x, flat.center().x, 1e-6));
}

#[test]
fn the_inflate_membrane_matches_known_solutions() {
    // ∇²φ = −1, zero on the outline: a disc of radius r peaks at r²/4, a square of side l at
    // about 0.07367 l².
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((100.0, 100.0), 50.0), 0.01));
    let (p, b) = shape(&c);
    let disc = Membrane::solve(&rings(&p, 100.0, MAX_SOLID_POINTS).unwrap(), b.origin(), b.width() / 48.0, 49, 49, None);
    assert!(close(disc.peak, 50.0 * 50.0 / 4.0, 625.0 * 0.02), "{}", disc.peak);
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let sq = Membrane::solve(&rings(&p, 100.0, MAX_SOLID_POINTS).unwrap(), b.origin(), 100.0 / 48.0, 49, 49, None);
    assert!(close(sq.peak, 0.07367 * 100.0 * 100.0, 736.7 * 0.02), "{}", sq.peak);
    // Smooth across the middle: no crease where a distance-to-the-edge height would have one
    // (the diagonals). The second difference along a diagonal stays small and even.
    let along: Vec<f64> = (10..=90).step_by(5).map(|t| sq.at(Point::new(f64::from(t), f64::from(t)))).collect();
    // (Near the corners φ grows like r², so the bend changes sign there: that is the solution,
    // not a crease. A crease would be one bend far larger than its neighbours.)
    let bends: Vec<f64> = along.windows(3).map(|w| w[0] - 2.0 * w[1] + w[2]).collect();
    let largest = bends.iter().fold(0.0f64, |m, b| m.max(b.abs()));
    assert!(bends.windows(2).all(|w| (w[1] - w[0]).abs() < 0.4 * largest), "no kink: {bends:?}");
    // Zero (or just below, extrapolated) on the outline.
    assert!(sq.at(Point::new(0.0, 50.0)).abs() < 1.0 && sq.at(Point::new(50.0, 0.0)).abs() < 1.0);
}

/// Grid cells across a hole's edge are cut to the shape itself: a hole's piece cancelling part of
/// the outline's only works flat, and lifted onto the surface it left notches along the edge.
#[test]
fn inflate_cells_across_a_hole_edge_are_cut_to_the_shape() {
    let mut b = square(80.0, 330.0, 220.0);
    b.move_to((140.0, 390.0));
    b.line_to((140.0, 480.0));
    b.line_to((240.0, 480.0));
    b.line_to((240.0, 390.0));
    b.close_path();
    let (p, r) = shape(&b);
    let v = View { rotation_x: -21.88, rotation_y: -23.4, rotation_z: 9.06, ..front() };
    let f = inflate(&p, r, Inflate::default(), v).unwrap();
    assert!(f.iter().all(|f| f.contours.len() == 1), "every facet is one piece of the surface");
    // Straight on, the facets still cover exactly the shape.
    let flat = inflate(&p, r, Inflate::default(), front()).unwrap();
    assert!(close(area(&flat), 220.0 * 220.0 - 100.0 * 90.0, 1e-3), "{}", area(&flat));
}

/// A row of `count` small bumps, each a curve, closed along the bottom: like an outlined word,
/// many short curves.
fn bumps(count: usize) -> (PathData, Rect) {
    let mut b = BezPath::new();
    b.move_to((0.0, 20.0));
    for i in 0..count {
        let x = i as f64 * 4.0;
        b.curve_to((x + 1.0, 14.0), (x + 3.0, 14.0), (x + 4.0, 20.0));
    }
    b.line_to((count as f64 * 4.0, 30.0));
    b.line_to((0.0, 30.0));
    b.close_path();
    shape(&b)
}

/// Curves are followed within a tolerance rather than by sixteen points each: small curves take a
/// few points, so shapes of many curves (outlined type) fit; Rotate, which only turns the outline,
/// takes far more.
#[test]
fn small_curves_take_few_points_and_rotate_takes_large_outlines() {
    // 300 curves were 4,800 points at sixteen each, over the old limit of 1,024.
    let (p, r) = bumps(300);
    assert!(extrude(&p, r, Extrude::default(), front()).is_ok());
    assert!(inflate(&p, r, Inflate { volume: 20.0, ..Inflate::default() }, front()).is_ok());
    assert!(plane(&p, r, front()).is_ok());
    // 4,000 curves: too many for a solid, not for Rotate.
    let (p, r) = bumps(4000);
    assert!(extrude(&p, r, Extrude::default(), front()).unwrap_err().contains("too complex"));
    assert_eq!(plane(&p, r, front()).unwrap().len(), 1);
    // Each piece stays within the tolerance of the curve: a circle's chords sag at most 0.05 pt
    // here (1/2000 of its 100 pt size is less).
    let mut c = BezPath::new();
    c.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((50.0, 50.0), 50.0), 0.01));
    let (p, r) = shape(&c);
    let ring = &rings(&p, 100.0, MAX_SOLID_POINTS).unwrap()[0];
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let mid = a.midpoint(*b);
        assert!(50.0 - mid.distance(Point::new(50.0, 50.0)) <= 0.05 + 1e-3, "{a:?} {b:?}");
    }
    assert!(r.width() > 0.0);
    // A small bump takes a few points; a large curve more than sixteen where it needs them.
    let (p, r) = bumps(300);
    let small = rings(&p, r.width().max(r.height()), MAX_SOLID_POINTS).unwrap()[0].len();
    assert!(small <= 300 * 4 + 3, "{small} points for 300 bumps");
    assert!(ring.len() > 4 * 16, "a 100 pt circle within 0.05 pt: {}", ring.len());
}

/// The bucketed nearest-edge search agrees with checking every edge, inside, outside and far
/// beyond the shape.
#[test]
fn the_edge_index_finds_the_nearest_edge() {
    let (p, r) = bumps(200);
    let rings = rings(&p, r.width(), MAX_SOLID_POINTS).unwrap();
    let edges = Edges::new(&rings);
    for k in 0..400 {
        let t = f64::from(k);
        let q = Point::new(-300.0 + (t * 37.0) % 1400.0, -50.0 + (t * 13.0) % 130.0);
        let (fast, slow) = (edges.distance(q), edge_distance(&rings, q));
        assert!((fast - slow).abs() < 1e-9, "{q:?}: {fast} against {slow}");
    }
}

/// From a point outside the grid, an edge further along the grid's side can be nearer than one
/// straight ahead: the search doesn't stop before it has looked there.
#[test]
fn the_edge_index_looks_along_the_side_from_outside() {
    let ring = |b: BezPath| rings(&shape(&b).0, 64.0, MAX_SOLID_POINTS).unwrap().remove(0);
    // Straight ahead of the point, a little further up its side, and the far corner (a 64 pt grid).
    let rings = vec![ring(square(2.5, 0.0, 1.0)), ring(square(0.0, 4.2, 0.8)), ring(square(63.0, 63.0, 1.0))];
    let edges = Edges::new(&rings);
    let q = Point::new(-10.0, 0.5);
    let (fast, slow) = (edges.distance(q), edge_distance(&rings, q));
    assert!((fast - slow).abs() < 1e-9, "{fast} against {slow}");
}

/// Twist runs 0° to 360°, as documented: beyond either end it holds at that end.
#[test]
fn twist_holds_between_0_and_360_degrees() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let v = View { rotation_x: -20.0, rotation_y: 30.0, ..front() };
    let at = |twist: f64| extrude(&p, b, Extrude { twist, ..Extrude::default() }, v).unwrap();
    assert_eq!(at(-45.0), at(0.0));
    assert_eq!(at(500.0), at(360.0));
    assert_ne!(at(45.0), at(0.0));
}

/// A twisted extrusion shows every wall facing the viewer: a twist bends each strip between two
/// slices into a saddle, and its normal from one corner's triangle leaned so far back that from
/// 60° on every wall was dropped. Checked against each strip's average normal, oriented away from
/// the square's axis.
#[test]
fn twisted_walls_face_the_way_they_turn() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let c = b.center();
    for twist in [60.0, 120.0, 180.0, 300.0] {
        for (x, y) in [(-20.0, 30.0), (0.0, 90.0), (30.0, -60.0), (-45.0, 150.0)] {
            let v = View { rotation_x: x, rotation_y: y, ..front() };
            let o = Extrude { twist, ..Extrude::default() };
            let got = extrude(&p, b, o, v).unwrap();
            // Expected: walls (and caps) whose outward normal faces the viewer.
            let o = o.sanitized();
            let camera = Camera::new(v, b, o.depth);
            let ring = &rings(&p, b.width().max(b.height()), MAX_SOLID_POINTS).unwrap()[0];
            let slices = ((twist / 6.0).ceil() as usize).clamp(8, 64);
            let z = |t: f64| o.depth / 2.0 - o.depth * t;
            let mut expected = 0;
            for k in 0..slices {
                let (t0, t1) = (k as f64 / slices as f64, (k + 1) as f64 / slices as f64);
                let (r0, r1) = (section(ring, b, &o, t0), section(ring, b, &o, t1));
                for i in 0..ring.len() {
                    let j = (i + 1) % ring.len();
                    let q = [camera.place(r0[i], z(t0)), camera.place(r0[j], z(t0)), camera.place(r1[j], z(t1)), camera.place(r1[i], z(t1))];
                    let mut n = newell(&[q.to_vec()]).unit();
                    let mid = r0[i].midpoint(r1[j]);
                    let away = camera.rotate(V3(mid.x - c.x, mid.y - c.y, 0.0));
                    if n.dot(away) < 0.0 {
                        n = V3(-n.0, -n.1, -n.2);
                    }
                    let centre = V3((q[0].0 + q[2].0) / 2.0, (q[0].1 + q[2].1) / 2.0, (q[0].2 + q[2].2) / 2.0);
                    // Kept as the builder keeps them: facing, or seen almost edge-on, with an area.
                    let projected: Vec<Point> = q.iter().map(|v| camera.project(*v)).collect();
                    if camera.facing(centre, n) > -0.02 && signed_area(&projected).abs() >= 1e-10 {
                        expected += 1;
                    }
                }
            }
            for (n, t) in [(1.0, 0.0), (-1.0, 1.0)] {
                let cap: Vec<Point> = section(ring, b, &o, t).iter().map(|p| camera.project(camera.place(*p, z(t)))).collect();
                if camera.facing(V3::default(), camera.rotate(V3(0.0, 0.0, n))) > -0.02 && signed_area(&cap).abs() >= 1e-10 {
                    expected += 1;
                }
            }
            assert_eq!(got.len(), expected, "twist {twist}, view ({x}, {y})");
        }
    }
}

/// Inflate takes Twist, Taper and Cap like the other types: the back turned and scaled (its puff
/// too), the sides following, and without a cap no back at all.
#[test]
fn inflate_twists_tapers_and_opens_its_back() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let side = View { rotation_y: 90.0, ..front() };
    let o = Inflate { depth: 60.0, volume: 0.0, ..Inflate::default() };
    // From the side, a flat (0 % volume) cushion is the extrusion: front 100 high, the back 50
    // at a 50 % taper.
    let tapered = inflate(&p, b, Inflate { taper: 0.5, ..o }, side).unwrap();
    let e = extent(&tapered);
    assert!(close(e.width(), 60.0, 1e-6) && close(e.height(), 100.0, 1e-6), "{e:?}");
    let back: Vec<Point> = tapered.iter().flat_map(|f| f.contours.iter().flatten().copied()).filter(|q| close(q.x, e.x0, 1e-6)).collect();
    let (lo, hi) = back.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    assert!(close(hi - lo, 50.0, 1e-6), "back {}", hi - lo);
    // From behind, a 45° twist shows the flat back as a diamond.
    let behind = View { rotation_y: 180.0, ..front() };
    let twisted = inflate(&p, b, Inflate { twist: 45.0, ..o }, behind).unwrap();
    assert!(close(extent(&twisted).width(), 100.0 * 2f64.sqrt(), 1e-6), "{:?}", extent(&twisted));
    // Hollow: no back; from behind the inside of the inflated front shows instead of a flat face.
    let solid = inflate(&p, b, Inflate { volume: 50.0, ..o }, behind).unwrap();
    assert_eq!(solid.len(), 1, "the flat back alone");
    let hollow = inflate(&p, b, Inflate { volume: 50.0, cap: false, ..o }, behind).unwrap();
    assert!(hollow.len() > 100 && hollow.iter().all(|f| f.contours.len() == 1 && f.contours[0].len() < 12));
    // With both sides puffed, the back puff is scaled with the taper: lower and smaller.
    let both = |taper: f64| extent(&inflate(&p, b, Inflate { both_sides: true, volume: 50.0, taper, depth: 0.0, ..o }, side).unwrap());
    assert!(both(0.5).width() < both(1.0).width() && both(0.5).height() <= both(1.0).height() + 1e-6);
    // Out of range values hold at the documented ends.
    let at = |i: Inflate| inflate(&p, b, i, View { rotation_x: -20.0, rotation_y: 30.0, ..front() }).unwrap();
    assert_eq!(at(Inflate { twist: -30.0, ..o }), at(o));
    assert_eq!(at(Inflate { taper: 0.0, ..o }), at(Inflate { taper: 0.01, ..o }));
}

/// At 0 % volume an inflated shape is the extruded one: twisted and tapered, their outlines
/// agree from every side (no wall goes missing in either).
#[test]
fn a_flat_twisted_inflate_matches_the_extrusion() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    for (x, y) in [(-20.0, 30.0), (0.0, 90.0), (30.0, -60.0), (-45.0, 150.0), (60.0, 200.0)] {
        let v = View { rotation_x: x, rotation_y: y, ..front() };
        let e = extent(&extrude(&p, b, Extrude { depth: 60.0, twist: 120.0, taper: 0.6, ..Extrude::default() }, v).unwrap());
        let i = extent(&inflate(&p, b, Inflate { depth: 60.0, twist: 120.0, taper: 0.6, volume: 0.0, ..Inflate::default() }, v).unwrap());
        for (a, c) in [(e.x0, i.x0), (e.y0, i.y0), (e.x1, i.x1), (e.y1, i.y1)] {
            assert!(close(a, c, 1e-6), "view ({x}, {y}): extrusion {e:?}, inflate {i:?}");
        }
    }
}

#[test]
fn coloured_lights_tint_each_channel_and_the_defaults_stay_grey() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    for f in extrude(&p, b, Extrude::default(), View::default()).unwrap() {
        assert!(f.tint.iter().all(|t| close(*t, f.brightness, 1e-12)), "white light: a grey tint");
    }
    let red = View { light_color: [1.0, 0.0, 0.0], ambient: 0.0, ..View::default() };
    for f in extrude(&p, b, Extrude::default(), red).unwrap() {
        assert!(f.tint[1] == 0.0 && f.tint[2] == 0.0, "{:?}", f.tint);
    }
    // A second light from the right brightens the right wall.
    let base = View { rotation_y: -40.0, ..front() };
    let right_wall = |v: View| extrude(&p, b, Extrude::default(), v).unwrap().iter().map(|f| f.brightness).fold(0.0, f64::max);
    let lit = View { extra_lights: [Light { azimuth: 90.0, elevation: 0.0, intensity: 1.0, ..Light::default() }; 3], extra_count: 1, ..base };
    assert!(right_wall(lit) > right_wall(base));
}

#[test]
fn a_solid_casts_its_shadow_below_or_behind_when_asked() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let none = extrude_lit(&p, b, Extrude::default(), View::default()).unwrap();
    assert!(none.shadow.is_none());
    let below = View { shadow: ShadowPosition::Below, shadow_distance: 20.0, ..View::default() };
    let s = extrude_lit(&p, b, Extrude::default(), below).unwrap();
    let solid = extent(&s.facets);
    let shadow = s.shadow.expect("a shadow below").bounds().unwrap();
    assert!(shadow.y1 > solid.y1, "the shadow reaches below the solid: {shadow:?} {solid:?}");
    let behind = View { shadow: ShadowPosition::Behind, ..View::default() };
    assert!(inflate_lit(&p, b, Inflate::default(), behind).unwrap().shadow.is_some());
    assert!(plane_lit(&p, b, behind).unwrap().shadow.is_some());
    // A light from below casts nothing on the ground.
    let under = View { light_elevation: -40.0, ..below };
    assert!(extrude_lit(&p, b, Extrude::default(), under).unwrap().shadow.is_none());
    // Revolve casts one too.
    let r = Revolve { shadow: ShadowPosition::Below, ..Revolve::default() };
    assert!(crate::revolve_lit(&p, b, r).unwrap().shadow.is_some());
}

/// Straight on, every edge of an inflated surface's pieces away from the outline is shared with
/// the piece next to it, the other way round: no cracks, no edges ending partway along another's
/// (T-junctions), and no zero-width slivers tying two parts of a cell together along its side,
/// which used to rise into fins where the outline left a cell and came back (a star's points).
#[test]
fn an_inflated_surface_is_watertight() {
    use vectorcraft_geom::kurbo::{Circle, Shape};
    let mut ring = BezPath::new();
    ring.extend(Circle::new((100.0, 100.0), 80.0).path_elements(0.01));
    let hole: BezPath = Circle::new((100.0, 100.0), 35.0).path_elements(0.01).collect();
    ring.extend(hole.reverse_subpaths());
    let mut star = BezPath::new();
    for k in 0..10 {
        let a = std::f64::consts::PI * f64::from(k) / 5.0;
        let r = if k % 2 == 0 { 90.0 } else { 40.0 };
        let p = (100.0 + r * a.sin(), 100.0 - r * a.cos());
        if k == 0 { star.move_to(p) } else { star.line_to(p) }
    }
    star.close_path();
    for (name, b) in [("ring", ring), ("star", star)] {
        let (p, r) = shape(&b);
        let f = inflate(&p, r, Inflate::default(), front()).unwrap();
        let outline = rings(&p, r.width().max(r.height()), MAX_SOLID_POINTS).unwrap();
        let edges = Edges::new(&outline);
        assert!(!rings_cross(&outline, &edges), "{name}");
        let key = |p: Point| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64);
        let all: Vec<(Point, Point)> = f
            .iter()
            .flat_map(|f| f.contours.iter())
            .flat_map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| (*a, *b)))
            .collect();
        let set: std::collections::HashSet<_> = all.iter().map(|(a, b)| (key(*a), key(*b))).collect();
        for (a, b) in &all {
            assert!(filled(&outline, *a) || edges.distance(*a) < 1e-6, "{name}: {a:?} is outside the shape");
            let mid = a.midpoint(*b);
            assert!(set.contains(&(key(*b), key(*a))) || edges.distance(mid) < 1e-6, "{name}: {a:?}–{b:?} is shared with no piece");
        }
    }
}

/// The exact cut of a cell keeps parts the outline leaves and re-enters apart, and its holes.
#[test]
fn a_cell_is_cut_to_the_shape_exactly() {
    // A U: the cell [0, 10]² takes both arms' feet, which only meet below it.
    let u = vec![
        Point::new(0.0, -10.0),
        Point::new(10.0, -10.0),
        Point::new(10.0, 20.0),
        Point::new(7.0, 20.0),
        Point::new(7.0, -5.0),
        Point::new(3.0, -5.0),
        Point::new(3.0, 20.0),
        Point::new(0.0, 20.0),
    ];
    let mut u = u;
    if signed_area(&u) < 0.0 {
        u.reverse();
    }
    let parts = cut_cell(&[u.clone()], Rect::new(-1.0, 0.0, 11.0, 10.0));
    assert_eq!(parts.len(), 2, "{parts:?}");
    let total: f64 = parts.iter().map(|c| signed_area(c)).sum();
    assert!(close(total, 2.0 * 3.0 * 10.0, 1e-9), "{total}");
    // A cell inside a hole is empty; a cell round a hole keeps it.
    let mut square = vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];
    if signed_area(&square) < 0.0 {
        square.reverse();
    }
    let mut hole = vec![Point::new(40.0, 40.0), Point::new(60.0, 40.0), Point::new(60.0, 60.0), Point::new(40.0, 60.0)];
    if signed_area(&hole) > 0.0 {
        hole.reverse();
    }
    let shape = [square, hole];
    assert!(cut_cell(&shape, Rect::new(45.0, 45.0, 55.0, 55.0)).is_empty());
    let round: f64 = cut_cell(&shape, Rect::new(30.0, 30.0, 70.0, 70.0)).iter().map(|c| signed_area(c)).sum();
    assert!(close(round, 40.0 * 40.0 - 20.0 * 20.0, 1e-9), "{round}");
    let half: f64 = cut_cell(&shape, Rect::new(50.0, 30.0, 70.0, 70.0)).iter().map(|c| signed_area(c)).sum();
    assert!(close(half, 20.0 * 40.0 - 10.0 * 20.0, 1e-9), "{half}");
    assert!(close(cut_cell(&shape, Rect::new(-5.0, -5.0, 5.0, 5.0)).iter().map(|c| signed_area(c)).sum(), 25.0, 1e-9));
    // Crossing rings are told apart from ones that merely meet at a corner.
    let tri = |o: f64| vec![Point::new(o, 0.0), Point::new(o + 10.0, 0.0), Point::new(o + 5.0, 8.0)];
    assert!(rings_cross(&[tri(0.0), tri(5.0)], &Edges::new(&[tri(0.0), tri(5.0)])));
    assert!(!rings_cross(&[tri(0.0), tri(20.0)], &Edges::new(&[tri(0.0), tri(20.0)])));
}

/// A ring's surface is round, so its normals point straight across the ring, with no sideways
/// lean, rim included: a mirror finish shows a lean of a degree or two as streaks across the rim.
/// (The finely flattened outline once had too few pieces per curve, and the slope near the
/// outline was read from grid nodes a hair from it: both leaned the normals by up to 0.05.)
#[test]
fn an_inflated_ring_has_no_sideways_lean_in_its_normals() {
    use vectorcraft_geom::kurbo::{Circle, Shape};
    let mut ring = BezPath::new();
    ring.extend(Circle::new((100.0, 100.0), 80.0).path_elements(0.01));
    let hole: BezPath = Circle::new((100.0, 100.0), 35.0).path_elements(0.01).collect();
    ring.extend(hole.reverse_subpaths());
    let (p, r) = shape(&ring);
    let view = View { material: Material { smooth: true, ..Material::default() }, ..front() };
    let shaded = inflate_lit(&p, r, Inflate { volume: 80.0, ..Inflate::default() }, view).unwrap();
    let shading = shaded.shading.unwrap();
    // The ring's centre in view space: the mean of its front's points.
    let front: Vec<(V3, V3)> =
        shading.polys().iter().flat_map(|poly| poly.positions.iter().copied().zip(poly.normals.iter().copied())).filter(|(_, n)| n.2 > 0.0).collect();
    let k = front.len() as f64;
    let (cx, cy) = front.iter().fold((0.0, 0.0), |s, (p, _)| (s.0 + p.0 / k, s.1 + p.1 / k));
    let mut worst: f64 = 0.0;
    for (p, n) in &front {
        let (dx, dy) = (p.0 - cx, p.1 - cy);
        let l = dx.hypot(dy);
        if l < 1.0 {
            continue;
        }
        worst = worst.max(((-dy * n.0 + dx * n.1) / l).abs());
    }
    assert!(worst < 0.012, "the normals lean sideways by up to {worst}");
}

/// A square's ring, as art to lay on a surface.
fn art_square(x: f64, y: f64, s: f64) -> Vec<Vec<Point>> {
    vec![vec![Point::new(x, y), Point::new(x + s, y), Point::new(x + s, y + s), Point::new(x, y + s)]]
}

#[test]
fn art_on_a_flat_front_seen_straight_on_stays_where_it_is() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let art = [art_square(20.0, 30.0, 40.0)];
    for s in [plane_mapped(&p, b, front(), &art).unwrap(), extrude_mapped(&p, b, Extrude::default(), front(), &art).unwrap()] {
        let [m] = s.mapped.as_slice() else { panic!("one piece") };
        let ring = &m.contours[0];
        // Its edges are cut into short pieces, all on the square's outline.
        assert!(ring.len() > 4);
        assert!(
            ring.iter().all(|q| (q.x - 20.0).abs() < 1e-9 || (q.x - 60.0).abs() < 1e-9 || (q.y - 30.0).abs() < 1e-9 || (q.y - 70.0).abs() < 1e-9)
        );
        assert!(ring.iter().all(|q| (20.0 - 1e-9..=60.0 + 1e-9).contains(&q.x) && (30.0 - 1e-9..=70.0 + 1e-9).contains(&q.y)));
    }
}

#[test]
fn art_on_a_front_turned_away_is_hidden() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    let art = [art_square(20.0, 30.0, 40.0)];
    let away = View { rotation_y: 150.0, ..front() };
    let s = extrude_mapped(&p, b, Extrude::default(), away, &art).unwrap();
    assert!(s.mapped[0].contours.is_empty());
    // Turned part way, it foreshortens with the front.
    let turned = View { rotation_y: 50.0, ..front() };
    let s = extrude_mapped(&p, b, Extrude::default(), turned, &art).unwrap();
    let xs: Vec<f64> = s.mapped[0].contours[0].iter().map(|q| q.x).collect();
    let width = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - xs.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(close(width, 40.0 * 50f64.to_radians().cos(), 1e-6), "{width}");
}

#[test]
fn art_on_an_inflated_front_rides_the_puff() {
    let mut circle = BezPath::new();
    circle.extend(vectorcraft_geom::kurbo::Shape::path_elements(&vectorcraft_geom::kurbo::Circle::new((50.0, 50.0), 50.0), 0.01));
    let (p, b) = shape(&circle);
    let tilted = View { rotation_x: -60.0, ..front() };
    let art = [art_square(45.0, 45.0, 10.0), art_square(50.0, 0.0, 0.0001)];
    let puffed = inflate_mapped(&p, b, Inflate::default(), tilted, &art).unwrap();
    let flat = plane_mapped(&p, b, tilted, &art).unwrap();
    // The middle is lifted by the puff (50 pt at 50 %), so tilted it shows 43 pt lower down.
    let centre = |s: &Shaded| s.mapped[0].contours[0].iter().map(|q| q.y).sum::<f64>() / s.mapped[0].contours[0].len() as f64;
    assert!(close(centre(&puffed) - centre(&flat), 50.0 * 60f64.to_radians().sin(), 2.0), "{} {}", centre(&flat), centre(&puffed));
    // At the outline the puff is nothing.
    let rim = |s: &Shaded| s.mapped[1].contours[0][0];
    assert!(rim(&puffed).distance(rim(&flat)) < 0.5, "{:?} {:?}", rim(&puffed), rim(&flat));
    // Shaded like the surface under it: in the middle it faces out as flat art does; at the rim
    // it turns to the side.
    assert_eq!(puffed.mapped[0].shade, flat.mapped[0].shade);
    assert_ne!(puffed.mapped[1].shade, flat.mapped[1].shade);
}

#[test]
fn mapped_art_is_bounded() {
    let (p, b) = shape(&square(0.0, 0.0, 100.0));
    // A huge piece is cut into at most so many points; later pieces are left off.
    let huge = vec![vec![Point::new(0.0, 0.0), Point::new(1e7, 0.0), Point::new(1e7, 1e7)]; 50];
    let s = plane_mapped(&p, b, front(), &[huge.clone(), huge]).unwrap();
    assert!(s.mapped.iter().flat_map(|m| m.contours.iter().flatten()).count() <= MAX_MAPPED_POINTS);
    let bad = [vec![vec![Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)]]];
    assert!(plane_mapped(&p, b, front(), &bad).is_err());
    assert!(plane_mapped(&p, b, front(), &[vec![]]).unwrap().mapped[0].contours.is_empty());
}
