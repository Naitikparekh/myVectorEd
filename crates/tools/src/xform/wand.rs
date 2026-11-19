//! Magic Wand (Y) and Lasso (Q).
//!
//! Magic Wand: click an object to select every object with similar attributes. The matching and
//! its settings (Magic Wand panel) live in the engine's `select.magicWand` command; Shift adds,
//! Alt subtracts. Lasso: freehand loop selecting anchor points inside it (direct-selection
//! style), and picking the gradient mesh points inside it with their meshes selected (the
//! tool's `meshPoints` option, which a solid fill colour recolours); Shift adds, Alt subtracts.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{AnchorRef, NodeId, NodeKind};
use vectorcraft_geom::{Line, PathData, PathSeg, Point};

use super::paint_owner;
use crate::meshedit::{MeshEdit, grid_of};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Default)]
pub struct MagicWandTool;

impl Tool for MagicWandTool {
    fn id(&self) -> &'static str {
        "magicWand"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        let Some(h) = hit_test(cx.doc, ev.pos, cx.hit_options()) else {
            return if ev.mods.shift || ev.mods.alt { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
        };
        let mode = if ev.mods.alt {
            "subtract"
        } else if ev.mods.shift {
            "add"
        } else {
            "set"
        };
        vec![Action::Exec("select.magicWand".into(), json!({ "id": paint_owner(cx.doc, h.leaf).0, "mode": mode }))]
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::MagicWand
    }
}

