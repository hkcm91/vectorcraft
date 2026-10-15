//! Smooth shading: each face shaded at its vertices and filled with a linear ramp between them.
//!
//! A vertex's normal is the area-weighted mean of the faces meeting there whose normals are within
//! [`CREASE`] of the face being shaded, so curved surfaces read as smooth while edges, corners
//! and the cap of a solid stay crisp. The lights and the eye sit at a finite distance, so a flat
//! face also takes a gentle falloff and its highlight moves across it. The ramp is the
//! line the brightness changes along fastest (a least-squares fit over the projected face); along
//! it the normal and the position are fitted too and shaded at samples, so a highlight can peak
//! inside a face and cel-shading bands meet in hard stops.

use std::collections::HashMap;

use vectorcraft_geom::Point;

use crate::V3;
use crate::lighting::{Rig, Shade};

/// The largest angle between two faces that still shades as one smooth surface (cosine of 50°).
const CREASE: f64 = 0.642_787_609_686_539_3;
/// A ramp whose ends differ by less than this is drawn as one colour.
const FLAT: f64 = 0.004;
/// The most vertices the pass shades; beyond it faces stay flat.
const MAX_VERTICES: usize = 600_000;

/// The most stops a ramp takes.
const MAX_STOPS: usize = 12;
/// Samples taken along a ramp before its stops are simplified.
const SAMPLES: usize = 33;

/// A face's shade along a line from `from` (offset 0) to `to` (offset 1): linear between its
/// stops (two at one offset make a hard edge) and constant beyond.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub from: Point,
    pub to: Point,
    pub stops: Vec<(f64, Shade)>,
}

/// One face to shade: its view-space contours, the unit normal it is shaded with, and the same
/// contours projected.
pub(crate) struct Source<'a> {
    pub contours: &'a [Vec<V3>],
    pub normal: V3,
    pub projected: &'a [Vec<Point>],
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

/// Each face's smooth shading: its ramp (None when it shades as one colour) and its mean shade.
/// `hidden` are the solid's other faces (contours and outward normal), which round off the normals
/// at its silhouette. `size` is the object's half extent, setting how far away the lights and the
/// eye are.
pub(crate) fn shade(faces: &[Source], hidden: &[(Vec<Vec<V3>>, V3)], rig: &Rig, size: f64) -> Vec<(Option<Ramp>, Shade)> {
    let total: usize = faces.iter().map(|f| f.contours.iter().map(Vec::len).sum::<usize>()).sum();
    let flat = |f: &Source| (None, rig.shade(f.normal));
    if total > MAX_VERTICES || !size.is_finite() || size <= 0.0 {
        return faces.iter().map(flat).collect();
    }
    let q = size * 1e-4;
    let key = |v: V3| ((v.0 / q).round() as i64, (v.1 / q).round() as i64, (v.2 / q).round() as i64);
    // Every face meeting at each vertex: its normal (twice: to judge creases by, and to average),
    // weighted by its area.
    let mut at: Incident = HashMap::new();
    let all = faces.iter().map(|f| (f.contours, f.normal)).chain(hidden.iter().map(|(c, n)| (c.as_slice(), *n)));
    for (contours, normal) in all {
        let weight: f64 = contours.iter().map(|c| area(c)).sum::<f64>().max(1e-12);
        for v in contours.iter().flatten() {
            at.entry(key(*v)).or_default().push((normal, normal, weight));
        }
    }
    let reach = 6.0 * size;
    // The brightness's direction is found without cel bands, which hide it inside a band.
    let plain = rig.unbanded();
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
    // Twice: the faces' smoothed normals averaged again, so the normals vary as evenly as the
    // surface (a tight highlight shows the least unevenness as speckle).
    let first = |f: &Source| -> V3 {
        let (mut sum, mut count) = (V3::default(), 0.0);
        for v in f.contours.iter().flatten() {
            let n = vertex(&at, *v, f.normal);
            sum = V3(sum.0 + n.0, sum.1 + n.1, sum.2 + n.2);
            count += 1.0;
        }
        if count > 0.0 && sum.dot(sum) > 1e-24 { sum.unit() } else { f.normal }
    };
    let mut again: Incident = HashMap::new();
    for (f, (_, value, weight)) in faces.iter().zip(faces.iter().map(|f| (f, first(f), f.contours.iter().map(|c| area(c)).sum::<f64>().max(1e-12)))) {
        for v in f.contours.iter().flatten() {
            again.entry(key(*v)).or_default().push((f.normal, value, weight));
        }
    }
    for (contours, normal) in hidden {
        let weight: f64 = contours.iter().map(|c| area(c)).sum::<f64>().max(1e-12);
        for v in contours.iter().flatten() {
            again.entry(key(*v)).or_default().push((*normal, *normal, weight));
        }
    }
    faces
        .iter()
        .map(|f| {
            let mut points = vec![];
            let mut vertices = vec![];
            for (c3, c2) in f.contours.iter().zip(f.projected) {
                for (v, p) in c3.iter().zip(c2) {
                    points.push(*p);
                    vertices.push((vertex(&again, *v, f.normal), *v));
                }
            }
            if points.is_empty() {
                return flat(f);
            }
            fit(&points, &vertices, &|n, v| plain.shade_at(n, v, reach), &|n, v| rig.shade_at(n, v, reach), rig.banded())
        })
        .collect()
}

