//! Outline Object (`path.outlineObject`, Effect › Path › Outline Object): type that carries it is
//! drawn, exported and expanded as its glyph outlines while it stays editable type.

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// Selected point type "OUTLINE" (48 pt) at (40, 100).
fn text(s: &mut Session) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "OUTLINE", "size": 48})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn apply(s: &mut Session, id: NodeId) {
    s.execute("effect.apply", &json!({"effect": "path.outlineObject", "ids": [id.0], "item": null})).unwrap();
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn count_text(n: &Node) -> usize {
    let mut k = 0;
    n.walk(&mut |m| k += usize::from(matches!(m.kind, NodeKind::Text(_))));
    k
}

#[test]
fn outline_object_is_applied_in_one_undo_step_and_takes_no_parameters() {
    let mut s = session();
    let id = text(&mut s);
    apply(&mut s, id);
    let ids: Vec<String> = node(&s, id).appearance.effects.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["path.outlineObject"]);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Outline Object"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.effects.is_empty());
    let err = s.execute("effect.apply", &json!({"effect": "path.outlineObject", "ids": [id.0], "params": {"width": 1}})).unwrap_err().to_string();
    assert!(err.contains("`width`") && err.contains("takes no parameters"), "{err}");
    assert!(node(&s, id).appearance.effects.is_empty());
}

#[test]
fn type_with_outline_object_stays_editable_type_and_keeps_its_geometric_bounds() {
    let mut s = session();
    let id = text(&mut s);
    let before = s.transform_bounds(&[id]);
    apply(&mut s, id);
    assert_eq!(s.transform_bounds(&[id]), before);
    // With Use Preview Bounds on, the type is measured by its glyph outlines.
    s.prefs.use_preview_bounds = true;
    let glyphs = vectorcraft_render::effects::reshape(None, &node(&s, id), None).unwrap().geometric_bounds();
    assert_eq!(s.transform_bounds(&[id]), glyphs);
    s.prefs.use_preview_bounds = false;
    // A native save keeps the effect.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let ids: Vec<String> = back.node(id).unwrap().appearance.effects.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["path.outlineObject"]);
    // The type is still type, drawn as its outlines, and a text edit changes them.
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Text(_)));
    let drawn = vectorcraft_render::effects::reshape(None, &n, None).unwrap();
    assert_eq!(count_text(&drawn), 0);
    s.execute("text.setText", &json!({"id": id.0, "text": "OUTLINED TYPE"})).unwrap();
    let longer = vectorcraft_render::effects::reshape(None, &node(&s, id), None).unwrap();
    assert!(longer.geometric_bounds().unwrap().width() > drawn.geometric_bounds().unwrap().width() + 50.0);
}

#[test]
fn type_with_outline_object_exports_and_expands_as_paths() {
    let mut s = session();
    let id = text(&mut s);
    apply(&mut s, id);
    let doc = s.doc().unwrap().doc.clone();
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert!(!svg.contains("<text") && !svg.contains("OUTLINE"), "{svg}");
    // Expand Appearance leaves paths and no type, in one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let doc = &s.doc().unwrap().doc;
    let layer = &doc.layers[0];
    assert_eq!(count_text(layer), 0);
    let mut paths = 0;
    layer.walk(&mut |m| paths += usize::from(matches!(m.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert!(paths > 0);
}

/// With Use Preview Bounds on, Align and Distribute measure type that carries Outline Object by its
/// glyph outlines, as the Transform panel does: aligned left to the artboard, the outlines' left
/// edge lands on it rather than the type's layout box; distributed, the outlines' left edges are
/// spaced evenly.
#[test]
fn align_and_distribute_measure_outline_object_type_by_its_glyph_outlines_with_preview_bounds() {
    let mut s = session();
    let id = text(&mut s);
    apply(&mut s, id);
    let outlines = |s: &Session, id: NodeId| vectorcraft_render::effects::reshape(None, &node(s, id), None).unwrap().geometric_bounds().unwrap();
    let layout = node(&s, id).geometric_bounds().unwrap();
    // The outlines sit inside the layout box, away from its left edge (the "O"'s side bearing).
    assert!(outlines(&s, id).x0 > layout.x0 + 0.5, "outlines {:?} layout {layout:?}", outlines(&s, id));
    s.execute("prefs.set", &json!({"key": "usePreviewBounds", "value": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.align", &json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert!(outlines(&s, id).x0.abs() < 1e-6, "outlines start at the artboard's edge: {:?}", outlines(&s, id));

    // Distribute left edges of three: the outlines' left edges are evenly spaced.
    let b = NodeId(s.execute("text.create", &json!({"x": 150, "y": 200, "text": "OUTLINE", "size": 48})).unwrap()["id"].as_u64().unwrap());
    let c = NodeId(s.execute("text.create", &json!({"x": 300, "y": 250, "text": "OUTLINE", "size": 48})).unwrap()["id"].as_u64().unwrap());
    apply(&mut s, b);
    s.execute("select.set", &json!({"ids": [id.0, b.0, c.0]})).unwrap();
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    let c_left = vectorcraft_render::effects::preview::bounds_of(&s.doc().unwrap().doc, &[c], true).unwrap().x0;
    let lefts = [outlines(&s, id).x0, outlines(&s, b).x0, c_left];
    assert!(((lefts[1] - lefts[0]) - (lefts[2] - lefts[1])).abs() < 0.5, "{lefts:?}");
}
