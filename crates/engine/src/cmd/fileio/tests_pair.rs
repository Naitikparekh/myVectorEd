//! Paired saves: an EPS and its native master written from one snapshot (`file.savePair`), saved
//! again by `document.save`, checked by `file.pairStatus`, reopened by `file.openPair`, and the EPS
//! preflight (`file.epsPreflight`).
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-pair-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn at(d: &Path, name: &str) -> String {
    d.join(name).to_string_lossy().into_owned()
}

/// A new document with a rectangle and a line of type.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 50})).unwrap();
    s.execute("text.create", &json!({"x": 20, "y": 150, "text": "Pair", "size": 24})).unwrap();
    s
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn status(s: &mut Session, eps: &str) -> Value {
    s.execute("file.pairStatus", &json!({"path": eps})).unwrap()
}

fn master_link(path: &str) -> Value {
    let bytes = std::fs::read(path).unwrap();
    load(path, &bytes).unwrap().doc.extra.get(pair::KEY).cloned().unwrap()
}

#[test]
fn a_paired_save_writes_both_files_linked_and_the_document_takes_the_eps() {
    let d = dir("both");
    let mut s = session();
    let eps = at(&d, "poster.eps");
    let r = s.execute("file.savePair", &json!({"path": eps})).unwrap();
    let master = at(&d, "poster.vectorcraft");
    assert_eq!(r["companion"], master.as_str());
    let eps_bytes = std::fs::read(&eps).unwrap();
    assert!(eps_bytes.starts_with(b"%!PS-Adobe") || eps_bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6]), "an EPS file");
    assert_eq!(r["sha256"], sha(&eps_bytes).as_str());
    let link = master_link(&master);
    assert_eq!(link["eps"]["sha256"], sha(&eps_bytes).as_str());
    assert_eq!(link["eps"]["name"], "poster.eps");
    assert_eq!(link["revision"], 1);
    assert_eq!(link["documentId"], r["documentId"]);
    assert_eq!(status(&mut s, &eps)["status"], "linked");
    let st = s.doc().unwrap();
    assert_eq!(st.path.as_deref(), Some(eps.as_str()));
    assert_eq!(st.format, "eps");
    assert!(!st.is_dirty(), "saved");
    assert!(!Path::new(&at(&d, ".poster.eps.pairtxn")).exists(), "the transaction record is removed");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn document_save_saves_the_pair_again_as_the_next_revision_of_the_same_document() {
    let d = dir("again");
    let mut s = session();
    let eps = at(&d, "card.eps");
    let first = s.execute("file.savePair", &json!({"path": eps})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 120, "y": 20, "width": 40, "height": 40})).unwrap();
    assert!(s.doc().unwrap().is_dirty());
    let second = s.execute("document.save", &json!({})).unwrap();
    assert_eq!(second["companion"], at(&d, "card.vectorcraft").as_str(), "document.save wrote the pair");
    assert_eq!(second["revision"], 2);
    assert_eq!(second["documentId"], first["documentId"]);
    assert_ne!(second["sha256"], first["sha256"]);
    assert_eq!(status(&mut s, &eps)["status"], "linked");
    assert!(!s.doc().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_vectorcraft_file_of_another_document_is_not_overwritten_without_replace_companion() {
    let d = dir("foreign");
    let master = at(&d, "logo.vectorcraft");
    let mut other = session();
    other.execute("document.save", &json!({"path": master})).unwrap();
    let before = std::fs::read(&master).unwrap();

    let mut s = session();
    let doc_before = s.doc().unwrap().doc.clone();
    let eps = at(&d, "logo.eps");
    let e = s.execute("file.savePair", &json!({"path": eps})).unwrap_err().to_string();
    assert!(e.contains("replaceCompanion"), "{e}");
    assert!(!Path::new(&eps).exists(), "nothing was written");
    assert_eq!(std::fs::read(&master).unwrap(), before, "the other document's file is untouched");
    let st = s.doc().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc_before), "the document is exactly as it was");
    assert!(st.path.is_none());

    s.execute("file.savePair", &json!({"path": eps, "replaceCompanion": true})).unwrap();
    assert_eq!(status(&mut s, &eps)["status"], "linked");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_eps_changed_elsewhere_makes_the_master_stale_and_opening_asks_which() {
    let d = dir("stale");
    let mut s = session();
    let eps = at(&d, "flyer.eps");
    s.execute("file.savePair", &json!({"path": eps})).unwrap();
    // Another program rewrites the EPS.
    let mut bytes = std::fs::read(&eps).unwrap();
    bytes.extend_from_slice(b"\n% edited elsewhere\n");
    std::fs::write(&eps, &bytes).unwrap();
    assert_eq!(status(&mut s, &eps)["status"], "stale");

    let mut s = Session::new();
    let e = s.execute("file.openPair", &json!({"path": eps})).unwrap_err().to_string();
    assert!(e.contains("stale") && e.contains("use"), "{e}");
    let r = s.execute("file.openPair", &json!({"path": eps, "use": "eps"})).unwrap();
    assert_eq!(r["opened"], "eps");
    let r = s.execute("file.openPair", &json!({"path": eps, "use": "master"})).unwrap();
    assert_eq!(r["opened"], "master");
    let st = s.doc().unwrap();
    assert_eq!(st.path.as_deref(), Some(eps.as_str()), "the master stands in for the EPS");
    assert_eq!(st.format, "eps");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_linked_pair_opens_the_master_with_its_type_editable() {
    let d = dir("open");
    let mut s = session();
    let eps = at(&d, "menu.eps");
    s.execute("file.savePair", &json!({"path": eps})).unwrap();
    let mut s = Session::new();
    let r = s.execute("file.openPair", &json!({"path": eps})).unwrap();
    assert_eq!(r["opened"], "master");
    assert_eq!(r["pair"]["status"], "linked");
    let mut kinds = vec![];
    s.doc().unwrap().doc.walk(|n| kinds.push(matches!(n.kind, vectorcraft_doc::NodeKind::Text(_))));
    assert!(kinds.iter().any(|t| *t), "the type is live text");
    assert!(!s.doc().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_missing_master_is_reported() {
    let d = dir("missing");
    let mut s = session();
    let eps = at(&d, "badge.eps");
    s.execute("file.savePair", &json!({"path": eps})).unwrap();
    std::fs::remove_file(at(&d, "badge.vectorcraft")).unwrap();
    assert_eq!(status(&mut s, &eps)["status"], "missing");
    let e = Session::new().execute("file.openPair", &json!({"path": eps})).unwrap_err().to_string();
    assert!(e.contains("missing"), "{e}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_master_that_cannot_be_written_leaves_a_partial_record_and_the_document_modified() {
    let d = dir("partial");
    let mut s = session();
    let eps = at(&d, "sign.eps");
    let companion = at(&d, "no-such-folder/sign.vectorcraft");
    let e = s.execute("file.savePair", &json!({"path": eps, "companion": companion})).unwrap_err().to_string();
    assert!(e.contains("native master") && e.contains("stays modified"), "{e}");
    assert!(Path::new(&eps).exists(), "the EPS was written first");
    let record: Value = serde_json::from_slice(&std::fs::read(at(&d, ".sign.eps.pairtxn")).unwrap()).unwrap();
    assert_eq!(record["state"], "partial");
    let st = s.execute("file.pairStatus", &json!({"path": eps, "companion": companion})).unwrap();
    assert_eq!(st["status"], "interrupted");
    let st = s.doc().unwrap();
    assert!(st.is_dirty(), "never marked saved");
    assert!(st.path.is_none(), "the EPS did not become the document's own file");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_eps_that_cannot_be_written_writes_nothing() {
    let d = dir("noeps");
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    let eps = at(&d, "gone/shirt.eps");
    s.execute("file.savePair", &json!({"path": eps, "companion": at(&d, "shirt.vectorcraft")})).unwrap_err();
    assert!(!Path::new(&at(&d, "shirt.vectorcraft")).exists());
    assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "no record, no master");
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_uppercase_eps_name_pairs_with_a_vectorcraft_master() {
    let d = dir("upper");
    let mut s = session();
    let eps = at(&d, "BANNER.EPS");
    let r = s.execute("file.savePair", &json!({"path": eps})).unwrap();
    assert_eq!(r["companion"], at(&d, "BANNER.vectorcraft").as_str());
    assert_eq!(status(&mut s, &eps)["status"], "linked");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn other_files_and_several_eps_files_are_refused_before_anything_is_written() {
    let d = dir("refused");
    let mut s = session();
    let e = s.execute("file.savePair", &json!({"path": at(&d, "art.pdf")})).unwrap_err().to_string();
    assert!(e.contains(".eps"), "{e}");
    let e = s.execute("file.savePair", &json!({"path": at(&d, "art.eps"), "companion": at(&d, "art.eps")})).unwrap_err().to_string();
    assert!(e.contains("different"), "{e}");
    s.execute("artboard.new", &json!({"width": 100, "height": 100})).unwrap();
    let e = s.execute("file.savePair", &json!({"path": at(&d, "art.eps"), "useArtboards": true})).unwrap_err().to_string();
    assert!(e.contains("one EPS file"), "{e}");
    assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "nothing written");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_plain_eps_save_is_not_routed_to_a_pair() {
    let d = dir("plain");
    let mut s = session();
    let eps = at(&d, "plain.eps");
    let r = s.execute("file.saveAs", &json!({"path": eps})).unwrap();
    assert!(r.get("companion").is_none());
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let r = s.execute("document.save", &json!({})).unwrap();
    assert!(r.get("companion").is_none(), "an EPS never saved as a pair saves alone");
    assert!(!Path::new(&at(&d, "plain.vectorcraft")).exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn preflight_names_flattening_outlined_type_and_artboards() {
    let mut s = session();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("artboard.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.epsPreflight", &json!({})).unwrap();
    let kinds: Vec<&str> = r["issues"].as_array().unwrap().iter().filter_map(|i| i["kind"].as_str()).collect();
    assert!(kinds.contains(&"transparency"), "{r}");
    assert!(kinds.contains(&"outlinedText"), "{r}");
    assert!(kinds.contains(&"artboards"), "{r}");
    assert_eq!(r["ok"], false);
    assert_eq!(r["artboards"], 2);
}
