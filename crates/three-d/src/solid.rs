//! Extrude & Bevel, Inflate and Rotate (a flat plane) as projected, shaded vector facets.
//!
//! The 2D art is the front of the solid: its outline is extruded backwards along the view's
//! z-axis, about the centre of its bounds, at the art's own position and size. Planar faces (caps
//! and the rotated plane) are drawn whole, holes included, so no triangulation is needed; curved
//! faces (bevels, side walls under twist or taper, inflated surfaces) are cut into small facets
//! and flat shaded, like Revolve.
//!
//! Visibility is a painter's algorithm, as for Revolve: faces of a closed solid that face away
//! from the viewer are left out, the rest are drawn back to front.

use vectorcraft_geom::{BezPath, ParamCurve, PathData, Point, Rect};

use crate::lighting::{self, Light, MAX_EXTRA_LIGHTS, Material, Shade, ShadowDirection, ShadowPosition};
use crate::raster::{Poly, Shading};
use crate::smooth::{self, Source};
use crate::{MAX_FACES, Revolve, V3, mean};

/// The view shared by every 3D effect: rotation, perspective, the lights and the shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective strength, 0 for orthographic projection, up to 1.
    pub perspective: f64,
    pub light_azimuth: f64,
    pub light_elevation: f64,
    pub light_intensity: f64,
    pub ambient: f64,
    pub shade: bool,
    /// The main light's colour (linear 0–1 RGB) and softness (0–1).
    pub light_color: [f64; 3],
    pub light_softness: f64,
    /// More lights: the first `extra_count` of `extra_lights`.
    pub extra_lights: [Light; MAX_EXTRA_LIGHTS],
    pub extra_count: usize,
    /// Where the object casts its shadow, how far beyond it (points) and which way.
    pub shadow: ShadowPosition,
    pub shadow_distance: f64,
    pub shadow_direction: ShadowDirection,
    /// How the surface takes the light.
    pub material: Material,
}

impl Default for View {
    fn default() -> Self {
        let r = Revolve::default();
        Self {
            rotation_x: r.rotation_x,
            rotation_y: r.rotation_y,
            rotation_z: r.rotation_z,
            perspective: r.perspective,
            light_azimuth: r.light_azimuth,
            light_elevation: r.light_elevation,
            light_intensity: r.light_intensity,
            ambient: r.ambient,
            shade: r.shade,
            light_color: r.light_color,
            light_softness: r.light_softness,
            extra_lights: r.extra_lights,
            extra_count: r.extra_count,
            shadow: r.shadow,
            shadow_distance: r.shadow_distance,
            shadow_direction: r.shadow_direction,
            material: r.material,
        }
    }
}

impl View {
    /// The same rotation, projection and lighting maths as Revolve's.
    fn revolve(self) -> Revolve {
        Revolve {
            rotation_x: self.rotation_x,
            rotation_y: self.rotation_y,
            rotation_z: self.rotation_z,
            perspective: self.perspective,
            light_azimuth: self.light_azimuth,
            light_elevation: self.light_elevation,
            light_intensity: self.light_intensity,
            ambient: self.ambient,
            shade: self.shade,
            light_color: self.light_color,
            light_softness: self.light_softness,
            extra_lights: self.extra_lights,
            extra_count: self.extra_count,
            shadow: self.shadow,
            shadow_distance: self.shadow_distance,
            shadow_direction: self.shadow_direction,
            material: self.material,
            ..Revolve::default()
        }
        .sanitized()
    }
}

/// The shape of a bevel's cross-section, from the cap's edge (0) to the side wall (1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BevelShape {
    /// A straight 45° chamfer when width and height are equal.
    #[default]
    Classic,
    /// A convex quarter round.
    Rounded,
    /// A concave quarter round.
    Cove,
    /// Two flat steps.
    Step,
    /// A convex then concave S curve.
    Ogee,
}

impl BevelShape {
    pub const ALL: [BevelShape; 5] = [Self::Classic, Self::Rounded, Self::Cove, Self::Step, Self::Ogee];

    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Rounded => "rounded",
            Self::Cove => "cove",
            Self::Step => "step",
            Self::Ogee => "ogee",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == s)
    }

    /// The cross-section at `s` (0..1): (share of the width, share of the height), both 0..1.
    fn at(self, s: f64) -> (f64, f64) {
        let s = s.clamp(0.0, 1.0);
        let q = s * std::f64::consts::FRAC_PI_2;
        match self {
            Self::Classic => (s, s),
            Self::Rounded => (1.0 - q.cos(), q.sin()),
            Self::Cove => (q.sin(), 1.0 - q.cos()),
            Self::Step if s >= 1.0 => (1.0, 1.0),
            Self::Step => {
                // Down, across, down, across.
                let k = s * 4.0;
                let (i, f) = (k.floor(), k.fract());
                match i as u8 {
                    0 => (0.0, f * 0.5),
                    1 => (f * 0.5, 0.5),
                    2 => (0.5, 0.5 + f * 0.5),
                    _ => (0.5 + f * 0.5, 1.0),
                }
            }
            Self::Ogee => {
                let h = (1.0 - (s * std::f64::consts::PI).cos()) / 2.0;
                (s, h)
            }
        }
    }

    /// Samples along the cross-section: the step shape needs its corners exactly.
    fn samples(self) -> Vec<f64> {
        match self {
            Self::Classic => vec![0.0, 1.0],
            Self::Step => vec![0.0, 0.25, 0.5, 0.75, 1.0],
            _ => (0..=8).map(|i| f64::from(i) / 8.0).collect(),
        }
    }
}

/// A bevel along the front (and with `both_sides` the back) edge of an extrusion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub shape: BevelShape,
    /// How far the bevel reaches across the face, in points.
    pub width: f64,
    /// How deep it reaches along the extrusion, in points.
    pub height: f64,
    /// How many times the cross-section repeats across the width.
    pub repeat: usize,
    /// The share (0..1) of the width left flat between repeats.
    pub space: f64,
    /// Carved into the shape (true) or added around it (false).
    pub inside: bool,
    pub both_sides: bool,
}

impl Default for Bevel {
    fn default() -> Self {
        Self { shape: BevelShape::Classic, width: 4.0, height: 4.0, repeat: 1, space: 0.0, inside: false, both_sides: false }
    }
}

/// Extrude & Bevel's options. Lengths are points, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrude {
    pub depth: f64,
    /// The turn of the back face against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back face, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// Closed ends (a solid) or open ones (a hollow tube).
    pub cap: bool,
    pub bevel: Option<Bevel>,
}

impl Default for Extrude {
    fn default() -> Self {
        Self { depth: 50.0, twist: 0.0, taper: 1.0, cap: true, bevel: None }
    }
}

/// Inflate's options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inflate {
    /// The flat depth between the front and back, in points.
    pub depth: f64,
    /// How far the surface puffs out (0..100 %; 50 % rounds it like a half sphere).
    pub volume: f64,
    /// Puff the back out too; otherwise it stays flat.
    pub both_sides: bool,
    /// The turn of the back against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// A closed back (a solid) or none, so the inflated front is a shell open behind.
    pub cap: bool,
    /// A quick draft (a live preview being dragged): the surface on a grid half as fine.
    pub draft: bool,
}

impl Default for Inflate {
    fn default() -> Self {
        Self { depth: 0.0, volume: 50.0, both_sides: false, twist: 0.0, taper: 1.0, cap: true, draft: false }
    }
}

/// One projected, flat-shaded polygon. Holes are contours of their own (nonzero winding).
/// `tint` multiplies the paint channel by channel and `highlight` is added on top; `brightness`
/// is the tint's mean, at most 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub contours: Vec<Vec<Point>>,
    pub depth: f64,
    pub brightness: f64,
    pub tint: [f64; 3],
    pub highlight: [f64; 3],
}

/// A solid's facets, back to front, and the shadow it casts (an outline to fill), if any. With
/// smooth shading, `shading` draws the surface pixel by pixel (the facets' shades are then their
/// means, for paints that aren't a flat colour).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shaded {
    pub facets: Vec<Facet>,
    pub shadow: Option<PathData>,
    pub shading: Option<Shading>,
    /// The contact shadow where the solid meets the shadow's plane, and how strongly it shows.
    pub contact: Option<(PathData, f64)>,
    /// The art laid on the front surface ([`extrude_mapped`], [`inflate_mapped`],
    /// [`plane_mapped`]), piece by piece.
    pub mapped: Vec<Mapped>,
}

/// A piece of art laid on a solid's front surface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapped {
    /// Its rings on the surface, projected: none where the surface under it faces away.
    pub contours: Vec<Vec<Point>>,
    /// How the surface under its centre is lit.
    pub shade: Shade,
}

/// Mapped art's points in all, at most: more pieces are left off.
const MAX_MAPPED_POINTS: usize = 200_000;

