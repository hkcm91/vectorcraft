//! Per-pixel smooth shading: a surface's faces drawn into a depth buffer at twice the image's
//! resolution each way, the normal and the position interpolated across each face (Phong
//! shading), each pixel shaded with the lights and its four samples averaged. Highlights, cel
//! band edges and the falloff of near lights are then as smooth as the pixels, whatever the size
//! of the faces. The image is opaque a little beyond the surface; the outline of the faces
//! ([`Shading::outline`]) clips it, so the silhouette stays a crisp vector edge.

use vectorcraft_geom::{BezPath, PathData, Point, Rect};

use crate::V3;
use crate::lighting::{Rig, Shade};

/// The longest side of a shading image, in pixels.
pub const MAX_SIDE: f64 = 768.0;
/// The longest side of a draft shading image (a live preview while something is dragged): a
/// ninth of the pixels to shade, the silhouette still the crisp vector outline.
pub const DRAFT_SIDE: f64 = 256.0;
/// The most pixels a shading image takes per point.
const MAX_DENSITY: f64 = 8.0;
/// Samples per pixel each way.
const SUB: usize = 2;
/// Pixels painted round the surface, so the clipping outline never shows past its colour.
const PAD: usize = 2;
/// The most colourings [`Shading::layers`] makes at once.
pub const MAX_LAYERS: usize = 2;
/// Ambient occlusion's samples about each pixel.
const OCCLUSION_SAMPLES: usize = 32;

/// One face to shade: its projected contours, and at each of their points (in order) the
/// smoothed normal and the view-space position; `normal` is the face's own.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Poly {
    pub projected: Vec<Vec<Point>>,
    pub normals: Vec<V3>,
    pub positions: Vec<V3>,
    pub normal: V3,
}

/// A smooth-shaded surface, ready to draw pixel by pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    rig: Rig,
    /// How far the lights and the eye are from the object's centre.
    reach: f64,
    polys: Vec<Poly>,
}

/// How a pixel's shade becomes its colour (0–1 RGB).
pub type Colouring<'a> = &'a (dyn Fn(&Shade) -> [f32; 3] + Sync);

/// A shading image: `width` × `height` straight RGBA pixels, row by row, its top left corner at
/// `origin` in the document, each pixel `step` points across.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadedImage {
    pub origin: Point,
    pub step: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A piece of a face as the rasterizer fills it: its edges (in sample space, each from its upper
/// to its lower end, with its winding), the rows it spans, for each of the normal's and the
/// position's components a plane `a + b·x + c·y` over sample space (the depth test's, and the
/// values of a face filled flat), and for a face without holes its corners and their values,
/// blended smoothly across it.
struct Piece {
    edges: Vec<(f64, f64, f64, f64, i32)>,
    rows: (usize, usize),
    planes: [[f64; 3]; 6],
    corners: Option<Corners>,
    /// Thinner than a sample: the values are taken along its nearest edge, not blended across it.
    thin: bool,
}

/// A face's corners (sample space) and the normal and position at each.
type Corners = (Vec<(f64, f64)>, Vec<[f64; 6]>);

impl Piece {
    fn at(&self, x: f64, y: f64) -> [f64; 6] {
        match &self.corners {
            Some((pts, values)) if self.thin => along(pts, values, x, y).unwrap_or_else(|| self.planes.map(|[a, b, c]| a + b * x + c * y)),
            Some((pts, values)) => blend(pts, values, x, y).unwrap_or_else(|| self.planes.map(|[a, b, c]| a + b * x + c * y)),
            None => self.planes.map(|[a, b, c]| a + b * x + c * y),
        }
    }
}

/// The values along a sliver of a polygon at (`x`, `y`): those of the nearest point of its
/// nearest edge, between that edge's ends. Across a sliver mean value coordinates swing wildly,
/// and its corners' mean is one flat value: a steep rim cut into slivers would show as streaks.
/// Along its edges a sliver's values follow its neighbours', which share them.
fn along(pts: &[(f64, f64)], values: &[[f64; 6]], x: f64, y: f64) -> Option<[f64; 6]> {
    let n = pts.len();
    let mut best: Option<(f64, usize, f64)> = None;
    for i in 0..n {
        let (a, b) = (pts.get(i)?, pts.get((i + 1) % n)?);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dy * dy;
        let t = if l2 > 1e-18 { (((x - a.0) * dx + (y - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        let d = (a.0 + dx * t - x).hypot(a.1 + dy * t - y);
        if best.is_none_or(|(e, _, _)| d < e) {
            best = Some((d, i, t));
        }
    }
    let (_, i, t) = best?;
    let (a, b) = (values.get(i)?, values.get((i + 1) % n)?);
    Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t))
}

/// The values at a polygon's corners blended at (`x`, `y`) by mean value coordinates (Floater):
/// smooth inside the polygon, exact at the corners and linear along the edges, with no creases
/// (a fan of triangles would crease along its spokes, and a highlight shows each). `None` where
/// the weights vanish.
fn blend(pts: &[(f64, f64)], values: &[[f64; 6]], x: f64, y: f64) -> Option<[f64; 6]> {
    let n = pts.len();
    let s: Vec<(f64, f64)> = pts.iter().map(|p| (p.0 - x, p.1 - y)).collect();
    let r: Vec<f64> = s.iter().map(|v| v.0.hypot(v.1)).collect();
    // On a corner, or on an edge: its value, or the edge's.
    for i in 0..n {
        if r[i] < 1e-9 {
            return values.get(i).copied();
        }
        let j = (i + 1) % n;
        let cross = s[i].0 * s[j].1 - s[i].1 * s[j].0;
        let dot = s[i].0 * s[j].0 + s[i].1 * s[j].1;
        if cross.abs() < 1e-9 * r[i] * r[j] && dot < 0.0 {
            let t = r[i] / (r[i] + r[j]);
            let (a, b) = (values.get(i)?, values.get(j)?);
            return Some(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
        }
    }
    // tan(α/2) for the angle each edge subtends.
    let half: Vec<f64> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let cross = s[i].0 * s[j].1 - s[i].1 * s[j].0;
            let dot = s[i].0 * s[j].0 + s[i].1 * s[j].1;
            cross / (r[i] * r[j] + dot)
        })
        .collect();
    let mut sum = [0.0; 6];
    let mut total = 0.0;
    for i in 0..n {
        let w = (half[(i + n - 1) % n] + half[i]) / r[i];
        total += w;
        for (s, v) in sum.iter_mut().zip(values.get(i)?) {
            *s += w * v;
        }
    }
    (total.abs() > 1e-12 && total.is_finite()).then(|| sum.map(|v| v / total))
}

/// The plane through some values at points (least squares, exact for three points), about their
/// centroid; flat at the values' mean when the points don't span an area.
fn plane(points: &[(f64, f64)], values: &[f64]) -> [f64; 3] {
    let n = points.len().max(1) as f64;
    let (cx, cy) = points.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0 / n, y + p.1 / n));
    let m = values.iter().sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy, mut sxv, mut syv) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (p, v) in points.iter().zip(values) {
        let (x, y, v) = (p.0 - cx, p.1 - cy, v - m);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sxv += x * v;
        syv += y * v;
    }
    let det = sxx * syy - sxy * sxy;
    // Collinear points (or values that aren't numbers): no slope to fit.
    if det.abs().partial_cmp(&(1e-9 * (sxx * syy).max(1e-300))) != Some(std::cmp::Ordering::Greater) || det.abs() < 1e-12 {
        return [m, 0.0, 0.0];
    }
    let b = (syy * sxv - sxy * syv) / det;
    let c = (sxx * syv - sxy * sxv) / det;
    [m - b * cx - c * cy, b, c]
}

