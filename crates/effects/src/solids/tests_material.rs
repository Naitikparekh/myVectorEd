//! Materials on Extrude & Bevel, Inflate and Rotate ([`super::material`]).
use serde_json::json;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeId};
use vectorcraft_geom::{BezPath, PathData};

use super::*;

const RED: Color = Color::Rgb { r: 0.8, g: 0.2, b: 0.1 };

fn square(params: Value, id: &str) -> Node {
    let mut b = BezPath::new();
    b.move_to((100.0, 100.0));
    b.line_to((200.0, 100.0));
    b.line_to((200.0, 200.0));
    b.line_to((100.0, 200.0));
    b.close_path();
    let mut n = Node::path(NodeId(1), PathData::from_bezpath(&b), Appearance::basic(Paint::solid(RED), Paint::None, 0.0));
    n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
    n
}

/// Every face of `art` with its fill color, and whether it lies on the sides of the solid
/// (recognised by its color, see the tests).
fn colors(art: &Node) -> Vec<[f32; 3]> {
    fn walk(n: &Node, out: &mut Vec<[f32; 3]>) {
        match n.children() {
            Some(c) => c.iter().for_each(|c| walk(c, out)),
            None => {
                if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first()
                    && let Some(c) = f.paint.color()
                {
                    out.push(c.to_rgb());
                }
            }
        }
    }
    let mut out = vec![];
    walk(art, &mut out);
    out
}

const FRONT_LIT: [(&str, f64); 5] = [("rotationX", 0.0), ("rotationY", 0.0), ("rotationZ", 0.0), ("lightRotation", 0.0), ("lightHeight", 90.0)];

fn front_lit(mut extra: Value) -> Value {
    for (k, v) in FRONT_LIT {
        extra[k] = json!(v);
    }
    extra
}

#[test]
fn no_material_draws_as_before() {
    for id in [EXTRUDE, INFLATE, ROTATE] {
        let plain = solid_art(&square(json!({}), id));
        let null = solid_art(&square(json!({"material": null, "sideMaterial": null}), id));
        assert_eq!(plain, null, "{id}");
    }
}

#[test]
fn a_glossy_material_adds_a_highlight_facing_the_light() {
    let plain = colors(&solid_art(&square(front_lit(json!({})), ROTATE)).unwrap());
    let gloss = colors(&solid_art(&square(front_lit(json!({"material": {"preset": "gloss"}})), ROTATE)).unwrap());
    assert_eq!(plain.len(), 1);
    assert_eq!(gloss.len(), 1);
    // Without a material the plane is at most its own red; the highlight lifts every channel.
    assert!(plain[0][0] <= 0.8 + 1e-6);
    assert!(gloss[0].iter().zip(plain[0]).all(|(g, p)| *g > p + 0.1), "{gloss:?} over {plain:?}");
    // A fully matte material is the diffuse shading.
    let matte = colors(&solid_art(&square(front_lit(json!({"material": {"preset": "matte"}})), ROTATE)).unwrap());
    assert!(matte[0].iter().zip(plain[0]).all(|(m, p)| (m - p).abs() < 1e-3), "{matte:?} vs {plain:?}");
}

#[test]
fn the_base_color_replaces_the_paint_and_the_side_material_covers_the_walls() {
    let p = json!({"material": {"preset": "matte", "baseColor": "#0000ff"}, "sideMaterial": {"preset": "matte", "baseColor": "#00ff00"}});
    let art = solid_art(&square(p, EXTRUDE)).unwrap();
    let c = colors(&art);
    let blue = c.iter().filter(|c| c[2] > 0.0 && c[0] == 0.0 && c[1] == 0.0).count();
    let green = c.iter().filter(|c| c[1] > 0.0 && c[0] == 0.0 && c[2] == 0.0).count();
    assert_eq!(blue, 1, "the front cap in the base color: {c:?}");
    assert!(green >= 2, "the walls seen at the default view: {c:?}");
    assert_eq!(blue + green, c.len(), "no face keeps the red paint: {c:?}");
}

#[test]
fn inflate_under_colored_lights_is_shaded_flat_per_facet_with_a_material() {
    let p = json!({"lightColor": "#ffcc88", "material": {"preset": "satin"}});
    let art = solid_art(&square(p, INFLATE)).unwrap();
    let c = colors(&art);
    assert!(c.len() > 100, "every facet is a path with a solid fill: {}", c.len());
    assert!(c.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
}

#[test]
fn commands_reject_bad_materials() {
    let n = square(json!({}), EXTRUDE);
    assert!(validate_three_d(&n, EXTRUDE, &json!({"material": {"roughness": 2}})).is_err());
    assert!(validate_three_d(&n, EXTRUDE, &json!({"sideMaterial": {"preset": "brushedMetal"}})).is_ok());
    assert!(validate_three_d(&n, CLASSIC_EXTRUDE, &json!({"material": {"preset": "gloss"}})).is_err());
}
