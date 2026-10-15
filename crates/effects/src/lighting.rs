//! The lighting and shadow parameters every 3D effect shares, and the shadow's art.
//!
//! Params (besides the main light's `lightAzimuth`, `lightElevation`, `lightIntensity`, `ambient`
//! and `shade`): `lightColor` ("#rrggbb"), `lightSoftness` (%), `lights` (up to three more:
//! `[{azimuth, elevation, intensity %, color, softness %}]`), `shadow` (none|below|behind),
//! `shadowDistance` (pt), `shadowOpacity` (%) and `shadowSoftness` (%).

use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, Node};
use vectorcraft_geom::PathData;
use vectorcraft_three_d::{Light, MAX_EXTRA_LIGHTS, ShadowPosition};

use crate::util::{num, text};

/// The lighting a 3D effect's params ask for beyond the main light's direction and intensity.
pub struct Lighting {
    pub light_color: [f64; 3],
    pub light_softness: f64,
    pub extra_lights: [Light; MAX_EXTRA_LIGHTS],
    pub extra_count: usize,
    pub shadow: ShadowPosition,
    pub shadow_distance: f64,
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

/// A face's colour: `paint`'s colour times the shading `tint`, channel by channel.
pub fn tinted(c: Color, tint: [f64; 3]) -> Paint {
    let [r, g, b] = c.to_rgb();
    let [tr, tg, tb] = tint.map(|t| t as f32);
    Paint::solid(Color::rgb(r * tr, g * tg, b * tb))
}
