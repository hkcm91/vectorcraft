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

/// Which way the shadow is cast onto its plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowDirection {
    /// Along the main light: it falls away from the light, longer as the light is lower.
    #[default]
    Light,
    /// Straight onto the plane, whatever the lights: the object's own footprint, a soft shadow
    /// right under it (or right behind it), as UI icons have.
    Straight,
}

impl ShadowDirection {
    pub fn id(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Straight => "straight",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Light, Self::Straight].into_iter().find(|d| d.id() == s)
    }
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
    /// Shade curved surfaces smoothly and give flat faces a gentle falloff (pixel by pixel)
    /// instead of one flat colour per face.
    pub smooth: bool,
    /// How much creases and hollows darken (ambient occlusion, with smooth shading).
    pub occlusion: f64,
    /// What the surface reflects: a metal's colour, and a glossy surface's sheen at grazing angles.
    pub environment: Environment,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            specular: 0.0,
            roughness: 0.5,
            metallic: 0.0,
            rim: 0.0,
            rim_color: [1.0; 3],
            steps: 0,
            ambient_color: [1.0; 3],
            smooth: false,
            occlusion: 0.0,
            environment: Environment::Gradient,
        }
    }
}

/// The surroundings a surface reflects, our own procedural backdrops. Directions are view space
/// (y down, z towards the viewer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Environment {
    /// Bright overhead fading to dark underfoot, one grey (what effects saved before the other
    /// backdrops reflect). Glossy surfaces reflect nothing of it.
    #[default]
    Gradient,
    /// A light grey studio sweep with a darker floor, a large softbox above the key light, a tall
    /// strip on the right, a light ceiling and a soft fill behind the camera.
    Studio,
    /// Out of doors: a blue sky paling to the horizon over dark ground (classic chrome).
    Outdoor,
    /// A warm horizon under a violet sky, the sun low on the left.
    Sunset,
    /// Black, with thin bright strips either side and overhead: edges light up.
    Dark,
}

impl Environment {
    pub const ALL: [Self; 5] = [Self::Gradient, Self::Studio, Self::Outdoor, Self::Sunset, Self::Dark];