/// `art` (pieces of rings, in the art's coordinates) laid on a surface and projected. `place`
/// gives a point's place on the surface and the surface's outward normal there (view space).
/// Edges are cut to at most `size / 96` so the art bends with a curved surface and perspective.
fn map_art(art: &[Vec<Vec<Point>>], size: f64, camera: &Camera, place: impl Fn(Point) -> (V3, V3)) -> Result<Vec<Mapped>, String> {
    let longest = (size / 96.0).max(0.01);
    let mut total = 0;
    let mut out = Vec::with_capacity(art.len());
    for piece in art {
        let count: usize = piece.iter().map(Vec::len).sum();
        if count == 0 {
            out.push(Mapped::default());
            continue;
        }
        let mean = piece.iter().flatten().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
        let centre = Point::new(mean.0 / count as f64, mean.1 / count as f64);
        if !centre.x.is_finite() || !centre.y.is_finite() {
            return Err("The art laid on the 3D surface contains invalid coordinates".into());
        }
        let (at, normal) = place(centre);
        // Faces away, or seen exactly edge-on (not a number): hidden.
        if camera.facing(at, normal).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            out.push(Mapped { contours: vec![], shade: camera.shade(normal) });
            continue;
        }
        let mut contours = Vec::with_capacity(piece.len());
        for ring in piece {
            let mut projected = Vec::with_capacity(ring.len());
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()) {
                let cuts = (a.distance(*b) / longest).ceil();
                let cuts = if cuts.is_finite() { (cuts as usize).clamp(1, 4096) } else { 1 };
                total += cuts;
                if total > MAX_MAPPED_POINTS {
                    return Ok(out);
                }
                for k in 0..cuts {
                    let p = a.lerp(*b, k as f64 / cuts as f64);
                    let q = camera.project(place(p).0);
                    if !q.x.is_finite() || !q.y.is_finite() {
                        return Err("The 3D projection is not finite".into());
                    }
                    projected.push(q);
                }
            }
            contours.push(projected);
        }
        out.push(Mapped { contours, shade: camera.shade(normal) });
    }
    Ok(out)
}

impl Facet {
    /// About how thick the facet is across: twice its area over its perimeter (a w × l rectangle
    /// long against its width gives about w).
    pub fn thickness(&self) -> f64 {
        let area: f64 = self.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs();
        let perimeter: f64 =
            self.contours.iter().map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
        if perimeter > 0.0 { 2.0 * area / perimeter } else { 0.0 }
    }

    pub fn path(&self) -> PathData {
        let mut bp = BezPath::new();
        for c in &self.contours {
            let Some(first) = c.first() else { continue };
            bp.move_to(*first);
            for p in c.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        }
        PathData::from_bezpath(&bp)
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Extrude {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 50.0, 0.0, 2000.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            bevel: self.bevel.map(|b| Bevel {
                width: finite(b.width, 4.0, 0.0, 1000.0),
                height: finite(b.height, 4.0, 0.0, 1000.0),
                repeat: b.repeat.clamp(1, 32),
                space: finite(b.space, 0.0, 0.0, 0.9),
                ..b
            }),
            ..self
        }
    }
}

impl Inflate {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 0.0, 0.0, 2000.0),
            volume: finite(self.volume, 50.0, 0.0, 100.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            ..self
        }
    }
}

/// Outline points Extrude & Bevel and Inflate sample at most: every point costs walls, bevel
/// bands and distance tests.
const MAX_SOLID_POINTS: usize = 2048;
/// Outline points Rotate samples at most: it only turns the outline, so an outlined paragraph fits.
const MAX_PLANE_POINTS: usize = 65536;
/// The most pieces one curve of the finely flattened outline Inflate's normals follow is cut into.
const FINE_PIECES: u32 = 512;

/// How many straight pieces follow `c` within `tolerance` (Wang's bound on a cubic's distance from
/// its chords), 1 to `most`.
fn pieces(c: vectorcraft_geom::CubicBez, tolerance: f64, most: u32) -> u32 {
    let d = |a: Point, b: Point, c: Point| (a.to_vec2() - 2.0 * b.to_vec2() + c.to_vec2()).hypot();
    let bend = d(c.p0, c.p1, c.p2).max(d(c.p1, c.p2, c.p3));
    let n = (0.75 * bend / tolerance).sqrt().ceil();
    if n.is_finite() { (n as u32).clamp(1, most) } else { most }
}

/// The rings of a shape: closed polygons with the filled side on their left, (-dy, dx): outer
/// outlines run one way and holes the other, so nonzero winding fills them as the art is. Curves
/// are followed within a tolerance of 1/2000 of `size` (at least 0.05 pt), so small curves take
/// few points and large ones more; more than `limit` points in all is refused.
fn rings(path: &PathData, size: f64, limit: usize) -> Result<Vec<Vec<Point>>, String> {
    rings_within(path, (size / 2000.0).max(0.05), 32, limit)
}

/// [`rings`], curves flattened to within `tolerance`, each in at most `most` pieces.
fn rings_within(path: &PathData, tolerance: f64, most: u32, limit: usize) -> Result<Vec<Vec<Point>>, String> {
    let mut count = 0;
    let mut out = vec![];
    for sp in &path.subpaths {
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        if segments > limit {
            return Err("The shape is too complex for a 3D effect".into());
        }
        let mut ring: Vec<Point> = vec![];
        if let Some(first) = sp.anchors.first() {
            ring.push(first.p);
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let samples = if sp.segment_is_line(i) { 1 } else { pieces(curve, tolerance, most) };
            count += samples as usize;
            if count > limit {
                return Err("The shape is too complex for a 3D effect".into());
            }
            for k in 1..=samples {
                ring.push(curve.eval(f64::from(k) / f64::from(samples)));
            }
        }
        if ring.iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e8 || p.y.abs() > 1e8) {
            return Err("The shape contains invalid coordinates".into());
        }
        ring.dedup_by(|a, b| a.distance(*b) < 1e-9);
        while ring.len() > 1 && ring.first().zip(ring.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-9) {
            ring.pop();
        }
        if ring.len() >= 3 && signed_area(&ring).abs() > 1e-9 {
            out.push(ring);
        }
    }
    // A ring inside an odd number of others is a hole.
    let holes: Vec<bool> = out
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let p = r.first().copied().unwrap_or_default();
            out.iter().enumerate().filter(|(j, o)| *j != i && contains(o, p)).count() % 2 == 1
        })
        .collect();
    for (r, hole) in out.iter_mut().zip(holes) {
        if (signed_area(r) > 0.0) == hole {
            r.reverse();
        }
    }
    Ok(out)
}

