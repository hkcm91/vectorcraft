//! The lighting and shadow parameters every 3D effect shares, and the shadow's art.
//!
//! Params (besides the main light's `lightAzimuth`, `lightElevation`, `lightIntensity`, `ambient`
//! and `shade`): `lightColor` ("#rrggbb"), `lightSoftness` (%), `lights` (up to three more:
//! `[{azimuth, elevation, intensity %, color, softness %}]`), `shadow` (none|below|behind),
//! `shadowDistance` (pt), `shadowOpacity` (%) and `shadowSoftness` (%); and the material:
//! `specular`, `roughness`, `metallic`, `rimLight` (%), `rimColor`, `shadingSteps` (0 or 2–8),
//! `ambientColor`, `environment` (what metal and gloss reflect) and `ambientOcclusion` (%); and
//! `smoothShading` and `contactShadow` (%). Options added later read as off for effects saved
//! before them ([`drawing_params`]).

use serde_json::{Value, json};
use std::sync::Arc;

use vectorcraft_doc::color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, PathData};
use vectorcraft_three_d::{Environment, Light, MAX_EXTRA_LIGHTS, MAX_SHADING_STEPS, Material, Shade, Shading, ShadowPosition};

use crate::util::{flag, num, text};

/// The lighting a 3D effect's params ask for beyond the main light's direction and intensity.
pub struct Lighting {
    pub light_color: [f64; 3],
    pub light_softness: f64,
    pub extra_lights: [Light; MAX_EXTRA_LIGHTS],
    pub extra_count: usize,
    pub shadow: ShadowPosition,
    pub shadow_distance: f64,
    pub material: Material,
}

/// A colour param as 0–1 RGB (white when missing or unreadable).
fn rgb(v: Option<&Value>) -> [f64; 3] {
    v.and_then(Value::as_str).and_then(Color::from_hex).map_or([1.0; 3], |c| c.to_rgb().map(f64::from))
}

fn field(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).filter(|f| f.is_finite()).unwrap_or(default)
}

impl Lighting {
    /// These settings on Revolve's defaults (the caller sets the rest).
    pub fn into_revolve(self) -> vectorcraft_three_d::Revolve {
        vectorcraft_three_d::Revolve {
            light_color: self.light_color,
            light_softness: self.light_softness,
            extra_lights: self.extra_lights,
            extra_count: self.extra_count,
            shadow: self.shadow,
            shadow_distance: self.shadow_distance,
            material: self.material,
            ..Default::default()
        }
    }
    /// These settings on the default 3D view (the caller sets the rest).
    pub fn into_view(self) -> vectorcraft_three_d::View {
        vectorcraft_three_d::View {
            light_color: self.light_color,
            light_softness: self.light_softness,
            extra_lights: self.extra_lights,
            extra_count: self.extra_count,
            shadow: self.shadow,
            shadow_distance: self.shadow_distance,
            material: self.material,
            ..Default::default()
        }
    }
}

pub fn lighting(p: &Value) -> Lighting {
    let mut extra_lights = [Light::default(); MAX_EXTRA_LIGHTS];
    let mut extra_count = 0;
    if let Some(list) = p.get("lights").and_then(Value::as_array) {
        for (slot, l) in extra_lights.iter_mut().zip(list.iter().filter(|l| l.is_object())) {
            *slot = Light {
                azimuth: field(l, "azimuth", 45.0),
                elevation: field(l, "elevation", 30.0),
                intensity: field(l, "intensity", 50.0) / 100.0,
                color: rgb(l.get("color")),
                softness: field(l, "softness", 0.0) / 100.0,
            };
            extra_count += 1;
        }
    }
    Lighting {
        light_color: rgb(p.get("lightColor")),
        light_softness: num(p, "lightSoftness", 0.0) / 100.0,
        extra_lights,
        extra_count,
        shadow: ShadowPosition::parse(text(p, "shadow", "none")).unwrap_or_default(),
        shadow_distance: num(p, "shadowDistance", 0.0),
        material: Material {
            specular: num(p, "specular", 0.0) / 100.0,
            roughness: num(p, "roughness", 50.0) / 100.0,
            metallic: num(p, "metallic", 0.0) / 100.0,
            rim: num(p, "rimLight", 0.0) / 100.0,
            rim_color: rgb(p.get("rimColor")),
            steps: num(p, "shadingSteps", 0.0).clamp(0.0, f64::from(MAX_SHADING_STEPS)) as u32,
            ambient_color: rgb(p.get("ambientColor")),
            smooth: flag(p, "smoothShading", false),
            occlusion: num(p, "ambientOcclusion", 0.0) / 100.0,
            environment: Environment::parse(text(p, "environment", "gradient")).unwrap_or_default(),
        },
    }
}

