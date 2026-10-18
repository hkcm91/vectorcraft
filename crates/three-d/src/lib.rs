//! Portable 3D effect geometry: Revolve, Extrude & Bevel, Inflate and Rotate. No windowing, GPU or
//! document dependencies.
//!
//! A capped Revolve closes an open profile against its axis and fills the cut ends of a partial
//! turn, so it reads as a solid; uncapped, it is a thin shell. Faces are projected and sorted back to front,
//! then flat shaded. This vector preview uses a painter's algorithm, not a depth buffer:
//! intersecting profiles can have ambiguous visibility and are not solid modelling inputs.
#![forbid(unsafe_code)]

mod lighting;
mod raster;
mod smooth;
mod solid;
mod visibility;
pub use lighting::{Environment, Light, MAX_EXTRA_LIGHTS, MAX_SHADING_STEPS, Material, Shade, ShadowDirection, ShadowPosition};
pub use raster::{DRAFT_SIDE, MAX_SIDE, ShadedImage, Shading};
pub use solid::{
    Bevel, BevelShape, Extrude, Facet, Inflate, Mapped, Shaded, View, extrude, extrude_lit, extrude_mapped, inflate, inflate_lit, inflate_mapped,
    plane, plane_lit, plane_mapped,
};
pub use visibility::{VisibleFace, visible_faces};

use vectorcraft_geom::{BezPath, ParamCurve, PathData, Point, Rect};

/// Resource limits shared by desktop, CLI and browser evaluation.
const MAX_PROFILE_POINTS: usize = 1024;
const MAX_FACES: usize = 32768;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct V3(f64, f64, f64);

impl V3 {
    fn sub(self, b: Self) -> Self {
        Self(self.0 - b.0, self.1 - b.1, self.2 - b.2)
    }
    fn cross(self, b: Self) -> Self {
        Self(self.1 * b.2 - self.2 * b.1, self.2 * b.0 - self.0 * b.2, self.0 * b.1 - self.1 * b.0)
    }
    fn dot(self, b: Self) -> f64 {
        self.0 * b.0 + self.1 * b.1 + self.2 * b.2
    }
    fn unit(self) -> Self {
        let l = self.dot(self).sqrt();
        if l < 1e-12 { Self::default() } else { Self(self.0 / l, self.1 / l, self.2 / l) }
    }
}

/// The vertical axis is offset outward from the selected edge of the profile's bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Left,
    Right,
}

/// All angles are degrees; offset is in document points, lighting values are 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revolve {
    pub angle: f64,
    pub offset: f64,
    pub edge: Edge,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective strength, 0 for orthographic projection, up to 1.
    pub perspective: f64,
    /// Radial subdivisions for a full turn, clamped to 8..128.
    pub segments: usize,
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
    /// Solid: an open profile is closed against the axis (discs where its ends turn) and a
    /// partial turn's cut ends are filled. Hollow (false): a thin shell.
    pub cap: bool,
}