fn signed_area(r: &[Point]) -> f64 {
    r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

fn contains(r: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (a, b) in r.iter().zip(r.iter().cycle().skip(1)).take(r.len()) {
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// The unit normal on the left of `a`→`b`: towards the filled side.
fn left(a: Point, b: Point) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = dx.hypot(dy);
    if l < 1e-12 { (0.0, 0.0) } else { (-dy / l, dx / l) }
}

/// `ring` moved `w` points towards its filled side (negative: away from it), each corner along
/// its bisector, its mitre limited to four times `w` so sharp corners don't shoot off. Each point
/// stays the partner of its source point, so rings at different insets join face to face.
fn inset(ring: &[Point], w: f64) -> Vec<Point> {
    if w == 0.0 {
        return ring.to_vec();
    }
    let n = ring.len();
    (0..n)
        .map(|i| {
            let p = ring[i];
            let prev = ring[(i + n - 1) % n];
            let next = ring[(i + 1) % n];
            let (ax, ay) = left(prev, p);
            let (bx, by) = left(p, next);
            let (mut mx, mut my) = (ax + bx, ay + by);
            let ml = mx.hypot(my);
            if ml < 1e-9 {
                (mx, my) = (ax, ay);
            } else {
                (mx, my) = (mx / ml, my / ml);
            }
            let cos = (mx * ax + my * ay).max(0.25);
            Point::new(p.x + mx * w / cos, p.y + my * w / cos)
        })
        .collect()
}

/// Turns 3D points into document points and shades faces for one view.
struct Camera {
    view: Revolve,
    origin: Point,
    distance: f64,
    light: V3,
    rig: lighting::Rig,
}

impl Camera {
    fn new(view: View, bounds: Rect, depth: f64) -> Self {
        let view = view.revolve();
        let [lx, ly, lz] = view.light_direction();
        let size = (bounds.width() / 2.0).max(bounds.height() / 2.0).max(depth / 2.0).max(1.0);
        Self { view, origin: bounds.center(), distance: 4.0 * size, light: V3(lx, ly, lz), rig: view.rig() }
    }
    fn rotate(&self, v: V3) -> V3 {
        self.view.rotate(v)
    }
    /// A point of the art, `z` points in front of its plane, in view space.
    fn place(&self, p: Point, z: f64) -> V3 {
        self.rotate(V3(p.x - self.origin.x, p.y - self.origin.y, z))
    }
    fn project(&self, v: V3) -> Point {
        let f = self.distance / (self.distance - v.2 * self.view.perspective);
        Point::new(self.origin.x + v.0 * f, self.origin.y + v.1 * f)
    }
    /// The cosine between a face's outward unit normal `n` and the direction from `at` (both in
    /// view space) to the eye: positive when the face is turned towards the viewer.
    fn facing(&self, at: V3, n: V3) -> f64 {
        let to_eye = if self.view.perspective > 0.0 { V3(0.0, 0.0, self.distance / self.view.perspective).sub(at).unit() } else { V3(0.0, 0.0, 1.0) };
        n.dot(to_eye)
    }
    fn shade(&self, n: V3) -> Shade {
        self.rig.shade(n)
    }
}

/// A face to shade smoothly: its view-space contours, its normal, and its exact vertex normals if
/// known.
type SmoothFace = (Vec<Vec<V3>>, V3, Option<Vec<V3>>);

/// Facets being collected for one solid.
struct Builder<'a> {
    camera: &'a Camera,
    /// A closed solid hides the faces turned away from the viewer; an open one shows their
    /// insides.
    closed: bool,
    facets: Vec<Facet>,
    /// Every face's contours (view space) when a shadow is wanted, front-facing or not.
    casters: Vec<Vec<Vec<V3>>>,
    /// With smooth shading, each facet's view-space contours, shading normal and exact vertex
    /// normals if known, and the hidden faces (contours, outward normal).
    smooth: Vec<SmoothFace>,
    hidden: Vec<(Vec<Vec<V3>>, V3)>,
}

impl Builder<'_> {
    /// Add a face of view-space contours with outward normal `n` (view space).
    fn face(&mut self, contours: Vec<Vec<V3>>, n: V3) -> Result<(), String> {
        self.face_with(contours, n, None)
    }

    /// [`Self::face`], with the surface's exact outward normal at each vertex (view space, in
    /// the contours' order) for smooth shading.
    fn face_with(&mut self, contours: Vec<Vec<V3>>, n: V3, given: Option<Vec<V3>>) -> Result<(), String> {
        let count: usize = contours.iter().map(Vec::len).sum();
        if count < 3 {
            return Ok(());
        }
        let centre = contours.iter().flatten().fold(V3::default(), |s, v| V3(s.0 + v.0, s.1 + v.1, s.2 + v.2));
        let centre = V3(centre.0 / count as f64, centre.1 / count as f64, centre.2 / count as f64);
        let n = n.unit();
        if self.camera.view.shadow != ShadowPosition::None {
            self.casters.push(contours.clone());
        }
        let facing = self.camera.facing(centre, n);
        // A closed solid hides the faces turned away. One seen almost exactly edge-on is kept but
        // drawn first, under everything: on a curved surface rounding alone turns such faces
        // either way, and dropping every other one would leave gaps.
        let (n, behind, given) = if facing > 0.0 {
            (n, false, given)
        } else if self.closed && facing > -0.02 {
            (n, true, given)
        } else if self.closed {
            if self.camera.rig.smooth() {
                self.hidden.push((contours, n));
            }
            return Ok(());
        } else {
            // The inside of an open surface, seen from behind.
            (V3(-n.0, -n.1, -n.2), false, given.map(|g| g.into_iter().map(|v| V3(-v.0, -v.1, -v.2)).collect()))
        };
        let projected: Vec<Vec<Point>> = contours.iter().map(|c| c.iter().map(|v| self.camera.project(*v)).collect()).collect();
        if projected.iter().flatten().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            return Err("The 3D projection is not finite".into());
        }
        if projected.iter().map(|c| signed_area(c).abs()).sum::<f64>() < 1e-10 {
            return Ok(());
        }
        let depth = if behind { centre.2 - 4.0 * self.camera.distance } else { centre.2 };
        let shade = self.camera.shade(n);
        if self.camera.rig.smooth() {
            self.smooth.push((contours, n, given));
        }
        self.facets.push(Facet { contours: projected, depth, brightness: mean(shade.tint).min(1.0), tint: shade.tint, highlight: shade.highlight });
        if self.facets.len() > MAX_FACES {
            return Err("The 3D surface is too complex; simplify the shape".into());
        }
        Ok(())
    }

    /// The band between two partner rings (same length), as one quad per edge. `outward` is the
    /// direction (before rotation) the band faces away from the solid, for orienting normals.
    fn band(&mut self, a: &[V3], b: &[V3], outward: impl Fn(usize) -> V3) -> Result<(), String> {
        let n = a.len().min(b.len());
        for i in 0..n {
            let j = (i + 1) % n;
            let quad = vec![a[i], a[j], b[j], b[i]];
            // A twist bends the strip between two slices into a saddle: its normal is the whole
            // quad's average (Newell's), not one corner's triangle, which leans far to one side.
            let mut normal = newell(std::slice::from_ref(&quad));
            if normal.dot(normal) < 1e-18 {
                continue;
            }
            if normal.dot(self.camera.rotate(outward(i))) < 0.0 {
                normal = V3(-normal.0, -normal.1, -normal.2);
            }
            self.face(vec![quad], normal)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Shaded {
        let camera = self.camera;
        let mut shading = None;
        if camera.rig.smooth() && self.smooth.len() == self.facets.len() {
            // The lights and the eye: six half extents from the centre.
            let reach = 1.5 * camera.distance;
            let sources: Vec<Source> =
                self.smooth.iter().map(|(contours, normal, given)| Source { contours, normal: *normal, given: given.as_deref() }).collect();
            if let Some(smoothed) = smooth::shade(&sources, &self.hidden, &camera.rig, reach) {
                let mut polys = Vec::with_capacity(smoothed.len());
                for ((f, (contours, normal, _)), s) in self.facets.iter_mut().zip(&self.smooth).zip(smoothed) {
                    f.tint = s.shade.tint;
                    f.highlight = s.shade.highlight;
                    f.brightness = mean(s.shade.tint).min(1.0);
                    polys.push(Poly {
                        projected: f.contours.clone(),
                        normals: s.normals,
                        positions: contours.iter().flatten().copied().collect(),
                        normal: *normal,
                    });
                }
                shading = Some(Shading::new(camera.rig.clone(), reach, polys));
            }
        }
        self.facets.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let view = &camera.view;
        let shadow = lighting::cast(&self.casters, camera.light, view.shadow_direction, view.shadow, view.shadow_distance, |v| camera.project(v));
        let contact = lighting::contact(&self.casters, camera.view.shadow, camera.view.shadow_distance, |v| camera.project(v));
        Shaded { facets: self.facets, shadow, shading, contact, mapped: vec![] }
    }
}

/// The area-weighted normal of (possibly non-planar) contours, by Newell's method: for contours
/// winding with their filled side on the left as seen from in front, it points to the front.
pub(crate) fn newell(contours: &[Vec<V3>]) -> V3 {
    let mut n = V3::default();
    for c in contours {
        for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
            let x = a.cross(*b);
            n = V3(n.0 + x.0, n.1 + x.1, n.2 + x.2);
        }
    }
    n
}

fn check_bounds(b: Rect) -> Result<(), String> {
    if [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
        Ok(())
    } else {
        Err("3D effects require finite coordinates within 100 million points".into())
    }
}

/// The art's outward direction at edge `i` of `ring` (before rotation), tilted towards the face
/// it borders by `z` (+1 front, -1 back, 0 a side wall).
fn outward(ring: &[Point], i: usize, z: f64) -> V3 {
    let n = ring.len();
    let (lx, ly) = left(ring[i], ring[(i + 1) % n]);
    V3(-lx, -ly, z)
}

/// The cross-section of a side: each ring of the shape at depth fraction `t` (0 front, 1 back),
/// turned by the twist and scaled by the taper about the centre of `bounds`.
fn section(ring: &[Point], bounds: Rect, o: &Extrude, t: f64) -> Vec<Point> {
    let turn = Turn::new(bounds, o.twist, o.taper, t);
    ring.iter().map(|p| turn.apply(*p)).collect()
}

/// The twist and taper at depth fraction `t` (0 front, 1 back): a turn by `twist · t` and a
/// scale from 1 to `taper`, about the centre of the bounds.
#[derive(Clone, Copy)]
struct Turn {
    centre: Point,
    sin: f64,
    cos: f64,
    scale: f64,
}

impl Turn {
    fn new(bounds: Rect, twist: f64, taper: f64, t: f64) -> Self {
        let (sin, cos) = (twist * t).to_radians().sin_cos();
        Self { centre: bounds.center(), sin, cos, scale: 1.0 + (taper - 1.0) * t }
    }
    fn apply(&self, p: Point) -> Point {
        let (x, y) = ((p.x - self.centre.x) * self.scale, (p.y - self.centre.y) * self.scale);
        Point::new(self.centre.x + x * self.cos - y * self.sin, self.centre.y + x * self.sin + y * self.cos)
    }
}

/// One step of an extrusion's outline: how far it is inset (negative: outset) and its depth
/// fraction (0 front, 1 back), with the face it borders (+1 front bevel, -1 back bevel, 0 wall).
#[derive(Clone, Copy)]
struct Step {
    inset: f64,
    t: f64,
    face: f64,
}

/// The outline's steps from the front cap's edge to the back cap's edge.
fn steps(o: &Extrude) -> Vec<Step> {
    let d = o.depth.max(1e-6);
    let mut out = vec![];
    let bevel = o.bevel.filter(|b| b.width > 0.0 || b.height > 0.0);
    // A bevel can't be deeper than the extrusion (half of it on each side with both sides).
    let sides = if bevel.is_some_and(|b| b.both_sides) { 2.0 } else { 1.0 };
    let height = bevel.map_or(0.0, |b| b.height.min(o.depth / sides));
    // The bevel's cross-section across the width: (share across, share down).
    let profile = |b: &Bevel| -> Vec<(f64, f64)> {
        let mut pts = vec![(0.0, 0.0)];
        let n = b.repeat.max(1) as f64;
        let run = (1.0 - b.space) / n;
        let gap = if b.repeat > 1 { b.space / (n - 1.0) } else { 0.0 };
        for k in 0..b.repeat.max(1) {
            let start = k as f64 * (run + gap);
            for s in b.shape.samples().into_iter().skip(1) {
                let (x, y) = b.shape.at(s);
                pts.push((start + x * run, (k as f64 + y) / n));
            }
            if b.repeat > 1 && k + 1 < b.repeat {
                pts.push((start + run + gap, (k as f64 + 1.0) / n));
            }
        }
        if let Some(last) = pts.last_mut() {
            last.0 = 1.0;
        }
        pts
    };
    // With the bevel inside, the cap is inset by its width and the wall is the outline; outside,
    // the cap is the outline and the wall is outset by the width.
    let wall = match bevel {
        Some(b) if !b.inside => -b.width,
        _ => 0.0,
    };
    if let Some(b) = bevel {
        for (x, y) in profile(&b) {
            let inset = if b.inside { b.width * (1.0 - x) } else { -b.width * x };
            out.push(Step { inset, t: y * height / d, face: 1.0 });
        }
    } else {
        out.push(Step { inset: 0.0, t: 0.0, face: 1.0 });
    }
    // Twist and taper bend the walls: cut them into slices.
    let slices = if o.twist != 0.0 || o.taper != 1.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
    let back_height = if bevel.is_some_and(|b| b.both_sides) { height } else { 0.0 };
    let (t0, t1) = (height / d, 1.0 - back_height / d);
    for k in 0..=slices {
        let t = t0 + (t1 - t0) * k as f64 / slices as f64;
        out.push(Step { inset: wall, t, face: 0.0 });
    }
    if let Some(b) = bevel.filter(|b| b.both_sides) {
        for (x, y) in profile(&b).into_iter().rev() {
            let inset = if b.inside { b.width * (1.0 - x) } else { -b.width * x };
            out.push(Step { inset, t: 1.0 - y * height / d, face: -1.0 });
        }
    }
    out.dedup_by(|a, b| (a.inset - b.inset).abs() < 1e-9 && (a.t - b.t).abs() < 1e-12);
    out
}

