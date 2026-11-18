//! Character panel menu › Ruby… (or `ui.ruby`), with Show East Asian Options: the ruby (furigana)
//! over the characters the Type tool has selected, or over all the text of the selected type. The
//! dialog opens on the ruby already over that range when one reading covers it exactly, else on
//! the defaults. OK runs `type.ruby` with the reading and its options as one undo step; OK with
//! an empty reading takes the ruby off the range.
//!
//! Fields: `text` (the reading), `mono` (mono ruby: one part per base character, separated by
//! spaces), `position` ("above" | "below"), `align` ("start" | "center" | "end" | "oneTwoOne" |
//! "equalSpace"), `size` (percent of the base text's size, 1..100) and `overhang`, as `type.ruby`
//! takes them.
//!
//! Behaviour references (public specifications): W3C JLREQ 3.3, ruby
//! <https://www.w3.org/TR/jlreq/#ruby_and_emphasis_dots>, and 3.3.7, the 1-2-1 rule
//! <https://www.w3.org/TR/jlreq/#positioning_of_jukugoruby>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{RubyAlign, RubyKind, RubyPosition};
use vectorcraft_engine::doc::{NodeId, NodeKind};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Ruby.
pub const KIND: &str = "ruby";

/// The fields holding the type object and the byte range the ruby goes over.
const ID: &str = "__id";
const START: &str = "__start";
const END: &str = "__end";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Ruby").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// The type object and byte range Ruby acts on: the Type tool's selection while it edits text
/// (all of it with only a caret), else the first selected type object's whole text.
fn target(app: &VectorcraftApp) -> Option<(NodeId, usize, usize)> {
    let doc = app.session.active()?;
    let whole = |id: NodeId| match &doc.doc.node(id)?.kind {
        NodeKind::Text(t) => Some(vectorcraft_text::edit::runs_len(&t.runs)),
        _ => None,
    };
    if let Some((id, a, b)) = crate::panels::character::text_editing(app) {
        return if b > a { Some((id, a, b)) } else { whole(id).map(|len| (id, 0, len)) };
    }
    doc.selection.objects.iter().find_map(|&id| whole(id).map(|len| (id, 0, len)))
}

/// Open Ruby on the characters the Character panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let (id, start, end) = target(app).ok_or("select some type first")?;
    let span = app.session.active().and_then(|d| match &d.doc.node(id)?.kind {
        NodeKind::Text(t) => t.ruby.iter().find(|r| r.start == start && r.end == end).cloned(),
        _ => None,
    });
    let span = span.unwrap_or_else(|| vectorcraft_doc::RubySpan::new(start, end, ""));
    let fields = json!({
        "text": span.text,
        "mono": span.kind == RubyKind::Mono,
        "position": name(span.position),
        "align": name(span.align),
        "size": span.safe_size_percent(),
        "overhang": span.overhang,
        ID: id.0,
        START: start,
        END: end,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Ruby:"));
        form::text(ui, d, "text", 200.0);
        ui.end_row();
        label(ui, tl!("Size (%):"));
        form::text(ui, d, "size", 60.0);
        ui.end_row();
    });
    form::check(ui, d, "mono", tl!("Mono Ruby (one reading per character, separated by spaces)"));
    form::choice(
        ui,
        d,
        "position",
        tl!("Position:"),
        (90.0, 160.0),
        &[(name(RubyPosition::Above), tl!("Above / Right")), (name(RubyPosition::Below), tl!("Below / Left"))],
    );
    form::choice(
        ui,
        d,
        "align",
        tl!("Alignment:"),
        (90.0, 160.0),
        &[
            (name(RubyAlign::Start), tl!("Start")),
            (name(RubyAlign::Center), tl!("Center")),
            (name(RubyAlign::End), tl!("End")),
            (name(RubyAlign::OneTwoOne), tl!("1-2-1 (JIS) Rule")),
            (name(RubyAlign::EqualSpace), tl!("Equal Space")),
        ],
    );
    form::check(ui, d, "overhang", tl!("Overhang"));
    false
}