fn components(s: &Shade) -> [f64; 6] {
    [s.tint[0], s.tint[1], s.tint[2], s.highlight[0], s.highlight[1], s.highlight[2]]
}

fn from_components(c: [f64; 6]) -> Shade {
    Shade { tint: [c[0], c[1], c[2]].map(|v| v.clamp(0.0, 2.0)), highlight: [c[3], c[4], c[5]].map(|v| v.clamp(0.0, 1.0)) }
}

fn lerp(a: &[f64; 6], b: &[f64; 6], t: f64) -> [f64; 6] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// A face's ramp and mean shade from its vertices: at each projected point, the (normal,
/// position) there. `guide` shades without cel bands, to find the ramp's direction; `shade` is the
/// real shading; `hard` keeps band edges as hard stops instead of simplifying them away.
fn fit(
    points: &[Point],
    vertices: &[(V3, V3)],
    guide: &dyn Fn(V3, V3) -> Shade,
    shade: &dyn Fn(V3, V3) -> Shade,
    hard: bool,
) -> (Option<Ramp>, Shade) {
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.x).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.y).sum::<f64>() / n;
    let real: Vec<[f64; 6]> = vertices.iter().map(|(nv, v)| components(&shade(*nv, *v))).collect();
    let mut mean = [0.0; 6];
    for c in &real {
        for (m, v) in mean.iter_mut().zip(c) {
            *m += v / n;
        }
    }
    let flat = (None, from_components(mean));
    // The least-squares plane through some value at the points: its slope (x, y).
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in points {
        let (x, y) = (p.x - cx, p.y - cy);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
    }
    let det = sxx * syy - sxy * sxy;
    let solvable = det.abs() > 1e-12 * (sxx * syy).max(1e-300);
    let slope = |value: &dyn Fn(usize) -> f64| -> (f64, f64) {
        let (mut sxl, mut syl) = (0.0, 0.0);
        for (k, p) in points.iter().enumerate() {
            sxl += (p.x - cx) * value(k);
            syl += (p.y - cy) * value(k);
        }
        if solvable { ((syy * sxl - sxy * syl) / det, (sxx * syl - sxy * sxl) / det) } else { (0.0, 0.0) }
    };
    let extent = sxx.max(syy).sqrt() / n.sqrt() * 2.0;
    // The ramp runs the way the brightness changes fastest (so cel bands and highlights meet it
    // square on). Where that misses most of the change, a highlight peaking inside a curved face,
    // it runs the way the normal turns fastest (the principal direction of the normal's slopes).
    let lum = |c: &[f64; 6]| (c[0] + c[1] + c[2] + c[3] + c[4] + c[5]) / 3.0;
    let guides: Vec<f64> = vertices.iter().map(|(nv, v)| lum(&components(&guide(*nv, *v)))).collect();
    let spread = guides.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b)) - guides.iter().fold(f64::INFINITY, |a, b| a.min(*b));
    let (lx, ly) = slope(&|i| guides[i]);
    let turns: Vec<(f64, f64)> = (0..3).map(|k| slope(&|i| [vertices[i].0.0, vertices[i].0.1, vertices[i].0.2][k])).collect();
    let (a, b, c) = turns.iter().fold((0.0, 0.0, 0.0), |(a, b, c), (x, y)| (a + x * x, b + x * y, c + y * y));
    let largest = (a + c) / 2.0 + (((a - c) / 2.0).powi(2) + b * b).sqrt();
    let (gx, gy) = if !solvable {
        // A sliver: along its length.
        if sxx >= syy { (1.0, 0.0) } else { (0.0, 1.0) }
    } else if lx.hypot(ly) * extent <= 0.5 * spread + 1e-9 && largest.sqrt() * extent > 0.01 {
        // The eigenvector of [[a, b], [b, c]] for its largest eigenvalue.
        if b.abs() > 1e-18 {
            (largest - c, b)
        } else if a >= c {
            (1.0, 0.0)
        } else {
            (0.0, 1.0)
        }
    } else {
        (lx, ly)
    };
    let g = gx.hypot(gy);
    if !g.is_finite() || g < 1e-12 {
        return flat;
    }
    let (ux, uy) = (gx / g, gy / g);
    let ts: Vec<f64> = points.iter().map(|p| (p.x - cx) * ux + (p.y - cy) * uy).collect();
    let (lo, hi) = ts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| (a.min(*t), b.max(*t)));
    if !(hi - lo).is_finite() || hi - lo < 1e-9 {
        return flat;
    }
    // The normal and the position along that direction, each component fitted on its own.
    let tm = ts.iter().sum::<f64>() / n;
    let stt: f64 = ts.iter().map(|t| (t - tm) * (t - tm)).sum();
    let line = |get: &dyn Fn(&(V3, V3)) -> f64| -> (f64, f64) {
        let m = vertices.iter().map(get).sum::<f64>() / n;
        let st: f64 = ts.iter().zip(vertices).map(|(t, v)| (t - tm) * (get(v) - m)).sum();
        (m, if stt > 1e-18 { st / stt } else { 0.0 })
    };
    let fits: [(f64, f64); 6] = [line(&|v| v.0.0), line(&|v| v.0.1), line(&|v| v.0.2), line(&|v| v.1.0), line(&|v| v.1.1), line(&|v| v.1.2)];
    let at = |t: f64| -> [f64; 6] {
        let e = |k: usize| fits[k].0 + fits[k].1 * (t - tm);
        let nv = V3(e(0), e(1), e(2));
        let nv = if nv.dot(nv) > 1e-24 { nv.unit() } else { vertices[0].0 };
        components(&shade(nv, V3(e(3), e(4), e(5))))
    };
    let mut samples: Vec<(f64, [f64; 6])> = (0..SAMPLES).map(|k| k as f64 / (SAMPLES - 1) as f64).map(|o| (o, at(lo + (hi - lo) * o))).collect();
    if !hard {
        // The ends follow the vertices' own shades (a linear fit of them), so neighbouring faces
        // meet in the same colour; the samples add what bulges between, a highlight's peak.
        let ends: Vec<([f64; 6], [f64; 6])> = (0..6)
            .map(|k| {
                let m = real.iter().map(|c| c[k]).sum::<f64>() / n;
                let st: f64 = ts.iter().zip(&real).map(|(t, c)| (t - tm) * (c[k] - m)).sum();
                let slope = if stt > 1e-18 { st / stt } else { 0.0 };
                let mut a = [0.0; 6];
                let mut b = [0.0; 6];
                a[k] = m + slope * (lo - tm);
                b[k] = m + slope * (hi - tm);
                (a, b)
            })
            .collect();
        let start: [f64; 6] = std::array::from_fn(|k| ends[k].0[k]);
        let end: [f64; 6] = std::array::from_fn(|k| ends[k].1[k]);
        // Only a face its normal turns across by more than a few degrees bulges: on a finer one
        // the vertices already catch a highlight, and one face catching a tight highlight its
        // neighbours miss would show as a speck.
        let turn = largest.sqrt() * extent;
        let w = ((turn - 0.06) / 0.14).clamp(0.0, 1.0);
        let w = w * w * (3.0 - 2.0 * w);
        let (s0, s1) = (samples[0].1, samples[SAMPLES - 1].1);
        for (o, c) in &mut samples {
            let fitted = lerp(&start, &end, *o);
            let chord = lerp(&s0, &s1, *o);
            *c = std::array::from_fn(|k| fitted[k] + w * (c[k] - chord[k]));
        }
    }
    let close = |a: &[f64; 6], b: &[f64; 6]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < FLAT);
    let Some(((_, first), rest)) = samples.split_first() else { return flat };
    if rest.iter().all(|(_, c)| close(c, first)) {
        return flat;
    }
    let mut stops: Vec<(f64, [f64; 6])> = vec![];
    if hard {
        // Cel bands: flat runs, meeting in hard edges halfway between the samples either side.
        stops.push((0.0, *first));
        let mut current = *first;
        for pair in samples.windows(2) {
            let [(o0, _), (o1, c1)] = pair else { continue };
            if !close(c1, &current) {
                let mid = (o0 + o1) / 2.0;
                stops.push((mid, current));
                stops.push((mid, *c1));
                current = *c1;
            }
        }
        stops.push((1.0, current));
    } else {
        // Smooth: the fewest stops that keep every sample within reach of the line between them,
        // the reach widened until they are few enough.
        for tolerance in [2.0 * FLAT, 0.02, 0.05, 0.1, 0.25] {
            let mut keep = vec![false; samples.len()];
            keep[0] = true;
            keep[samples.len() - 1] = true;
            let mut spans = vec![(0usize, samples.len() - 1)];
            while let Some((a, b)) = spans.pop() {
                let (oa, ca) = samples[a];
                let (ob, cb) = samples[b];
                let worst = (a + 1..b)
                    .map(|k| {
                        let (o, c) = samples[k];
                        let line = lerp(&ca, &cb, (o - oa) / (ob - oa));
                        (k, c.iter().zip(&line).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max))
                    })
                    .max_by(|x, y| x.1.total_cmp(&y.1));
                if let Some((k, err)) = worst
                    && err > tolerance
                {
                    keep[k] = true;
                    spans.push((a, k));
                    spans.push((k, b));
                }
            }
            stops = samples.iter().zip(&keep).filter(|(_, k)| **k).map(|(s, _)| *s).collect();
            if stops.len() <= MAX_STOPS {
                break;
            }
        }
    }
    if stops.len() > MAX_STOPS {
        // Too many bands for one face: its mean instead.
        return flat;
    }
    let from = Point::new(cx + ux * lo, cy + uy * lo);
    let to = Point::new(cx + ux * hi, cy + uy * hi);
    (Some(Ramp { from, to, stops: stops.into_iter().map(|(o, c)| (o, from_components(c))).collect() }), from_components(mean))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{Light, Material};

    fn rig() -> Rig {
        Rig::new(Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() }, &[], 0.0, Material::default(), true)
    }

    #[test]
    fn a_ramp_runs_where_the_light_changes_and_peaks_where_the_highlight_is() {
        let pts = [Point::new(-10.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(-10.0, 10.0)];
        // A cylinder's normals turning left to right; the light straight ahead.
        let tilt = |x: f64| V3(x / 20.0, 0.0, 1.0).unit();
        let vertices: Vec<(V3, V3)> = pts.iter().map(|p| (tilt(p.x), V3(p.x, p.y, 0.0))).collect();
        let glossy = Rig::new(
            Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() },
            &[],
            0.0,
            Material { specular: 1.0, roughness: 0.2, ..Material::default() },
            true,
        );
        let s = |n: V3, _v: V3| glossy.shade(n);
        let (ramp, _) = fit(&pts, &vertices, &s, &s, false);
        let r = ramp.unwrap();
        assert!((r.from.y - r.to.y).abs() < 1e-6, "across the turn: {r:?}");
        // Brightest in the middle, where the normal faces the light: a stop there.
        let peak = r.stops.iter().max_by(|a, b| a.1.highlight[0].total_cmp(&b.1.highlight[0])).unwrap();
        assert!((peak.0 - 0.5).abs() < 0.1 && peak.1.highlight[0] > 0.9, "{:?}", r.stops);
        assert!(r.stops.len() >= 3 && r.stops.len() <= MAX_STOPS);
        // Cel bands: hard stops, two at each band edge.
        let cel = Rig::new(
            Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() },
            &[],
            0.0,
            Material { steps: 2, ..Material::default() },
            true,
        );
        let wide: Vec<(V3, V3)> = pts.iter().map(|p| (V3(p.x / 4.0, 0.0, 1.0).unit(), V3(p.x, p.y, 0.0))).collect();
        let plain = cel.unbanded();
        let (ramp, _) = fit(&pts, &wide, &|n, _| plain.shade(n), &|n, _| cel.shade(n), true);
        let stops = ramp.unwrap().stops;
        assert!(stops.windows(2).any(|w| w[0].0 == w[1].0 && w[0].1 != w[1].1), "{stops:?}");
        // The same shade everywhere: no ramp.
        let even: Vec<(V3, V3)> = pts.iter().map(|p| (V3(0.0, 0.0, 1.0), V3(p.x, p.y, 0.0))).collect();
        assert!(fit(&pts, &even, &s, &s, false).0.is_none());
    }

    #[test]
    fn adjacent_faces_share_a_smooth_normal_but_not_across_a_crease() {
        // Two faces of a gentle fold (20° apart) and one at a right angle.
        let a = vec![vec![V3(-1.0, 0.0, 0.0), V3(0.0, 0.0, 0.0), V3(0.0, 1.0, 0.0), V3(-1.0, 1.0, 0.0)]];
        let tilt = 20f64.to_radians();
        let b = vec![vec![V3(0.0, 0.0, 0.0), V3(tilt.cos(), 0.0, -tilt.sin()), V3(tilt.cos(), 1.0, -tilt.sin()), V3(0.0, 1.0, 0.0)]];
        let flat = |c: &Vec<Vec<V3>>| c.iter().map(|c| c.iter().map(|v| Point::new(v.0, v.1)).collect()).collect::<Vec<Vec<Point>>>();
        let (pa, pb) = (flat(&a), flat(&b));
        let na = V3(0.0, 0.0, 1.0);
        let nb = V3(tilt.sin(), 0.0, tilt.cos());
        let faces = [Source { contours: &a, normal: na, projected: &pa }, Source { contours: &b, normal: nb, projected: &pb }];
        let out = shade(&faces, &[], &rig(), 1.0);
        // Each face ramps towards the shared edge, where both meet at the same shade.
        let (ra, rb) = (out[0].0.clone().unwrap(), out[1].0.clone().unwrap());
        let ends = |r: &Ramp| (r.stops.first().unwrap().1, r.stops.last().unwrap().1);
        let edge_a = if (ra.from.x).abs() < (ra.to.x).abs() { ends(&ra).0 } else { ends(&ra).1 };
        let edge_b = if (rb.from.x).abs() < (rb.to.x).abs() { ends(&rb).0 } else { ends(&rb).1 };
        assert!((edge_a.tint[0] - edge_b.tint[0]).abs() < 0.02, "{edge_a:?} {edge_b:?}");
        // At a right angle, nothing is shared: each side keeps its own normal.
        let c = vec![vec![V3(0.0, 0.0, 0.0), V3(0.0, 0.0, -1.0), V3(0.0, 1.0, -1.0), V3(0.0, 1.0, 0.0)]];
        let pc = flat(&c);
        let faces = [Source { contours: &a, normal: na, projected: &pa }, Source { contours: &c, normal: V3(1.0, 0.0, 0.0), projected: &pc }];
        let out = shade(&faces, &[], &rig(), 1.0);
        let lit = out[0].1.tint[0];
        assert!(lit > 0.9, "the face towards the light stays lit: {lit}");
    }
}