/// Even-odd point-in-polygon.
pub fn point_in_polygon(poly: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[derive(Default)]
pub struct LassoTool {
    points: Vec<Point>,
    active: bool,
    /// The picked gradient mesh points.
    mesh: MeshEdit,
}

fn anchors_json(v: &BTreeSet<AnchorRef>) -> Value {
    Value::Array(v.iter().map(|(s, a)| json!([s, a])).collect())
}

/// The segments of `path` that the edges of the closed loop `poly` cross with neither of their
/// anchors inside the loop, as (subpath, segment) pairs: the Lasso selects a path segment it is
/// dragged around part of ("With the Lasso tool, drag around part of the path segment",
/// <http://web.archive.org/web/20211230015528/https://helpx.adobe.com/illustrator/using/selecting-objects.html>).
/// A segment whose ends are both outside the loop and that enters it must cross one of its edges.
fn lasso_crossed_segments(path: &PathData, poly: &[Point]) -> Vec<AnchorRef> {
    let Some(&last) = poly.last() else { return vec![] };
    let edges: Vec<Line> = std::iter::once(last).chain(poly.iter().copied()).zip(poly.iter().copied()).map(|(a, b)| Line::new(a, b)).collect();
    let mut out = vec![];
    for (si, sp) in path.subpaths.iter().enumerate() {
        let n = sp.anchors.len();
        let count = if sp.closed { n } else { n.saturating_sub(1) };
        for seg in 0..count {
            let (Some(a), Some(b)) = (sp.anchors.get(seg), sp.anchors.get((seg + 1) % n)) else { continue };
            if point_in_polygon(poly, a.p) || point_in_polygon(poly, b.p) {
                continue;
            }
            // A segment with no handles is drawn as a line, as `SubPath::to_bezpath_into` draws it.
            let piece = if !a.has_out() && !b.has_in() {
                PathSeg::Line(Line::new(a.p, b.p))
            } else {
                PathSeg::Cubic(vectorcraft_geom::CubicBez::new(a.p, a.h_out, b.h_in, b.p))
            };
            if edges.iter().any(|e| !piece.intersect_line(*e).is_empty()) {
                out.push((si, seg));
            }
        }
    }
    out
}

/// The selection a closed loop `poly` (even-odd) makes, for the Lasso and the Polygonal Lasso:
/// the anchor points inside it on every editable path a click can pick, the segments it crosses
/// with neither anchor inside it, and the gradient mesh points inside it, which `mesh` keeps
/// picked, with their meshes. `add` (Shift) adds to the selection and `sub` (Alt) subtracts from
/// it (a crossed segment is deselected when it is selected).
pub(crate) fn lasso_select(cx: &ToolContext, mesh: &mut MeshEdit, poly: &[Point], add: bool, sub: bool) -> Vec<Action> {
    // Anchors inside the loop, and segments it crosses, for every editable path the Lasso may pick.
    let mut hits: Vec<(NodeId, BTreeSet<AnchorRef>)> = vec![];
    let mut crossed: Vec<(NodeId, Vec<AnchorRef>)> = vec![];
    let mut inside: Vec<(NodeId, usize)> = vec![];
    crate::draw2::walk_pickable(cx, |n| {
        if let Some(grid) = grid_of(n) {
            inside.extend(grid.points.iter().enumerate().filter(|(_, q)| point_in_polygon(poly, q.p)).map(|(i, _)| (n.id, i)));
        }
        if let NodeKind::Path { path, .. } = &n.kind {
            let v: BTreeSet<AnchorRef> = path.anchors().filter(|(_, _, a)| point_in_polygon(poly, a.p)).map(|(s, i, _)| (s, i)).collect();
            if !v.is_empty() {
                hits.push((n.id, v));
            }
            let segs = lasso_crossed_segments(path, poly);
            if !segs.is_empty() {
                crossed.push((n.id, segs));
            }
        }
    });
    let editable = cx.doc.editable();
    hits.retain(|(id, _)| editable(*id) && cx.doc.is_visible(*id));
    crossed.retain(|(id, _)| editable(*id) && cx.doc.is_visible(*id));
    if sub {
        // Alt deselects only the crossed segments that are selected now.
        let selected = |id: &NodeId| cx.selection.segments.get(id).cloned().unwrap_or_default();
        for (id, segs) in &mut crossed {
            let now = selected(id);
            segs.retain(|s| now.contains(s));
        }
        crossed.retain(|(_, segs)| !segs.is_empty());
    }
    // The mesh points inside the loop are picked, and their meshes selected whole;
    // Alt deselects a mesh only when it loses its last picked point.
    inside.retain(|(id, _)| editable(*id) && cx.doc.is_visible(*id));
    for (id, any) in mesh.lasso(&inside, add, sub) {
        if any != sub {
            hits.push((id, BTreeSet::new()));
        }
    }
    let items: Vec<Value> = hits.iter().map(|(id, v)| json!({ "id": id.0, "anchors": anchors_json(v) })).collect();
    let mode = if sub {
        "subtract"
    } else if add {
        "add"
    } else {
        "set"
    };
    let art = Action::Exec("select.anchorsMany".into(), json!({ "items": items, "mode": mode }));
    let seg_mode = if sub { "toggle" } else { "add" };
    let segments = crossed.iter().map(|(id, segs)| {
        let refs: Vec<Value> = segs.iter().map(|(s, i)| json!([s, i])).collect();
        Action::Exec("select.segments".into(), json!({ "id": id.0, "segments": refs, "mode": seg_mode }))
    });
    std::iter::once(art).chain(segments).collect()
}

impl Tool for LassoTool {
    fn id(&self) -> &'static str {
        "lasso"
    }

    fn busy(&self) -> bool {
        self.active
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                self.points = vec![p];
                self.active = true;
                vec![]
            }
            PointerKind::Drag if self.active => {
                if self.points.last().is_none_or(|q| q.distance(p) >= cx.tol(1.0)) {
                    self.points.push(p);
                }
                vec![]
            }
            PointerKind::Up if self.active => {
                self.active = false;
                let poly = std::mem::take(&mut self.points);
                let (add, sub) = (ev.mods.shift, ev.mods.alt);
                if poly.len() < 3 {
                    return if add || sub { vec![] } else { vec![Action::Exec("select.none".into(), json!({}))] };
                }
                lasso_select(cx, &mut self.mesh, &poly, add, sub)
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.active = false;
            self.points.clear();
        }
        vec![]
    }

    fn options(&self) -> Value {
        self.mesh.options()
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = self.mesh.overlays(cx, &|id| cx.selection.contains(id));
        if self.active && self.points.len() >= 2 {
            out.push(Overlay::Path { path: super::polygon(&self.points, false), color: [0x40, 0x40, 0x40], width: 1.0, dashed: true });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Lasso
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn magic_wand_emits_the_engine_command() {
        let (d, a) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = MagicWandTool;
        let r = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(r, vec![Action::Exec("select.magicWand".into(), json!({"id": a.0, "mode": "set"}))]);
        let shift = Mods { shift: true, ..Default::default() };
        let r = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(r, vec![Action::Exec("select.magicWand".into(), json!({"id": a.0, "mode": "add"}))]);
        let miss = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 900.0, 900.0));
        assert_eq!(miss, vec![Action::Exec("select.none".into(), json!({}))]);
    }

    /// The Lasso selects anchor points only on the art a click can pick: not on a Shaper Group's
    /// original shapes unless they are isolated (Construction mode), and in isolation mode only on
    /// the isolated object.
    #[test]
    fn lasso_leaves_out_shaper_sources_and_art_outside_isolation() {
        let (d, _, sources, face) = doc_with_shaper_group();
        let rect = d.layers[0].children().unwrap()[0].id;
        let b = d.node(sources).and_then(|n| n.children()).unwrap()[1].id;
        let (s, p) = (Selection::default(), paint());
        let lasso = |isolation: Option<NodeId>, poly: [(f64, f64); 3]| {
            let cx = ToolContext { isolation, ..cx(&d, &s, &p) };
            let mut t = LassoTool::default();
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, poly[0].0, poly[0].1));
            for (x, y) in &poly[1..] {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, *x, *y));
            }
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, poly[2].0, poly[2].1));
            let Some(Action::Exec(_, v)) = a.first() else { panic!("a selection command: {a:?}") };
            v["items"].as_array().unwrap().iter().map(|it| NodeId(it["id"].as_u64().unwrap())).collect::<Vec<_>>()
        };
        // Around the top right corner of the second square, which only its original shape has.
        let corner = [(130.0, 280.0), (180.0, 280.0), (150.0, 320.0)];
        assert_eq!(lasso(None, corner), vec![]);
        assert_eq!(lasso(Some(sources), corner), vec![b]);
        // Around the face's top left corner, where the first original shape has an anchor too.
        let shared = [(-20.0, 280.0), (30.0, 280.0), (0.0, 320.0)];
        assert_eq!(lasso(None, shared), vec![face]);
        assert_eq!(lasso(Some(rect), shared), vec![]);
    }

    #[test]
    fn lasso_selects_anchors_inside() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LassoTool::default();
        // A triangle around the top-left corner (100,100) only.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 80.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 140.0));
        assert_eq!(t.overlays(&cx).len(), 1);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 80.0, 140.0));
        let Action::Exec(c, v) = &a[0] else { panic!() };
        assert_eq!(c, "select.anchorsMany");
        assert_eq!(v["items"][0]["id"], id.0);
        assert_eq!(v["items"][0]["anchors"].as_array().unwrap().len(), 1);
        assert_eq!(v["mode"], "set");
        assert!(point_in_polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], Point::new(2.0, 2.0)));
    }

    /// A loop dragged around part of a segment, with neither of its anchors inside, selects that
    /// segment; with Alt it deselects it only when it is selected, and leaves others alone.
    #[test]
    fn lasso_selects_a_segment_it_is_dragged_around_part_of() {
        let (d, id) = doc_with_rect();
        let p = paint();
        let run = |s: &Selection, mods: Mods| {
            let cx = cx(&d, s, &p);
            let mut t = LassoTool::default();
            // A triangle across the middle of the top edge (y = 100), no corner inside.
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 140.0, 80.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 160.0, 80.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 130.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 130.0).with_mods(mods))
        };
        let a = run(&Selection::default(), Mods::default());
        assert_eq!(a.len(), 2, "{a:?}");
        let Action::Exec(c, v) = &a[0] else { panic!() };
        assert_eq!((c.as_str(), v["items"].as_array().unwrap().len(), v["mode"].as_str()), ("select.anchorsMany", 0, Some("set")));
        let Action::Exec(c, v) = &a[1] else { panic!() };
        assert_eq!((c.as_str(), v["id"].as_u64(), v["mode"].as_str()), ("select.segments", Some(id.0), Some("add")));
        let segs = v["segments"].as_array().unwrap();
        assert_eq!(segs.len(), 1);
        let (si, seg) = (segs[0][0].as_u64().unwrap() as usize, segs[0][1].as_u64().unwrap() as usize);
        let path = d.node(id).and_then(|n| n.path_data()).unwrap();
        let sp = &path.subpaths[si];
        let ends = [sp.anchors[seg].p, sp.anchors[(seg + 1) % sp.anchors.len()].p];
        assert!(ends.iter().all(|q| q.y == 100.0), "the top edge: {ends:?}");
        // Alt with the segment not selected: nothing to deselect.
        let alt = Mods { alt: true, ..Default::default() };
        let a = run(&Selection::default(), alt);
        assert_eq!(a.len(), 1, "{a:?}");
        // Alt with it selected: it is toggled off.
        let mut sel = Selection::default();
        sel.add(id);
        sel.segments.insert(id, [(si, seg)].into_iter().collect());
        let a = run(&sel, alt);
        let Some(Action::Exec(c, v)) = a.get(1) else { panic!("{a:?}") };
        assert_eq!((c.as_str(), v["mode"].as_str()), ("select.segments", Some("toggle")));
    }

    /// A Lasso loop round a corner of a path warped by a mesh envelope still selects the path's
    /// anchor there, besides picking the envelope's grid point (#5351).
    #[test]
    fn lasso_keeps_picking_the_anchors_of_an_envelopes_path() {
        use std::sync::Arc;
        use vectorcraft_doc::Node;
        use vectorcraft_doc::live::{EnvelopeKind, EnvelopeOptions, grid_points};
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, a) = doc_with_rect();
        let rect = d.node(a).cloned().unwrap();
        let kind = EnvelopeKind::Mesh { rows: 1, cols: 1, points: grid_points(Rect::new(100.0, 100.0, 200.0, 200.0), 1, 1), handles: vec![] };
        let env = NodeKind::Envelope {
            content: vec![Arc::new(rect)],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: EnvelopeOptions::NEW,
            frame: Box::new(Affine::IDENTITY),
        };
        d.remove(a).unwrap();
        let e = d.alloc_id();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, Node::new(e, env)).unwrap();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        let mut t = LassoTool::default();
        // A triangle around the top-left corner (100,100) only.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 80.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 80.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 140.0));
        let r = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 80.0, 140.0));
        let Some(Action::Exec(c, v)) = r.first() else { panic!("a selection command: {r:?}") };
        assert_eq!(c, "select.anchorsMany");
        let items = v["items"].as_array().unwrap();
        let path = items.iter().find(|it| it["id"] == a.0).expect("the warped path's anchor is selected");
        assert_eq!(path["anchors"].as_array().unwrap().len(), 1);
        assert!(items.iter().any(|it| it["id"] == e.0), "the envelope is selected with its grid point");
        assert_eq!(t.options()["meshPoints"].as_array().unwrap().len(), 1);
    }
}