/// The name `type.ruby` and the file give an option by.
fn name<T: serde::Serialize>(v: T) -> &'static str {
    // The options' serde names, as `type.ruby` reads them back.
    match serde_json::to_value(v).ok().as_ref().and_then(Value::as_str) {
        Some("above") => "above",
        Some("below") => "below",
        Some("start") => "start",
        Some("end") => "end",
        Some("oneTwoOne") => "oneTwoOne",
        Some("equalSpace") => "equalSpace",
        _ => "center",
    }
}

/// `type.ruby` parameters from the dialog: the reading and its options over the range it opened
/// on, or `remove` when the reading is empty. A size that is not a number from 1 to 100 is an
/// error.
fn params(d: &Dialog) -> Result<Map<String, Value>, String> {
    let get = |k: &str| d.fields.get(k).and_then(Value::as_u64);
    let (Some(id), Some(start), Some(end)) = (get(ID), get(START), get(END)) else {
        return Err("no type to set ruby on".into());
    };
    let mut p = Map::new();
    p.insert("ids".into(), json!([id]));
    p.insert("start".into(), json!(start));
    p.insert("end".into(), json!(end));
    let reading = d.str("text");
    if reading.trim().is_empty() {
        p.insert("remove".into(), json!(true));
        return Ok(p);
    }
    let size = match d.fields.get("size") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|s| s.is_finite() && (1.0..=100.0).contains(s))
    .ok_or("Size must be a percentage from 1 to 100")?;
    p.insert("text".into(), json!(reading));
    p.insert("mono".into(), json!(d.bool("mono")));
    p.insert("position".into(), json!(d.str("position")));
    p.insert("align".into(), json!(d.str("align")));
    p.insert("size".into(), json!(size));
    p.insert("overhang".into(), json!(d.bool("overhang")));
    Ok(p)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d)?;
    app.ui.dialog = None;
    app.run("type.ruby", Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A selected point type object holding 漢字です.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "漢字です", "size": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn ruby(app: &VectorcraftApp, id: u64) -> Vec<vectorcraft_doc::RubySpan> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.ruby.clone(),
            _ => panic!("not text"),
        }
    }

    /// `ui.ruby` opens on the selected type's whole text with the defaults; OK sets mono ruby with
    /// the 1-2-1 alignment in one undo step; it opens again on that ruby; OK with an empty reading
    /// takes it off; a size out of range is an error that keeps the dialog open.
    #[test]
    fn ruby_dialog_sets_reopens_and_removes_ruby() {
        let (mut app, id) = app();
        app.run("ui.ruby", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Ruby opened");
        assert_eq!(d.kind, KIND);
        assert_eq!(
            (d.str("text"), d.str("align"), d.str("position"), d.bool("overhang"), d.bool("mono")),
            (String::new(), "center".into(), "above".into(), true, false)
        );
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("text".into(), json!("かん じ"));
        d.fields.insert("mono".into(), json!(true));
        d.fields.insert("align".into(), json!("oneTwoOne"));
        d.fields.insert("size".into(), json!("60"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let r = ruby(&app, id);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].start, r[0].end, r[0].text.as_str()), (0, 12, "かん じ"));
        assert_eq!((r[0].kind, r[0].align, r[0].size_percent), (RubyKind::Mono, RubyAlign::OneTwoOne, 60.0));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // It opens on the ruby set.
        app.run("ui.ruby", json!({})).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.str("text"), d.bool("mono"), d.str("align"), d.f64("size", 0.0)), ("かん じ".into(), true, "oneTwoOne".into(), 60.0));
        // A size out of range keeps the dialog open and changes nothing.
        app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!("0"));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        assert_eq!(ruby(&app, id).len(), 1);
        // An empty reading takes the ruby off.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("text".into(), json!(" "));
        super::super::confirm(&mut app).unwrap();
        assert!(ruby(&app, id).is_empty());
    }

    /// With the Type tool editing, Ruby acts on the selected characters only.
    #[test]
    fn ruby_dialog_acts_on_the_type_tools_selection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "漢字です", "size": 20})).unwrap()["id"].as_u64().unwrap();
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 3, "end": 6}));
        assert_eq!(crate::panels::character::text_editing(&app).map(|(n, a, b)| (n.0, a, b)), Some((id, 3, 6)));
        app.run("ui.ruby", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("text".into(), json!("じ"));
        super::super::confirm(&mut app).unwrap();
        let r = ruby(&app, id);
        assert_eq!((r.len(), r[0].start, r[0].end), (1, 3, 6));
    }
}
