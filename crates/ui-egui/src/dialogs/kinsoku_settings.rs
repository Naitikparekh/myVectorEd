//! Paragraph panel menu › Kinsoku Shori Settings… (or `ui.kinsokuSettings`), with Show East Asian
//! Options: a custom kinsoku set for the paragraphs the Paragraph panel acts on: its name and the
//! characters that may not start a line, may not end one, may hang (Burasagari) and may not be
//! separated. The dialog opens on the custom set those paragraphs share, or else on the lists of
//! their predefined set (Hard when they differ or have none), named "Custom". OK runs
//! `text.setFormat {kinsoku: {…}}` on the paragraphs as one undo step.
//!
//! Fields: `name`, `noStart`, `noEnd`, `hanging`, `inseparable`, as `text.setFormat` takes them.
//!
//! Behaviour reference (public specification): W3C JLREQ's line-adjustment classes,
//! <https://www.w3.org/TR/jlreq/#line_breaking_rules> (cl-08 for inseparable characters).

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Kinsoku, KinsokuSet};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Kinsoku Shori Settings.
pub const KIND: &str = "kinsokuSettings";

/// The list fields, as `text.setFormat` names them.
const LISTS: [&str; 4] = ["noStart", "noEnd", "hanging", "inseparable"];

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Kinsoku Shori Settings").into(), body, confirm, min_width: 420.0, ..DialogSpec::FORM };

/// Open Kinsoku Shori Settings on the paragraphs the Paragraph panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let shared = crate::panels::paragraph::shared_para(app, |p| p.kinsoku_set.clone()).flatten();
    let set = shared.unwrap_or_else(|| {
        let base = crate::panels::paragraph::shared_para(app, |p| p.kinsoku).filter(|k| *k != Kinsoku::None).unwrap_or(Kinsoku::Hard);
        let [no_start, no_end, hanging, inseparable] = vectorcraft_text::kinsoku_set_lists(base);
        KinsokuSet { name: tl!("Custom").into(), no_start, no_end, hanging, inseparable }
    });
    let fields = json!({
        "name": set.name,
        "noStart": set.no_start,
        "noEnd": set.no_end,
        "hanging": set.hanging,
        "inseparable": set.inseparable,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        form::text(ui, d, "name", 200.0);
        ui.end_row();
    });
    for (key, text) in
        LISTS.into_iter().zip([tl!("Cannot Start a Line:"), tl!("Cannot End a Line:"), tl!("Hanging (Burasagari):"), tl!("Cannot Be Separated:")])
    {
        ui.label(text);
        form::text_area(ui, d, key, 400.0, 2);
    }
    false
}

/// `text.setFormat` parameters: the set as the fields give it, its lists without line breaks. An
/// empty name is an error.
fn params(d: &Dialog) -> Result<Map<String, Value>, String> {
    let name = d.str("name");
    if name.trim().is_empty() {
        return Err("The set needs a name".into());
    }
    let mut set = Map::new();
    set.insert("name".into(), json!(name.trim()));
    for key in LISTS {
        set.insert(key.into(), json!(d.str(key).chars().filter(|c| !c.is_whitespace()).collect::<String>()));
    }
    let mut p = Map::new();
    p.insert("kinsoku".into(), Value::Object(set));
    Ok(p)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d)?;
    app.ui.dialog = None;
    crate::panels::paragraph::run_on_paragraphs(app, "text.setFormat", Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "あいう。え"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn set_of(app: &VectorcraftApp, id: u64) -> Option<KinsokuSet> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.para_at(0).kinsoku_set.clone(),
            _ => panic!("not text"),
        }
    }

    /// `ui.kinsokuSettings` opens on the paragraphs' predefined Hard lists, named Custom; OK gives
    /// them the edited set in one undo step (line breaks and spaces in a list left out); it opens
    /// again on that set; an empty name is an error that keeps the dialog open.
    #[test]
    fn kinsoku_settings_dialog_sets_a_custom_set() {
        let (mut app, id) = app();
        app.run("ui.kinsokuSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Kinsoku Shori Settings opened");
        assert_eq!(d.kind, KIND);
        assert_eq!(d.str("name"), "Custom");
        assert!(d.str("noStart").contains('」') && d.str("noEnd").contains('「'), "Hard's lists");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("name".into(), json!("Mine"));
        d.fields.insert("noStart".into(), json!("」\n）"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let set = set_of(&app, id).expect("custom set");
        assert_eq!((set.name.as_str(), set.no_start.as_str()), ("Mine", "」）"));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        app.run("ui.kinsokuSettings", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Mine");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("  "));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }
}