/// Twice the signed area of a polygon.
fn signed(c: &[(f64, f64)]) -> f64 {
    c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum()
}

impl Shading {
    /// The faces it shades.
    #[cfg(test)]
    pub(crate) fn polys(&self) -> &[Poly] {
        &self.polys
    }

    pub(crate) fn new(rig: Rig, reach: f64, polys: Vec<Poly>) -> Self {
        Self { rig, reach, polys }
    }

    /// The area the faces cover: each face wound positively (its holes against it), so the
    /// nonzero fill of them all is their union. A face folded over in projection (seen nearly
    /// edge-on, a bow tie) would wind both ways and cancel its neighbours: it goes in as the
    /// triangles about its centre, each wound positively. Neighbouring faces share their
    /// corners exactly, so the union has no cracks.
    pub fn outline(&self) -> PathData {
        let mut bp = BezPath::new();
        let mut ring = |pts: &[Point]| {
            let Some(first) = pts.first() else { return };
            bp.move_to(*first);
            for p in pts.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        };
        let area = |c: &[Point]| signed(&c.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>());
        let forward = |c: &[Point], positive: bool| -> Vec<Point> { if positive { c.to_vec() } else { c.iter().rev().copied().collect() } };
        for poly in &self.polys {
            let outer = poly.projected.iter().map(|c| area(c)).max_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0);
            if let [c] = poly.projected.as_slice() {
                let k = c.len().max(1) as f64;
                let centre = c.iter().fold(Point::ZERO, |s, p| Point::new(s.x + p.x / k, s.y + p.y / k));
                let fan = c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| area(&[centre, *a, *b]));
                if outer.abs() > 1e-12 && fan.clone().all(|t| t * outer >= 0.0) {
                    ring(&forward(c, outer > 0.0));
                } else {
                    for (i, t) in fan.enumerate() {
                        let (a, b) = (c[i], c[(i + 1) % c.len()]);
                        if t.abs() > 1e-12 {
                            ring(&forward(&[centre, a, b], t > 0.0));
                        }
                    }
                }
                continue;
            }
            if outer.abs() < 1e-12 {
                continue;
            }
            for c in &poly.projected {
                ring(&forward(c, outer > 0.0));
            }
        }
        PathData::from_bezpath(&bp)
    }

    /// The size of a shading image's pixels, in points, for a surface covering `r` in an image
    /// at most `longest` pixels along its longer side.
    fn step(r: Rect, longest: f64) -> f64 {
        let side = r.width().max(r.height());
        1.0 / (longest / side.max(1e-9)).min(MAX_DENSITY)
    }

    /// The document area the faces cover.
    fn bounds(&self) -> Option<Rect> {
        let mut pts = self.polys.iter().flat_map(|p| p.projected.iter().flatten());
        let first = pts.next()?;
        let r = pts.fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p));
        [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()).then_some(r)
    }

    /// The surface shaded pixel by pixel, each pixel's colour `colour` of its shade (0–1 RGB).
    /// `None` for a surface with no area. Rows are shared out among the cores.
    pub fn image(&self, colour: Colouring) -> Option<ShadedImage> {
        self.layers(&[colour])?.into_iter().next()
    }

    /// [`Self::image`] for up to [`MAX_LAYERS`] colourings of the same shading at once (the
    /// shading is worked out once), in their order. `None` for a surface with no area or no
    /// colouring.
    pub fn layers(&self, colours: &[Colouring]) -> Option<Vec<ShadedImage>> {
        self.layers_at(colours, MAX_SIDE)
    }

    /// [`Self::layers`] in an image at most `longest` pixels along its longer side (16 up to
    /// [`MAX_SIDE`]): [`DRAFT_SIDE`] for a quick preview.
    pub fn layers_at(&self, colours: &[Colouring], longest: f64) -> Option<Vec<ShadedImage>> {
        let longest = if longest.is_finite() { longest.clamp(16.0, MAX_SIDE) } else { MAX_SIDE };
        let colours = colours.get(..colours.len().min(MAX_LAYERS))?;
        if colours.is_empty() {
            return None;
        }
        let r = self.bounds()?;
        let side = r.width().max(r.height());
        if side <= 1e-9 {
            return None;
        }
        let step = Self::step(r, longest);
        let density = 1.0 / step;
        let pad = PAD as f64 * step;
        let origin = Point::new(r.x0 - pad, r.y0 - pad);
        let (w, h) = ((r.width() * density).ceil() as usize + 2 * PAD, (r.height() * density).ceil() as usize + 2 * PAD);
        let (sw, sh) = (w * SUB, h * SUB);
        let scale = density * SUB as f64;
        let to_sample = |p: Point| ((p.x - origin.x) * scale, (p.y - origin.y) * scale);
        let pieces = self.pieces(&to_sample, sh);
        // Each sample's nearest piece (larger z is towards the viewer) and its depth.
        let mut samples = vec![(f64::NEG_INFINITY, u32::MAX); sw * sh];
        bands(&mut samples, sw, &|first, band: &mut [(f64, u32)]| {
            let last = first + band.len() / sw;
            let mut crossings: Vec<(f64, i32)> = vec![];
            for (index, piece) in pieces.iter().enumerate() {
                for row in piece.rows.0.max(first)..piece.rows.1.min(last) {
                    let y = row as f64 + 0.5;
                    crossings.clear();
                    for &(x0, y0, x1, y1, wind) in &piece.edges {
                        if y >= y0 && y < y1 {
                            crossings.push((x0 + (y - y0) * (x1 - x0) / (y1 - y0), wind));
                        }
                    }
                    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut winding = 0;
                    for pair in crossings.windows(2) {
                        let [(xa, wa), (xb, _)] = pair else { continue };
                        winding += wa;
                        if winding == 0 {
                            continue;
                        }
                        let start = (xa - 0.5).ceil().max(0.0) as usize;
                        let end = ((xb - 0.5).ceil().max(0.0) as usize).min(sw);
                        let [a, b, c] = piece.planes[5];
                        for col in start..end {
                            let z = a + b * (col as f64 + 0.5) + c * y;
                            if let Some(slot) = band.get_mut((row - first) * sw + col)
                                && z > slot.0
                            {
                                *slot = (z, index as u32);
                            }
                        }
                    }
                }
            }
        });
        let centre = |px: usize, py: usize| ((px * SUB) as f64 + SUB as f64 / 2.0, (py * SUB) as f64 + SUB as f64 / 2.0);
        let owners = |px: usize, py: usize| -> [u32; SUB * SUB] {
            std::array::from_fn(|s| samples.get((py * SUB + s / SUB) * sw + px * SUB + s % SUB).map_or(u32::MAX, |v| v.1))
        };
        // How open each pixel is to its surroundings (1 without ambient occlusion).
        let strength = self.rig.occlusion();
        let open = if strength > 0.0 {
            // Each pixel's normal and position, from the piece most of its samples show.
            let mut surface = vec![None; w * h];
            bands(&mut surface, w, &|first, band: &mut [Option<(V3, V3)>]| {
                for (k, out) in band.iter_mut().enumerate() {
                    let (px, py) = (k % w, first + k / w);
                    let ids = owners(px, py);
                    let id = ids.iter().copied().filter(|i| *i != u32::MAX).max_by_key(|i| ids.iter().filter(|j| *j == i).count());
                    if let Some(piece) = id.and_then(|i| pieces.get(i as usize)) {
                        let (x, y) = centre(px, py);
                        let [nx, ny, nz, qx, qy, qz] = piece.at(x, y);
                        *out = Some((V3(nx, ny, nz).unit(), V3(qx, qy, qz)));
                    }
                }
            });
            // The reach of the occlusion: a quarter of the object's half extent.
            let reach = self.reach / 24.0;
            occlusion(&surface, w, h, reach / step, reach, strength)
        } else {
            vec![1.0; w * h]
        };
        let banded = self.rig.banded();
        let shade_at = |piece: &Piece, x: f64, y: f64, open: f64| -> Shade {
            let [nx, ny, nz, px, py, pz] = piece.at(x, y);
            self.rig.shade_open(V3(nx, ny, nz).unit(), V3(px, py, pz), self.reach, open)
        };
        // Each pixel: one shade where its samples share a piece (a cel band's edge sampled too),
        // else its samples' mean. Alpha 0 marks a pixel no sample covers.
        let mut pixels = vec![[[0u8; 4]; MAX_LAYERS]; w * h];
        bands(&mut pixels, w, &|first, band: &mut [[[u8; 4]; MAX_LAYERS]]| {
            for (k, out) in band.iter_mut().enumerate() {
                let (px, py) = (k % w, first + k / w);
                let ids = owners(px, py);
                if ids.iter().all(|i| *i == u32::MAX) {
                    continue;
                }
                let o = open.get(py * w + px).copied().unwrap_or(1.0);
                let mut sum = [[0.0f32; 3]; MAX_LAYERS];
                let mut count = 0.0f32;
                let mut add = |shade: &Shade| {
                    for (layer, c) in sum.iter_mut().zip(colours) {
                        for (t, v) in layer.iter_mut().zip(c(shade)) {
                            *t += v;
                        }
                    }
                    count += 1.0;
                };
                if !banded && ids.iter().all(|i| *i == ids[0]) {
                    if let Some(piece) = pieces.get(ids[0] as usize) {
                        let (x, y) = centre(px, py);
                        add(&shade_at(piece, x, y, o));
                    }
                } else {
                    for (s, id) in ids.iter().enumerate() {
                        let Some(piece) = pieces.get(*id as usize) else { continue };
                        add(&shade_at(piece, (px * SUB + s % SUB) as f64 + 0.5, (py * SUB + s / SUB) as f64 + 0.5, o));
                    }
                }
                if count > 0.0 {
                    let q = |v: f32| ((v / count).clamp(0.0, 1.0) * 255.0).round() as u8;
                    for (slot, layer) in out.iter_mut().zip(&sum) {
                        *slot = [q(layer[0]), q(layer[1]), q(layer[2]), 255];
                    }
                }
            }
        });
        // The colour carried past the surface's edge, for the clipping outline to cut.
        for _ in 0..PAD {
            let before = pixels.clone();
            let at = |x: isize, y: isize| (x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h).then(|| before[y as usize * w + x as usize]);
            for (k, out) in pixels.iter_mut().enumerate() {
                if out[0][3] != 0 {
                    continue;
                }
                let (px, py) = ((k % w) as isize, (k / w) as isize);
                let mut sum = [[0u32; 3]; MAX_LAYERS];
                let mut count = 0;
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    if let Some(p) = at(px + dx, py + dy).filter(|p| p[0][3] != 0) {
                        for (layer, src) in sum.iter_mut().zip(p) {
                            for (s, v) in layer.iter_mut().zip(src) {
                                *s += u32::from(v);
                            }
                        }
                        count += 1;
                    }
                }
                if count > 0 {
                    let q = |s: u32| ((s + count / 2) / count) as u8;
                    for (slot, layer) in out.iter_mut().zip(&sum) {
                        *slot = [q(layer[0]), q(layer[1]), q(layer[2]), 255];
                    }
                }
            }
        }
        let (width, height) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
        Some(
            (0..colours.len())
                .map(|l| ShadedImage {
                    origin,
                    step,
                    width,
                    height,
                    rgba: pixels.iter().flat_map(|p| p.get(l).copied().unwrap_or_default()).collect(),
                })
                .collect(),
        )
    }

    /// The faces as pieces in sample space: a face without holes with its corners' values blended
    /// across it, one with holes (a cap) filled flat with its own normal and the plane through
    /// its points.
    fn pieces(&self, to_sample: &dyn Fn(Point) -> (f64, f64), rows: usize) -> Vec<Piece> {
        let mut out = vec![];
        for poly in &self.polys {
            let pts: Vec<(f64, f64)> = poly.projected.iter().flatten().map(|p| to_sample(*p)).collect();
            if pts.len() != poly.normals.len() || pts.len() != poly.positions.len() || pts.len() < 3 {
                continue;
            }
            let attrs: Vec<[f64; 6]> = poly.normals.iter().zip(&poly.positions).map(|(n, p)| [n.0, n.1, n.2, p.0, p.1, p.2]).collect();
            if poly.projected.len() == 1 {
                // Without holes: the corners' values blended across it, the depth test on their
                // plane. One thinner than a sample takes its values along its nearest edge (a blend
                // over the few samples it covers would swing wildly).
                let mut planes: [[f64; 3]; 6] = std::array::from_fn(|k| plane(&pts, &attrs.iter().map(|a| a[k]).collect::<Vec<_>>()));
                let thin = signed(&pts).abs() < 0.5;
                if thin {
                    let k = attrs.len() as f64;
                    for (i, p) in planes.iter_mut().enumerate().take(5) {
                        *p = [attrs.iter().map(|a| a[i]).sum::<f64>() / k, 0.0, 0.0];
                    }
                }
                // Neighbours share their corners exactly, and an edge's crossings depend on its ends
                // alone (taken upper end first), so a sample on a shared edge goes to exactly one
                // of the faces: no pinholes for a surface behind to show through.
                let edges = pts.iter().zip(pts.iter().cycle().skip(1)).take(pts.len()).filter_map(|(a, b)| edge(*a, *b)).collect();
                if let Some(mut p) = piece(edges, planes, rows) {
                    p.corners = Some((pts.clone(), attrs.clone()));
                    p.thin = thin;
                    out.push(p);
                }
                continue;
            }
            // Filled whole: the face's own normal, the plane through its positions.
            let mut planes = [[0.0; 3]; 6];
            for (k, slot) in planes.iter_mut().enumerate() {
                *slot = if k < 3 {
                    [[poly.normal.0, poly.normal.1, poly.normal.2][k], 0.0, 0.0]
                } else {
                    plane(&pts, &attrs.iter().map(|a| a[k]).collect::<Vec<_>>())
                };
            }
            let mut edges = vec![];
            let mut start = 0;
            for c in &poly.projected {
                let ring = pts.get(start..start + c.len()).unwrap_or_default();
                start += c.len();
                edges.extend(ring.iter().zip(ring.iter().cycle().skip(1)).take(ring.len()).filter_map(|(a, b)| edge(*a, *b)));
            }
            if let Some(p) = piece(edges, planes, rows) {
                out.push(p);
            }
        }
        out
    }
}

