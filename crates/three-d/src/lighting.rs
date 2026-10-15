//! Lights and cast shadows shared by every 3D effect.
//!
//! Shading sums, per colour channel, the ambient light and each light's diffuse share:
//! `intensity × colour × wrap(n·l)`, where `wrap(x) = (x + s) / (1 + s)` lets a light with softness
//! `s` reach round the edge of the lit side (0 is a hard terminator, 1 lights half the back).
//! The result multiplies the object's colour, clamped to 0–1 per channel.
//!
//! A [`Material`] adds what makes a surface read as plastic, glossy or metal: a Blinn-Phong
//! highlight per light (`specular × intensity × colour × (n·h)^k`, `k` from the roughness),
//! added on top of the paint; for a metal the highlight takes the paint's colour, the diffuse
//! light fades and a soft studio reflection (bright above, dark below) takes its place. A rim
//! light brightens the edges turned away from the viewer (`rim × (1 − n·v)³`), and shading steps
//! quantize the light into flat bands, cel style. Our own model, not measured from any product.
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

/// The most shading steps (bands) a material takes.
pub const MAX_SHADING_STEPS: u32 = 8;

/// How a surface takes the light. Fractions 0–1; colours linear 0–1 RGB. The default is the
/// plain matte surface the 3D effects always had.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// How strong the highlight is (0: matte, no highlight).
    pub specular: f64,
    /// How spread the highlight is: 0 a tight mirror-like spot, 1 a broad sheen.
    pub roughness: f64,
    /// 0 a dielectric (plastic, paint), 1 a metal: coloured highlights and a studio reflection.
    pub metallic: f64,
    /// A light round the silhouette, towards the viewer's edges.
    pub rim: f64,
    pub rim_color: [f64; 3],
    /// 0 for smooth shading, else 2–8 flat bands (cel shading).
    pub steps: u32,
    /// The ambient light's colour.
    pub ambient_color: [f64; 3],
    /// Shade curved surfaces smoothly and give flat faces a gentle falloff (each face a linear
    /// ramp) instead of one flat colour per face.
    pub smooth: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self { specular: 0.0, roughness: 0.5, metallic: 0.0, rim: 0.0, rim_color: [1.0; 3], steps: 0, ambient_color: [1.0; 3], smooth: false }
    }
}

impl Material {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            specular: finite(self.specular, 0.0, 0.0, 1.0),
            roughness: finite(self.roughness, 0.5, 0.0, 1.0),
            metallic: finite(self.metallic, 0.0, 0.0, 1.0),
            rim: finite(self.rim, 0.0, 0.0, 1.0),
            rim_color: self.rim_color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            steps: if self.steps < 2 { 0 } else { self.steps.min(MAX_SHADING_STEPS) },
            ambient_color: self.ambient_color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            smooth: self.smooth,
        }
    }
    /// The Blinn-Phong exponent: from 2 (rough) to 256 (polished). Sharper would make
    /// highlights pinpricks at icon sizes, finer than the facets that carry them.
    fn shininess(&self) -> f64 {
        2.0 * 2f64.powf((1.0 - self.roughness) * 7.0)
    }
}

/// A face's shading: its paint is multiplied by `tint` and `highlight` is added on top, channel
/// by channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shade {
    pub tint: [f64; 3],
    pub highlight: [f64; 3],
}

impl Shade {
    pub const NONE: Self = Self { tint: [1.0; 3], highlight: [0.0; 3] };
}

/// Every light of a view, ready to shade with.
#[derive(Clone, Debug)]
pub(crate) struct Rig {
    /// (direction, intensity, colour, softness) per light, the main one first.
    lights: Vec<(V3, f64, [f64; 3], f64)>,
    ambient: f64,
    material: Material,
    on: bool,
}

/// `x` in `steps` flat bands (0 keeps it smooth).
fn band(x: f64, steps: u32) -> f64 {
    if steps < 2 {
        return x;
    }
    let s = f64::from(steps - 1);
    (x.clamp(0.0, 1.0) * s).round() / s
}

impl Rig {
    pub(crate) fn new(main: Light, extra: &[Light], ambient: f64, material: Material, on: bool) -> Self {
        let lights = std::iter::once(main).chain(extra.iter().copied()).map(|l| (l.direction(), l.intensity, l.color, l.softness)).collect();
        Self { lights, ambient, material: material.sanitized(), on }
    }

