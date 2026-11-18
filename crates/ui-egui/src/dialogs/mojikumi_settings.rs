//! Paragraph panel menu › Mojikumi Settings… (or `ui.mojikumiSettings`), with Show East Asian
//! Options: a custom Mojikumi set for the paragraphs the Paragraph panel acts on: its name, how
//! punctuation along the line is spaced (full width, sharing one half-em space between
//! consecutive marks, or half width), the line-start and line-end rules, and the space between
//! Japanese and Latin text. The dialog opens on the custom set those paragraphs share, else on the
//! switches of Line-end Punctuation Half Width, named "Custom". OK runs
//! `text.setFormat {mojikumi: {…}}` on the paragraphs as one undo step.
//!
//! Fields: `name`, `punctuation` ("full" | "shared" | "half"), `lineStartFlush`, `lineEndHalf`,
//! `lineEndDot` and `latinSpace` (% of an em, 0..50), as `text.setFormat` takes them.
//!
//! Behaviour reference (public specification): W3C JLREQ 3.1.4 and 3.2.2,
//! <https://www.w3.org/TR/jlreq/#positioning_of_punctuation_marks>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{MojikumiPunctuation, MojikumiSet};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Mojikumi Settings.
pub const KIND: &str = "mojikumiSettings";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Mojikumi Settings").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// Open Mojikumi Settings on the paragraphs the Paragraph panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let shared = crate::panels::paragraph::shared_para(app, |p| p.mojikumi_set.clone()).flatten();
    let set = shared.unwrap_or_else(|| MojikumiSet {
        name: tl!("Custom").into(),
        punctuation: MojikumiPunctuation::Shared,
        line_start_flush: true,
        line_end_half: true,
        line_end_dot: false,
        latin_space: 0.25,
    });
    let punctuation = match set.punctuation {
        MojikumiPunctuation::Full => "full",
        MojikumiPunctuation::Shared => "shared",
        MojikumiPunctuation::Half => "half",
    };
    let fields = json!({
        "name": set.name,
        "punctuation": punctuation,
        "lineStartFlush": set.line_start_flush,
        "lineEndHalf": set.line_end_half,
        "lineEndDot": set.line_end_dot,
        "latinSpace": (set.latin_space * 100.0 * 100.0).round() / 100.0,
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
    form::choice(
        ui,
        d,
        "punctuation",
        tl!("Punctuation:"),
        (130.0, 200.0),
        &[("full", tl!("Full Width")), ("shared", tl!("Consecutive Marks Share Half an Em")), ("half", tl!("Half Width"))],
    );
    form::check(ui, d, "lineStartFlush", tl!("Opening Bracket Flush at Line Start"));
    form::check(ui, d, "lineEndHalf", tl!("Closing Mark Half Width at Line End"));
    form::check(ui, d, "lineEndDot", tl!("Middle Dot Trimmed at Line End"));
    grid(ui, |ui| {
        label(ui, tl!("Japanese–Latin Space (% of an em):"));
        form::text(ui, d, "latinSpace", 60.0);
        ui.end_row();
    });
    false
}

/// `text.setFormat` parameters: the set as the fields give it. An empty name, or a space that is
/// not a number from 0 to 50, is an error.
fn params(d: &Dialog) -> Result<Map<String, Value>, String> {
    let name = d.str("name");
    if name.trim().is_empty() {
        return Err("The set needs a name".into());
    }
    let space = match d.fields.get("latinSpace") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
        None | Some(Value::Null) => Some(0.0),
        _ => None,
    }
    .filter(|s| s.is_finite() && (0.0..=50.0).contains(s))
    .ok_or("The Japanese–Latin space must be from 0 to 50% of an em")?;
    let set = json!({
        "name": name.trim(),
        "punctuation": d.str("punctuation"),
        "lineStartFlush": d.bool("lineStartFlush"),
        "lineEndHalf": d.bool("lineEndHalf"),
        "lineEndDot": d.bool("lineEndDot"),
        "latinSpace": space,
    });
    let mut p = Map::new();
    p.insert("mojikumi".into(), set);
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
    use vectorcraft_engine::doc::{Mojikumi, NodeId, NodeKind};

    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "雅A「一」"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn para(app: &VectorcraftApp, id: u64) -> (Mojikumi, Option<MojikumiSet>) {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (t.para_at(0).mojikumi, t.para_at(0).mojikumi_set.clone()),
            _ => panic!("not text"),
        }
    }

    /// `ui.mojikumiSettings` opens on Line-end Punctuation Half Width's switches named Custom; OK
    /// gives the paragraphs the edited set in one undo step; it opens again on that set; a space
    /// out of range is an error that keeps the dialog open.
    #[test]
    fn mojikumi_settings_dialog_sets_a_custom_set() {
        let (mut app, id) = app();
        app.run("ui.mojikumiSettings", json!({})).unwrap();
        let d = app.ui.dialog.clone().expect("Mojikumi Settings opened");
        assert_eq!(d.kind, KIND);
        assert_eq!(
            (d.str("name"), d.str("punctuation"), d.bool("lineEndHalf"), d.f64("latinSpace", 0.0)),
            ("Custom".into(), "shared".into(), true, 25.0)
        );
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("name".into(), json!("Wide"));
        d.fields.insert("punctuation".into(), json!("half"));
        d.fields.insert("latinSpace".into(), json!("50"));
        let undo = app.session.active().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let (m, set) = para(&app, id);
        let set = set.expect("custom set");
        assert_eq!((m, set.name.as_str(), set.punctuation, set.latin_space), (Mojikumi::Custom, "Wide", MojikumiPunctuation::Half, 0.5));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        app.run("ui.mojikumiSettings", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("latinSpace", 0.0), 50.0);
        app.ui.dialog.as_mut().unwrap().fields.insert("latinSpace".into(), json!("60"));
        assert!(super::super::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }
}