/// How open each pixel of a surface is (1: nothing of the surface over it; less in creases and
/// hollows), from each pixel's normal and position (`None` where the surface isn't): screen-space
/// ambient occlusion. Samples spiral out to `radius` pixels (`reach` points in view space); a
/// sample whose point rises above the pixel's tangent plane, near enough, hides part of the sky.
/// `strength` (0–1) scales the darkening.
fn occlusion(surface: &[Option<(V3, V3)>], w: usize, h: usize, radius: f64, reach: f64, strength: f64) -> Vec<f64> {
    // A golden-angle spiral: every sample its own direction and distance, so no pattern repeats.
    // None nearer than a third of the radius: up close the faces' flat facets, not the surface's
    // shape, decide what rises above the (smooth) tangent plane.
    let offsets: Vec<(isize, isize)> = (0..OCCLUSION_SAMPLES)
        .map(|k| {
            let t = (k as f64 + 0.5) / OCCLUSION_SAMPLES as f64;
            let a = k as f64 * 2.399_963_229_728_653;
            let r = radius.max(3.0) * (0.35 + 0.65 * t);
            ((r * a.cos()).round() as isize, (r * a.sin()).round() as isize)
        })
        .collect();
    let at = |x: isize, y: isize| -> Option<(V3, V3)> {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return None;
        }
        surface.get(y as usize * w + x as usize).copied().flatten()
    };
    let mut open = vec![1.0; w * h];
    bands(&mut open, w, &|first, band: &mut [f64]| {
        for (k, out) in band.iter_mut().enumerate() {
            let (px, py) = ((k % w) as isize, (first + k / w) as isize);
            let Some((n, p)) = at(px, py) else { continue };
            let mut hidden = 0.0;
            for (dx, dy) in &offsets {
                let Some((_, q)) = at(px + dx, py + dy) else { continue };
                let v = q.sub(p);
                let d = v.dot(v).sqrt();
                if d < 1e-9 {
                    continue;
                }
                // Above the tangent plane (less a little for the surface's own curve), fading
                // with distance.
                let rise = (n.dot(v) / d - 0.1).max(0.0);
                hidden += rise * (1.0 - d / (2.0 * reach)).max(0.0);
            }
            // Gained so that a deep crease (half the samples rising steeply) reads fully occluded.
            let occluded = (6.0 * hidden / OCCLUSION_SAMPLES as f64).clamp(0.0, 1.0);
            *out = 1.0 - strength * occluded;
        }
    });
    // A blur, across then down, over the surface alone: the samples are few.
    const BLUR: isize = 6;
    for (dx, dy) in [(1isize, 0isize), (0, 1)] {
        let before = open.clone();
        for (k, out) in open.iter_mut().enumerate() {
            let (px, py) = ((k % w) as isize, (k / w) as isize);
            if at(px, py).is_none() {
                continue;
            }
            let (mut sum, mut count) = (0.0, 0.0);
            for t in -BLUR..=BLUR {
                let (x, y) = (px + dx * t, py + dy * t);
                if at(x, y).is_some() {
                    sum += before[y as usize * w + x as usize];
                    count += 1.0;
                }
            }
            if count > 0.0 {
                *out = sum / count;
            }
        }
    }
    open
}