/// What a 3D effect saved before an option existed draws without it: the look it had then. New
/// effects store each option (their defaults turn it on).
fn legacy() -> [(&'static str, Value); 4] {
    [("smoothShading", json!(false)), ("ambientOcclusion", json!(0.0)), ("contactShadow", json!(0.0)), ("environment", json!("gradient"))]
}

/// A 3D effect's params for drawing: `raw` (as saved) over the effect's defaults, except that an
/// option saved before it existed keeps the look it had then ([`legacy`]).
pub fn drawing_params(id: &str, raw: &Value) -> Value {
    let mut p = crate::merged_params(id, raw);
    if let Some(o) = p.as_object_mut() {
        for (key, then) in legacy() {
            if raw.get(key).is_none() {
                o.insert(key.into(), then);
            }
        }
    }
    p
}

/// Material presets for UI work: (id, the params they set). Our own values. The dialog shows a
/// preset's name while every one of its params matches.
pub const MATERIAL_PRESETS: &[(&str, &[(&str, f64)])] = &[
    ("matte", &[("specular", 0.0), ("roughness", 50.0), ("metallic", 0.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("softTouch", &[("specular", 25.0), ("roughness", 70.0), ("metallic", 0.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("plastic", &[("specular", 55.0), ("roughness", 40.0), ("metallic", 0.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("glossy", &[("specular", 90.0), ("roughness", 20.0), ("metallic", 0.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("metal", &[("specular", 100.0), ("roughness", 35.0), ("metallic", 100.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("chrome", &[("specular", 100.0), ("roughness", 12.0), ("metallic", 100.0), ("rimLight", 0.0), ("shadingSteps", 0.0)]),
    ("rimGlow", &[("specular", 40.0), ("roughness", 35.0), ("metallic", 0.0), ("rimLight", 55.0), ("shadingSteps", 0.0)]),
    ("toon", &[("specular", 60.0), ("roughness", 30.0), ("metallic", 0.0), ("rimLight", 0.0), ("shadingSteps", 3.0)]),
];

/// One light of a lighting preset: azimuth, elevation (degrees), intensity (%), colour, softness (%).
pub type PresetLight = (f64, f64, f64, &'static str, f64);

/// A lighting preset: the main light, the ambient light and up to three more lights.
pub struct LightingPreset {
    pub id: &'static str,
    pub main: PresetLight,
    pub ambient: f64,
    pub ambient_color: &'static str,
    pub lights: &'static [PresetLight],
}

/// Lighting presets for UI work. Our own values.
pub const LIGHTING_PRESETS: &[LightingPreset] = &[
    LightingPreset { id: "standard", main: (-45.0, 45.0, 80.0, "#ffffff", 0.0), ambient: 25.0, ambient_color: "#ffffff", lights: &[] },
    LightingPreset {
        id: "studio",
        main: (-40.0, 40.0, 75.0, "#ffffff", 30.0),
        ambient: 22.0,
        ambient_color: "#ffffff",
        lights: &[(60.0, 10.0, 35.0, "#dbe6ff", 60.0), (160.0, 35.0, 60.0, "#ffffff", 0.0)],
    },
    LightingPreset {
        id: "soft",
        main: (-30.0, 55.0, 60.0, "#ffffff", 80.0),
        ambient: 40.0,
        ambient_color: "#ffffff",
        lights: &[(90.0, 20.0, 25.0, "#ffffff", 80.0)],
    },
    LightingPreset { id: "topLight", main: (0.0, 75.0, 85.0, "#ffffff", 20.0), ambient: 22.0, ambient_color: "#ffffff", lights: &[] },
    LightingPreset {
        id: "dramatic",
        // A hard key from the side; the shadow side kept readable (it showed near black with
        // little ambient and only a back light) by a cool ambient, a blue light wrapping round
        // the far edge and a faint fill.
        main: (-70.0, 25.0, 100.0, "#ffffff", 0.0),
        ambient: 14.0,
        ambient_color: "#c9d6ff",
        lights: &[(125.0, 20.0, 75.0, "#9fc4ff", 35.0), (50.0, 0.0, 15.0, "#c9d8ff", 50.0)],
    },
    LightingPreset {
        id: "warmCool",
        main: (-50.0, 35.0, 85.0, "#ffd2a1", 25.0),
        ambient: 25.0,
        ambient_color: "#a9c1ff",
        lights: &[(120.0, 15.0, 40.0, "#7aa2ff", 40.0)],
    },
    LightingPreset { id: "flat", main: (-30.0, 60.0, 35.0, "#ffffff", 100.0), ambient: 65.0, ambient_color: "#ffffff", lights: &[] },
];

impl LightingPreset {
    /// The params this preset sets.
    pub fn params(&self) -> serde_json::Map<String, Value> {
        let (az, el, i, c, s) = self.main;
        let lights: Vec<Value> =
            self.lights.iter().map(|(az, el, i, c, s)| json!({"azimuth": az, "elevation": el, "intensity": i, "color": c, "softness": s})).collect();
        let v = json!({
            "lightAzimuth": az, "lightElevation": el, "lightIntensity": i, "lightColor": c, "lightSoftness": s,
            "ambient": self.ambient, "ambientColor": self.ambient_color, "lights": lights,
        });
        v.as_object().cloned().unwrap_or_default()
    }

    /// Do `params` hold this preset's lighting (numbers to 0.005, colours ignoring case)?
    pub fn matches(&self, params: &serde_json::Map<String, Value>, defaults: &Value) -> bool {
        self.params().iter().all(|(k, v)| params.get(k).or_else(|| defaults.get(k)).is_some_and(|have| same(have, v)))
    }
}

/// Two param values alike: numbers to 0.005, colours ignoring case, lights light by light.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => (a.as_f64().unwrap_or(f64::NAN) - b.as_f64().unwrap_or(f64::NAN)).abs() < 0.005,
        (Value::String(x), Value::String(y)) => x.eq_ignore_ascii_case(y),
        (Value::Array(x), Value::Array(y)) => {
            let keys = ["azimuth", "elevation", "intensity", "color", "softness"];
            x.len() == y.len()
                && x.iter().zip(y).all(|(a, b)| keys.iter().all(|k| a.get(k).is_some_and(|av| b.get(k).is_some_and(|bv| same(av, bv)))))
        }
        _ => a == b,
    }
}

/// Check the lighting params: a bad shadow position, colour or light is an error, not a silent
/// default.
pub fn validate(p: &Value) -> Result<(), String> {
    if let Some(s) = p.get("shadow")
        && !s.as_str().is_some_and(|s| ShadowPosition::parse(s).is_some())
    {
        return Err("shadow must be none, below or behind".into());
    }
    let colour = |v: Option<&Value>, what: &str| match v {
        None => Ok(()),
        Some(v) if v.as_str().and_then(Color::from_hex).is_some() => Ok(()),
        Some(_) => Err(format!("{what} must be a colour such as \"#ffffff\"")),
    };
    colour(p.get("lightColor"), "lightColor")?;
    colour(p.get("rimColor"), "rimColor")?;
    colour(p.get("ambientColor"), "ambientColor")?;
    if let Some(v) = p.get("shadingSteps")
        && !v.as_f64().is_some_and(|n| (0.0..=f64::from(MAX_SHADING_STEPS)).contains(&n))
    {
        return Err(format!("shadingSteps must be 0 to {MAX_SHADING_STEPS} (2 or more: cel bands)"));
    }
    if let Some(e) = p.get("environment")
        && !e.as_str().is_some_and(|e| Environment::parse(e).is_some())
    {
        let names: Vec<_> = Environment::ALL.iter().map(|e| e.id()).collect();
        return Err(format!("environment must be one of {}", names.join(", ")));
    }
    match p.get("lights") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Array(list)) if list.len() <= MAX_EXTRA_LIGHTS => list.iter().try_for_each(|l| {
            if !l.is_object() {
                return Err("each of lights must be an object such as {\"azimuth\": 45, \"elevation\": 30}".to_string());
            }
            colour(l.get("color"), "a light's color")
        }),
        Some(_) => Err(format!("lights must be a list of at most {MAX_EXTRA_LIGHTS} lights")),
    }
}

/// The shadow's art: `path` filled black at the shadow's opacity, blurred by its softness
/// (relative to `size`, the object's size). It keeps the object's id, like the faces.
pub fn shadow_node(n: &Node, path: PathData, p: &Value, size: f64) -> Node {
    let opacity = (num(p, "shadowOpacity", 50.0) / 100.0).clamp(0.0, 1.0) as f32;
    let softness = (num(p, "shadowSoftness", 40.0) / 100.0).clamp(0.0, 1.0);
    let radius = (softness * 0.12 * size).clamp(0.0, 250.0);
    let effects =
        if radius > 0.05 { vec![Effect { id: "blur.gaussian".into(), params: json!({ "radius": radius }), visible: true }] } else { vec![] };
    let items = vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK)))];
    let mut node = Node::path(n.id, path, Appearance { items, effects, ..Default::default() });
    node.opacity = opacity;
    node
}

/// A face's colour: `paint`'s colour times the shading `tint` plus its `highlight`, channel by
/// channel, at most white.
pub fn shaded(c: Color, tint: [f64; 3], highlight: [f64; 3]) -> Paint {
    let [r, g, b] = c.to_rgb();
    let ch = |c: f32, t: f64, h: f64| (c * t as f32 + h as f32).clamp(0.0, 1.0);
    Paint::solid(Color::rgb(ch(r, tint[0], highlight[0]), ch(g, tint[1], highlight[1]), ch(b, tint[2], highlight[2])))
}

/// A smooth-shaded surface in `paint`, drawn pixel by pixel ([`Shading::image`]) and clipped to
/// the outline its faces cover, as a clip group keeping `id`. A flat colour is shaded straight
/// into the image (as [`shaded`] colours it); another paint (a gradient, a pattern) fills the
/// outline, multiplied by an image of the shading's tint and screened by one of its highlight.
/// The pixels go to [`vectorcraft_doc::generated`]. `None` when the surface has no area.
pub fn shaded_surface(id: NodeId, shading: &Shading, paint: &Paint) -> Option<Node> {
    let image = |img: vectorcraft_three_d::ShadedImage, blend: BlendMode| -> Option<Arc<Node>> {
        let (width, height) = (img.width, img.height);
        let key = vectorcraft_doc::generated::put(width, height, img.rgba)?;
        let xf = Affine::translate(img.origin.to_vec2()) * Affine::scale(img.step);
        let mut node = Node::new(id, NodeKind::Image(Box::new(ImageObject { key, width, height, xf, link: None, placement: Default::default() })));
        node.blend = blend;
        Some(Arc::new(node))
    };
    let outline = shading.outline();
    let mut clip = Node::path(id, outline.clone(), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let children = match paint.color() {
        Some(c) => {
            let [r, g, b] = c.to_rgb();
            let colour = |s: &Shade| {
                let ch = |c: f32, t: f64, h: f64| (c * t as f32 + h as f32).clamp(0.0, 1.0);
                [ch(r, s.tint[0], s.highlight[0]), ch(g, s.tint[1], s.highlight[1]), ch(b, s.tint[2], s.highlight[2])]
            };
            vec![Arc::new(clip), image(shading.image(&colour)?, BlendMode::Normal)?]
        }
        None => {
            let tint = |s: &Shade| s.tint.map(|t| t.clamp(0.0, 1.0) as f32);
            let highlight = |s: &Shade| s.highlight.map(|h| h.clamp(0.0, 1.0) as f32);
            let mut layers = shading.layers(&[&tint, &highlight])?.into_iter();
            let fill = Node::path(id, outline, Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint.clone()))], ..Default::default() });
            vec![Arc::new(clip), Arc::new(fill), image(layers.next()?, BlendMode::Multiply)?, image(layers.next()?, BlendMode::Screen)?]
        }
    };
    let mut group = Node::new(id, NodeKind::Group { children, clip: true });
    // The multiply and the screen act on the paint alone, not on what lies behind.
    group.isolate = true;
    Some(group)
}

/// The shadows a 3D effect draws under its surface: the cast `shadow`, then the `contact` shadow
/// (an outline and how strongly it shows) where the object meets the plane, dark and tight. `size`
/// is the object's size, which the blurs follow.
pub fn shadow_nodes(n: &Node, shadow: Option<PathData>, contact: Option<(PathData, f64)>, p: &Value, size: f64) -> Vec<Arc<Node>> {
    let mut out: Vec<Arc<Node>> = shadow.map(|s| Arc::new(shadow_node(n, s, p, size))).into_iter().collect();
    let amount = (num(p, "contactShadow", 0.0) / 100.0).clamp(0.0, 1.0);
    if let Some((path, strength)) = contact
        && amount > 0.0
    {
        let radius = (0.03 * size).clamp(0.0, 250.0);
        let effects =
            if radius > 0.05 { vec![Effect { id: "blur.gaussian".into(), params: json!({ "radius": radius }), visible: true }] } else { vec![] };
        let items = vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::BLACK)))];
        let mut node = Node::path(n.id, path, Appearance { items, effects, ..Default::default() });
        node.opacity = (0.85 * amount * strength.clamp(0.0, 1.0)) as f32;
        out.push(Arc::new(node));
    }
    out
}

/// The fill layers that shade a paint that isn't a flat colour (a gradient, a pattern): the paint,
/// darkened by black at `1 − brightness`, then the highlight's colour at its strength.
pub fn shading_layers(paint: &Paint, brightness: f64, highlight: [f64; 3]) -> Vec<AppearanceItem> {
    let mut items = vec![AppearanceItem::Fill(FillLayer::new(paint.clone()))];
    let mut dark = FillLayer::new(Paint::solid(Color::BLACK));
    dark.opacity = (1.0 - brightness).clamp(0.0, 1.0) as f32;
    items.push(AppearanceItem::Fill(dark));
    let peak = highlight.iter().copied().fold(0.0, f64::max);
    if peak > 1e-3 {
        let [r, g, b] = highlight.map(|h| (h / peak) as f32);
        let mut light = FillLayer::new(Paint::solid(Color::rgb(r, g, b)));
        light.opacity = peak.clamp(0.0, 1.0) as f32;
        items.push(AppearanceItem::Fill(light));
    }
    items
}
