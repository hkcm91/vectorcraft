//! Smooth shading: a normal at every vertex of every face, for [`crate::raster`] to interpolate
//! and shade pixel by pixel.
//!
//! A vertex's normal is the area-weighted mean of the faces meeting there whose normals are within
//! [`CREASE`] of the face being shaded, so curved surfaces read as smooth while edges, corners
//! and the cap of a solid stay crisp. It is averaged twice (the faces' smoothed normals averaged
//! again), so the normals vary as evenly as the surface: a tight highlight shows the least
//! unevenness.

use std::collections::HashMap;

use crate::V3;
use crate::lighting::{Rig, Shade};

/// The largest angle between two faces that still shades as one smooth surface (cosine of 50°).
const CREASE: f64 = 0.642_787_609_686_539_3;
/// The most vertices the pass shades; beyond it faces stay flat.
const MAX_VERTICES: usize = 600_000;

/// One face to shade: its view-space contours, the unit normal it is shaded with, and its vertex
/// normals when the surface knows them exactly (they are then kept, not averaged).
pub(crate) struct Source<'a> {
    pub contours: &'a [Vec<V3>],
    pub normal: V3,
    pub given: Option<&'a [V3]>,
}

/// The faces meeting at each vertex (by its rounded position): the normal to judge creases by,
/// the normal to average, and the face's area.
type Incident = HashMap<(i64, i64, i64), Vec<(V3, V3, f64)>>;

/// Twice the area of a planar 3D polygon.
fn area(c: &[V3]) -> f64 {
    let mut s = V3::default();
    for (a, b) in c.iter().zip(c.iter().cycle().skip(1)).take(c.len()) {
        let x = a.cross(*b);
        s = V3(s.0 + x.0, s.1 + x.1, s.2 + x.2);
    }
    s.dot(s).sqrt()
}

/// A face's smoothed vertex normals (its contours' vertices in order) and its mean shade.
pub(crate) struct Smoothed {
    pub normals: Vec<V3>,
    pub shade: Shade,
}

/// Each face's smoothed vertex normals and mean shade, `None` when there are too many vertices
/// (the faces then shade flat). `hidden` are the solid's other faces (contours and outward
/// normal), which round off the normals at its silhouette. `reach` is how far the lights and the
/// eye are from the object's centre.
pub(crate) fn shade(faces: &[Source], hidden: &[(Vec<Vec<V3>>, V3)], rig: &Rig, reach: f64) -> Option<Vec<Smoothed>> {
    let total: usize = faces.iter().map(|f| f.contours.iter().map(Vec::len).sum::<usize>()).sum();
    if total > MAX_VERTICES || !reach.is_finite() || reach <= 0.0 {
        return None;
    }
    // Vertices closer than this (a ten-thousandth of the object's half extent) are one.
    let q = reach / 6.0 * 1e-4;
    let key = |v: V3| ((v.0 / q).round() as i64, (v.1 / q).round() as i64, (v.2 / q).round() as i64);
    let weight = |contours: &[Vec<V3>]| contours.iter().map(|c| area(c)).sum::<f64>().max(1e-12);
    // Every face meeting at each vertex: its normal (twice: to judge creases by, and to average),
    // weighted by its area.
    let mut at: Incident = HashMap::new();
    let all = faces.iter().map(|f| (f.contours, f.normal)).chain(hidden.iter().map(|(c, n)| (c.as_slice(), *n)));
    for (contours, normal) in all {
        let w = weight(contours);
        for v in contours.iter().flatten() {
            at.entry(key(*v)).or_default().push((normal, normal, w));
        }
    }
    // A vertex's normal: the faces meeting there that don't cross a crease from `normal`, each
    // contributing `value` (its own normal, or on the second pass its smoothed one), by area.
    let vertex = |at: &Incident, v: V3, normal: V3| -> V3 {
        let mut n = V3::default();
        for (m, value, w) in at.get(&key(v)).map(Vec::as_slice).unwrap_or_default() {
            // The other side of a two-sided surface counts as the same face.
            let flip = m.dot(normal) < -CREASE;
            let m = if flip { V3(-m.0, -m.1, -m.2) } else { *m };
            if m.dot(normal) >= CREASE {
                let s = if flip { -w } else { *w };
                n = V3(n.0 + value.0 * s, n.1 + value.1 * s, n.2 + value.2 * s);
            }
        }
        if n.dot(n) > 1e-24 { n.unit() } else { normal }
    };
    let first = |f: &Source| -> V3 {
        let sum = f.contours.iter().flatten().fold(V3::default(), |s, v| {
            let n = vertex(&at, *v, f.normal);
            V3(s.0 + n.0, s.1 + n.1, s.2 + n.2)
        });
        if sum.dot(sum) > 1e-24 { sum.unit() } else { f.normal }
    };
    let mut again: Incident = HashMap::new();
    for f in faces {
        let (value, w) = (first(f), weight(f.contours));
        for v in f.contours.iter().flatten() {
            again.entry(key(*v)).or_default().push((f.normal, value, w));
        }
    }
    for (contours, normal) in hidden {
        let w = weight(contours);
        for v in contours.iter().flatten() {
            again.entry(key(*v)).or_default().push((*normal, *normal, w));
        }
    }
    Some(
        faces
            .iter()
            .map(|f| {
                let count = f.contours.iter().map(Vec::len).sum::<usize>();
                let normals: Vec<V3> = match f.given {
                    Some(given) if given.len() == count => given.to_vec(),
                    _ => f.contours.iter().flatten().map(|v| vertex(&again, *v, f.normal)).collect(),
                };
                let shades: Vec<Shade> = f.contours.iter().flatten().zip(&normals).map(|(v, n)| rig.shade_at(*n, *v, reach)).collect();
                let shade = mean_shade(&shades).unwrap_or_else(|| rig.shade(f.normal));
                Smoothed { normals, shade }
            })
            .collect(),
    )
}