/// `f` over `data` cut into bands of whole rows (`row` items each), given each band's first row,
/// the bands shared out among the cores.
#[cfg(not(target_arch = "wasm32"))]
fn bands<T: Send>(data: &mut [T], row: usize, f: &(dyn Fn(usize, &mut [T]) + Sync)) {
    let rows = data.len().checked_div(row).unwrap_or(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8).min(rows);
    if threads <= 1 {
        f(0, data);
        return;
    }
    // Several bands a core: the surface rarely fills the rows evenly.
    let per = rows.div_ceil(threads * 4).max(1);
    let queue = std::sync::Mutex::new(data.chunks_mut(per * row).enumerate().collect::<Vec<_>>());
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    loop {
                        let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
                        let Some((k, band)) = next else { break };
                        f(k * per, band);
                    }
                })
            })
            .collect();
        // A worker's panic goes on in this thread, as if it had run here.
        for w in workers {
            if let Err(e) = w.join() {
                std::panic::resume_unwind(e);
            }
        }
    });
}

/// `f` over all of `data` at once (no threads on the web).
#[cfg(target_arch = "wasm32")]
fn bands<T: Send>(data: &mut [T], _: usize, f: &(dyn Fn(usize, &mut [T]) + Sync)) {
    f(0, data);
}

