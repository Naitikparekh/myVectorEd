//! The ray-traced render of a 3D effect's facets: an image of the solid in which every sample finds
//! the nearest facet by its depth (so faces that cross or interlock are seen exactly, where the
//! vector art sorts whole faces), is lit by the materials and lights of [`crate::Material::shade`],
//! and sends a ray toward each light to find whether another facet shadows it. A light's Softness
//! spreads its rays over a cone, so its shadows are soft. VectorCraft's own renderer, from the
//! public description of Ray Tracing and its Quality setting on the
//! [Render 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/render-3d-vector-artwork.html)
//! page; it traces direct light only (no reflections between faces).

use vectorcraft_geom::{Point, Rect};

use crate::{Facet, Material, V3, View};

/// One facet to trace, with the color of its paint (0..1 RGB), the opacity its pixels take and its
/// material. A facet with `smooth` set and a smooth light ramp ([`Facet::rgb_gradient`]) is lit by
/// that ramp, as the vector art lights it, instead of by its flat normal.
#[derive(Clone, Copy, Debug)]
pub struct TraceFace<'a> {
    pub facet: &'a Facet,
    pub color: [f64; 3],
    pub alpha: f64,
    pub material: Material,
    pub smooth: bool,
}

/// How finely a render samples the facets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TraceQuality {
    /// 1 pixel a point, one sample a pixel, hard shadows.
    Low,
    /// 2 pixels a point, 2 × 2 samples a pixel, 4 rays to a soft light.
    #[default]
    Medium,
    /// 3 pixels a point, 3 × 3 samples a pixel, 9 rays to a soft light.
    High,
}

impl TraceQuality {
    pub const ALL: [TraceQuality; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.id() == s)
    }

    fn step(self) -> usize {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

/// Most pixels one render holds: past it the render takes fewer pixels a point.
pub const MAX_TRACE_PIXELS: usize = 4_000_000;
/// Most facets one render traces: past it [`trace`] gives `None` and the vector art is drawn.
pub const MAX_TRACE_FACES: usize = 65_536;
/// Most facet tests the shadow rays of one render may make; past it the render has no cast shadows
/// (the faces stay lit by their normals). VectorCraft's own budget.
const SHADOW_BUDGET: usize = 400_000_000;

/// A traced image: `width` × `height` pixels of premultiplied-free RGBA, row by row from the top,
/// its top-left corner at `origin` on the page and `scale` pixels a point.
#[derive(Clone, Debug, PartialEq)]
pub struct Traced {
    pub width: u32,
    pub height: u32,
    pub origin: Point,
    pub scale: f64,
    pub rgba: Vec<u8>,
    /// Whether the shadow rays were traced (within the budget).
    pub shadows: bool,
}

/// A facet ready to trace: its plane's frame and its projected bounds.
struct Prepared<'a> {
    face: TraceFace<'a>,
    bounds: Rect,
}

/// The facets that reach each cell of a grid over the drawing.
struct Grid {
    bounds: Rect,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl Grid {
    fn new(faces: &[Prepared], bounds: Rect) -> Self {
        let side = ((faces.len() as f64).sqrt().ceil() as usize).clamp(1, 256);
        let (cols, rows) = (side, side);
        let mut cells = vec![vec![]; cols * rows];
        let grid = Self { bounds, cols, rows, cells: vec![] };
        for (i, f) in faces.iter().enumerate() {
            let (c0, r0) = grid.cell(Point::new(f.bounds.x0, f.bounds.y0));
            let (c1, r1) = grid.cell(Point::new(f.bounds.x1, f.bounds.y1));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    if let Some(cell) = cells.get_mut(r * cols + c) {
                        cell.push(i);
                    }
                }
            }
        }
        Self { cells, ..grid }
    }

    fn cell(&self, p: Point) -> (usize, usize) {
        let w = self.bounds.width().max(1e-9);
        let h = self.bounds.height().max(1e-9);
        let c = ((p.x - self.bounds.x0) / w * self.cols as f64).floor().clamp(0.0, (self.cols - 1) as f64) as usize;
        let r = ((p.y - self.bounds.y0) / h * self.rows as f64).floor().clamp(0.0, (self.rows - 1) as f64) as usize;
        (c, r)
    }

    fn at(&self, p: Point) -> &[usize] {
        if !self.bounds.contains(p) {
            return &[];
        }
        let (c, r) = self.cell(p);
        self.cells.get(r * self.cols + c).map_or(&[], Vec::as_slice)
    }

    fn cell_size(&self) -> f64 {
        (self.bounds.width() / self.cols as f64).min(self.bounds.height() / self.rows as f64).max(1e-6)
    }
}