/// Extrude & Bevel: the shape's outline extruded backwards, with optional twist, taper, bevels
/// and caps.
pub fn extrude(path: &PathData, bounds: Rect, options: Extrude, view: View) -> Result<Vec<Facet>, String> {
    extrude_lit(path, bounds, options, view).map(|s| s.facets)
}

/// [`extrude`], with the shadow the solid casts.
pub fn extrude_lit(path: &PathData, bounds: Rect, options: Extrude, view: View) -> Result<Shaded, String> {
    extrude_mapped(path, bounds, options, view, &[])
}

/// [`extrude_lit`], with `art` (pieces of rings in the art's coordinates) laid on the front.
pub fn extrude_mapped(path: &PathData, bounds: Rect, options: Extrude, view: View, art: &[Vec<Vec<Point>>]) -> Result<Shaded, String> {
    check_bounds(bounds)?;
    let o = options.sanitized();
    // No depth: the flat shape, seen from either side.
    if o.depth <= 0.0 {
        return plane_mapped(path, bounds, view, art);
    }
    let rings = rings(path, bounds.width().max(bounds.height()), MAX_SOLID_POINTS)?;
    let camera = Camera::new(view, bounds, o.depth);
    let mut b = Builder { camera: &camera, closed: o.cap, facets: vec![], casters: vec![], smooth: vec![], hidden: vec![] };
    let steps = steps(&o);
    let z = |t: f64| o.depth / 2.0 - o.depth * t;
    let ring_at =
        |ring: &[Point], s: &Step| -> Vec<V3> { section(&inset(ring, s.inset), bounds, &o, s.t).iter().map(|p| camera.place(*p, z(s.t))).collect() };
    for ring in &rings {
        let placed: Vec<Vec<V3>> = steps.iter().map(|s| ring_at(ring, s)).collect();
        for (k, pair) in placed.windows(2).enumerate() {
            let ([a, c], Some(s0), Some(s1)) = (pair, steps.get(k), steps.get(k + 1)) else { continue };
            // A band between two front bevel steps borders the front, one reaching a back bevel
            // step the back; the rest are walls.
            let face = if s1.face < 0.0 {
                -1.0
            } else if s0.face > 0.0 && s1.face > 0.0 {
                1.0
            } else {
                0.0
            };
            b.band(a, c, |i| outward(ring, i, face))?;
        }
    }
    if o.cap {
        for (s, n) in [(steps.first(), 1.0), (steps.last(), -1.0)] {
            let Some(s) = s else { continue };
            b.face(rings.iter().map(|r| ring_at(r, s)).collect(), camera.rotate(V3(0.0, 0.0, n)))?;
        }
    }
    let front = camera.rotate(V3(0.0, 0.0, 1.0));
    let mapped = map_art(art, bounds.width().max(bounds.height()), &camera, |p| (camera.place(p, z(0.0)), front))?;
    Ok(Shaded { mapped, ..b.finish() })
}

/// Rotate: the flat art turned in 3D about the centre of `bounds` (which may be larger than the
/// shape's own, so that every part of one object turns about the same point).
pub fn plane(path: &PathData, bounds: Rect, view: View) -> Result<Vec<Facet>, String> {
    plane_lit(path, bounds, view).map(|s| s.facets)
}

/// [`plane`], with the shadow the turned art casts.
pub fn plane_lit(path: &PathData, bounds: Rect, view: View) -> Result<Shaded, String> {
    plane_mapped(path, bounds, view, &[])
}

/// [`plane_lit`], with `art` (pieces of rings in the art's coordinates) laid on the art's front.
pub fn plane_mapped(path: &PathData, bounds: Rect, view: View, art: &[Vec<Vec<Point>>]) -> Result<Shaded, String> {
    check_bounds(bounds)?;
    let rings = rings(path, bounds.width().max(bounds.height()), MAX_PLANE_POINTS)?;
    let camera = Camera::new(view, bounds, 0.0);
    let mut b = Builder { camera: &camera, closed: false, facets: vec![], casters: vec![], smooth: vec![], hidden: vec![] };
    let contours: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| camera.place(*p, 0.0)).collect()).collect();
    let front = camera.rotate(V3(0.0, 0.0, 1.0));
    b.face(contours, front)?;
    let mapped = map_art(art, bounds.width().max(bounds.height()), &camera, |p| (camera.place(p, 0.0), front))?;
    Ok(Shaded { mapped, ..b.finish() })
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-18 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
    p.distance(Point::new(a.x + dx * t, a.y + dy * t))
}

/// The distance from `p` to the nearest edge of `rings`, edge by edge.
#[cfg(test)]
fn edge_distance(rings: &[Vec<Point>], p: Point) -> f64 {
    rings
        .iter()
        .flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()))
        .map(|(a, b)| segment_distance(p, *a, *b))
        .fold(f64::INFINITY, f64::min)
}

/// The edges of `rings` filed in a grid of square buckets, so the nearest edge to a point is
/// found among the buckets around it rather than among all the edges.
struct Edges {
    edges: Vec<(Point, Point)>,
    origin: Point,
    size: f64,
    n: usize,
    buckets: Vec<Vec<u32>>,
}

impl Edges {
    fn new(rings: &[Vec<Point>]) -> Self {
        const N: usize = 64;
        let edges: Vec<(Point, Point)> =
            rings.iter().flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len()).map(|(a, b)| (*a, *b))).collect();
        let bounds = edges.iter().fold(None, |b: Option<Rect>, (a, c)| Some(b.map_or(Rect::from_points(*a, *c), |b| b.union_pt(*a).union_pt(*c))));
        let bounds = bounds.unwrap_or(Rect::ZERO);
        let size = (bounds.width().max(bounds.height()) / N as f64).max(1e-9);
        let mut buckets = vec![vec![]; N * N];
        let index = |v: f64, o: f64| (((v - o) / size).floor().max(0.0) as usize).min(N - 1);
        for (k, (a, b)) in edges.iter().enumerate() {
            let (i0, i1) = (index(a.x.min(b.x), bounds.x0), index(a.x.max(b.x), bounds.x0));
            let (j0, j1) = (index(a.y.min(b.y), bounds.y0), index(a.y.max(b.y), bounds.y0));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    buckets[j * N + i].push(k as u32);
                }
            }
        }
        Self { edges, origin: bounds.origin(), size, n: N, buckets }
    }

    /// The bucket row or column of coordinate `v`, from the grid's origin `o` on that axis.
    fn cell(&self, v: f64, o: f64) -> isize {
        (((v - o) / self.size).floor() as isize).clamp(0, self.n as isize - 1)
    }

    /// Is `p` within `tolerance` of an edge? Only the buckets `p` could share an edge with are
    /// looked at.
    fn near(&self, p: Point, tolerance: f64) -> bool {
        self.touch(Rect::new(p.x - tolerance, p.y - tolerance, p.x + tolerance, p.y + tolerance)) && {
            let n = self.n as isize;
            let (i0, i1) = (self.cell(p.x - tolerance, self.origin.x), self.cell(p.x + tolerance, self.origin.x));
            let (j0, j1) = (self.cell(p.y - tolerance, self.origin.y), self.cell(p.y + tolerance, self.origin.y));
            (j0..=j1).any(|j| {
                (i0..=i1).any(|i| {
                    self.buckets[(j * n + i) as usize]
                        .iter()
                        .filter_map(|k| self.edges.get(*k as usize))
                        .any(|(a, b)| segment_distance(p, *a, *b) <= tolerance)
                })
            })
        }
    }

    /// Call `f` with each edge whose bounding box overlaps `r` (an edge in several of the
    /// buckets `r` covers comes more than once).
    fn each(&self, r: Rect, mut f: impl FnMut(Point, Point)) {
        let n = self.n as isize;
        for j in self.cell(r.y0, self.origin.y)..=self.cell(r.y1, self.origin.y) {
            for i in self.cell(r.x0, self.origin.x)..=self.cell(r.x1, self.origin.x) {
                for (a, b) in self.buckets[(j * n + i) as usize].iter().filter_map(|k| self.edges.get(*k as usize)) {
                    if a.x.min(b.x) <= r.x1 && a.x.max(b.x) >= r.x0 && a.y.min(b.y) <= r.y1 && a.y.max(b.y) >= r.y0 {
                        f(*a, *b);
                    }
                }
            }
        }
    }

    /// Does any edge pass through `r` (its bounding box, at least, overlapping it)?
    fn touch(&self, r: Rect) -> bool {
        let n = self.n as isize;
        let (i0, i1) = (self.cell(r.x0, self.origin.x), self.cell(r.x1, self.origin.x));
        let (j0, j1) = (self.cell(r.y0, self.origin.y), self.cell(r.y1, self.origin.y));
        (j0..=j1).any(|j| {
            (i0..=i1).any(|i| {
                self.buckets[(j * n + i) as usize]
                    .iter()
                    .filter_map(|k| self.edges.get(*k as usize))
                    .any(|(a, b)| a.x.min(b.x) <= r.x1 && a.x.max(b.x) >= r.x0 && a.y.min(b.y) <= r.y1 && a.y.max(b.y) >= r.y0)
            })
        })
    }

    /// The distance from `p` to the nearest edge: the buckets are searched in growing squares
    /// around `p`'s, until no unsearched bucket could hold a nearer edge.
    fn distance(&self, p: Point) -> f64 {
        let (fx, fy) = ((p.x - self.origin.x) / self.size, (p.y - self.origin.y) / self.size);
        if !fx.is_finite() || !fy.is_finite() {
            return f64::INFINITY;
        }
        let n = self.n as isize;
        let (ci, cj) = ((fx.floor() as isize).clamp(0, n - 1), (fy.floor() as isize).clamp(0, n - 1));
        // How far `p` is outside the grid: the nearest point of the grid to it lies in `p`'s bucket.
        let outside = (fx.clamp(0.0, n as f64) - fx).hypot(fy.clamp(0.0, n as f64) - fy) * self.size;
        let mut best = f64::INFINITY;
        for r in 0..n {
            for j in (cj - r)..=(cj + r) {
                for i in (ci - r)..=(ci + r) {
                    if (j - cj).abs() != r && (i - ci).abs() != r || i < 0 || j < 0 || i >= n || j >= n {
                        continue;
                    }
                    for k in &self.buckets[(j * n + i) as usize] {
                        if let Some((a, b)) = self.edges.get(*k as usize) {
                            best = best.min(segment_distance(p, *a, *b));
                        }
                    }
                }
            }
            // Buckets r + 1 away are at least r buckets from any point of `p`'s own, and that
            // far across the grid from a `p` outside it (at right angles to its way in, at worst).
            if best <= outside.hypot(r as f64 * self.size) {
                break;
            }
        }
        best
    }
}