    pub(crate) fn smooth(&self) -> bool {
        self.on && self.material.smooth
    }

    /// Does the material shade in cel bands?
    pub(crate) fn banded(&self) -> bool {
        self.material.steps >= 2
    }

    /// The same lights and material without cel bands.
    pub(crate) fn unbanded(&self) -> Self {
        Self { material: Material { steps: 0, ..self.material }, ..self.clone() }
    }

    /// How a face with unit normal `n` (view space, towards the viewer) is shaded, the lights
    /// and the eye infinitely far away.
    pub(crate) fn shade(&self, n: V3) -> Shade {
        self.shade_with(n, |d| d, V3(0.0, 0.0, 1.0))
    }

    /// How the point `p` (view space, about the object's centre) with unit normal `n` is shaded,
    /// the lights and the eye `reach` from the centre.
    pub(crate) fn shade_at(&self, n: V3, p: V3, reach: f64) -> Shade {
        let towards = |d: V3| V3(d.0 * reach - p.0, d.1 * reach - p.1, d.2 * reach - p.2).unit();
        self.shade_with(n, towards, V3(-p.0, -p.1, reach - p.2).unit())
    }

    /// The shade of normal `n`, each light's direction `d` turned into the direction towards it by
    /// `towards`, seen from direction `view`.
    fn shade_with(&self, n: V3, towards: impl Fn(V3) -> V3, view: V3) -> Shade {
        if !self.on {
            return Shade::NONE;
        }
        let m = &self.material;
        let shininess = m.shininess();
        // A metal's own colour comes from its reflections, not from diffuse light.
        let diffuse_share = 1.0 - 0.85 * m.metallic;
        let mut diffuse = m.ambient_color.map(|c| self.ambient * c);
        let mut spec = [0.0; 3];
        for (dir, intensity, color, softness) in &self.lights {
            let dir = &towards(*dir);
            let wrap = band(((n.dot(*dir) + softness) / (1.0 + softness)).max(0.0), m.steps);
            for (ch, k) in diffuse.iter_mut().zip(color) {
                *ch += intensity * k * wrap;
            }
            if m.specular > 0.0 && n.dot(*dir) > 0.0 {
                let h = V3(dir.0 + view.0, dir.1 + view.1, dir.2 + view.2).unit();
                let mut s = n.dot(h).max(0.0).powf(shininess);
                if m.steps >= 2 {
                    s = if s > 0.5 { 1.0 } else { 0.0 };
                }
                for (ch, k) in spec.iter_mut().zip(color) {
                    *ch += m.specular * intensity * k * s;
                }
            }
        }
        let mut tint = diffuse.map(|v| v.clamp(0.0, 1.0) * diffuse_share);
        if m.metallic > 0.0 {
            // A studio reflection: what the surface mirrors, bright overhead, dark underfoot.
            let nv = n.dot(view);
            let r = V3(2.0 * nv * n.0 - view.0, 2.0 * nv * n.1 - view.1, 2.0 * nv * n.2 - view.2);
            let sky = band((0.5 - 0.5 * r.1).clamp(0.0, 1.0), m.steps);
            let reflection = m.metallic * (0.3 + 0.7 * sky);
            for (t, s) in tint.iter_mut().zip(spec) {
                *t += reflection + m.metallic * s;
            }
        }
        let mut highlight = spec.map(|s| s * (1.0 - m.metallic));
        if m.rim > 0.0 {
            let mut edge = (1.0 - n.dot(view).clamp(0.0, 1.0)).powi(3);
            if m.steps >= 2 {
                edge = if edge > 0.35 { 1.0 } else { 0.0 };
            }
            for (h, k) in highlight.iter_mut().zip(m.rim_color) {
                *h += m.rim * k * edge;
            }
        }
        Shade { tint: tint.map(|v| v.clamp(0.0, 2.0)), highlight: highlight.map(|v| v.clamp(0.0, 1.0)) }
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
        let rig = Rig::new(main, &[], 0.25, Material::default(), true);
        let n = V3(0.0, 0.0, 1.0);
        let l = main.direction();
        let expect = (0.25 + 0.8 * n.dot(l).max(0.0)).clamp(0.0, 1.0);
        assert_eq!(rig.shade(n), Shade { tint: [expect; 3], highlight: [0.0; 3] });
        assert_eq!(Rig::new(main, &[], 0.25, Material::default(), false).shade(n), Shade::NONE);
    }