/// Nonzero winding of `contours` about `p`.
fn inside(contours: &[Vec<Point>], p: Point) -> bool {
    let mut winding = 0i32;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (Some(a), Some(b)) = (c.get(i), c.get((i + 1) % n)) else { continue };
            if a.y <= p.y {
                if b.y > p.y && (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y) > 0.0 {
                    winding += 1;
                }
            } else if b.y <= p.y && (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y) < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

/// The view-space point of `face`'s plane that projects to `p`, and its depth.
fn view_point(face: &Facet, p: Point) -> Option<V3> {
    let plane = face.plane?;
    let z = plane.depth_at(p)?;
    let f = plane.eye.map_or(1.0, |e| (e - z) / e);
    Some(V3((p.x - plane.center.x) * f, (p.y - plane.center.y) * f, z))
}

/// Where view-space point `v` is drawn, with `face`'s projection.
fn project(face: &Facet, v: V3) -> Option<Point> {
    let plane = face.plane?;
    let f = crate::perspective_scale(plane.eye, v.2);
    let p = Point::new(plane.center.x + v.0 * f, plane.center.y + v.1 * f);
    (p.x.is_finite() && p.y.is_finite()).then_some(p)
}

/// The nearest facet at `p` and its depth.
fn nearest(faces: &[Prepared], grid: &Grid, p: Point) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for &i in grid.at(p) {
        let Some(f) = faces.get(i) else { continue };
        if !f.bounds.contains(p) || !inside(&f.face.facet.contours, p) {
            continue;
        }
        let depth = f.face.facet.depth_at(p).unwrap_or(f.face.facet.depth);
        if best.is_none_or(|(_, d)| depth > d) {
            best = Some((i, depth));
        }
    }
    best
}

/// Unit directions spread over a cone of half-angle `spread` (radians) about `axis`: `count` rays on
/// a golden-angle spiral, the first along the axis.
fn cone(axis: V3, spread: f64, count: usize) -> Vec<V3> {
    let axis = axis.unit();
    if count <= 1 || spread <= 0.0 {
        return vec![axis];
    }
    let helper = if axis.0.abs() < 0.9 { V3(1.0, 0.0, 0.0) } else { V3(0.0, 1.0, 0.0) };
    let u = axis.cross(helper).unit();
    let w = axis.cross(u);
    (0..count)
        .map(|k| {
            let r = spread * ((k as f64 + 0.5) / count as f64).sqrt();
            let a = k as f64 * 2.399_963_229_728_653;
            let (s, c) = (r.sin(), r.cos());
            V3(
                axis.0 * c + (u.0 * a.cos() + w.0 * a.sin()) * s,
                axis.1 * c + (u.1 * a.cos() + w.1 * a.sin()) * s,
                axis.2 * c + (u.2 * a.cos() + w.2 * a.sin()) * s,
            )
            .unit()
        })
        .collect()
}

/// The widest cone a light at full Softness spreads its rays over, in radians (VectorCraft's own).
const MAX_SPREAD: f64 = 0.35;

struct Shadows<'a> {
    faces: &'a [Prepared<'a>],
    grid: &'a Grid,
    /// How far a ray is followed, in view-space points.
    reach: f64,
    stamp: Vec<u32>,
    pass: u32,
}