    pub fn id(self) -> &'static str {
        match self {
            Self::Gradient => "gradient",
            Self::Studio => "studio",
            Self::Outdoor => "outdoor",
            Self::Sunset => "sunset",
            Self::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.id() == s)
    }

    /// The light arriving from direction `r` (a unit vector), linear RGB; its features soften
    /// over `blur` (radians, from the roughness).
    pub(crate) fn radiance(self, r: V3, blur: f64) -> [f64; 3] {
        let up = -r.1;
        // 0 below `edge − w`, 1 above `edge + w`, smooth between.
        let step = |x: f64, edge: f64, w: f64| {
            let t = ((x - edge + w) / (2.0 * w)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let w = blur.max(0.01);
        let angle = |d: V3| r.dot(d.unit()).clamp(-1.0, 1.0).acos();
        let azimuth = r.0.atan2(r.2);
        let mix = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] { std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t) };
        match self {
            Self::Gradient => [0.3 + 0.7 * (0.5 + 0.5 * up).clamp(0.0, 1.0); 3],
            Self::Studio => {
                // A light grey sweep, a darker floor, the lamps brighter than white.
                let walls = 0.55 + 0.15 * up.max(0.0);
                let floor = mix([0.28; 3], [walls; 3], step(up, -0.05, w.max(0.08)));
                let softbox = 1.0 * (1.0 - step(angle(V3(-0.5, -0.7, 0.5)), 0.45, w));
                let strip = 0.7 * (1.0 - step((azimuth - 1.2).abs(), 0.14, w)) * (1.0 - step(up.abs(), 0.6, w));
                let ceiling = 0.4 * step(up, 0.85, w);
                // A broad soft fill behind the camera: what a face turned to the viewer mirrors.
                let front = 0.45 * step(r.2, 0.55, 0.25);
                floor.map(|f| f + softbox + strip + ceiling + front)
            }
            Self::Outdoor => {
                let sky = mix([1.0, 1.0, 1.0], [0.35, 0.55, 0.95], up.max(0.0).sqrt());
                let ground = mix([0.42, 0.38, 0.33], [0.2, 0.18, 0.16], (-up).max(0.0).sqrt());
                mix(ground, sky, step(up, 0.0, w))
            }
            Self::Sunset => {
                let sky = mix([1.1, 0.6, 0.3], [0.25, 0.2, 0.45], up.max(0.0).sqrt());
                let ground = mix([0.3, 0.17, 0.15], [0.12, 0.08, 0.1], (-up).max(0.0).sqrt());
                let sun = 1.5 * (1.0 - step(angle(V3(-0.8, -0.1, 0.6)), 0.12, w));
                mix(ground, sky, step(up, 0.0, w)).map(|c| c + sun * if up > -0.1 { 1.0 } else { 0.0 })
            }
            Self::Dark => {
                let side = |centre: f64| 1.2 * (1.0 - step((azimuth - centre).abs(), 0.06, w)) * (1.0 - step(up.abs(), 0.7, w));
                let top = 0.8 * step(up, 0.95, w);
                [0.02 + side(-1.4) + side(1.4) + top; 3]
            }
        }
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
            occlusion: finite(self.occlusion, 0.0, 0.0, 1.0),
            environment: self.environment,
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

/// Unshaded: the paint as it is.
impl Default for Shade {
    fn default() -> Self {
        Self::NONE
    }
}

/// Every light of a view, ready to shade with.
#[derive(Clone, Debug, PartialEq)]
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

    /// The same lights on a matte finish: no highlight, no metal, no rim light (cel bands and
    /// occlusion kept), as art printed on the surface takes them.
    pub(crate) fn matte(&self) -> Self {
        let material = Material { specular: 0.0, metallic: 0.0, rim: 0.0, ..self.material };
        Self { material, ..self.clone() }
    }

    /// Does the material shade in cel bands?
    pub(crate) fn banded(&self) -> bool {
        self.material.steps >= 2
    }

    /// How a face with unit normal `n` (view space, towards the viewer) is shaded, the lights
    /// and the eye infinitely far away.
    pub(crate) fn shade(&self, n: V3) -> Shade {
        self.shade_with(n, |d| d, V3(0.0, 0.0, 1.0), 1.0)
    }

    /// How the point `p` (view space, about the object's centre) with unit normal `n` is shaded,
    /// the lights and the eye `reach` from the centre.
    pub(crate) fn shade_at(&self, n: V3, p: V3, reach: f64) -> Shade {
        self.shade_open(n, p, reach, 1.0)
    }

    /// [`Self::shade_at`] where only `open` (0–1) of the surroundings reach the point: the rest
    /// is hidden by the surface itself (a crease, a hollow). The ambient light and reflections
    /// dim by it, the lights by half as much.
    pub(crate) fn shade_open(&self, n: V3, p: V3, reach: f64, open: f64) -> Shade {
        let towards = |d: V3| V3(d.0 * reach - p.0, d.1 * reach - p.1, d.2 * reach - p.2).unit();
        self.shade_with(n, towards, V3(-p.0, -p.1, reach - p.2).unit(), open)
    }

    /// The material's ambient occlusion strength, 0 when it doesn't darken creases.
    pub(crate) fn occlusion(&self) -> f64 {
        if self.on { self.material.occlusion } else { 0.0 }
    }

    /// The shade of normal `n`, each light's direction `d` turned into the direction towards it by
    /// `towards`, seen from direction `view`.
    fn shade_with(&self, n: V3, towards: impl Fn(V3) -> V3, view: V3, open: f64) -> Shade {
        if !self.on {
            return Shade::NONE;
        }
        let m = &self.material;
        let shininess = m.shininess();
        let open = open.clamp(0.0, 1.0);
        let lit = 0.5 + 0.5 * open;
        // A metal's own colour comes from its reflections, not from diffuse light.
        let diffuse_share = 1.0 - 0.85 * m.metallic;
        let mut diffuse = m.ambient_color.map(|c| self.ambient * c * open);
        let mut spec = [0.0; 3];
        for (dir, intensity, color, softness) in &self.lights {
            let dir = &towards(*dir);
            let wrap = ((n.dot(*dir) + softness) / (1.0 + softness)).max(0.0);
            for (ch, k) in diffuse.iter_mut().zip(color) {
                *ch += intensity * k * wrap * lit;
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
        if m.steps >= 2 {
            // Cel bands: the light all the lights give together, cut into flat bands between the
            // ambient light alone and full light, its colour kept. (Banding each light on its own
            // would stack their bands' edges into a patchwork.)
            let luma = |c: &[f64; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
            let floor = (luma(&m.ambient_color) * self.ambient * open).clamp(0.0, 0.99);
            let total = luma(&diffuse);
            if total > 1e-9 {
                let t = ((total - floor) / (1.0 - floor)).clamp(0.0, 1.0);
                let banded = floor + band(t, m.steps) * (1.0 - floor);
                diffuse = diffuse.map(|c| c * banded / total);
            }
        }
        let mut tint = diffuse.map(|v| v.clamp(0.0, 1.0) * diffuse_share);
        // What the surface mirrors: the view reflected about the normal, seen in the
        // environment, blurred by the roughness.
        let nv = n.dot(view);
        let r = V3(2.0 * nv * n.0 - view.0, 2.0 * nv * n.1 - view.1, 2.0 * nv * n.2 - view.2);
        let blur = 0.02 + 0.6 * m.roughness * m.roughness;
        let mirrored = || m.environment.radiance(r, blur).map(|c| band(c.clamp(0.0, 1.5), m.steps) * open);
        if m.metallic > 0.0 {
            let reflection = mirrored();
            for ((t, s), e) in tint.iter_mut().zip(spec).zip(reflection) {
                *t += m.metallic * (e + s);
            }
        }
        let mut highlight = spec.map(|s| s * (1.0 - m.metallic));
        if m.environment != Environment::Gradient && m.specular > 0.0 && m.metallic < 1.0 && m.steps < 2 {
            // A glossy surface mirrors its surroundings faintly, most at grazing angles (Schlick's
            // Fresnel term for a plastic), less the rougher it is. (Not in cel bands: it would
            // only add patches.)
            let fresnel = 0.04 + 0.96 * (1.0 - nv.clamp(0.0, 1.0)).powi(5);
            let share = m.specular * (1.0 - m.metallic) * (1.0 - m.roughness).powi(2) * (0.08 + 0.6 * fresnel);
            for (h, e) in highlight.iter_mut().zip(mirrored()) {
                *h += share * e;
            }
        }
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

/// The shadow `faces` (each a face's view-space contours) cast along the light towards `light` (or
/// straight, as `direction` says) on the plane at `position`, `distance` beyond the object,
/// projected by `project`: one outline whose nonzero fill is the shadow. None when there is no
/// shadow plane or the light grazes it or lies beyond it.
pub(crate) fn cast(
    faces: &[Vec<Vec<V3>>],
    light: V3,
    direction: ShadowDirection,
    position: ShadowPosition,
    distance: f64,
    project: impl Fn(V3) -> Point,
) -> Option<PathData> {
    let n = shadow_plane(position)?;
    let light = if direction == ShadowDirection::Straight { n } else { light };
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

/// How far above the shadow's plane a contact shadow gathers the object, as a share of its size.
const CONTACT_BAND: f64 = 0.08;

/// The contact shadow: the parts of `faces` (view-space contours) within a thin band of the plane
/// the shadow falls on (at `position`, `distance` beyond the object), dropped straight onto it and
/// projected by `project`: the footprint where the object meets the ground, as one outline whose
/// nonzero fill is the shadow, and how strongly it shows (1 resting on the plane, fading to 0 as
/// the object lifts off it by the band). None without a plane or with the object clear of it.
pub(crate) fn contact(faces: &[Vec<Vec<V3>>], position: ShadowPosition, distance: f64, project: impl Fn(V3) -> Point) -> Option<(PathData, f64)> {
    let n = shadow_plane(position)?;
    let points = faces.iter().flatten().flatten();
    let lowest = points.clone().map(|v| v.dot(n)).fold(f64::INFINITY, f64::min);
    let (lo, hi) = points.fold((V3(f64::MAX, f64::MAX, f64::MAX), V3(f64::MIN, f64::MIN, f64::MIN)), |(a, b), v| {
        (V3(a.0.min(v.0), a.1.min(v.1), a.2.min(v.2)), V3(b.0.max(v.0), b.1.max(v.1), b.2.max(v.2)))
    });
    if !lowest.is_finite() {
        return None;
    }
    let size = (hi.0 - lo.0).max(hi.1 - lo.1).max(hi.2 - lo.2).max(1e-9);
    let band = CONTACT_BAND * size;
    let distance = if distance.is_finite() { distance.max(0.0) } else { 0.0 };
    let strength = 1.0 - distance / band;
    if strength <= 0.0 {
        return None;
    }
    let (plane, top) = (lowest - distance, lowest + band);
    let mut bp = BezPath::new();
    let mut count = 0usize;
    for face in faces {
        // Each contour cut to the band (Sutherland–Hodgman against one plane), then dropped.
        let dropped: Vec<Vec<Point>> = face
            .iter()
            .map(|c| {
                let mut out = vec![];
                for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
                    let (da, db) = (top - a.dot(n), top - b.dot(n));
                    if da >= 0.0 {
                        out.push(*a);
                    }
                    if (da >= 0.0) != (db >= 0.0) {
                        let t = da / (da - db);
                        out.push(V3(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t));
                    }
                }
                out.iter()
                    .map(|v| {
                        let h = v.dot(n) - plane;
                        project(V3(v.0 - n.0 * h, v.1 - n.1 * h, v.2 - n.2 * h))
                    })
                    .collect()
            })
            .filter(|c: &Vec<Point>| c.len() >= 3)
            .collect();
        count += dropped.iter().map(Vec::len).sum::<usize>();
        if count > MAX_SHADOW_POINTS {
            return None;
        }
        let outer = dropped.iter().map(|c| area(c)).max_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0);
        if outer.abs() < 1e-9 {
            continue;
        }
        for c in &dropped {
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
    (!bp.elements().is_empty()).then(|| (PathData::from_bezpath(&bp), strength))
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
        // With several lights the bands are still that many: the light they give together is
        // banded, not each light on its own (their bands' edges would stack into a patchwork).
        let fill = Light { azimuth: 70.0, elevation: 20.0, intensity: 0.5, ..Light::default() };
        let back = Light { azimuth: 160.0, elevation: 35.0, intensity: 0.6, ..Light::default() };
        let rig = Rig::new(main, &[fill, back], 0.2, cel, true);
        let mut tones: Vec<i64> = vec![];
        for i in -20..=20 {
            for j in -20..=20 {
                let (x, y) = (f64::from(i) / 20.0, f64::from(j) / 20.0);
                if x * x + y * y < 1.0 {
                    let t = rig.shade(V3(x, y, (1.0 - x * x - y * y).sqrt())).tint;
                    tones.push((1e6 * (t[0] + t[1] + t[2])).round() as i64);
                }
            }
        }
        tones.sort_unstable();
        tones.dedup();
        assert!(tones.len() <= 3, "{} tones", tones.len());
        assert_eq!(Material { steps: 1, ..cel }.sanitized().steps, 0);
        assert_eq!(Material { steps: 99, ..cel }.sanitized().steps, MAX_SHADING_STEPS);
    }

    #[test]
    fn shadows_fall_below_or_behind_and_need_the_light_on_the_right_side() {
        // A unit square facing the viewer, 10 above the origin.
        let square = vec![vec![V3(-1.0, -11.0, 0.0), V3(1.0, -11.0, 0.0), V3(1.0, -9.0, 0.0), V3(-1.0, -9.0, 0.0)]];
        let flat = |v: V3| Point::new(v.0, v.1);
        let along = ShadowDirection::Light;
        let above = direction(0.0, 60.0);
        let below = cast(std::slice::from_ref(&square), above, along, ShadowPosition::Below, 5.0, flat).unwrap();
        let b = below.bounds().unwrap();
        assert!(b.y0 > -9.0, "the shadow is under the square: {b:?}");
        assert!(
            cast(std::slice::from_ref(&square), direction(0.0, -30.0), along, ShadowPosition::Below, 0.0, flat).is_none(),
            "a light from below casts none down"
        );
        let behind = cast(std::slice::from_ref(&square), direction(30.0, 30.0), along, ShadowPosition::Behind, 10.0, flat).unwrap();
        let b = behind.bounds().unwrap();
        assert!(b.x0 < -1.0 && b.y0 > -11.0, "pushed away from a light at the upper right: {b:?}");
        assert!(cast(std::slice::from_ref(&square), direction(0.0, 0.0), along, ShadowPosition::None, 0.0, flat).is_none());
        // Cast straight, the shadow ignores the light: right under the square, as wide as it, even
        // with the light below; and straight behind it, exactly where it is.
        let straight = ShadowDirection::Straight;
        let b = cast(std::slice::from_ref(&square), direction(70.0, -30.0), straight, ShadowPosition::Below, 5.0, flat).unwrap().bounds().unwrap();
        assert!(b.y0 > -9.0 && (b.x0 + 1.0).abs() < 1e-9 && (b.x1 - 1.0).abs() < 1e-9, "{b:?}");
        let b = cast(&[square], direction(30.0, 30.0), straight, ShadowPosition::Behind, 10.0, flat).unwrap().bounds().unwrap();
        assert!((b.x0 + 1.0).abs() < 1e-9 && (b.y0 + 11.0).abs() < 1e-9 && (b.x1 - 1.0).abs() < 1e-9 && (b.y1 + 9.0).abs() < 1e-9, "{b:?}");
        assert_eq!(ShadowDirection::parse("straight"), Some(straight));
        assert_eq!(ShadowDirection::parse("light"), Some(along));
        assert_eq!(ShadowDirection::parse("down"), None);
    }

    #[test]
    fn backdrops_light_a_metal_and_give_gloss_a_sheen() {
        let main = Light::default();
        let up = V3(0.0, -1.0, 1.0).unit();
        let down = V3(0.0, 1.0, 1.0).unit();
        // Every backdrop is finite and lit somewhere, over the whole sphere of directions.
        for e in Environment::ALL {
            let mut brightest: f64 = 0.0;
            for i in 0..24 {
                for j in 0..12 {
                    let (a, b) = (i as f64 * 15f64.to_radians(), (j as f64 * 15.0 - 82.5).to_radians());
                    let c = e.radiance(V3(b.cos() * a.sin(), -b.sin(), b.cos() * a.cos()), 0.05);
                    assert!(c.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 3.0), "{e:?} {c:?}");
                    brightest = brightest.max(c[0]).max(c[1]).max(c[2]);
                }
            }
            assert!(brightest > 0.8, "{e:?}");
            assert_eq!(Environment::parse(e.id()), Some(e));
        }
        // Out of doors the sky above is blue and the ground below darker.
        let sky = Environment::Outdoor.radiance(V3(0.0, -0.7, 0.7).unit(), 0.05);
        let ground = Environment::Outdoor.radiance(V3(0.0, 0.7, 0.7).unit(), 0.05);
        assert!(sky[2] > sky[0] && sky[2] > ground[2]);
        // A metal reflects its backdrop: brighter facing the studio's softbox than the floor.
        let metal = Material { specular: 1.0, roughness: 0.2, metallic: 1.0, environment: Environment::Studio, ..Material::default() };
        let rig = Rig::new(main, &[], 0.2, metal, true);
        assert!(rig.shade(up).tint[0] > rig.shade(down).tint[0]);
        // The plain gradient is the old studio reflection: glossy plastic mirrors nothing of it…
        let plastic = Material { specular: 0.9, roughness: 0.2, ..Material::default() };
        let grazing = V3(0.95, 0.0, 0.3).unit();
        assert!(Rig::new(main, &[], 0.2, plastic, true).shade(grazing).highlight.iter().all(|h| *h < 1e-9));
        // …a real backdrop gives it a sheen, fainter the rougher it is.
        let unlit = Light { intensity: 0.0, ..main };
        let sheen = |roughness: f64| {
            let rig = Rig::new(unlit, &[], 0.2, Material { environment: Environment::Studio, roughness, ..plastic }, true);
            (rig.shade(V3(0.0, 0.0, 1.0)).highlight[0], rig.shade(grazing).highlight[0])
        };
        let (face_on, edge) = sheen(0.2);
        assert!(face_on > 0.0 && edge > 0.0, "{face_on} {edge}");
        assert!(sheen(0.9).0 < face_on);
    }

    #[test]
    fn occlusion_dims_the_ambient_light_and_reflections() {
        let main = Light { azimuth: 0.0, elevation: 0.0, intensity: 0.5, ..Light::default() };
        let rig = Rig::new(main, &[], 0.4, Material::default(), true);
        let n = V3(0.0, 0.0, 1.0);
        let open = rig.shade_open(n, V3::default(), 100.0, 1.0).tint[0];
        let hidden = rig.shade_open(n, V3::default(), 100.0, 0.0).tint[0];
        assert!((open - rig.shade_at(n, V3::default(), 100.0).tint[0]).abs() < 1e-12);
        // Open: ambient 0.4 and the light head-on at 0.5. Fully hidden: no ambient light, the
        // light at half strength.
        assert!((open - 0.9).abs() < 1e-9 && (hidden - 0.25).abs() < 1e-9, "{open} {hidden}");
    }

    #[test]
    fn a_contact_shadow_gathers_what_rests_on_the_ground() {
        // A cube of side 10 standing on the wall plane behind it (z from 0 to 10).
        let face = |pts: [(f64, f64, f64); 4]| vec![pts.iter().map(|(x, y, z)| V3(*x, *y, *z)).collect::<Vec<_>>()];
        let cube = vec![
            face([(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0), (0.0, 10.0, 0.0)]),
            face([(0.0, 0.0, 10.0), (10.0, 0.0, 10.0), (10.0, 10.0, 10.0), (0.0, 10.0, 10.0)]),
            face([(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 0.0, 10.0), (0.0, 0.0, 10.0)]),
        ];
        let flat = |v: V3| Point::new(v.0, v.1);
        let (path, strength) = contact(&cube, ShadowPosition::Behind, 0.0, flat).unwrap();
        assert_eq!(strength, 1.0);
        let b = path.bounds().unwrap();
        assert!((b.width() - 10.0).abs() < 1e-9 && (b.height() - 10.0).abs() < 1e-9, "the footprint: {b:?}");
        // Lifted off the plane it fades, and beyond the band it is gone.
        let (_, half) = contact(&cube, ShadowPosition::Behind, 0.4, flat).unwrap();
        assert!((half - 0.5).abs() < 1e-9, "{half}");
        assert!(contact(&cube, ShadowPosition::Behind, 1.0, flat).is_none());
        assert!(contact(&cube, ShadowPosition::None, 0.0, flat).is_none());
    }
}
