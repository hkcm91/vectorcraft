//! Lights and cast shadows shared by every 3D effect.
//!
//! Shading sums, per colour channel, the ambient light and each light's diffuse share:
//! `intensity × colour × wrap(n·l)`, where `wrap(x) = (x + s) / (1 + s)` lets a light with softness
//! `s` reach round the edge of the lit side (0 is a hard terminator, 1 lights half the back).
//! The result multiplies the object's colour, clamped to 0–1 per channel.
//!
//! A shadow is the solid projected along the main light onto a plane: below the object (a ground
//! tilted towards the viewer, so it shows from the front) or behind it. Our own model, from the
//! public description of the 3D and Materials panel's lighting and shadow options.

use vectorcraft_geom::{BezPath, PathData, Point};

use crate::V3;

/// How many lights besides the main one an effect takes.
pub const MAX_EXTRA_LIGHTS: usize = 3;
/// How far the ground plane tilts towards the viewer, in degrees.
const GROUND_TILT: f64 = 20.0;
/// The most points a shadow outline takes.
const MAX_SHADOW_POINTS: usize = 400_000;

/// One directional light. Angles in degrees, intensity and softness 0–1, colour linear 0–1 RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub azimuth: f64,
    pub elevation: f64,
    pub intensity: f64,
    pub color: [f64; 3],
    pub softness: f64,
}

impl Default for Light {
    fn default() -> Self {
        Self { azimuth: 45.0, elevation: 30.0, intensity: 0.5, color: [1.0; 3], softness: 0.0 }
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Light {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            azimuth: finite(self.azimuth, 45.0, -360.0, 360.0),
            elevation: finite(self.elevation, 30.0, -90.0, 90.0),
            intensity: finite(self.intensity, 0.5, 0.0, 1.0),
            color: self.color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            softness: finite(self.softness, 0.0, 0.0, 1.0),
        }
    }
    /// The direction towards the light in view space (y down, z towards the viewer).
    pub(crate) fn direction(&self) -> V3 {
        direction(self.azimuth, self.elevation)
    }
}

/// The view-space direction towards a light at `azimuth`, `elevation` (degrees).
pub(crate) fn direction(azimuth: f64, elevation: f64) -> V3 {
    let (sl, cl) = elevation.to_radians().sin_cos();
    let (sa, ca) = azimuth.to_radians().sin_cos();
    V3(cl * sa, -sl, cl * ca)
}

/// Where a 3D object casts its shadow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowPosition {
    #[default]
    None,
    /// On the ground under the object.
    Below,
    /// On a wall behind the object.
    Behind,
}

impl ShadowPosition {
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Below => "below",
            Self::Behind => "behind",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::None, Self::Below, Self::Behind].into_iter().find(|p| p.id() == s)
    }
}

/// Every light of a view, ready to shade with.
#[derive(Clone, Debug)]
pub(crate) struct Rig {
    /// (direction, intensity, colour, softness) per light, the main one first.
    lights: Vec<(V3, f64, [f64; 3], f64)>,
    ambient: f64,
    on: bool,
}

impl Rig {
    pub(crate) fn new(main: Light, extra: &[Light], ambient: f64, on: bool) -> Self {
        let lights = std::iter::once(main).chain(extra.iter().copied()).map(|l| (l.direction(), l.intensity, l.color, l.softness)).collect();
        Self { lights, ambient, on }
    }

    /// The colour a face with unit normal `n` (view space, towards the viewer) multiplies its
    /// paint by.
    pub(crate) fn shade(&self, n: V3) -> [f64; 3] {
        if !self.on {
            return [1.0; 3];
        }
        let mut c = [self.ambient; 3];
        for (dir, intensity, color, softness) in &self.lights {
            let wrap = ((n.dot(*dir) + softness) / (1.0 + softness)).max(0.0);
            for (ch, k) in c.iter_mut().zip(color) {
                *ch += intensity * k * wrap;
            }
        }
        c.map(|v| v.clamp(0.0, 1.0))
    }
}

/// The plane a shadow falls on: its unit normal (pointing to the object's side) and which way.
fn shadow_plane(position: ShadowPosition) -> Option<V3> {
    match position {
        ShadowPosition::None => None,
        ShadowPosition::Behind => Some(V3(0.0, 0.0, 1.0)),
        ShadowPosition::Below => {
            let (s, c) = GROUND_TILT.to_radians().sin_cos();
            Some(V3(0.0, -c, s))
        }
    }
}

