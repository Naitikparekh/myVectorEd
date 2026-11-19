//! Render Settings › Ray Tracing for Extrude & Bevel, Inflate and Rotate: `render.rayTracing` draws the
//! effect's solids as one image traced by [`vectorcraft_three_d::trace`] in place of the shaded
//! vector faces, at `render.quality` (`low`, `medium` the default, or `high`), as the public
//! [Render 3D vector artwork](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/render-3d-vector-artwork.html)
//! page describes turning on Ray Tracing and choosing its Quality.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::color::Paint;
use vectorcraft_doc::{AppearanceItem, Document, Effect, ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, PathData};
use vectorcraft_three_d::{Decal, Facet, Material, TraceFace, TraceQuality};

use super::{EXTRUDE, INFLATE, Profile, RENDER, ROTATE};

/// The 3D effects Ray Tracing applies to.
const TRACED: [&str; 4] = [EXTRUDE, INFLATE, ROTATE, crate::REVOLVE];

/// The quality effect `id` with parameters `p` is ray traced at, `None` when it is not.
pub(crate) fn ray_traced(id: &str, p: &Value) -> Option<TraceQuality> {
    let render = p.get(RENDER)?;
    if !TRACED.contains(&id) || render.get("rayTracing").and_then(Value::as_bool) != Some(true) || super::wireframe::wireframe_params(id, p) {
        return None;
    }
    Some(render.get("quality").and_then(Value::as_str).and_then(TraceQuality::parse).unwrap_or_default())
}

/// The material without one: fully rough, so a face is shaded as the vector art shades it.
const DIFFUSE: Material = Material { base: None, roughness: 1.0, metallic: 0.0 };

/// The color (0..1 RGB) a paint gives the traced faces: a solid color its own, a gradient the color
/// at its middle; `None` for a pattern or no paint (that solid is left out).
fn color_of(paint: &Paint) -> Option<[f64; 3]> {
    if let Some(c) = paint.color() {
        return Some(c.to_rgb().map(f64::from));
    }
    match paint {
        Paint::Gradient(g) => Some(g.gradient.sample(0.5).0.to_rgb().map(f64::from)),
        _ => None,
    }
}

fn opacity_of(item: &AppearanceItem, member: f32) -> f64 {
    let own = match item {
        AppearanceItem::Fill(f) => f.opacity,
        AppearanceItem::Stroke(s) => s.opacity,
    };
    f64::from((own * member).clamp(0.0, 1.0))
}

/// Each solid of `fx` on `shapes` with the paint color and opacity it is traced in. Extrude & Bevel
/// keeps the faces turned away from the viewer, so they can cast shadows. `None` when a solid can't
/// be built (the canvas then draws the source art, as for the vector faces).
/// One solid's facets, the color of its paint (0..1 RGB), its opacity, and whether it is a fill's.
type TracedSolid = (Vec<Facet>, [f64; 3], f64, bool);

/// Each solid of `fx` on `shapes` ([`TracedSolid`]), and the bounds they are built about.
fn solids(fx: &Effect, p: &Value, shapes: &[Profile]) -> Option<(Vec<TracedSolid>, vectorcraft_geom::Rect)> {
    let b = super::union_bounds(shapes)?;
    let view = super::view_options(p);
    let mut out = vec![];
    let mut total = 0usize;
    let mut keep = |f: Vec<Facet>, item: &AppearanceItem, member: f32, out: &mut Vec<_>| -> Option<()> {
        total = total.saturating_add(f.len());
        if total > vectorcraft_three_d::MAX_TRACE_FACES {
            return None;
        }
        let paint = match item {
            AppearanceItem::Fill(f) => &f.paint,
            AppearanceItem::Stroke(s) => &s.paint,
        };
        if let Some(color) = color_of(paint) {
            out.push((f, color, opacity_of(item, member), matches!(item, AppearanceItem::Fill(_))));
        }
        Some(())
    };
    if fx.id == ROTATE {
        for s in shapes {
            for item in &s.items {
                let (area, rule) = super::region(&s.path, s.rule, item);
                let f = vectorcraft_three_d::plane(&area, rule, b, view).ok()?;
                keep(f, item, s.opacity, &mut out)?;
            }
        }
        return Some((out, b));
    }
    let (parts, bounds) = super::solid_parts(shapes, b);
    if fx.id == EXTRUDE {
        let options = vectorcraft_three_d::Extrude { hidden: true, ..super::extrude_options(p) };
        for (item, opacity, area, rule, ..) in &parts {
            match vectorcraft_three_d::extrude(area, *rule, bounds, options, view) {
                Ok(f) => keep(f, item, *opacity, &mut out)?,
                // A stroke's solid past its limits is left out, as in the vector art.
                Err(_) if matches!(item, AppearanceItem::Stroke(_)) => {}
                Err(_) => return None,
            }
        }
        return Some((out, bounds));
    }
    for ((item, opacity, ..), f) in parts.iter().zip(super::built_solids(&fx.id, p, &parts, bounds, super::MAX_OBJECT_FACES)) {
        if let Some(f) = f.ok()? {
            keep(f, item, *opacity, &mut out)?;
        }
    }
    Some((out, bounds))
}