    #[test]
    fn colour_softness_and_more_lights() {
        let m = Material::default();
        let red = Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, color: [1.0, 0.0, 0.0], softness: 0.0 };
        let rig = Rig::new(red, &[], 0.0, m, true);
        assert_eq!(rig.shade(V3(0.0, 0.0, 1.0)).tint, [1.0, 0.0, 0.0]);
        // A face turned away gets nothing from a hard light, something from a soft one.
        let side = V3(1.0, 0.0, 0.0);
        assert_eq!(rig.shade(side).tint, [0.0; 3]);
        let soft = Rig::new(Light { softness: 1.0, ..red }, &[], 0.0, m, true);
        assert!((soft.shade(side).tint[0] - 0.5).abs() < 1e-12);
        // A blue light from the side adds blue there.
        let blue = Light { azimuth: 90.0, elevation: 0.0, intensity: 1.0, color: [0.0, 0.0, 1.0], softness: 0.0 };
        let both = Rig::new(red, &[blue], 0.0, m, true);
        let c = both.shade(side).tint;
        assert!(c[0] < 1e-12 && (c[2] - 1.0).abs() < 1e-9, "{c:?}");
    }

    #[test]
    fn highlights_are_tight_when_polished_and_broad_when_rough() {
        // A light straight ahead: the face towards it carries the highlight's peak.
        let front = Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() };
        let glossy = Material { specular: 1.0, roughness: 0.1, ..Material::default() };
        let rough = Material { roughness: 0.9, ..glossy };
        let n = V3(0.0, 0.0, 1.0);
        let tilted = V3(0.3, 0.0, 1.0).unit();
        let at = |m: Material, n: V3| Rig::new(front, &[], 0.0, m, true).shade(n).highlight[0];
        assert!((at(glossy, n) - 1.0).abs() < 1e-9 && (at(rough, n) - 1.0).abs() < 1e-9);
        assert!(at(glossy, tilted) < 0.05 && at(rough, tilted) > 0.5, "{} {}", at(glossy, tilted), at(rough, tilted));
        // Matte: no highlight at all.
        assert_eq!(at(Material::default(), n), 0.0);
    }

    #[test]
    fn metal_reflects_its_paint_and_rim_light_lifts_the_edges() {
        let main = Light::default();
        let metal = Material { specular: 1.0, roughness: 0.3, metallic: 1.0, ..Material::default() };
        let up = V3(0.0, -1.0, 1.0).unit();
        let down = V3(0.0, 1.0, 1.0).unit();
        let rig = Rig::new(main, &[], 0.2, metal, true);
        assert!(rig.shade(up).tint[0] > rig.shade(down).tint[0], "the studio reflection is brighter above");
        assert_eq!(rig.shade(up).highlight, [0.0; 3], "a metal's highlight takes its paint's colour, in the tint");
        let rim = Material { rim: 1.0, rim_color: [0.0, 0.0, 1.0], ..Material::default() };
        let r = Rig::new(main, &[], 0.2, rim, true);
        let edge = V3(1.0, 0.0, 0.05).unit();
        assert!(r.shade(edge).highlight[2] > 0.8 && r.shade(V3(0.0, 0.0, 1.0)).highlight[2] < 1e-9);
    }

    #[test]
    fn shading_steps_make_flat_bands() {
        let main = Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() };
        let cel = Material { steps: 3, ..Material::default() };
        let rig = Rig::new(main, &[], 0.0, cel, true);
        let mut seen: Vec<f64> = (0..=40).map(|i| rig.shade(V3(f64::from(i) / 40.0, 0.0, 1.0).unit()).tint[0]).collect();
        seen.dedup();
        assert!(seen.len() <= 3, "{seen:?}");
        assert_eq!(Material { steps: 1, ..cel }.sanitized().steps, 0);
        assert_eq!(Material { steps: 99, ..cel }.sanitized().steps, MAX_SHADING_STEPS);
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