impl Shadows<'_> {
    /// Is the ray from `from` (on facet `own`) along `dir` blocked by another facet?
    fn blocked(&mut self, own: usize, from: V3, dir: V3) -> bool {
        let Some(origin_face) = self.faces.get(own) else { return false };
        let facet = origin_face.face.facet;
        let eps = 1e-6 * self.reach.max(1.0);
        let mut to = V3(from.0 + dir.0 * self.reach, from.1 + dir.1 * self.reach, from.2 + dir.2 * self.reach);
        // Keep the far end in front of the eye.
        if let Some(e) = facet.plane.and_then(|p| p.eye)
            && to.2 > 0.99 * e
            && dir.2 > 1e-12
        {
            let t = ((0.99 * e - from.2) / dir.2).max(0.0);
            to = V3(from.0 + dir.0 * t, from.1 + dir.1 * t, from.2 + dir.2 * t);
        }
        let (Some(a), Some(b)) = (project(facet, from), project(facet, to)) else { return false };
        self.pass = self.pass.wrapping_add(1);
        let step = self.grid.cell_size() / 2.0;
        let length = a.distance(b);
        let steps = ((length / step).ceil() as usize).clamp(1, 4 * (self.grid.cols + self.grid.rows));
        for k in 0..=steps {
            let p = a.lerp(b, k as f64 / steps as f64);
            if !self.grid.bounds.contains(p) && k > 0 {
                // Past the drawing's bounds there is nothing more to hit.
                if (p - a).dot(b - a) > 0.0 && !self.grid.bounds.inflate(step, step).contains(p) {
                    break;
                }
                continue;
            }
            for &i in self.grid.at(p) {
                if i == own || self.stamp.get(i) == Some(&self.pass) {
                    continue;
                }
                if let Some(s) = self.stamp.get_mut(i) {
                    *s = self.pass;
                }
                let Some(other) = self.faces.get(i) else { continue };
                let Some(plane) = other.face.facet.plane else { continue };
                let n = V3(plane.normal[0], plane.normal[1], plane.normal[2]);
                let den = n.dot(dir);
                if den.abs() < 1e-12 {
                    continue;
                }
                let t = (plane.offset - n.dot(from)) / den;
                if !t.is_finite() || t <= eps {
                    continue;
                }
                let hit = V3(from.0 + dir.0 * t, from.1 + dir.1 * t, from.2 + dir.2 * t);
                if let Some(q) = project(other.face.facet, hit)
                    && other.bounds.contains(q)
                    && inside(&other.face.facet.contours, q)
                {
                    return true;
                }
            }
        }
        false
    }
}