/// The mean of some shades, `None` for none.
fn mean_shade(shades: &[Shade]) -> Option<Shade> {
    if shades.is_empty() {
        return None;
    }
    let k = shades.len() as f64;
    let sum = |f: &dyn Fn(&Shade) -> [f64; 3]| -> [f64; 3] {
        shades.iter().map(f).fold([0.0; 3], |a, b| [a[0] + b[0] / k, a[1] + b[1] / k, a[2] + b[2] / k])
    };
    Some(Shade { tint: sum(&|s| s.tint), highlight: sum(&|s| s.highlight) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{Light, Material};

    fn rig() -> Rig {
        Rig::new(Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() }, &[], 0.0, Material::default(), true)
    }

    #[test]
    fn adjacent_faces_share_a_smooth_normal_but_not_across_a_crease() {
        // Two faces of a gentle fold (20° apart) and one at a right angle.
        let a = vec![vec![V3(-1.0, 0.0, 0.0), V3(0.0, 0.0, 0.0), V3(0.0, 1.0, 0.0), V3(-1.0, 1.0, 0.0)]];
        let tilt = 20f64.to_radians();
        let b = vec![vec![V3(0.0, 0.0, 0.0), V3(tilt.cos(), 0.0, -tilt.sin()), V3(tilt.cos(), 1.0, -tilt.sin()), V3(0.0, 1.0, 0.0)]];
        let na = V3(0.0, 0.0, 1.0);
        let nb = V3(tilt.sin(), 0.0, tilt.cos());
        let faces = [Source { contours: &a, normal: na, given: None }, Source { contours: &b, normal: nb, given: None }];
        let out = shade(&faces, &[], &rig(), 4.0).unwrap();
        // The shared edge (x = 0) has one normal, between the two faces'.
        let shared_a = out[0].normals[1];
        let shared_b = out[1].normals[0];
        assert!(shared_a.sub(shared_b).dot(shared_a.sub(shared_b)) < 1e-12, "{shared_a:?} {shared_b:?}");
        assert!(shared_a.0 > 0.01 && shared_a.0 < nb.0);
        // Away from it, each face keeps leaning its own way.
        assert!(out[0].normals[0].0 < shared_a.0);
        // At a right angle, nothing is shared: each side keeps its own normal.
        let c = vec![vec![V3(0.0, 0.0, 0.0), V3(0.0, 0.0, -1.0), V3(0.0, 1.0, -1.0), V3(0.0, 1.0, 0.0)]];
        let faces = [Source { contours: &a, normal: na, given: None }, Source { contours: &c, normal: V3(1.0, 0.0, 0.0), given: None }];
        let out = shade(&faces, &[], &rig(), 4.0).unwrap();
        assert!(out[0].normals.iter().all(|n| (n.2 - 1.0).abs() < 1e-12));
        assert!(out[0].shade.tint[0] > 0.9, "the face towards the light stays lit: {:?}", out[0].shade);
    }
}