/// An edge from `a` to `b` as the rasterizer keeps it (upper end first, with its winding), `None`
/// for a horizontal one.
fn edge(a: (f64, f64), b: (f64, f64)) -> Option<(f64, f64, f64, f64, i32)> {
    if a.1 == b.1 || ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(if a.1 < b.1 { (a.0, a.1, b.0, b.1, 1) } else { (b.0, b.1, a.0, a.1, -1) })
}

fn piece(edges: Vec<(f64, f64, f64, f64, i32)>, planes: [[f64; 3]; 6], rows: usize) -> Option<Piece> {
    let top = edges.iter().map(|e| e.1).fold(f64::INFINITY, f64::min);
    let bottom = edges.iter().map(|e| e.3).fold(f64::NEG_INFINITY, f64::max);
    if !top.is_finite() || !bottom.is_finite() {
        return None;
    }
    let first = (top - 0.5).ceil().max(0.0) as usize;
    let last = ((bottom - 0.5).ceil().max(0.0) as usize).min(rows);
    (first < last && planes.iter().flatten().all(|v| v.is_finite())).then_some(Piece {
        edges,
        rows: (first, last),
        planes,
        corners: None,
        thin: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{Light, Material};

    fn rig(material: Material) -> Rig {
        Rig::new(Light { azimuth: 0.0, elevation: 0.0, intensity: 1.0, ..Light::default() }, &[], 0.0, material, true)
    }

    /// A hemisphere of radius 50 seen from the front, as quads, its normals exact.
    fn dome(material: Material) -> Shading {
        let mut polys = vec![];
        let (rings, segments) = (16, 32);
        let at = |i: usize, j: usize| {
            let polar = std::f64::consts::FRAC_PI_2 * i as f64 / rings as f64;
            let az = std::f64::consts::TAU * j as f64 / segments as f64;
            V3(polar.sin() * az.cos(), polar.sin() * az.sin(), polar.cos())
        };
        for i in 0..rings {
            for j in 0..segments {
                let n = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
                let pos = n.map(|v| V3(v.0 * 50.0, v.1 * 50.0, v.2 * 50.0));
                let mid = V3(n.iter().map(|v| v.0).sum(), n.iter().map(|v| v.1).sum(), n.iter().map(|v| v.2).sum()).unit();
                polys.push(Poly {
                    projected: vec![pos.iter().map(|v| Point::new(100.0 + v.0, 100.0 + v.1)).collect()],
                    normals: n.to_vec(),
                    positions: pos.to_vec(),
                    normal: mid,
                });
            }
        }
        Shading::new(rig(material), 300.0, polys)
    }

    fn grey(s: &Shade) -> [f32; 3] {
        [0.5 * s.tint[0] as f32 + s.highlight[0] as f32; 3]
    }

    #[test]
    fn a_dome_shades_smoothly_with_one_round_highlight() {
        let image = dome(Material { specular: 1.0, roughness: 0.15, smooth: true, ..Material::default() }).image(&grey).unwrap();
        let (w, h) = (image.width as usize, image.height as usize);
        assert!((700..=800).contains(&w) && w == h, "{w} × {h}");
        let px = |x: usize, y: usize| image.rgba[(y * w + x) * 4];
        let alpha = |x: usize, y: usize| image.rgba[(y * w + x) * 4 + 3];
        // Opaque over the dome and just past it, clear in the corners.
        assert_eq!(alpha(w / 2, h / 2), 255);
        assert_eq!(alpha(0, 0), 0);
        // The highlight peaks in the middle and falls off evenly every way: no facet edges, so
        // neighbouring pixels along a row never jump.
        let centre = px(w / 2, h / 2);
        assert!(centre > 240, "{centre}");
        let row: Vec<i32> = (PAD + 4..w - PAD - 4).map(|x| i32::from(px(x, h / 2))).collect();
        let jumps = row.windows(3).map(|t| (t[0] - 2 * t[1] + t[2]).abs()).max().unwrap();
        assert!(jumps <= 6, "the shading bends smoothly: {jumps}");
        // Round: the same profile down a column as along a row, and as far out on a diagonal.
        let column: Vec<i32> = (PAD + 4..h - PAD - 4).map(|y| i32::from(px(w / 2, y))).collect();
        assert_eq!(row, column);
        let at = |dx: i32, dy: i32| i32::from(px((w as i32 / 2 + dx) as usize, (h as i32 / 2 + dy) as usize));
        assert!((at(42, 42) - at(59, 0)).abs() <= 8, "{} {}", at(42, 42), at(59, 0));
    }

    #[test]
    fn cel_bands_meet_in_round_edges() {
        let image = dome(Material { steps: 3, smooth: true, ..Material::default() }).image(&grey).unwrap();
        let w = image.width as usize;
        // Along the middle row the shade steps down band by band, few values besides the bands
        // (the antialiased pixels on each edge).
        let row: Vec<u8> = (PAD..w - PAD).map(|x| image.rgba[((w / 2) * w + x) * 4]).collect();
        let edges = row.windows(2).filter(|p| p[0] != p[1]).count();
        assert!(edges <= 12, "{edges} changes along the row");
        // The edge between the bands is a circle: the same radius along a row and a diagonal.
        let radius = |dx: f64, dy: f64| {
            let c = w as f64 / 2.0;
            let top = image.rgba[((c as usize) * w + c as usize) * 4];
            (1..w / 2).find(|r| image.rgba[(((c + dy * *r as f64) as usize) * w + (c + dx * *r as f64) as usize) * 4] != top).unwrap() as f64
        };
        let (a, b) = (radius(1.0, 0.0), radius(std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2));
        assert!((a - b).abs() <= 2.0, "{a} {b}");
    }

    #[test]
    fn the_nearer_face_wins_and_the_outline_is_the_union() {
        let quad = |z: f64, x0: f64, shade: f64| Poly {
            projected: vec![vec![Point::new(x0, 0.0), Point::new(x0 + 20.0, 0.0), Point::new(x0 + 20.0, 20.0), Point::new(x0, 20.0)]],
            normals: vec![V3(0.0, 0.0, 1.0); 4],
            positions: vec![V3(shade, 0.0, z); 4],
            normal: V3(0.0, 0.0, 1.0),
        };
        // The position's x carries a marker the colour reads back.
        let s = Shading::new(rig(Material::default()), 1e6, vec![quad(5.0, 0.0, 0.25), quad(1.0, 10.0, 0.75)]);
        let marker = |_: &Shade| [0.0f32; 3];
        let image = s.image(&marker).unwrap();
        assert!(image.rgba.chunks(4).any(|p| p[3] == 255));
        let pieces = s.pieces(&|p| (p.x, p.y), 100);
        assert_eq!(pieces.len(), 2);
        let outline = s.outline();
        let b = outline.bounds().unwrap();
        // Exactly the faces' union.
        assert!(b.x0.abs() < 1e-9 && (b.x1 - 30.0).abs() < 1e-9, "{b:?}");
        // A face wound the other way is turned round, so the union's fill stays nonzero.
        let mut flipped = quad(1.0, 40.0, 0.0);
        flipped.projected[0].reverse();
        let s = Shading::new(rig(Material::default()), 1e6, vec![quad(5.0, 0.0, 0.0), flipped]);
        let areas: Vec<f64> = s.outline().subpaths.iter().map(|sp| signed(&sp.anchors.iter().map(|a| (a.p.x, a.p.y)).collect::<Vec<_>>())).collect();
        assert!(areas.iter().all(|a| *a > 0.0), "{areas:?}");
    }

    /// Faces that share their corners cover every sample between them: none falls through the
    /// shared edges, however they slant.
    #[test]
    fn faces_sharing_edges_leave_no_pinholes() {
        // A disc of thin wedges about an off-grid centre, each its own face.
        let (cx, cy) = (50.3, 49.7);
        let n = 97;
        let at = |k: usize| {
            let a = std::f64::consts::TAU * (k % n) as f64 / n as f64;
            Point::new(cx + 40.0 * a.cos(), cy + 40.0 * a.sin())
        };
        let mut polys: Vec<Poly> = (0..n)
            .map(|k| Poly {
                projected: vec![vec![Point::new(cx, cy), at(k), at(k + 1)]],
                normals: vec![V3(0.0, 0.0, 1.0); 3],
                positions: vec![V3(0.0, 0.0, 0.0); 3],
                normal: V3(0.0, 0.0, 1.0),
            })
            .collect();
        // Behind it, a face turned from the light: a sample falling through shows darker.
        let away = V3(0.0, 0.0, -1.0);
        let corners = vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];
        polys.push(Poly { projected: vec![corners], normals: vec![away; 4], positions: vec![V3(0.0, 0.0, -10.0); 4], normal: away });
        let s = Shading::new(rig(Material::default()), 1e6, polys);
        let image = s.image(&grey).unwrap();
        let w = image.width as usize;
        let step = image.step;
        let px = |x: usize, y: usize| image.rgba[(y * w + x) * 4];
        let lit = px(((cx - image.origin.x) / step) as usize, ((cy - image.origin.y) / step) as usize);
        assert!(lit > 100, "{lit}");
        let mut inside = 0;
        for y in 0..image.height as usize {
            for x in 0..w {
                let p = Point::new(image.origin.x + (x as f64 + 0.5) * step, image.origin.y + (y as f64 + 0.5) * step);
                if (p.x - cx).hypot(p.y - cy) < 38.0 {
                    inside += 1;
                    assert_eq!(px(x, y), lit, "a pinhole at {p:?}");
                }
            }
        }
        assert!(inside > 1000);
    }

    #[test]
    fn a_large_cap_with_a_hole_fills_flat_and_empty_input_draws_nothing() {
        let ring = |r: f64| -> Vec<Point> { (0..24).map(|k| (k as f64).to_radians() * 15.0).map(|a| Point::new(r * a.cos(), r * a.sin())).collect() };
        let mut hole = ring(10.0);
        hole.reverse();
        let count = 48;
        let cap = Poly {
            projected: vec![ring(40.0), hole],
            normals: vec![V3(0.6, 0.0, 0.8); count],
            positions: vec![V3(0.0, 0.0, 0.0); count],
            normal: V3(0.0, 0.0, 1.0),
        };
        let s = Shading::new(rig(Material::default()), 1e6, vec![cap]);
        let image = s.image(&grey).unwrap();
        let w = image.width as usize;
        let c = w / 2;
        // The hole is empty (only the padding beside the ring's inner edge is painted).
        assert_eq!(image.rgba[(c * w + c) * 4 + 3], 0);
        // The cap shades with its own normal, straight at the light: full brightness.
        let lit = image.rgba[(c * w + c + (w * 3 / 8)) * 4];
        assert_eq!(lit, 128, "{lit}");
        assert!(Shading::new(rig(Material::default()), 1.0, vec![]).image(&grey).is_none());
    }

    #[test]
    fn occlusion_darkens_a_valley_but_not_a_plane() {
        // A valley: two slopes meeting in a crease at x = 0, the deepest line.
        let slope = |x0: f64, x1: f64, z0: f64, z1: f64| {
            let pos = [V3(x0, -20.0, z0), V3(x1, -20.0, z1), V3(x1, 20.0, z1), V3(x0, 20.0, z0)];
            let n = V3(-(z1 - z0), 0.0, x1 - x0).unit();
            Poly { projected: vec![pos.iter().map(|v| Point::new(v.0, v.1)).collect()], normals: vec![n; 4], positions: pos.to_vec(), normal: n }
        };
        let valley = vec![slope(-20.0, 0.0, 20.0, 0.0), slope(0.0, 20.0, 0.0, 20.0)];
        let plane = vec![slope(-20.0, 20.0, 0.0, 0.0)];
        let shine = |polys: Vec<Poly>, occlusion: f64| {
            let rig = rig(Material { occlusion, smooth: true, ..Material::default() });
            let image = Shading::new(rig, 240.0, polys).image(&grey).unwrap();
            let w = image.width as usize;
            let at = |x: usize| image.rgba[((image.height as usize / 2) * w + x) * 4];
            (at(w / 2), at(w / 4))
        };
        let (crease, side) = shine(valley.clone(), 0.0);
        let (dark_crease, dark_side) = shine(valley, 1.0);
        assert!(f64::from(dark_crease) < 0.8 * f64::from(crease), "the crease darkens: {crease} → {dark_crease}");
        assert!(dark_side <= side && dark_crease < dark_side, "less away from it: {side} → {dark_side}");
        assert_eq!(shine(plane.clone(), 1.0), shine(plane, 0.0), "a plane hides nothing of itself");
    }

    #[test]
    fn layers_share_one_shading() {
        let s = dome(Material { specular: 1.0, roughness: 0.3, smooth: true, ..Material::default() });
        let tint = |s: &Shade| s.tint.map(|t| t as f32);
        let highlight = |s: &Shade| s.highlight.map(|h| h as f32);
        let layers = s.layers(&[&tint, &highlight]).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!((layers[0].width, layers[0].origin), (layers[1].width, layers[1].origin));
        assert_eq!(s.image(&tint).unwrap(), layers[0]);
        assert!(layers[1].rgba.chunks(4).any(|p| p[0] > 200), "the highlight layer holds the highlight");
        assert!(s.layers(&[]).is_none());
    }

    #[test]
    fn a_draft_shades_a_smaller_image_of_the_same_place() {
        let s = dome(Material { specular: 1.0, roughness: 0.3, smooth: true, ..Material::default() });
        let full = s.image(&grey).unwrap();
        let draft = s.layers_at(&[&grey], DRAFT_SIDE).unwrap().remove(0);
        assert!(draft.width as f64 <= DRAFT_SIDE + 2.0 * PAD as f64 && full.width > 2 * draft.width, "{} {}", full.width, draft.width);
        // The same area, coarser pixels.
        let span = |i: &ShadedImage| f64::from(i.width) * i.step;
        assert!((span(&full) - span(&draft)).abs() < 4.0 * draft.step);
        // Nonsense sizes fall back to the full one.
        assert_eq!(s.layers_at(&[&grey], f64::NAN).unwrap().remove(0), full);
    }
}