/// Is `p` filled by `rings` (nonzero winding, as they are oriented)?
fn filled(rings: &[Vec<Point>], p: Point) -> bool {
    rings.iter().filter(|r| contains(r, p)).count() % 2 == 1
}

/// `ring` cut to the rectangle `cell` (Sutherland–Hodgman: the rectangle is convex, the ring
/// needn't be).
fn clip(ring: &[Point], cell: Rect) -> Vec<Point> {
    let mut pts = ring.to_vec();
    let sides: [fn(Point, Rect) -> f64; 4] = [|p, c| p.x - c.x0, |p, c| c.x1 - p.x, |p, c| p.y - c.y0, |p, c| c.y1 - p.y];
    for side in sides {
        if pts.is_empty() {
            break;
        }
        let mut out = vec![];
        for (a, b) in pts.iter().zip(pts.iter().cycle().skip(1)).take(pts.len()) {
            let (da, db) = (side(*a, cell), side(*b, cell));
            if da >= 0.0 {
                out.push(*a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(a.lerp(*b, da / (da - db)));
            }
        }
        pts = out;
    }
    pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
    if pts.len() >= 3 && signed_area(&pts).abs() > 1e-12 { pts } else { vec![] }
}

/// Do any two edges of `rings` cross, touch or overlap, other than neighbours meeting at their
/// shared corner? Rings that don't outline a shape [`cut_cell`] can follow exactly.
fn rings_cross(rings: &[Vec<Point>], edges: &Edges) -> bool {
    let orient = |a: Point, b: Point, c: Point| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    // Is `p`, on the line through `a` and `b`, within the segment?
    let within = |a: Point, b: Point, p: Point| p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y);
    let meet = |a: Point, b: Point, c: Point, d: Point| -> bool {
        let shared = [(a, c), (a, d), (b, c), (b, d)].iter().filter(|(p, q)| p == q).count();
        match shared {
            0 => {
                let (o1, o2, o3, o4) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
                (o1 * o2 < 0.0 && o3 * o4 < 0.0)
                    || (o1 == 0.0 && within(a, b, c))
                    || (o2 == 0.0 && within(a, b, d))
                    || (o3 == 0.0 && within(c, d, a))
                    || (o4 == 0.0 && within(c, d, b))
            }
            1 => {
                // Neighbours: they overlap only if they run back along each other.
                let corner = if a == c || a == d { a } else { b };
                let mine = if corner == a { b } else { a };
                let other = if corner == c { d } else { c };
                let along = (other.x - corner.x) * (mine.x - corner.x) + (other.y - corner.y) * (mine.y - corner.y);
                orient(corner, mine, other) == 0.0 && along > 0.0
            }
            // The same edge, or one run back over.
            _ => !(a == c && b == d),
        }
    };
    for (a, b) in rings.iter().flat_map(|r| r.iter().zip(r.iter().cycle().skip(1)).take(r.len())) {
        let mut crossed = false;
        edges.each(Rect::from_points(*a, *b), |c, d| crossed = crossed || meet(*a, *b, c, d));
        if crossed {
            return true;
        }
    }
    false
}

/// The part of a shape inside the rectangle `cell`, as contours (holes running the other way),
/// for `rings` that don't cross ([`rings_cross`]). Unlike [`clip`] ring by ring, the pieces of
/// the outline inside the cell are joined along the cell's sides as the shape is: a cell the
/// outline leaves and comes back into is cut into separate parts, not one part tied to the
/// other along the side. Where the outline crosses a side, the point depends only on the
/// outline's edge and the side's line, so the cells either side cut it at exactly the same point.
fn cut_cell(rings: &[Vec<Point>], cell: Rect) -> Vec<Vec<Point>> {
    let (w, h) = (cell.width(), cell.height());
    if !(w > 0.0 && h > 0.0) {
        return vec![];
    }
    let perimeter = 2.0 * (w + h);
    let inside = |p: Point| p.x >= cell.x0 && p.x <= cell.x1 && p.y >= cell.y0 && p.y <= cell.y1;
    let on_x = |a: Point, b: Point, x: f64| Point::new(x, a.y + (x - a.x) / (b.x - a.x) * (b.y - a.y));
    let on_y = |a: Point, b: Point, y: f64| Point::new(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x), y);
    // The part of `a`→`b` inside the cell, as its ends' parameters and points (Liang–Barsky); an
    // end on a side is computed from that side's line.
    let span = |a: Point, b: Point| -> Option<(f64, Point, f64, Point)> {
        let (mut t0, mut p0, mut t1, mut p1) = (0.0, a, 1.0, b);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let mut side = |d: f64, t: f64, at: Point| {
            if d > 0.0 && t > t0 {
                (t0, p0) = (t, at);
            } else if d < 0.0 && t < t1 {
                (t1, p1) = (t, at);
            }
        };
        if dx == 0.0 {
            if a.x < cell.x0 || a.x > cell.x1 {
                return None;
            }
        } else {
            side(dx, (cell.x0 - a.x) / dx, on_x(a, b, cell.x0));
            side(-dx, (cell.x1 - a.x) / dx, on_x(a, b, cell.x1));
        }
        if dy == 0.0 {
            if a.y < cell.y0 || a.y > cell.y1 {
                return None;
            }
        } else {
            side(dy, (cell.y0 - a.y) / dy, on_y(a, b, cell.y0));
            side(-dy, (cell.y1 - a.y) / dy, on_y(a, b, cell.y1));
        }
        (t0 < t1).then_some((t0, p0, t1, p1))
    };
    // How far round the cell's border a point on it is, anticlockwise (the way the cell's own
    // outline runs, filled on its left) from its first corner.
    let around = |p: Point| -> f64 {
        let d = [(p.y - cell.y0).abs(), (p.x - cell.x1).abs(), (p.y - cell.y1).abs(), (p.x - cell.x0).abs()];
        let nearest = (0..4).min_by(|i, j| d[*i].total_cmp(&d[*j])).unwrap_or(0);
        match nearest {
            0 => (p.x - cell.x0).clamp(0.0, w),
            1 => w + (p.y - cell.y0).clamp(0.0, h),
            2 => w + h + (cell.x1 - p.x).clamp(0.0, w),
            _ => (2.0 * w + h + (cell.y1 - p.y).clamp(0.0, h)) % perimeter,
        }
    };
    let corners = [
        (0.0, Point::new(cell.x0, cell.y0)),
        (w, Point::new(cell.x1, cell.y0)),
        (w + h, Point::new(cell.x1, cell.y1)),
        (2.0 * w + h, Point::new(cell.x0, cell.y1)),
    ];
    // The outline's runs through the cell, each from where it comes in to where it leaves, and
    // the rings wholly inside the cell.
    let mut runs: Vec<Vec<Point>> = vec![];
    let mut whole: Vec<Vec<Point>> = vec![];
    let mut beyond: Vec<&Vec<Point>> = vec![];
    for ring in rings {
        let n = ring.len();
        let Some(first) = ring.iter().position(|p| !inside(*p)) else {
            whole.push(ring.clone());
            continue;
        };
        let mut run: Option<Vec<Point>> = None;
        let mut touched = false;
        for k in 0..n {
            let (Some(a), Some(b)) = (ring.get((first + k) % n), ring.get((first + k + 1) % n)) else { continue };
            match span(*a, *b) {
                None => {
                    // Out of the cell from `a` on: a run that reached `a` ends there.
                    if let Some(r) = run.take() {
                        runs.push(r);
                    }
                }
                Some((t0, p0, t1, p1)) => {
                    touched = true;
                    if t0 > 0.0 {
                        if let Some(r) = run.take() {
                            runs.push(r);
                        }
                        run = Some(vec![p0]);
                    }
                    let r = run.get_or_insert_with(|| vec![p0]);
                    if t1 < 1.0 {
                        r.push(p1);
                        runs.extend(run.take());
                    } else {
                        r.push(*b);
                    }
                }
            }
        }
        runs.extend(run.take());
        if !touched {
            beyond.push(ring);
        }
    }
    runs.retain(|r| r.len() >= 2);
    let mut out: Vec<Vec<Point>> = vec![];
    if runs.is_empty() {
        // No outline crosses the cell's border, so the rings beyond it either fill all of the cell or none of it.
        let filled_here = beyond.iter().filter(|r| contains(r, cell.center())).count() % 2 == 1;
        if filled_here {
            out.push(corners.iter().map(|c| c.1).collect());
        }
    } else {
        // Join the runs: from where one leaves, along the border the way the cell runs (keeping
        // the shape on the left) to where the next comes in.
        let ends: Vec<(f64, f64)> = runs.iter().map(|r| (around(r[0]), around(r[r.len() - 1]))).collect();
        let mut used = vec![false; runs.len()];
        for start in 0..runs.len() {
            if used[start] {
                continue;
            }
            let mut contour: Vec<Point> = vec![];
            let mut k = start;
            for _ in 0..=runs.len() {
                used[k] = true;
                contour.extend(runs[k].iter().copied());
                let leave = ends[k].1;
                // The next run in along the border (this one's own start closes the contour).
                let gap = |s: f64| (s - leave).rem_euclid(perimeter);
                let next = (0..runs.len()).filter(|j| !used[*j] || *j == start).min_by(|i, j| gap(ends[*i].0).total_cmp(&gap(ends[*j].0)));
                let Some(next) = next else { break };
                let reach = gap(ends[next].0);
                let mut passed: Vec<(f64, Point)> =
                    corners.iter().map(|(s, p)| ((s - leave).rem_euclid(perimeter), *p)).filter(|(g, _)| *g > 0.0 && *g < reach).collect();
                passed.sort_by(|a, b| a.0.total_cmp(&b.0));
                contour.extend(passed.into_iter().map(|(_, p)| p));
                if next == start {
                    break;
                }
                k = next;
            }
            contour.dedup_by(|a, b| a.distance(*b) < 1e-12);
            while contour.len() > 1 && contour.first().zip(contour.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-12) {
                contour.pop();
            }
            out.push(contour);
        }
    }
    out.extend(whole);
    out.retain(|c| c.len() >= 3 && signed_area(c).abs() > 1e-12 * cell.area().max(1e-12));
    out
}