/// The filled areas of art `node` (a graphic's art as drawn on a face, unlit), as decals in
/// drawing order: each path's top fill in its solid color, or a gradient's middle color, at the
/// fill's opacity times the opacities of the groups it is in. Patterns are left out.
fn decals_of(node: &Node, opacity: f64, out: &mut Vec<Decal>) {
    let opacity = opacity * f64::from(node.opacity);
    match &node.kind {
        NodeKind::Path { rule, .. } => {
            let Some(path) = node.path_data() else { return };
            for item in &node.appearance.items {
                if let AppearanceItem::Fill(f) = item
                    && let Some(color) = color_of(&f.paint)
                    && let Some(d) =
                        Decal::new(path.to_bezpath(), *rule == vectorcraft_geom::FillRule::EvenOdd, color, opacity * f64::from(f.opacity))
                {
                    out.push(d);
                }
            }
        }
        NodeKind::Group { children, clip } => {
            // A clip group's first child is its clip path: the decal is limited to its facet anyway.
            for child in children.iter().skip(usize::from(*clip)) {
                decals_of(child, opacity, out);
            }
        }
        _ => {}
    }
}

/// `facets` unlit: every facet in full light with no light ramp, so the graphics drawn on them keep
/// their own colors, which the tracer then lights.
fn unlit(facets: &[Facet]) -> Vec<Facet> {
    facets
        .iter()
        .cloned()
        .map(|mut f| {
            f.illumination = [1.0; 3];
            f.brightness = 1.0;
            f.rgb_gradient = None;
            f
        })
        .collect()
}

/// For each of `facets` (a solid's, about `bounds`), the decals of the graphics of effect `id` with
/// parameters `p` drawn on it: the caps' graphics on each cap the viewer sees, and those on Extrude &
/// Bevel's sides or Inflate's front on each facet they fall on ([`super::graphics::facet_parts`]).
fn facet_decals(
    doc: Option<&Document>,
    id: NodeId,
    (effect, p): (&str, &Value),
    bounds: vectorcraft_geom::Rect,
    facets: &[Facet],
) -> Vec<Vec<Decal>> {
    let mut out: Vec<Vec<Decal>> = vec![vec![]; facets.len()];
    let list = super::mapped_graphics(p);
    if doc.is_none() || list.is_empty() {
        return out;
    }
    let flat = unlit(facets);
    let caps: Vec<&Facet> = super::seen_ends(&flat).collect();
    let mut elements = 0usize;
    for (face, art) in super::graphic_art(doc, id, (effect, p), bounds, &caps, 1.0, &mut elements) {
        let mut decals = vec![];
        decals_of(&art, 1.0, &mut decals);
        for (slot, f) in out.iter_mut().zip(facets) {
            if f.end == Some(face) && !f.away {
                slot.extend(decals.iter().cloned());
            }
        }
    }
    if (effect == EXTRUDE || effect == INFLATE)
        && let Some(parts) = super::graphics::facet_parts(doc, id, &list, &flat, effect == EXTRUDE)
    {
        for (slot, parts) in out.iter_mut().zip(parts) {
            for part in parts {
                decals_of(&part, 1.0, slot);
            }
        }
    }
    out
}