impl Default for Revolve {
    fn default() -> Self {
        Self {
            angle: 360.0,
            offset: 0.0,
            edge: Edge::Left,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            perspective: 0.0,
            segments: 64,
            light_azimuth: -45.0,
            light_elevation: 45.0,
            light_intensity: 0.8,
            ambient: 0.25,
            shade: true,
            light_color: [1.0; 3],
            light_softness: 0.0,
            extra_lights: [Light::default(); MAX_EXTRA_LIGHTS],
            extra_count: 0,
            shadow: ShadowPosition::None,
            shadow_distance: 0.0,
            shadow_direction: ShadowDirection::Light,
            material: Material::default(),
            cap: false,
        }
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Revolve {
    /// Unit bases of the rings that adjust the Euler X, Y and Z view parameters.
    /// X turns before Y and Z, Y before Z, and Z turns in the screen plane.
    /// Each pair has a cross product pointing along that parameter's rotation axis.
    pub fn rotation_planes(self) -> [[[f64; 3]; 2]; 3] {
        let o = self.sanitized();
        let x_frame = Self { rotation_x: 0.0, ..o };
        let y_frame = Self { rotation_x: 0.0, rotation_y: 0.0, ..o };
        [
            [x_frame.rotate(V3(0.0, 1.0, 0.0)), x_frame.rotate(V3(0.0, 0.0, 1.0))],
            [y_frame.rotate(V3(1.0, 0.0, 0.0)), y_frame.rotate(V3(0.0, 0.0, -1.0))],
            [V3(1.0, 0.0, 0.0), V3(0.0, 1.0, 0.0)],
        ]
        .map(|plane| plane.map(|v| [v.0, v.1, v.2]))
    }

    /// The fixed centre of view rotation, in document coordinates.
    pub fn rotation_origin(self, bounds: Rect) -> Point {
        let o = self.sanitized();
        Point::new(if o.edge == Edge::Left { bounds.x0 - o.offset } else { bounds.x1 + o.offset }, bounds.center().y)
    }

    /// Rotated unit axes for a view-orientation guide (screen Y points down).
    pub fn view_axes(self) -> [Point; 3] {
        [V3(1.0, 0.0, 0.0), V3(0.0, 1.0, 0.0), V3(0.0, 0.0, 1.0)].map(|v| {
            let p = self.sanitized().rotate(v);
            Point::new(p.0, p.1)
        })
    }

    /// The same projected vertical axis used by the surface evaluator.
    pub fn projected_axis(self, bounds: Rect) -> [Point; 2] {
        let o = self.sanitized();
        let axis = match o.edge {
            Edge::Left => bounds.x0 - o.offset,
            Edge::Right => bounds.x1 + o.offset,
        };
        let center_y = bounds.center().y;
        let radius = (bounds.x0 - axis).abs().max((bounds.x1 - axis).abs());
        let distance = 4.0 * radius.max(bounds.height().abs()).max(1.0);
        [bounds.y0, bounds.y1].map(|y| {
            let v = o.rotate(V3(0.0, y - center_y, 0.0));
            let factor = distance / (distance - v.2 * o.perspective);
            Point::new(axis + v.0 * factor, center_y + v.1 * factor)
        })
    }

    /// Directional light in view space, shared by shading and interactive guides.
    pub fn light_direction(self) -> [f64; 3] {
        let o = self.sanitized();
        let (sl, cl) = o.light_elevation.to_radians().sin_cos();
        let (sa, ca) = o.light_azimuth.to_radians().sin_cos();
        [cl * sa, -sl, cl * ca]
    }

    /// Light-direction orbits as (centre, cosine basis, sine basis), in view space.
    /// The first changes azimuth at the current elevation. The second changes elevation
    /// at the current azimuth. The meridian's valid parameter range is -90..90 degrees.
    pub fn light_orbits(self) -> [[[f64; 3]; 3]; 2] {
        let o = self.sanitized();
        let (se, ce) = o.light_elevation.to_radians().sin_cos();
        let (sa, ca) = o.light_azimuth.to_radians().sin_cos();
        [[[0.0, -se, 0.0], [0.0, 0.0, ce], [ce, 0.0, 0.0]], [[0.0, 0.0, 0.0], [sa, 0.0, ca], [0.0, -1.0, 0.0]]]
    }

    /// Move a light on its current hemisphere, preserving the azimuth at the poles.
    /// A light behind the object stays behind when its projected handle is dragged.
    pub fn light_at_on_side(self, point: Point) -> [f64; 2] {
        let o = self.sanitized();
        let point = Point::new(finite(point.x, 0.0, -1e8, 1e8), finite(point.y, 0.0, -1e8, 1e8));
        let length = point.x.hypot(point.y).max(1.0);
        let (x, y) = (point.x / length, point.y / length);
        let z = (1.0 - x * x - y * y).max(0.0).sqrt().copysign(o.light_direction()[2]);
        let azimuth = if x.hypot(z) < 1e-9 { o.light_azimuth } else { x.atan2(z).to_degrees() };
        [azimuth, (-y).clamp(-1.0, 1.0).asin().to_degrees()]
    }

    /// A dragged light position on the front hemisphere, in normalized view coordinates.
    pub fn light_at(point: Point) -> [f64; 2] {
        let length = point.x.hypot(point.y).max(1.0);
        let (x, y) = (point.x / length, point.y / length);
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        [x.atan2(z).to_degrees(), (-y).clamp(-1.0, 1.0).asin().to_degrees()]
    }

    fn sanitized(self) -> Self {
        Self {
            angle: finite(self.angle, 360.0, 0.0, 360.0),
            offset: finite(self.offset, 0.0, 0.0, 1e5),
            rotation_x: finite(self.rotation_x, 0.0, -360.0, 360.0),
            rotation_y: finite(self.rotation_y, 0.0, -360.0, 360.0),
            rotation_z: finite(self.rotation_z, 0.0, -360.0, 360.0),
            perspective: finite(self.perspective, 0.0, 0.0, 1.0),
            segments: self.segments.clamp(8, 128),
            light_azimuth: finite(self.light_azimuth, -45.0, -360.0, 360.0),
            light_elevation: finite(self.light_elevation, 45.0, -90.0, 90.0),
            light_intensity: finite(self.light_intensity, 0.8, 0.0, 1.0),
            ambient: finite(self.ambient, 0.25, 0.0, 1.0),
            light_color: self.light_color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            light_softness: finite(self.light_softness, 0.0, 0.0, 1.0),
            extra_lights: self.extra_lights.map(Light::sanitized),
            extra_count: self.extra_count.min(MAX_EXTRA_LIGHTS),
            shadow_distance: finite(self.shadow_distance, 0.0, 0.0, 1e5),
            material: self.material.sanitized(),
            ..self
        }
    }

    /// The lights to shade with.
    pub(crate) fn rig(self) -> lighting::Rig {
        let main = Light {
            azimuth: self.light_azimuth,
            elevation: self.light_elevation,
            intensity: self.light_intensity,
            color: self.light_color,
            softness: self.light_softness,
        };
        lighting::Rig::new(main, self.extra_lights.get(..self.extra_count).unwrap_or_default(), self.ambient, self.material, self.shade)
    }
    fn rotate(self, v: V3) -> V3 {
        let (sx, cx) = self.rotation_x.to_radians().sin_cos();
        let (sy, cy) = self.rotation_y.to_radians().sin_cos();
        let (sz, cz) = self.rotation_z.to_radians().sin_cos();
        let v = V3(v.0, cx * v.1 - sx * v.2, sx * v.1 + cx * v.2);
        let v = V3(cy * v.0 + sy * v.2, v.1, -sy * v.0 + cy * v.2);
        V3(cz * v.0 - sz * v.1, sz * v.0 + cz * v.1, v.2)
    }
}

/// One opaque vector polygon, already projected. `tint` multiplies the source colour channel by
/// channel and `highlight` is added on top; `brightness` is the tint's mean, at most 1 (what
/// darkens paints that aren't a flat colour).
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub points: Vec<Point>,
    pub depth: f64,
    pub brightness: f64,
    pub tint: [f64; 3],
    pub highlight: [f64; 3],
}

/// The mean of a tint's channels.
pub(crate) fn mean(t: [f64; 3]) -> f64 {
    (t[0] + t[1] + t[2]) / 3.0
}

impl Face {
    pub fn path(&self) -> PathData {
        let mut bp = BezPath::new();
        if let Some(first) = self.points.first() {
            bp.move_to(*first);
            for p in self.points.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        }
        PathData::from_bezpath(&bp)
    }
}

/// Evaluate a profile in document space. All subpaths share one axis from `bounds`.
/// Invalid or excessively complex inputs return an error, never a partial surface.
pub fn revolve(path: &PathData, bounds: Rect, options: Revolve) -> Result<Vec<Face>, String> {
    revolve_lit(path, bounds, options).map(|r| r.faces)
}

/// A revolved surface: its faces back to front, the shadow it casts (an outline to fill) when the
/// options ask for one and the light can cast it, and with smooth shading the surface to draw
/// pixel by pixel (the faces' shades are then their means).
#[derive(Clone, Debug, PartialEq)]
pub struct Revolved {
    pub faces: Vec<Face>,
    pub shadow: Option<PathData>,
    pub shading: Option<Shading>,
    /// The contact shadow where the surface meets the shadow's plane, and how strongly it shows.
    pub contact: Option<(PathData, f64)>,
}

/// [`revolve`], with its shadow and smooth shading.
pub fn revolve_lit(path: &PathData, bounds: Rect, options: Revolve) -> Result<Revolved, String> {
    let o = options.sanitized();
    let rig = o.rig();
    let mut casters: Vec<Vec<Vec<V3>>> = vec![];
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
        return Err("Revolve requires finite profile coordinates within 100 million points".into());
    }
    let axis = match o.edge {
        Edge::Left => bounds.x0 - o.offset,
        Edge::Right => bounds.x1 + o.offset,
    };
    let center_y = (bounds.y0 + bounds.y1) * 0.5;
    let radius = (bounds.x0 - axis).abs().max((bounds.x1 - axis).abs());
    let distance = 4.0 * radius.max(bounds.height().abs()).max(1.0);
    let steps = ((o.segments as f64 * o.angle / 360.0).ceil() as usize).max(1);
    let sweep = o.angle.to_radians();
    let [lx, ly, lz] = o.light_direction();
    let light = V3(lx, ly, lz);
    let project = |v: V3| {
        let factor = distance / (distance - v.2 * o.perspective);
        Point::new(axis + v.0 * factor, center_y + v.1 * factor)
    };
    let vertex = |p: Point, step: usize| {
        let (s, c) = (sweep * step as f64 / steps as f64).sin_cos();
        o.rotate(V3((p.x - axis) * c, p.y - center_y, (p.x - axis) * s))
    };
    // Points this close to the axis are on it.
    let on_axis = 1e-9 * radius.max(bounds.height().abs()).max(1.0);
    // A partial turn's cut ends, filled when capped.
    let cut = o.cap && sweep > 1e-9 && o.angle < 360.0;
    let mut faces = Vec::new();
    // With smooth shading, each face's corners and shading normal.
    let mut smooth: Vec<(Vec<Vec<V3>>, V3)> = Vec::new();
    let mut count = 0;
    for sp in &path.subpaths {
        let mut profile = Vec::new();
        if let Some(first) = sp.anchors.first() {
            profile.push(first.p);
            count += 1;
            if count > MAX_PROFILE_POINTS {
                return Err("Revolve profile is too complex".into());
            }
        }
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        // Bound work before sampling any data. Curves get sixteen samples, straight edges one.
        if segments > MAX_PROFILE_POINTS {
            return Err("Revolve profile is too complex".into());
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let samples = if sp.segment_is_line(i) { 1 } else { 16 };
            for k in 1..=samples {
                profile.push(curve.eval(k as f64 / samples as f64));
                count += 1;
                if count > MAX_PROFILE_POINTS {
                    return Err("Revolve profile is too complex".into());
                }
            }
        }
        // Capped, an open profile runs on to the axis and back along it, so the ends it sweeps
        // are discs and the cut ends are the region between the profile and the axis.
        if o.cap
            && !sp.closed
            && let (Some(&first), Some(&last)) = (profile.first(), profile.last())
        {
            for p in [Point::new(axis, last.y), Point::new(axis, first.y), first] {
                if profile.last().is_some_and(|q| (q.x - p.x).abs() > on_axis || (q.y - p.y).abs() > on_axis) {
                    profile.push(p);
                }
            }
        }
        let mut quads: Vec<Vec<V3>> = Vec::new();
        for pair in profile.windows(2) {
            let [a, b] = pair else { continue };
            if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
                return Err("Revolve profile contains invalid coordinates".into());
            }
            for j in 0..steps {
                quads.push(vec![vertex(*a, j), vertex(*b, j), vertex(*b, j + 1), vertex(*a, j + 1)]);
            }
        }
        if cut && profile.len() >= 3 {
            // The closing point repeats the first.
            let ring = match (profile.first(), profile.last()) {
                (Some(f), Some(l)) if f.distance(*l) <= on_axis => profile.get(..profile.len() - 1).unwrap_or_default(),
                _ => profile.as_slice(),
            };
            for step in [0, steps] {
                quads.push(ring.iter().map(|p| vertex(*p, step)).collect());
            }
        }
        for vertices in quads {
            let mut normal = match vertices.as_slice() {
                [v0, v1, v2, v3] => {
                    // The diagonal gives a nonzero normal even when one profile endpoint is on-axis.
                    let n = v1.sub(*v0).cross(v2.sub(*v0)).unit();
                    if n.dot(n) < 0.5 { v2.sub(*v0).cross(v3.sub(*v0)).unit() } else { n }
                }
                _ => solid::newell(std::slice::from_ref(&vertices)).unit(),
            };
            if normal.dot(normal) < 0.5 {
                continue;
            }
            if o.shadow != ShadowPosition::None {
                casters.push(vec![vertices.clone()]);
            }
            // Profiles may run up or down. Shade the visible side of this two-sided surface.
            if normal.2 < 0.0 {
                normal = V3(-normal.0, -normal.1, -normal.2);
            }
            let shade = rig.shade(normal);
            let points: Vec<_> = vertices.iter().map(|v| project(*v)).collect();
            if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                return Err("Revolve projection is not finite".into());
            }
            let area = points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .take(points.len())
                .map(|(a, b)| (a.x - axis) * (b.y - center_y) - (b.x - axis) * (a.y - center_y))
                .sum::<f64>();
            if area.abs() < 1e-10 {
                continue;
            }
            let depth = vertices.iter().map(|v| v.2).sum::<f64>() / vertices.len() as f64;
            if rig.smooth() {
                smooth.push((vec![vertices], normal));
            }
            faces.push(Face { points, depth, brightness: mean(shade.tint).min(1.0), tint: shade.tint, highlight: shade.highlight });
            if faces.len() > MAX_FACES {
                return Err("Revolve surface is too complex; reduce Segments or simplify the profile".into());
            }
        }
    }
    let mut shading = None;
    if rig.smooth() && smooth.len() == faces.len() {
        // The lights and the eye: six half extents from the centre.
        let reach = 1.5 * distance;
        let sources: Vec<smooth::Source> =
            smooth.iter().map(|(contours, normal)| smooth::Source { contours, normal: *normal, given: None }).collect();
        if let Some(smoothed) = smooth::shade(&sources, &[], &rig, reach) {
            let mut polys = Vec::with_capacity(smoothed.len());
            for ((f, (contours, normal)), s) in faces.iter_mut().zip(&smooth).zip(smoothed) {
                f.tint = s.shade.tint;
                f.highlight = s.shade.highlight;
                f.brightness = mean(s.shade.tint).min(1.0);
                polys.push(raster::Poly {
                    projected: vec![f.points.clone()],
                    normals: s.normals,
                    positions: contours.iter().flatten().copied().collect(),
                    normal: *normal,
                });
            }
            shading = Some(Shading::new(rig.clone(), reach, polys));
        }
    }
    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    let shadow = lighting::cast(&casters, light, o.shadow_direction, o.shadow, o.shadow_distance, project);
    let contact = lighting::contact(&casters, o.shadow, o.shadow_distance, project);
    Ok(Revolved { faces, shadow, shading, contact })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_orbits_and_handle_projection_match_the_shading_direction_on_both_sides() {
        for azimuth in [-170.0f64, -45.0, 0.0, 83.0, 167.0] {
            for elevation in [-85.0f64, -30.0, 0.0, 45.0, 90.0] {
                let o = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..Default::default() };
                let direction = o.light_direction();
                for ([center, u, v], angle) in o.light_orbits().into_iter().zip([azimuth, elevation]) {
                    let (s, c) = angle.to_radians().sin_cos();
                    for i in 0..3 {
                        assert!((center[i] + u[i] * c + v[i] * s - direction[i]).abs() < 1e-12);
                    }
                }
                let [a, e] = o.light_at_on_side(Point::new(direction[0], direction[1]));
                assert!((a - azimuth).abs() < 1e-9 && (e - elevation).abs() < 1e-9, "{azimuth}, {elevation} -> {a}, {e}");
            }
        }
        let o = Revolve { light_azimuth: 135.0, ..Default::default() };
        let [azimuth, elevation] = o.light_at_on_side(Point::new(0.3, -0.4));
        let [x, y, z] = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..o }.light_direction();
        assert!((x - 0.3).abs() < 1e-12 && (y + 0.4).abs() < 1e-12 && z < 0.0);
        let [a, e] = o.light_at_on_side(Point::new(f64::NAN, f64::INFINITY));
        assert!(a.is_finite() && e.is_finite());
    }

    #[test]
    fn ring_axes_match_euler_parameter_rotation_and_origin_stays_fixed() {
        let o = Revolve { rotation_x: 27.0, rotation_y: -39.0, rotation_z: 53.0, ..Default::default() };
        let seed = V3(0.3, -0.5, 0.7);
        let before = o.rotate(seed);
        let planes = o.rotation_planes();
        for (axis, [u, v]) in planes.iter().enumerate() {
            let normal = V3(u[0], u[1], u[2]).cross(V3(v[0], v[1], v[2]));
            assert!((normal.dot(normal) - 1.0).abs() < 1e-12);
            let mut changed = o;
            match axis {
                0 => changed.rotation_x += 0.0001,
                1 => changed.rotation_y += 0.0001,
                _ => changed.rotation_z += 0.0001,
            }
            let actual = changed.rotate(seed).sub(before);
            let expected = normal.cross(before);
            let step = 0.0001f64.to_radians();
            for (a, b) in [(actual.0 / step, expected.0), (actual.1 / step, expected.1), (actual.2 / step, expected.2)] {
                assert!((a - b).abs() < 1e-5);
            }
        }
        assert_eq!(Revolve { edge: Edge::Right, offset: 12.0, ..o }.rotation_origin(Rect::new(20.0, 30.0, 80.0, 100.0)), Point::new(92.0, 65.0));
    }
    #[test]
    fn guides_match_projection_and_dragged_light_matches_shading_direction() {
        let o = Revolve { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, offset: 10.0, ..Default::default() };
        assert_eq!(o.view_axes(), [Point::new(1.0, 0.0), Point::new(0.0, 1.0), Point::new(0.0, 0.0)]);
        assert_eq!(o.projected_axis(Rect::new(20.0, 30.0, 80.0, 100.0)), [Point::new(10.0, 30.0), Point::new(10.0, 100.0)]);
        let [azimuth, elevation] = Revolve::light_at(Point::new(0.3, -0.4));
        let [x, y, z] = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..o }.light_direction();
        assert!((x - 0.3).abs() < 1e-9 && (y + 0.4).abs() < 1e-9 && z > 0.0);
        let [azimuth, elevation] = Revolve::light_at(Point::new(2.0, -2.0));
        assert!(azimuth.is_finite() && elevation.is_finite() && elevation.abs() <= 90.0);
    }
    fn cylinder() -> (PathData, Rect) {
        let mut bp = BezPath::new();
        bp.move_to((40.0, 10.0));
        bp.line_to((40.0, 90.0));
        (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0))
    }
    #[test]
    fn cylinder_has_expected_silhouette_depth_order_and_shading() {
        let (p, b) = cylinder();
        let faces = revolve(&p, b, Revolve { rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
        let points: Vec<_> = faces.iter().flat_map(|f| &f.points).collect();
        assert!((points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) + 40.0).abs() < 1e-6);
        assert!((points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) - 40.0).abs() < 1e-6);
        assert!(faces.windows(2).all(|w| w[0].depth <= w[1].depth));
        assert!(faces.iter().any(|f| f.brightness > 0.8) && faces.iter().any(|f| f.brightness < 0.4));
    }
    #[test]
    fn edge_offset_sweep_and_rotation_change_the_surface() {
        let (p, b) = cylinder();
        let base = revolve(&p, b, Revolve::default()).unwrap();
        for o in [
            Revolve { edge: Edge::Right, offset: 20.0, ..Default::default() },
            Revolve { angle: 120.0, ..Default::default() },
            Revolve { rotation_z: 60.0, perspective: 0.8, ..Default::default() },
        ] {
            assert_ne!(base, revolve(&p, b, o).unwrap());
        }
        assert!(revolve(&p, b, Revolve { angle: 0.0, ..Default::default() }).unwrap().is_empty());
    }
    fn inside(points: &[Point], p: Point) -> bool {
        let mut inside = false;
        for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                inside = !inside;
            }
        }
        inside
    }
    #[test]
    fn a_capped_open_profile_is_closed_with_discs_where_its_ends_turn() {
        // A tube 20 points off the axis, seen from above: hollow, the axis shows through.
        let (p, b) = cylinder();
        let top = Revolve { offset: 20.0, rotation_x: -80.0, ..Default::default() };
        let axis_top = |faces: &[Face]| {
            let axis = top.projected_axis(b)[0];
            faces.iter().any(|f| inside(&f.points, axis))
        };
        let hollow = revolve(&p, b, top).unwrap();
        assert!(!axis_top(&hollow), "an uncapped tube is open at the top");
        let solid = revolve(&p, b, Revolve { cap: true, ..top }).unwrap();
        assert!(axis_top(&solid), "a capped tube has a lid");
        assert!(solid.len() > hollow.len());
        // A full turn has no cut ends: every face is a quad.
        assert!(solid.iter().all(|f| f.points.len() == 4));
    }
    #[test]
    fn a_capped_partial_turn_fills_its_cut_ends_with_the_profile() {
        let (p, b) = cylinder();
        let half = Revolve { angle: 180.0, offset: 20.0, rotation_y: 30.0, rotation_x: -20.0, ..Default::default() };
        let hollow = revolve(&p, b, half).unwrap();
        assert!(hollow.iter().all(|f| f.points.len() == 4));
        let solid = revolve(&p, b, Revolve { cap: true, ..half }).unwrap();
        // Half a turn is 32 steps: a disc of 32 pieces at each end of the profile and the two
        // cut ends (the profile closed against the axis, a rectangle).
        assert_eq!(solid.len(), hollow.len() + 2 * 32 + 2);
        // A closed profile's cut ends are the profile itself.
        let mut bp = BezPath::new();
        bp.move_to((20.0, 20.0));
        bp.line_to((40.0, 20.0));
        bp.line_to((30.0, 60.0));
        bp.close_path();
        let triangle = PathData::from_bezpath(&bp);
        let tb = Rect::new(20.0, 20.0, 40.0, 60.0);
        let faces = revolve(&triangle, tb, Revolve { cap: true, ..half }).unwrap();
        assert_eq!(faces.iter().filter(|f| f.points.len() == 3).count(), 2, "two triangular cut ends");
        assert!(revolve(&triangle, tb, half).unwrap().iter().all(|f| f.points.len() == 4));
        // Nothing turned, nothing drawn, capped or not.
        assert!(revolve(&p, b, Revolve { angle: 0.0, cap: true, ..Default::default() }).unwrap().is_empty());
    }
    #[test]
    fn malformed_and_degenerate_inputs_are_bounded() {
        let (p, b) = cylinder();
        let f = revolve(&p, b, Revolve { segments: usize::MAX, angle: f64::NAN, offset: f64::INFINITY, ..Default::default() }).unwrap();
        assert!(f.len() <= 128);
        assert!(revolve(&p, Rect::new(f64::NAN, 0.0, 1.0, 1.0), Revolve::default()).is_err());
        assert!(revolve(&PathData::default(), b, Revolve::default()).unwrap().is_empty());
    }
}