/// The shape's surface as a membrane under pressure, held at zero along its outline (and its
/// holes): the solution of ∇²φ = −1 on a grid, with each node's distance to the outline along the
/// grid lines taken into account where it is less than a grid step (Shortley–Weller), so the
/// outline needn't fall on the grid. √φ rises vertically from the outline and rounds smoothly
/// inside, without the creases a distance-to-the-edge height has along a shape's middle.
struct Membrane {
    origin: Point,
    step: f64,
    nx: usize,
    ny: usize,
    /// φ at each node; outside the shape, extrapolated (negative) from the inside, so
    /// interpolation crosses zero near the outline.
    phi: Vec<f64>,
    /// The slope s of φ at the outline, as each node sees it (φ ≈ s·d − d²/2 at a distance d
    /// from the outline); outside, the slope φ was continued at.
    ratio: Vec<f64>,
    peak: f64,
}

impl Membrane {
    fn solve(rings: &[Vec<Point>], origin: Point, step: f64, nx: usize, ny: usize, guess: Option<&Membrane>) -> Self {
        let edges = Edges::new(rings);
        let node = |i: usize, j: usize| Point::new(origin.x + i as f64 * step, origin.y + j as f64 * step);
        let idx = |i: usize, j: usize| j * nx + i;
        // A node on the outline itself is held at zero with the outside.
        let inside: Vec<bool> = (0..nx * ny)
            .map(|k| {
                let p = node(k % nx, k / nx);
                filled(rings, p) && !edges.near(p, 1e-6 * step)
            })
            .collect();
        // The nearest crossing of the outline from `p` along (dx, dy), within one step.
        let crossing = |p: Point, dx: f64, dy: f64| -> f64 {
            let mut best = step;
            let reach = Rect::from_points(p, Point::new(p.x + dx * step, p.y + dy * step));
            edges.each(reach, |a, b| {
                let t = if dy == 0.0 {
                    if (a.y > p.y) == (b.y > p.y) {
                        return;
                    }
                    (a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) - p.x) * dx
                } else {
                    if (a.x > p.x) == (b.x > p.x) {
                        return;
                    }
                    (a.y + (p.x - a.x) / (b.x - a.x) * (b.y - a.y) - p.y) * dy
                };
                if t > 1e-9 * step && t < best {
                    best = t;
                }
            });
            best
        };
        // Each inside node's four neighbours: (weight, neighbour inside or None at the outline).
        let mut stencil: Vec<[(f64, Option<usize>); 4]> = vec![[(0.0, None); 4]; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let k = idx(i, j);
                if !inside[k] {
                    continue;
                }
                let p = node(i, j);
                let dirs = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
                let h: Vec<f64> = dirs.iter().map(|(dx, dy)| crossing(p, *dx, *dy)).collect();
                for (d, (dx, dy)) in dirs.iter().enumerate() {
                    let opp = h[d ^ 1];
                    let w = 2.0 / (h[d] * (h[d] + opp));
                    let (ni, nj) = (i as isize + *dx as isize, j as isize + *dy as isize);
                    let neighbour = (h[d] >= step && ni >= 0 && nj >= 0 && (ni as usize) < nx && (nj as usize) < ny)
                        .then(|| idx(ni as usize, nj as usize))
                        .filter(|n| inside[*n]);
                    stencil[k][d] = (w, neighbour);
                }
            }
        }
        // Successive over-relaxation, at the factor that converges fastest on a grid this size.
        // Started from a coarser solution where there is one: far fewer sweeps to settle.
        let mut phi: Vec<f64> =
            (0..nx * ny).map(|k| if inside[k] { guess.map_or(0.0, |g| g.at(node(k % nx, k / nx)).max(0.0)) } else { 0.0 }).collect();
        let tolerance = 1e-9 * step * step * (nx * ny) as f64;
        let omega = 2.0 / (1.0 + (std::f64::consts::PI / nx.max(ny).max(2) as f64).sin());
        for _ in 0..2000 {
            let mut change: f64 = 0.0;
            for k in 0..nx * ny {
                if !inside[k] {
                    continue;
                }
                let (mut sum, mut weight) = (1.0, 0.0);
                for (w, n) in stencil[k] {
                    weight += w;
                    sum += w * n.map_or(0.0, |n| phi[n]);
                }
                let next = phi[k] + omega * (sum / weight - phi[k]);
                change = change.max((next - phi[k]).abs());
                phi[k] = next;
            }
            if change < tolerance {
                break;
            }
        }
        let peak = phi.iter().copied().fold(0.0, f64::max);
        // Near the outline φ ≈ s·d − d²/2 at a distance d from it (∇²φ = −1 across a straight
        // edge), s its slope at the outline. Each inside node's s; a node within a step and a
        // quarter of the outline would divide its small error by a small distance, so its s is its
        // neighbours'. (Not a whole step: an edge along a grid line lies a whole number of steps
        // from the nodes, and rounding would decide which side of the line they fall.)
        // Outside nodes near the shape (three rings of them, for the cubic interpolation's reach)
        // get their inner neighbours' s too, and φ continued past the outline with it, so values
        // interpolated between them reach zero at the outline.
        // Only nodes this near the outline take part (the rim band and the cubic's reach), so only
        // their distances are found.
        let reach = (Self::RIM + 4.0) * step;
        let dist: Vec<f64> = (0..nx * ny)
            .map(|k| {
                let p = node(k % nx, k / nx);
                if edges.near(p, reach) { edges.distance(p) } else { f64::INFINITY }
            })
            .collect();
        let mut ext = phi.clone();
        let mut ratio: Vec<f64> = (0..nx * ny)
            .map(|k| if inside[k] && dist[k].is_finite() { (phi[k] + dist[k] * dist[k] / 2.0) / dist[k].max(1e-9 * step) } else { 0.0 })
            .collect();
        let mut known: Vec<bool> = (0..nx * ny).map(|k| inside[k] && dist[k] >= 1.25 * step).collect();
        // First from the nodes farther inside; then, for a shape too thin to have any
        // nearby, from every inside node.
        for round in 0..8 {
            if round == 4 {
                for (k, i) in known.iter_mut().zip(&inside) {
                    *k |= *i;
                }
            }
            let before = known.clone();
            for j in 0..ny {
                for i in 0..nx {
                    let k = idx(i, j);
                    if before[k] {
                        continue;
                    }
                    let mut slopes = vec![];
                    for (di, dj) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                        let (ni, nj) = (i as isize + di, j as isize + dj);
                        if ni < 0 || nj < 0 || ni as usize >= nx || nj as usize >= ny || !before[idx(ni as usize, nj as usize)] {
                            continue;
                        }
                        slopes.push(ratio[idx(ni as usize, nj as usize)]);
                    }
                    if !slopes.is_empty() {
                        let slope = slopes.iter().sum::<f64>() / slopes.len() as f64;
                        if !inside[k] {
                            let d = dist[k];
                            ext[k] = if d.is_finite() { -d * slope - d * d / 2.0 } else { 0.0 };
                        }
                        ratio[k] = slope;
                        known[k] = true;
                    }
                }
            }
        }
        Self { origin, step, nx, ny, phi: ext, ratio, peak }
    }

    /// How many cells in from the outline φ follows the distance to it ([`Self::at_rim`]).
    const RIM: f64 = 1.5;

    /// φ at `p`, which lies `d` from the outline. Within [`Self::RIM`] cells of it, φ follows that
    /// exact distance (s·d − d²/2): the grid alone staircases along a curved outline, and the
    /// lifted rim, steep there, would show that as ripples. Farther in, the grid is the better
    /// guide: the slope's own small errors grow with the distance.
    fn at_rim(&self, p: Point, d: f64) -> f64 {
        let grid = self.at(p);
        let band = Self::RIM * self.step;
        // A distance that isn't a number (or none: far inside) leaves the grid's value.
        if d.partial_cmp(&band) != Some(std::cmp::Ordering::Less) {
            return grid;
        }
        let t = (d / band).clamp(0.0, 1.0);
        let w = t * t * (3.0 - 2.0 * t);
        (self.interpolate(&self.ratio, p) * d - d * d.abs() / 2.0) * (1.0 - w) + grid * w
    }

    /// φ at `p`, interpolated between the four nodes around it.
    fn at(&self, p: Point) -> f64 {
        self.interpolate(&self.phi, p)
    }

    /// One of the node fields at `p`, interpolated smoothly (Catmull-Rom) between the sixteen
    /// nodes around it: its slope is continuous, so the surface lifted from it has no creases
    /// along the grid lines for a highlight to show.
    fn interpolate(&self, field: &[f64], p: Point) -> f64 {
        let fx = ((p.x - self.origin.x) / self.step).clamp(0.0, (self.nx - 1) as f64);
        let fy = ((p.y - self.origin.y) / self.step).clamp(0.0, (self.ny - 1) as f64);
        let v = |i: isize, j: isize| {
            let i = i.clamp(0, self.nx as isize - 1) as usize;
            let j = j.clamp(0, self.ny as isize - 1) as usize;
            field.get(j * self.nx + i).copied().unwrap_or(0.0)
        };
        if self.nx < 2 || self.ny < 2 {
            return v(0, 0);
        }
        let (i, j) = ((fx.floor() as isize).min(self.nx as isize - 2), (fy.floor() as isize).min(self.ny as isize - 2));
        let weights = |t: f64| {
            let (t2, t3) = (t * t, t * t * t);
            [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
        };
        let (wx, wy) = (weights(fx - i as f64), weights(fy - j as f64));
        let mut sum = 0.0;
        for (dj, wj) in wy.iter().enumerate() {
            for (di, wi) in wx.iter().enumerate() {
                sum += wi * wj * v(i + di as isize - 1, j + dj as isize - 1);
            }
        }
        sum
    }
}