/// The traced art of `fx` on `shapes` for object `id`: one image of every solid, lit by the effect's
/// lights and materials ([`super::material`]), with shadows the solids cast on themselves and each
/// other. `None` when `fx` is not ray traced, or when there is nothing to trace or it is past
/// [`vectorcraft_three_d::MAX_TRACE_FACES`]: the vector faces are drawn instead.
pub(crate) fn traced_surfaces(doc: Option<&Document>, id: NodeId, fx: &Effect, shapes: &[Profile]) -> Option<Vec<Arc<Node>>> {
    let p = crate::merged_params(&fx.id, &fx.params);
    let quality = ray_traced(&fx.id, &p)?;
    let (built, bounds) = solids(fx, &p, shapes)?;
    let materials = super::material::materials(&fx.id, &p);
    let view = super::view_options(&p);
    // The graphics go on the first fill's solid ([`facet_decals`]).
    let first_fill = built.iter().position(|s| s.3);
    let decals: Vec<Vec<Vec<Decal>>> = built
        .iter()
        .enumerate()
        .map(|(k, (facets, ..))| if Some(k) == first_fill { facet_decals(doc, id, (&fx.id, &p), bounds, facets) } else { vec![vec![]; facets.len()] })
        .collect();
    let faces: Vec<TraceFace> = built
        .iter()
        .zip(&decals)
        .flat_map(|((facets, color, alpha, _), decals)| {
            facets.iter().zip(decals).map(move |(facet, decals)| {
                let material = materials.as_ref().and_then(|m| m.of(facet));
                TraceFace { facet, color: *color, alpha: *alpha, material: material.unwrap_or(DIFFUSE), smooth: material.is_none(), decals }
            })
        })
        .collect();
    Some(vec![traced_image(id, &faces, &view, quality)?])
}

/// The image of `faces` traced with `view` at `quality` ([`vectorcraft_three_d::trace`]), placed on
/// the page, as a node of object `id`.
fn traced_image(id: NodeId, faces: &[TraceFace], view: &vectorcraft_three_d::View, quality: TraceQuality) -> Option<Arc<Node>> {
    let traced = vectorcraft_three_d::trace(faces, view, quality)?;
    let image = image::RgbaImage::from_raw(traced.width, traced.height, traced.rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    let blob = ImageBlob::png(png.into_inner());
    let im = ImageObject {
        key: blob.content_key(),
        width: traced.width,
        height: traced.height,
        xf: Affine::translate((traced.origin.x, traced.origin.y)) * Affine::scale(1.0 / traced.scale),
        link: None,
        placement: Default::default(),
        pixels: Some(Arc::new(blob)),
    };
    Some(Arc::new(Node::new(id, NodeKind::Image(Box::new(im)))))
}

/// The smooth light of Revolve's face `face` as independent RGB ramps, for its facet: its own RGB
/// ramps under a list of lights or a colored light, else its white light ramp (ambient included),
/// else its brightness ramp. `None` for a flat face.
fn smooth_light(face: &vectorcraft_three_d::Face) -> Option<vectorcraft_three_d::RgbGradient> {
    use vectorcraft_three_d::{RgbGradient, ShadeGradient};
    if let Some(g) = face.rgb_gradient {
        return Some(g);
    }
    if let Some(g) = face.light_gradient {
        let channel = |c: f64| ShadeGradient { brightness: g.diffuse.brightness.map(|b| g.ambient + b * c), ..g.diffuse };
        return Some(RgbGradient { channels: g.color.map(channel) });
    }
    face.gradient.map(|g| RgbGradient { channels: [g; 3] })
}

/// One surface Revolve sweeps for the ray-traced image: its profile with its sweep's options, and
/// the paint (with the member's opacity) it is drawn in.
pub(crate) struct RevolveSurface<'a> {
    pub path: &'a PathData,
    pub options: vectorcraft_three_d::Revolve,
    pub item: &'a AppearanceItem,
    pub member: f32,
    /// The effect's graphics go on this surface's side.
    pub graphics: bool,
}