/// The shadow `faces` (each a face's view-space contours) cast along the light towards `light`
/// on the plane at `position`, `distance` beyond the object, projected by `project`: one outline
/// whose nonzero fill is the shadow. None when there is no shadow plane or the light grazes it or
/// lies beyond it.
pub(crate) fn cast(faces: &[Vec<Vec<V3>>], light: V3, position: ShadowPosition, distance: f64, project: impl Fn(V3) -> Point) -> Option<PathData> {
    let n = shadow_plane(position)?;
    let along = light.dot(n);
    if along < 0.05 || faces.is_empty() {
        return None;
    }
    let distance = if distance.is_finite() { distance.clamp(0.0, 1e5) } else { 0.0 };
    let points = faces.iter().flatten().flatten();
    let lowest = points.clone().map(|v| v.dot(n)).fold(f64::INFINITY, f64::min);
    let (lo, hi) = points.fold((V3(f64::MAX, f64::MAX, f64::MAX), V3(f64::MIN, f64::MIN, f64::MIN)), |(a, b), v| {
        (V3(a.0.min(v.0), a.1.min(v.1), a.2.min(v.2)), V3(b.0.max(v.0), b.1.max(v.1), b.2.max(v.2)))
    });
    let size = (hi.0 - lo.0).max(hi.1 - lo.1).max(hi.2 - lo.2).max(1.0);
    if !lowest.is_finite() {
        return None;
    }
    let plane = lowest - distance;
    // A light just above the plane would stretch the shadow out of all proportion.
    let reach = 20.0 * size + distance;
    let mut bp = BezPath::new();
    let mut count = 0usize;
    for face in faces {
        let projected: Vec<Vec<Point>> = face
            .iter()
            .map(|c| {
                c.iter()
                    .map(|v| {
                        let t = ((v.dot(n) - plane) / along).clamp(0.0, reach);
                        project(V3(v.0 - light.0 * t, v.1 - light.1 * t, v.2 - light.2 * t))
                    })
                    .collect()
            })
            .collect();
        count += projected.iter().map(Vec::len).sum::<usize>();
        if count > MAX_SHADOW_POINTS {
            return None;
        }
        // Each face is turned to wind positively (keeping its holes against it), so the nonzero
        // fill of all of them together is their union.
        let outer = projected.iter().map(|c| area(c)).max_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0);
        if outer.abs() < 1e-9 {
            continue;
        }
        for c in &projected {
            if c.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                return None;
            }
            let mut pts: Box<dyn Iterator<Item = &Point>> = if outer > 0.0 { Box::new(c.iter()) } else { Box::new(c.iter().rev()) };
            let Some(first) = pts.next() else { continue };
            bp.move_to(*first);
            for p in pts {
                bp.line_to(*p);
            }
            bp.close_path();
        }
    }
    (!bp.elements().is_empty()).then(|| PathData::from_bezpath(&bp))
}

/// The signed area of a polygon (positive: clockwise on screen, y down).
fn area(c: &[Point]) -> f64 {
    c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_white_hard_light_shades_as_before() {
        let main = Light { azimuth: -45.0, elevation: 45.0, intensity: 0.8, ..Light::default() };
        let rig = Rig::new(main, &[], 0.25, true);
        let n = V3(0.0, 0.0, 1.0);
        let l = main.direction();
        let expect = (0.25 + 0.8 * n.dot(l).max(0.0)).clamp(0.0, 1.0);
        assert_eq!(rig.shade(n), [expect; 3]);
        assert_eq!(Rig::new(main, &[], 0.25, false).shade(n), [1.0; 3]);
    }

    #[test]
    fn colour_softness_and_more_lights() {
        let red = Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, color: [1.0, 0.0, 0.0], softness: 0.0 };
        let rig = Rig::new(red, &[], 0.0, true);
        assert_eq!(rig.shade(V3(0.0, 0.0, 1.0)), [1.0, 0.0, 0.0]);
        // A face turned away gets nothing from a hard light, something from a soft one.
        let side = V3(1.0, 0.0, 0.0);
        assert_eq!(rig.shade(side), [0.0; 3]);
        let soft = Rig::new(Light { softness: 1.0, ..red }, &[], 0.0, true);
        assert!((soft.shade(side)[0] - 0.5).abs() < 1e-12);
        // A blue light from the side adds blue there.
        let blue = Light { azimuth: 90.0, elevation: 0.0, intensity: 1.0, color: [0.0, 0.0, 1.0], softness: 0.0 };
        let both = Rig::new(red, &[blue], 0.0, true);
        let c = both.shade(side);
        assert!(c[0] < 1e-12 && (c[2] - 1.0).abs() < 1e-9, "{c:?}");
    }

    #[test]
    fn shadows_fall_below_or_behind_and_need_the_light_on_the_right_side() {
        // A unit square facing the viewer, 10 above the origin.
        let square = vec![vec![V3(-1.0, -11.0, 0.0), V3(1.0, -11.0, 0.0), V3(1.0, -9.0, 0.0), V3(-1.0, -9.0, 0.0)]];
        let flat = |v: V3| Point::new(v.0, v.1);
        let above = direction(0.0, 60.0);
        let below = cast(std::slice::from_ref(&square), above, ShadowPosition::Below, 5.0, flat).unwrap();
        let b = below.bounds().unwrap();
        assert!(b.y0 > -9.0, "the shadow is under the square: {b:?}");
        assert!(
            cast(std::slice::from_ref(&square), direction(0.0, -30.0), ShadowPosition::Below, 0.0, flat).is_none(),
            "a light from below casts none down"
        );
        let behind = cast(std::slice::from_ref(&square), direction(30.0, 30.0), ShadowPosition::Behind, 10.0, flat).unwrap();
        let b = behind.bounds().unwrap();
        assert!(b.x0 < -1.0 && b.y0 > -11.0, "pushed away from a light at the upper right: {b:?}");
        assert!(cast(&[square], direction(0.0, 0.0), ShadowPosition::None, 0.0, flat).is_none());
    }
}