/// The part of `outline` inside `cell`, as rings (holes running the other way).
fn material(outline: &PathData, cell: Rect) -> Option<Vec<Vec<Point>>> {
    let r = PathData::single(vectorcraft_geom::SubPath::polyline(
        &[Point::new(cell.x0, cell.y0), Point::new(cell.x1, cell.y0), Point::new(cell.x1, cell.y1), Point::new(cell.x0, cell.y1)],
        true,
    ));
    let rule = vectorcraft_geom::FillRule::NonZero;
    let part =
        vectorcraft_pathops::try_boolean(&r, rule, outline, rule, vectorcraft_pathops::BoolOp::Intersect, vectorcraft_pathops::DEFAULT_PRECISION)
            .ok()?;
    rings(&part, cell.width(), MAX_PLANE_POINTS).ok()
}

/// Inflate: the shape puffed out like a cushion, its edge kept where the art's outline is.
pub fn inflate(path: &PathData, bounds: Rect, options: Inflate, view: View) -> Result<Vec<Facet>, String> {
    inflate_lit(path, bounds, options, view).map(|s| s.facets)
}

/// [`inflate`], with the shadow the solid casts.
pub fn inflate_lit(path: &PathData, bounds: Rect, options: Inflate, view: View) -> Result<Shaded, String> {
    inflate_mapped(path, bounds, options, view, &[])
}