/// Trace `faces` seen with `view` at `quality`. `None` when there is nothing to draw, past
/// [`MAX_TRACE_FACES`], or when the facets have no finite bounds.
pub fn trace(faces: &[TraceFace], view: &View, quality: TraceQuality) -> Option<Traced> {
    if faces.is_empty() || faces.len() > MAX_TRACE_FACES {
        return None;
    }
    let prepared: Vec<Prepared> = faces
        .iter()
        .filter_map(|&face| {
            let mut points = face.facet.contours.iter().flatten();
            let first = *points.next()?;
            let bounds = points.fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
            [bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite()).then_some(Prepared { face, bounds })
        })
        .collect();
    let bounds = prepared.iter().map(|f| f.bounds).reduce(|a, b| a.union(b))?;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return None;
    }
    let grid = Grid::new(&prepared, bounds);
    let mut scale = quality.step() as f64;
    let area = bounds.width() * bounds.height();
    if area * scale * scale > MAX_TRACE_PIXELS as f64 {
        scale = (MAX_TRACE_PIXELS as f64 / area).sqrt();
    }
    let width = ((bounds.width() * scale).ceil() as usize).max(1);
    let height = ((bounds.height() * scale).ceil() as usize).max(1);
    if width.saturating_mul(height) > MAX_TRACE_PIXELS.saturating_mul(2) {
        return None;
    }
    let sub = quality.step();
    let lights = view.light_list();
    let size = bounds.width().max(bounds.height());
    let rays: Vec<Vec<V3>> = lights
        .iter()
        .map(|l| {
            let [x, y, z] = l.direction();
            cone(V3(x, y, z), l.softness * MAX_SPREAD, if l.softness > 0.0 { sub * sub } else { 1 })
        })
        .collect();
    let ray_count: usize = rays.iter().map(Vec::len).sum();
    let samples = width * height * sub * sub;
    // Each shadow ray tests the facets of the cells it crosses: about as many as a few cells hold.
    let per_ray = prepared.len().min(64);
    let shadows_on = view.shade && samples.saturating_mul(ray_count).saturating_mul(per_ray) <= SHADOW_BUDGET;
    let mut shadows = Shadows { faces: &prepared, grid: &grid, reach: 4.0 * size, stamp: vec![0; prepared.len()], pass: 0 };
    let mut rgba = vec![0u8; width * height * 4];
    let mut visibility = vec![1.0; lights.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f64; 4];
            for sy in 0..sub {
                for sx in 0..sub {
                    let p = Point::new(
                        bounds.x0 + (x as f64 + (sx as f64 + 0.5) / sub as f64) / scale,
                        bounds.y0 + (y as f64 + (sy as f64 + 0.5) / sub as f64) / scale,
                    );
                    let Some((i, _)) = nearest(&prepared, &grid, p) else { continue };
                    let Some(f) = prepared.get(i) else { continue };
                    let face = f.face;
                    let rgb = if face.smooth
                        && let Some(g) = face.facet.rgb_gradient
                    {
                        let l = g.at(p);
                        std::array::from_fn(|c| face.color[c] * l[c])
                    } else {
                        for (v, rays) in visibility.iter_mut().zip(&rays) {
                            *v = 1.0;
                            if shadows_on && let Some(from) = view_point(face.facet, p) {
                                let open = rays.iter().filter(|dir| !shadows.blocked(i, from, **dir)).count();
                                *v = open as f64 / rays.len().max(1) as f64;
                            }
                        }
                        face.material.shade_lit(view, face.facet.normal, face.color, &visibility)
                    };
                    let a = face.alpha.clamp(0.0, 1.0);
                    for c in 0..3 {
                        sum[c] += rgb[c] * a;
                    }
                    sum[3] += a;
                }
            }
            let n = (sub * sub) as f64;
            let alpha = sum[3] / n;
            let at = (y * width + x) * 4;
            if let Some(px) = rgba.get_mut(at..at + 4) {
                for (c, out) in px.iter_mut().take(3).enumerate() {
                    let v = if sum[3] > 0.0 { sum[c] / sum[3] } else { 0.0 };
                    *out = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                if let Some(a) = px.get_mut(3) {
                    *a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    Some(Traced { width: width as u32, height: height as u32, origin: Point::new(bounds.x0, bounds.y0), scale, rgba, shadows: shadows_on })
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::{BezPath, FillRule, PathData};

    use super::*;
    use crate::{Extrude, Light, Lights};

    fn square(x: f64, y: f64, s: f64) -> PathData {
        let mut b = BezPath::new();
        b.move_to((x, y));
        b.line_to((x + s, y));
        b.line_to((x + s, y + s));
        b.line_to((x, y + s));
        b.close_path();
        PathData::from_bezpath(&b)
    }

    fn front() -> View {
        View { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, ..View::default() }
    }

    const MATTE: Material = Material { base: None, roughness: 1.0, metallic: 0.0 };

    fn faces(facets: &[Facet]) -> Vec<TraceFace<'_>> {
        facets.iter().map(|facet| TraceFace { facet, color: [0.8, 0.2, 0.1], alpha: 1.0, material: MATTE, smooth: false }).collect()
    }

    fn pixel(t: &Traced, p: Point) -> [u8; 4] {
        let x = ((p.x - t.origin.x) * t.scale) as usize;
        let y = ((p.y - t.origin.y) * t.scale) as usize;
        let at = (y * t.width as usize + x) * 4;
        [t.rgba[at], t.rgba[at + 1], t.rgba[at + 2], t.rgba[at + 3]]
    }

    #[test]
    fn a_plane_seen_straight_on_fills_its_square_in_its_shaded_color() {
        let view = View { light_rotation: 0.0, light_height: 90.0, light_intensity: 1.0, ..front() };
        let facets = crate::plane(&square(100.0, 100.0, 50.0), FillRule::NonZero, Rect::new(100.0, 100.0, 150.0, 150.0), view).unwrap();
        for q in TraceQuality::ALL {
            let t = trace(&faces(&facets), &view, q).unwrap();
            assert_eq!(t.scale, q.step() as f64);
            assert_eq!((t.width, t.height), (50 * q.step() as u32, 50 * q.step() as u32));
            // Lit straight on, the color is the paint's own.
            let px = pixel(&t, Point::new(125.0, 125.0));
            assert!(px.iter().zip([204, 51, 26, 255]).all(|(a, b)| a.abs_diff(b) <= 1), "{q:?}: {px:?}");
        }
    }

    #[test]
    fn the_nearest_facet_is_seen_and_a_facet_behind_another_is_hidden() {
        // A box seen from the upper left with its hidden faces kept: the back faces are behind.
        let view = View { rotation_x: -20.0, rotation_y: 25.0, rotation_z: 0.0, ..View::default() };
        let opts = Extrude { hidden: true, ..Extrude::default() };
        let facets = crate::extrude(&square(0.0, 0.0, 100.0), FillRule::NonZero, Rect::new(0.0, 0.0, 100.0, 100.0), opts, view).unwrap();
        let prepared: Vec<Prepared> = faces(&facets)
            .into_iter()
            .map(|face| {
                let pts: Vec<Point> = face.facet.contours.iter().flatten().copied().collect();
                let b = pts.iter().fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
                Prepared { face, bounds: b }
            })
            .collect();
        let bounds = prepared.iter().map(|f| f.bounds).reduce(|a, b| a.union(b)).unwrap();
        let grid = Grid::new(&prepared, bounds);
        let centre = Point::new(50.0, 50.0);
        let (i, _) = nearest(&prepared, &grid, centre).unwrap();
        let seen = prepared[i].face.facet;
        assert!(!seen.away, "a face turned toward the viewer is seen");
        assert_eq!(seen.end, Some(crate::PlanarFace::Front));
        // A ray from the front cap straight into the solid meets the back cap; one away from it meets nothing.
        let mut shadows = Shadows { faces: &prepared, grid: &grid, reach: 400.0, stamp: vec![0; prepared.len()], pass: 0 };
        let from = view_point(seen, centre).unwrap();
        let [x, y, z] = seen.normal;
        assert!(shadows.blocked(i, from, V3(-x, -y, -z)));
        assert!(!shadows.blocked(i, from, V3(x, y, z)));
    }

    #[test]
    fn a_light_whose_rays_are_blocked_leaves_the_face_in_shadow() {
        // An unblocked plane lit straight on is fully lit; the share of a light's rays that are
        // blocked takes that share of its light away.
        let light = Light { rotation: 0.0, height: 90.0, intensity: 1.0, ..Light::default() };
        let view = View { lights: Some(Lights::new(&[light]).unwrap()), ambient: 0.0, ..front() };
        let facets = crate::plane(&square(0.0, 0.0, 10.0), FillRule::NonZero, Rect::new(0.0, 0.0, 10.0, 10.0), view).unwrap();
        let lit = trace(&faces(&facets), &view, TraceQuality::Low).unwrap();
        assert!(lit.shadows);
        assert_eq!(pixel(&lit, Point::new(5.0, 5.0))[0], 204);
        let shade = MATTE.shade_lit(&view, [0.0, 0.0, 1.0], [0.8, 0.2, 0.1], &[0.0]);
        assert_eq!(shade, [0.0; 3], "a fully shadowed light adds nothing");
        let half = MATTE.shade_lit(&view, [0.0, 0.0, 1.0], [0.8, 0.2, 0.1], &[0.5]);
        assert!((half[0] - 0.4).abs() < 1e-12);
    }

    #[test]
    fn a_wall_shadows_the_floor_beside_it() {
        // An L extruded and tilted toward the viewer so the top wall of its short arm (facing up)
        // is seen, lit low from the upper left: the tall arm shadows that wall near it.
        let mut b = BezPath::new();
        for (i, p) in [(0.0, 0.0), (20.0, 0.0), (20.0, 80.0), (100.0, 80.0), (100.0, 100.0), (0.0, 100.0)].into_iter().enumerate() {
            if i == 0 { b.move_to(p) } else { b.line_to(p) }
        }
        b.close_path();
        let path = PathData::from_bezpath(&b);
        let light = Light { rotation: 160.0, height: 10.0, intensity: 1.0, ..Light::default() };
        let view = View {
            rotation_x: -30.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            lights: Some(Lights::new(&[light]).unwrap()),
            ambient: 0.0,
            ..View::default()
        };
        let opts = Extrude { hidden: true, depth: 60.0, ..Extrude::default() };
        let facets = crate::extrude(&path, FillRule::NonZero, Rect::new(0.0, 0.0, 100.0, 100.0), opts, view).unwrap();
        let t = trace(&faces(&facets), &view, TraceQuality::Low).unwrap();
        assert!(t.shadows);
        // The top of the short arm: near the tall arm it is in the arm's shadow, far from it lit.
        let top: Vec<&Facet> = facets.iter().filter(|f| !f.away && f.side && f.normal[1] < -0.5).collect();
        assert!(!top.is_empty(), "the short arm's top is seen");
        let row = |x: f64| {
            // Down the drawing from the top of the image, the first pixel of an upward face at x.
            (0..t.height).find_map(|y| {
                let p = Point::new(x, t.origin.y + (y as f64 + 0.5) / t.scale);
                let on_top = top.iter().any(|f| inside(&f.contours, p));
                on_top.then(|| pixel(&t, p))
            })
        };
        let near = row(24.0).unwrap();
        let far = row(95.0).unwrap();
        assert!(near[0] < far[0], "shadowed {near:?} vs lit {far:?}");
    }

    #[test]
    fn nothing_to_trace_gives_none_and_bad_input_stays_finite() {
        assert!(trace(&[], &View::default(), TraceQuality::High).is_none());
        let cone = cone(V3(0.0, 0.0, 1.0), 0.3, 9);
        assert_eq!(cone.len(), 9);
        assert!(cone.iter().all(|v| (v.dot(*v) - 1.0).abs() < 1e-9 && v.2 > 0.9));
    }
}