/// For each of Revolve's `faces`, the decals of the graphics in `p` drawn on it, unlit
/// ([`super::graphics::surface_parts`]). None past the graphics' limits.
fn revolve_decals(doc: Option<&Document>, id: NodeId, p: &Value, faces: &[vectorcraft_three_d::Face]) -> Vec<Vec<Decal>> {
    let mut out: Vec<Vec<Decal>> = vec![vec![]; faces.len()];
    let list = super::mapped_graphics(p);
    if doc.is_none() || list.is_empty() {
        return out;
    }
    let flat: Vec<vectorcraft_three_d::Face> = faces
        .iter()
        .cloned()
        .map(|mut f| {
            f.illumination = [1.0; 3];
            f.brightness = 1.0;
            (f.gradient, f.light_gradient, f.rgb_gradient) = (None, None, None);
            f
        })
        .collect();
    let mut copies = 0usize;
    if let Some(parts) = super::graphics::surface_parts(doc, id, &list, &flat, &|_, c| Paint::solid(c), &mut copies, false) {
        for (slot, parts) in out.iter_mut().zip(parts) {
            for part in parts {
                decals_of(&part, 1.0, slot);
            }
        }
    }
    out
}

/// The ray-traced art of Revolve `fx` for object `id`: one image of the surfaces swept about the
/// bounds `b`, lit by the effect's lights with each face's smooth light, with the shadows the
/// surfaces cast on themselves and each other. `None` when `fx` is not ray traced, when a surface
/// can't be swept, or when there is nothing to trace or too much: the vector faces are drawn then.
pub(crate) fn revolve_traced(
    doc: Option<&Document>,
    id: NodeId,
    fx: &Effect,
    surfaces: &[RevolveSurface],
    b: vectorcraft_geom::Rect,
) -> Option<Arc<Node>> {
    let p = crate::merged_params(&fx.id, &fx.params);
    let quality = ray_traced(&fx.id, &p)?;
    let mut solids: Vec<TracedSolid> = vec![];
    let mut decals: Vec<Vec<Vec<Decal>>> = vec![];
    let mut total = 0usize;
    for s in surfaces {
        let paint = match s.item {
            AppearanceItem::Fill(f) => &f.paint,
            AppearanceItem::Stroke(st) => &st.paint,
        };
        let Some(color) = color_of(paint) else { continue };
        let options = vectorcraft_three_d::Revolve { map_surface: s.graphics, ..s.options };
        let faces = vectorcraft_three_d::revolve(s.path, b, options).ok()?;
        total = total.saturating_add(faces.len());
        if total > vectorcraft_three_d::MAX_TRACE_FACES {
            return None;
        }
        decals.push(if s.graphics { revolve_decals(doc, id, &p, &faces) } else { vec![vec![]; faces.len()] });
        let facets = faces
            .into_iter()
            .map(|face| {
                let light = smooth_light(&face);
                let mut facet = Facet::of_face(face);
                facet.rgb_gradient = light;
                facet
            })
            .collect();
        solids.push((facets, color, opacity_of(s.item, s.member), true));
    }
    let view = super::view_options(&p);
    let faces: Vec<TraceFace> = solids
        .iter()
        .zip(&decals)
        .flat_map(|((facets, color, alpha, _), decals)| {
            facets.iter().zip(decals).map(move |(facet, decals)| TraceFace {
                facet,
                color: *color,
                alpha: *alpha,
                material: DIFFUSE,
                smooth: true,
                decals,
            })
        })
        .collect();
    traced_image(id, &faces, &view, quality)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::color::Color;
    use vectorcraft_doc::{Appearance, Document, Node, NodeId};
    use vectorcraft_geom::{BezPath, PathData};

    use super::*;

    fn square(params: Value, id: &str) -> Node {
        let mut b = BezPath::new();
        b.move_to((100.0, 100.0));
        b.line_to((200.0, 100.0));
        b.line_to((200.0, 200.0));
        b.line_to((100.0, 200.0));
        b.close_path();
        let red = Paint::solid(Color::rgb(0.8, 0.2, 0.1));
        let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(red, Paint::None, 0.0));
        n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
        n
    }

    fn images(art: &Node) -> Vec<ImageObject> {
        let mut out = vec![];
        art.walk(&mut |n| {
            if let NodeKind::Image(im) = &n.kind {
                out.push((**im).clone());
            }
        });
        out
    }

    #[test]
    fn ray_tracing_draws_one_image_of_the_solid_in_place_of_its_faces() {
        for id in [EXTRUDE, INFLATE, ROTATE] {
            let shaded = super::super::solid_art(&square(json!({}), id)).unwrap();
            assert!(images(&shaded).is_empty(), "{id}: vector faces by default");
            let p = json!({"render": {"mode": "shaded", "rayTracing": true, "quality": "low"}});
            let art = super::super::solid_art(&square(p, id)).unwrap();
            let im = images(&art);
            assert_eq!(im.len(), 1, "{id}");
            let im = &im[0];
            assert!(im.pixels.is_some());
            // At low quality one pixel a point, over the bounds of the faces drawn.
            assert_eq!(im.xf.as_coeffs()[0], 1.0, "{id}");
            let faces = shaded.visual_bounds().unwrap();
            let placed = im.xf.transform_rect_bbox(vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)));
            assert!((placed.x0 - faces.x0).abs() < 2.0 && (placed.y1 - faces.y1).abs() < 2.0, "{id}: {placed:?} {faces:?}");
            // Higher quality, more pixels a point.
            let p = json!({"render": {"rayTracing": true, "quality": "high"}});
            let high = &images(&super::super::solid_art(&square(p, id)).unwrap())[0];
            assert_eq!(high.xf.as_coeffs()[0], 1.0 / 3.0, "{id}");
        }
    }

    #[test]
    fn render_settings_are_checked() {
        let n = square(json!({}), EXTRUDE);
        let ok = |p: Value| super::super::validate_three_d(&n, EXTRUDE, &p);
        assert!(ok(json!({"render": {"rayTracing": true, "quality": "medium"}})).is_ok());
        assert!(ok(json!({"render": {"rayTracing": "yes"}})).is_err());
        assert!(ok(json!({"render": {"quality": "ultra"}})).is_err());
        assert!(ok(json!({"render": {"mode": "shaded", "noise": 1}})).is_err());
        assert_eq!(ray_traced(EXTRUDE, &json!({"render": {"rayTracing": true}})), Some(TraceQuality::Medium));
        assert_eq!(ray_traced(EXTRUDE, &json!({"render": {"mode": "wireframe", "rayTracing": true}})), None, "wireframe wins");
        assert_eq!(ray_traced(crate::REVOLVE, &json!({"render": {"rayTracing": true}})), Some(TraceQuality::Medium));
        assert_eq!(ray_traced(crate::CLASSIC_REVOLVE, &json!({"render": {"rayTracing": true}})), None);
    }

    #[test]
    fn revolve_draws_one_traced_image_of_its_surfaces_over_its_shadow() {
        let mut b = BezPath::new();
        b.move_to((100.0, 100.0));
        b.line_to((140.0, 100.0));
        b.line_to((140.0, 200.0));
        b.line_to((100.0, 200.0));
        b.close_path();
        let red = Paint::solid(Color::rgb(0.8, 0.2, 0.1));
        let node = |p: Value| {
            let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(red.clone(), Paint::None, 0.0));
            n.appearance.effects.push(crate::new_effect(crate::REVOLVE, &p).unwrap());
            n
        };
        let vector = crate::revolve_art(&node(json!({}))).unwrap();
        assert!(images(&vector).is_empty());
        let p = json!({"shadow": true, "render": {"rayTracing": true, "quality": "low"}});
        let art = crate::revolve_art(&node(p)).unwrap();
        let im = images(&art);
        assert_eq!(im.len(), 1, "one image in place of the faces");
        // The image covers the swept surface; the vector shadow stays under it.
        let placed = im[0].xf.transform_rect_bbox(vectorcraft_geom::Rect::new(0.0, 0.0, f64::from(im[0].width), f64::from(im[0].height)));
        let faces = vector.visual_bounds().unwrap();
        assert!((placed.width() - faces.width()).abs() < 3.0 && (placed.height() - faces.height()).abs() < 3.0, "{placed:?} {faces:?}");
        let children = art.children().unwrap();
        assert!(matches!(children.last().map(|c| &c.kind), Some(NodeKind::Image(_))), "the image is drawn last, over the shadow");
        assert!(children.len() >= 2);
        // A group's members are traced together into one image, about one shared axis.
        let member = |dy: f64| {
            let mut m = Node::path(NodeId(2), PathData::from_bezpath(&b), Appearance::basic(red.clone(), Paint::None, 0.0));
            m.transform(Affine::translate((0.0, dy)), false);
            Arc::new(m)
        };
        let mut g = Node::group(NodeId(5), vec![member(0.0), member(150.0)]);
        g.appearance.effects.push(crate::new_effect(crate::REVOLVE, &json!({"render": {"rayTracing": true, "quality": "low"}})).unwrap());
        let art = crate::revolve_art(&g).unwrap();
        assert_eq!(images(&art).len(), 1);
    }

    /// A document with symbol "Mark": a red 10 × 10 square.
    fn marked() -> Document {
        let mut d = Document::new(1000.0, 1000.0);
        let art = Node::path(
            NodeId(0),
            vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0)),
            Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
        );
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Mark".into(), art: Arc::new(art) });
        d
    }

    /// The color of the traced image in `art` at page point `p`.
    fn traced_at(art: &Node, p: vectorcraft_geom::Point) -> [u8; 4] {
        let im = images(art).into_iter().next().expect("a traced image");
        let blob = im.pixels.expect("its pixels");
        let img = image::load_from_memory(&blob.bytes).expect("a PNG").to_rgba8();
        let q = im.xf.inverse() * p;
        img.get_pixel(q.x as u32, q.y as u32).0
    }

    #[test]
    fn graphics_are_painted_into_the_traced_image_on_their_faces() {
        let d = marked();
        let straight =
            json!({"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0, "shade": false, "render": {"rayTracing": true, "quality": "low"}});
        let centre = vectorcraft_geom::Point::new(150.0, 150.0);
        let corner = vectorcraft_geom::Point::new(105.0, 105.0);
        for id in [EXTRUDE, INFLATE, ROTATE] {
            let mut p = straight.clone();
            p["graphics"] = json!([{"symbol": "Mark", "scale": 300.0}]);
            let art = super::super::solid_art_in(Some(&d), &square(p, id)).unwrap();
            assert_eq!(traced_at(&art, centre), [255, 0, 0, 255], "{id}: the graphic's red at its center");
            let paint = traced_at(&art, corner);
            assert!(paint[0] > 150 && paint[1] > 30, "{id}: the object's own paint away from it: {paint:?}");
            // Without the document's symbol nothing is painted.
            let art = super::super::solid_art_in(None, &square(straight.clone(), id)).unwrap();
            assert_ne!(traced_at(&art, centre), [255, 0, 0, 255], "{id}");
        }
        // A graphic on Extrude & Bevel's sides is painted on the wall it names.
        let mut p = straight.clone();
        p["rotationY"] = json!(-40.0);
        p["graphics"] = json!([{"symbol": "Mark", "face": "side", "x": 150.0, "scale": 200.0}]);
        let side = super::super::solid_art_in(Some(&d), &square(p.clone(), EXTRUDE)).unwrap();
        p["rotationY"] = json!(40.0);
        let other = super::super::solid_art_in(Some(&d), &square(p, EXTRUDE)).unwrap();
        let reds = |art: &Node| {
            let im = images(art).into_iter().next().unwrap();
            let img = image::load_from_memory(&im.pixels.unwrap().bytes).unwrap().to_rgba8();
            img.pixels().filter(|px| px.0 == [255, 0, 0, 255]).count()
        };
        assert_eq!([reds(&side) > 20, reds(&other) > 20].iter().filter(|r| **r).count(), 1, "seen on the right wall from one side only");
    }

    #[test]
    fn graphics_on_revolve_are_painted_into_its_traced_image() {
        let mut b = BezPath::new();
        b.move_to((100.0, 100.0));
        b.line_to((160.0, 100.0));
        b.line_to((160.0, 200.0));
        b.line_to((100.0, 200.0));
        b.close_path();
        let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
        let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(gray, Paint::None, 0.0));
        let p = json!({"shade": false, "graphics": [{"symbol": "Mark", "scale": 300.0}], "render": {"rayTracing": true, "quality": "low"}});
        n.appearance.effects.push(crate::new_effect(crate::REVOLVE, &p).unwrap());
        let reds = |art: &Node| {
            let im = images(art).into_iter().next().unwrap();
            let img = image::load_from_memory(&im.pixels.unwrap().bytes).unwrap().to_rgba8();
            img.pixels().filter(|px| px.0[0] > 200 && px.0[1] < 40).count()
        };
        let d = marked();
        assert!(reds(&crate::revolve_art_in(Some(&d), &n).unwrap()) > 100, "the label on the side");
        assert_eq!(reds(&crate::revolve_art_in(None, &n).unwrap()), 0);
    }
}