/// [`inflate_lit`], with `art` (pieces of rings in the art's coordinates) laid on the puffed front.
pub fn inflate_mapped(path: &PathData, bounds: Rect, options: Inflate, view: View, art: &[Vec<Vec<Point>>]) -> Result<Shaded, String> {
    check_bounds(bounds)?;
    let o = options.sanitized();
    let rings = rings(path, bounds.width().max(bounds.height()), MAX_SOLID_POINTS)?;
    if rings.is_empty() {
        return Ok(Shaded::default());
    }
    let size = bounds.width().max(bounds.height());
    if size <= 0.0 {
        return Ok(Shaded::default());
    }
    let cells = if o.draft { 24 } else { 48 };
    let cell = size / cells as f64;
    let (nx, ny) = (((bounds.width() / cell).ceil() as usize).max(1), ((bounds.height() / cell).ceil() as usize).max(1));
    // The membrane is solved on a grid twice as fine as the facets (a draft's as coarse): its
    // error shrinks with the square of the step, and a mirror finish shows even a slight unevenness
    // of the surface's slope.
    let fineness = if o.draft { 1 } else { 2 };
    let coarse = Membrane::solve(&rings, bounds.origin(), cell, nx + 1, ny + 1, None);
    let m = if fineness > 1 {
        Membrane::solve(&rings, bounds.origin(), cell / fineness as f64, nx * fineness + 1, ny * fineness + 1, Some(&coarse))
    } else {
        coarse
    };
    let edges = Edges::new(&rings);
    let outline = PathData::new(rings.iter().map(|r| vectorcraft_geom::SubPath::polyline(r, true)).collect());
    // A circle of radius r puffs into a half sphere of radius r at 50 % (its membrane peaks at r²/4).
    let height = 2.0 * m.peak.sqrt() * o.volume / 50.0;
    let lift = |p: Point| {
        if m.peak <= 0.0 || edges.near(p, 1e-9) {
            return 0.0;
        }
        let d = if edges.near(p, Membrane::RIM * m.step) { edges.distance(p) } else { f64::INFINITY };
        height * (m.at_rim(p, d).max(0.0) / m.peak).sqrt()
    };
    // The front's exact outward normal at `p` (art space, before the view turns it): the lifted
    // surface z = H·√φ has the normal (−H·∇φ, 2√φ), which turns flat to the side at the outline
    // (φ = 0). φ is continued past the outline (negative) for the slope there.
    let gain = if m.peak > 0.0 { height / m.peak.sqrt() } else { 0.0 };
    // Distances for the normals come from the curves flattened 64 times finer, in as many pieces
    // as that takes: the distance to a polyline turns its direction a step at each corner, and a
    // tight highlight or a mirror's reflection shows each step as a streak across the rim. (A
    // draft makes do with the outline as it is.)
    let fine_rings = (!o.draft)
        .then(|| rings_within(path, (size / 2000.0).max(0.05) / 64.0, FINE_PIECES, MAX_PLANE_POINTS).ok())
        .flatten()
        .filter(|r| !r.is_empty());
    let fine = fine_rings.as_ref().map(|r| Edges::new(r));
    let (smooth_rings, smooth_edges) = (fine_rings.as_ref().unwrap_or(&rings), fine.as_ref().unwrap_or(&edges));
    // The mesh's points lie in the shape as flattened coarsely, which is within that flattening's
    // tolerance of the fine one: a point farther than that from the fine outline is inside it,
    // with no need to test (a test runs through every edge).
    let coarse = (size / 2000.0).max(0.05);
    let e = 0.02 * cell;
    let signed = |p: Point, surely_inside: bool, rim: bool| {
        if !rim {
            return m.at(p);
        }
        let d = smooth_edges.distance(p);
        m.at_rim(p, if surely_inside || filled(smooth_rings, p) { d } else { -d })
    };
    let slope_normal = |p: Point| -> V3 {
        // Beyond the rim band φ is the grid's alone: no distances needed.
        let rim = smooth_edges.near(p, Membrane::RIM * m.step + 2.0 * e);
        let inside = !rim || smooth_edges.distance(p) > e + 2.0 * coarse;
        let signed = |q: Point| signed(q, inside, rim);
        let gx = (signed(Point::new(p.x + e, p.y)) - signed(Point::new(p.x - e, p.y))) / (2.0 * e);
        let gy = (signed(Point::new(p.x, p.y + e)) - signed(Point::new(p.x, p.y - e))) / (2.0 * e);
        let rise = if edges.near(p, 1e-9) { 0.0 } else { 2.0 * signed(p).max(0.0).sqrt() };
        let n = V3(-gain * gx, -gain * gy, rise);
        if n.dot(n) > 1e-24 && n.0.is_finite() && n.1.is_finite() { n.unit() } else { V3(0.0, 0.0, 1.0) }
    };
    let camera = Camera::new(view, bounds, o.depth + 2.0 * height);
    // Without its back cap the object is a shell open behind, its inside showing.
    let mut b = Builder { camera: &camera, closed: o.cap, facets: vec![], casters: vec![], smooth: vec![], hidden: vec![] };
    let half = o.depth / 2.0;
    // The back: turned and scaled by the twist and taper, its puff too.
    let back_turn = Turn::new(bounds, o.twist, o.taper, 1.0);
    // The grid's cells, those the surface rises steeply across (near the outline) cut into
    // halves or quarters each way, so the steep rim is drawn in facets as fine as its curve.
    // Every corner sits on one fine lattice (a quarter cell), computed the same way wherever it
    // is used, and a cell's border takes its finer neighbour's lattice points too: where cells
    // of different fineness meet, their edges follow the surface through the same points, with
    // no cracks.
    const FINE: usize = 4;
    let unit = cell / FINE as f64;
    let at = |qx: usize, qy: usize| Point::new(bounds.x0 + qx as f64 * unit, bounds.y0 + qy as f64 * unit);
    let base = |i: usize, j: usize| Rect::from_points(at(i * FINE, j * FINE), at((i + 1) * FINE, (j + 1) * FINE));
    // The shape's part of a rectangle, as rings.
    // Rings that cross each other are cut ring by ring instead, and joined by the fill rule.
    let simple = !rings_cross(&rings, &edges);
    let cut = |r: Rect| -> Vec<Vec<Point>> {
        // A cell no edge crosses is wholly inside the shape or wholly outside it.
        if !edges.touch(r) {
            let corners = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            return if filled(&rings, r.center()) { vec![corners] } else { vec![] };
        }
        if simple {
            return cut_cell(&rings, r);
        }
        let pieces: Vec<Vec<Point>> = rings.iter().map(|ring| clip(ring, r)).filter(|c| !c.is_empty()).collect();
        if pieces.iter().map(|p| signed_area(p)).sum::<f64>() <= 1e-9 * r.area() {
            return vec![];
        }
        // Where a hole's piece cancels part of an outline's, that only holds flat: lifted onto
        // the surface the two no longer meet. Such a cell takes the shape's own part of it.
        let mixed = pieces.iter().any(|p| signed_area(p) < 0.0) && pieces.iter().any(|p| signed_area(p) > 0.0);
        if mixed { material(&outline, r).unwrap_or(pieces) } else { pieces }
    };
    // How many times each cell is halved each way (0, 1 or 2), from how steeply it rises.
    let levels: Vec<u32> = (0..nx * ny)
        .map(|k| {
            let pieces = cut(base(k % nx, k / nx));
            let (lo, hi) = pieces.iter().flatten().map(|q| lift(*q)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| (lo.min(h), hi.max(h)));
            let rise = if pieces.is_empty() { 0.0 } else { (hi - lo) / cell };
            if rise > 6.0 {
                2
            } else if rise > 2.0 {
                1
            } else {
                0
            }
        })
        .collect();
    let level =
        |i: isize, j: isize| -> u32 { if i < 0 || j < 0 || i as usize >= nx || j as usize >= ny { 0 } else { levels[j as usize * nx + i as usize] } };
    for j in 0..ny {
        for i in 0..nx {
            let l = level(i as isize, j as isize);
            let step = FINE >> l;
            // The lattice spacing along each side of this cell: its own or its neighbour's.
            let (ii, jj) = (i as isize, j as isize);
            let side_step = |dl: u32| FINE >> l.max(dl);
            let (left, right, top, bottom) =
                (side_step(level(ii - 1, jj)), side_step(level(ii + 1, jj)), side_step(level(ii, jj - 1)), side_step(level(ii, jj + 1)));
            let (bx0, by0, bx1, by1) = (i * FINE, j * FINE, (i + 1) * FINE, (j + 1) * FINE);
            for sy in (by0..by1).step_by(step) {
                for sx in (bx0..bx1).step_by(step) {
                    let r = Rect::from_points(at(sx, sy), at(sx + step, sy + step));
                    let pieces = cut(r);
                    if pieces.is_empty() {
                        continue;
                    }
                    // Lattice points along the cell's own border, between each edge's ends.
                    let stitch = |a: Point, b: Point| -> Vec<Point> {
                        let tol = 1e-7 * unit;
                        let along = |fixed_x: Option<usize>, fixed_y: Option<usize>, every: usize| -> Vec<Point> {
                            let mut out = vec![];
                            match (fixed_x, fixed_y) {
                                (Some(qx), None) => {
                                    let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
                                    for qy in (by0..=by1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.y > lo + tol && p.y < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                (None, Some(qy)) => {
                                    let (lo, hi) = (a.x.min(b.x), a.x.max(b.x));
                                    for qx in (bx0..=bx1).step_by(every) {
                                        let p = at(qx, qy);
                                        if p.x > lo + tol && p.x < hi - tol {
                                            out.push(p);
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if (a.x > b.x) || (a.y > b.y) {
                                out.reverse();
                            }
                            out
                        };
                        let on = |v: f64, w: f64| (v - w).abs() <= tol;
                        let (x0, x1, y0, y1) = (at(bx0, by0).x, at(bx1, by1).x, at(bx0, by0).y, at(bx1, by1).y);
                        if on(a.x, x0) && on(b.x, x0) {
                            along(Some(bx0), None, left)
                        } else if on(a.x, x1) && on(b.x, x1) {
                            along(Some(bx1), None, right)
                        } else if on(a.y, y0) && on(b.y, y0) {
                            along(None, Some(by0), top)
                        } else if on(a.y, y1) && on(b.y, y1) {
                            along(None, Some(by1), bottom)
                        } else {
                            vec![]
                        }
                    };
                    let pieces: Vec<Vec<Point>> = pieces
                        .iter()
                        .map(|p| {
                            let mut out = Vec::with_capacity(p.len());
                            for (a, b) in p.iter().zip(p.iter().cycle().skip(1)).take(p.len()) {
                                out.push(*a);
                                out.extend(stitch(*a, *b));
                            }
                            out
                        })
                        .collect();
                    let lifts: Vec<Vec<f64>> = pieces.iter().map(|p| p.iter().map(|q| lift(*q)).collect()).collect();
                    // Each piece lifted onto the surface. Its normal is the lifted piece's own average
                    // (Newell's), not the slope at one point: near the edge the surface turns
                    // vertical, and a point sample there misjudges which way a whole piece faces.
                    let front: Vec<Vec<V3>> =
                        pieces.iter().zip(&lifts).map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(*q, half + h)).collect()).collect();
                    let n = newell(&front);
                    // The surface's exact normals (art space), for smooth shading.
                    let slopes: Option<Vec<V3>> = camera.rig.smooth().then(|| pieces.iter().flatten().map(|q| slope_normal(*q)).collect());
                    let given = slopes.as_ref().map(|s| s.iter().map(|m| camera.rotate(*m)).collect());
                    b.face_with(front, n, given)?;
                    if o.both_sides && o.cap {
                        // The back winds the same way round as the front, so its outward normal is
                        // reversed.
                        let back: Vec<Vec<V3>> = pieces
                            .iter()
                            .zip(&lifts)
                            .map(|(p, h)| p.iter().zip(h).map(|(q, h)| camera.place(back_turn.apply(*q), -half - h * back_turn.scale)).collect())
                            .collect();
                        let n = newell(&back);
                        // Its exact normals too: the front's mirrored front to back, turned with
                        // it (a taper scales it evenly, which leaves them be). Averaged ones would
                        // differ from the front's where the two meet at the rim, and the depth test
                        // switching between them there shows as a comb.
                        let (sin, cos) = (back_turn.sin, back_turn.cos);
                        let given = slopes
                            .as_ref()
                            .map(|s| s.iter().map(|m| camera.rotate(V3(m.0 * cos - m.1 * sin, m.0 * sin + m.1 * cos, -m.2))).collect());
                        b.face_with(back, V3(-n.0, -n.1, -n.2), given)?;
                    }
                }
            }
        }
    }
    if !o.both_sides && o.cap {
        let back: Vec<Vec<V3>> = rings.iter().map(|r| r.iter().map(|p| camera.place(back_turn.apply(*p), -half)).collect()).collect();
        b.face(back, camera.rotate(V3(0.0, 0.0, -1.0)))?;
    }
    if o.depth > 0.0 {
        // The sides, cut into slices that follow a twist or taper.
        let slices = if o.twist != 0.0 || o.taper != 1.0 { ((o.twist.abs() / 6.0).ceil() as usize).clamp(8, 64) } else { 1 };
        for ring in &rings {
            let at = |k: usize| -> Vec<V3> {
                let t = k as f64 / slices as f64;
                let turn = Turn::new(bounds, o.twist, o.taper, t);
                ring.iter().map(|p| camera.place(turn.apply(*p), half - o.depth * t)).collect()
            };
            for k in 0..slices {
                b.band(&at(k), &at(k + 1), |i| outward(ring, i, 0.0))?;
            }
        }
    }
    // Art outside the shape stays on the plane of its outline.
    let mapped = map_art(art, size, &camera, |p| (camera.place(p, half + lift(p)), camera.rotate(slope_normal(p))))?;
    Ok(Shaded { mapped, ..b.finish() })
}

#[cfg(test)]
#[path = "tests_solid.rs"]
mod tests;
