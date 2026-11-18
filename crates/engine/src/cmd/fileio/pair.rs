//! EPS-first paired saving: one save writes an EPS deliverable and a native `.vectorcraft` master
//! beside it, from the same snapshot of the document (`file.savePair`), and reopening uses the
//! master only while it still matches the EPS (`file.pairStatus`, `file.openPair`).
//! `file.epsPreflight` lists what the EPS will flatten, outline or rasterize before it is written.
//!
//! The link between the two files lives in the documents' `extra` map under [`KEY`]: the document
//! and the EPS carry `{documentId, revision}`; the master also records the EPS file's name, size
//! and SHA-256, so a master is trusted only for the exact EPS it was written with.
//!
//! Both files are encoded and checked in memory before either is written. A transaction record
//! (`.<eps name>.pairtxn` beside the EPS) is written first and removed last, so an interrupted save
//! leaves evidence that `file.pairStatus` reports. Each file is replaced atomically on its own;
//! replacing two files is not one atomic step, so a failure between them is reported as a partial
//! save and the document stays modified.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use vectorcraft_doc::Document;

use super::super::{CommandSpec, Result, Session, bad, bool_or, cmd, has_doc, str_param};

/// The key in [`Document::extra`] that holds a paired save's linkage.
pub const KEY: &str = "myvectoredPair";

/// What the transaction record's file name ends with.
const TXN_SUFFIX: &str = ".pairtxn";

const SAVE: &str = "file.savePair";
const STATUS: &str = "file.pairStatus";
const OPEN: &str = "file.openPair";

/// The keys `file.savePair` reads itself; every other key goes to the EPS save as its options.
const PAIR_KEYS: [&str; 3] = ["companion", "replaceCompanion", "path"];

/// SHA-256 of `bytes`, as lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        // Writing to a String can't fail.
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Is `path` named as an EPS file (`.eps` or `.epsf`, in any case)?
fn is_eps_name(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("eps") || e.eq_ignore_ascii_case("epsf"))
}

/// The master written beside `eps` unless one is named: the same name with `.vectorcraft`.
fn default_companion(eps: &str) -> String {
    std::path::Path::new(eps).with_extension("vectorcraft").to_string_lossy().into_owned()
}

/// The transaction record of a paired save to `eps`: `.<file name>.pairtxn` in its folder.
fn txn_path(eps: &str) -> String {
    let p = std::path::Path::new(eps);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!(".{name}{TXN_SUFFIX}")).to_string_lossy().into_owned()
}

/// The file name of `path` (what the master records of its EPS).
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Do `a` and `b` name the same file? The same path ignoring case (two names that differ only in
/// case are one file on the usual Windows and macOS volumes), or the same file once both resolve.
fn same_path(a: &str, b: &str) -> bool {
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// The linkage `doc` carries, if any.
fn linkage(doc: &Document) -> Option<&Map<String, Value>> {
    doc.extra.get(KEY).and_then(Value::as_object)
}

fn document_id(doc: &Document) -> Option<&str> {
    linkage(doc).and_then(|l| l.get("documentId")).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A new document identity: 32 hex digits from the time, the process, a counter and the document's
/// title. Unique in practice; it only tells documents apart, it guards nothing.
fn new_document_id(doc: &Document) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut h = Sha256::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        h.update(nanos.to_le_bytes());
        h.update(std::process::id().to_le_bytes());
    }
    h.update(n.to_le_bytes());
    h.update(doc.title.as_bytes());
    sha256_hex(&h.finalize()).chars().take(32).collect()
}

/// The EPS a paired save of the active document writes: `path`, else the document's own file when
/// it is an EPS.
fn eps_target(s: &Session, cmd: &str, p: &Value) -> Result<String> {
    let path = match str_param(p, "path").filter(|p| !p.trim().is_empty()) {
        Some(p) => p.to_string(),
        None => {
            let st = s.doc()?;
            match (&st.path, st.format) {
                (Some(path), "eps") => path.clone(),
                _ => return Err(bad(cmd, "give `path`: the EPS file to write (the document has no EPS file of its own yet)")),
            }
        }
    };
    if !is_eps_name(&path) {
        return Err(bad(cmd, format!("{path}: a paired save writes an EPS file (.eps): use document.save for other formats")));
    }
    Ok(path)
}

/// Would a paired save of the document whose linkage is `id` replace the file at `companion`? A
/// file that isn't there is free; one that is must be a master of the same document unless
/// `replace` says to overwrite it.
#[cfg(not(target_arch = "wasm32"))]
fn check_companion(companion: &str, id: Option<&str>, replace: bool) -> Result<()> {
    let path = std::path::Path::new(companion);
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        return Err(bad(SAVE, format!("{companion}: not a file, so it can't be the native master")));
    }
    if replace {
        return Ok(());
    }
    let theirs = super::read_file(companion).ok().and_then(|b| super::load(companion, &b).ok()).and_then(|l| document_id(&l.doc).map(str::to_string));
    match (theirs.as_deref(), id) {
        (Some(t), Some(mine)) if t == mine => Ok(()),
        _ => Err(bad(
            SAVE,
            format!(
                "{companion} already exists and is not this document's native master: give replaceCompanion: true to overwrite it, or another `companion` path"
            ),
        )),
    }
}

/// `file.savePair`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_pair(s: &mut Session, p: &Value) -> Result<Value> {
    let eps = eps_target(s, SAVE, p)?;
    let companion = str_param(p, "companion").filter(|c| !c.trim().is_empty()).map_or_else(|| default_companion(&eps), str::to_string);
    if same_path(&eps, &companion) {
        return Err(bad(SAVE, "the EPS and its native master must be different files"));
    }
    if !std::path::Path::new(&companion).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("vectorcraft")) {
        return Err(bad(SAVE, format!("{companion}: the native master is a .vectorcraft file")));
    }
    let previous = s.doc()?.doc.clone();
    let prior = linkage(&previous);
    let id = document_id(&previous).map_or_else(|| new_document_id(&previous), str::to_string);
    let revision = prior.and_then(|l| l.get("revision")).and_then(Value::as_u64).unwrap_or(0).saturating_add(1);
    check_companion(&companion, document_id(&previous), bool_or(p, "replaceCompanion", false))?;

    // The document (and so the EPS, which carries it) records who it is and which save this is.
    let mut tagged = (*previous).clone();
    tagged.extra.insert(KEY.into(), json!({ "documentId": id, "revision": revision }));
    s.doc_mut()?.doc = std::sync::Arc::new(tagged);
    let result = write_pair(s, p, &eps, &companion, &id, revision);
    if result.is_err() {
        // Nothing was saved as the document's own: it goes back exactly as it was (the same
        // shared document, so an unmodified document stays unmodified).
        if let Ok(st) = s.doc_mut() {
            st.doc = previous;
        }
    }
    let (job, reply) = result?;
    let uid = s.doc()?.uid;
    job.complete(s, uid);
    Ok(reply)
}

/// Encode both files, check them, then write the record, the EPS and the master. On success the
/// EPS job, still to be completed, and the reply.
#[cfg(not(target_arch = "wasm32"))]
fn write_pair(s: &mut Session, p: &Value, eps: &str, companion: &str, id: &str, revision: u64) -> Result<(super::SaveJob, Value)> {
    use super::save::{SaveMode, save_job, save_plan};

    let mut options: Map<String, Value> = p
        .as_object()
        .map(|o| o.iter().filter(|(k, _)| !PAIR_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    options.insert("path".into(), json!(eps));
    options.insert("format".into(), json!("eps"));
    let plan = save_plan(s, SaveMode::SaveAs, &Value::Object(options))?;
    let lossy = plan.lossy_source.clone();
    let job = save_job(s, plan)?;
    let enc = job.encode()?;
    let [(_, eps_bytes)] = enc.files.as_slice() else {
        return Err(bad(SAVE, "a paired save writes one EPS file: turn useArtboards off, or name a single artboard"));
    };
    if let Some(source) = &lossy {
        super::check_files_not_lossy_overwrite(source, [eps, companion], SAVE)?;
    }
    let well_formed =
        vectorcraft_eps::sections(eps_bytes).is_some_and(|(ps, _)| ps.starts_with(b"%!PS-Adobe") && ps.windows(5).any(|w| w == b"%%EOF"));
    if !well_formed {
        return Err(bad(SAVE, "the EPS failed its check before writing (no PostScript header or end): nothing was written"));
    }
    let sha = sha256_hex(eps_bytes);

    // The master: the same snapshot, with the EPS it belongs to.
    let native_plan = save_plan(s, SaveMode::Copy, &json!({ "path": companion, "format": "vectorcraft" }))?;
    let native_job = save_job(s, native_plan)?;
    let mut master = (**native_job.doc()).clone();
    master.extra.insert(
        KEY.into(),
        json!({ "documentId": id, "revision": revision, "eps": { "name": file_name(eps), "sha256": sha, "bytes": eps_bytes.len() } }),
    );
    let native = native_job.encode_doc(&master)?;
    let [(_, native_bytes)] = native.files.as_slice() else {
        return Err(bad(SAVE, "the native master could not be encoded as one file"));
    };
    let reread =
        super::load(companion, native_bytes).map_err(|e| bad(SAVE, format!("the native master could not be read back before writing: {e}")))?;
    let recorded = linkage(&reread.doc).and_then(|l| l.get("eps")).and_then(|e| e.get("sha256")).and_then(Value::as_str);
    if recorded != Some(sha.as_str()) {
        return Err(bad(SAVE, "the native master lost its link to the EPS when read back: nothing was written"));
    }

    // Write: the record, the EPS (the deliverable), the master, then remove the record.
    let txn = txn_path(eps);
    let record = |state: &str| {
        json!({ "state": state, "eps": eps, "companion": companion, "sha256": sha, "documentId": id, "revision": revision }).to_string().into_bytes()
    };
    super::write_save_file(&txn, &record("writing"))?;
    if let Err(e) = super::write_save_file(eps, eps_bytes) {
        // Best effort: nothing else was written, so the record has nothing to recover.
        let _ = std::fs::remove_file(&txn);
        return Err(e);
    }
    if let Err(e) = super::write_save_file(companion, native_bytes) {
        // Best effort: the record now says which half was written; if even that fails, the
        // "writing" record left behind still marks the save as interrupted.
        let _ = super::write_save_file(&txn, &record("partial"));
        return Err(bad(
            SAVE,
            format!(
                "the EPS was saved, but its native master {companion} could not be written ({e}); the document stays modified: save again (the EPS carries the editable document too)"
            ),
        ));
    }
    let mut warnings = enc.notes();
    if let Err(e) = std::fs::remove_file(&txn) {
        warnings.push(format!("the save finished, but its record {txn} could not be removed ({e}); it is safe to delete"));
    }
    let reply = json!({
        "path": eps,
        "companion": companion,
        "format": "eps",
        "bytes": eps_bytes.len(),
        "companionBytes": native_bytes.len(),
        "sha256": sha,
        "documentId": id,
        "revision": revision,
        "warnings": warnings,
    });
    Ok((job, reply))
}

/// How the master at `companion` stands against the EPS at `eps`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pair_status_of(eps: &str, companion: &str) -> Value {
    let mut out = json!({ "path": eps, "companion": companion });
    let txn = txn_path(eps);
    if std::path::Path::new(&txn).is_file() {
        let record = super::read_file(&txn).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
        out["status"] = json!("interrupted");
        out["transaction"] = json!({ "path": txn, "state": record.get("state").cloned().unwrap_or(Value::Null) });
        out["reason"] = json!("a paired save to this EPS did not finish: the EPS and the master may not match");
        return out;
    }
    let status = |out: &mut Value, status: &str, reason: &str| {
        out["status"] = json!(status);
        out["reason"] = json!(reason);
    };
    if !std::path::Path::new(companion).is_file() {
        status(&mut out, "missing", "there is no native master beside the EPS");
        return out;
    }
    let Some(master) = super::read_file(companion).ok().and_then(|b| super::load(companion, &b).ok()) else {
        status(&mut out, "unreadable", "the native master could not be read");
        return out;
    };
    let Some(link) = linkage(&master.doc) else {
        status(&mut out, "foreign", "the .vectorcraft file beside the EPS was not written by a paired save");
        return out;
    };
    out["documentId"] = link.get("documentId").cloned().unwrap_or(Value::Null);
    out["revision"] = link.get("revision").cloned().unwrap_or(Value::Null);
    let Ok(bytes) = super::read_file(eps) else {
        status(&mut out, "missingEps", "the EPS could not be read");
        return out;
    };
    let recorded = link.get("eps").and_then(|e| e.get("sha256")).and_then(Value::as_str);
    if recorded != Some(sha256_hex(&bytes).as_str()) {
        status(&mut out, "stale", "the EPS changed since the master was saved with it (edited elsewhere, or the master is older)");
        return out;
    }
    // The EPS carries the document it was saved from: it must be the master's document.
    let eps_id = super::load(eps, &bytes).ok().and_then(|l| document_id(&l.doc).map(str::to_string));
    if eps_id.as_deref() != link.get("documentId").and_then(Value::as_str) {
        status(&mut out, "foreign", "the EPS and the master belong to different documents");
        return out;
    }
    status(&mut out, "linked", "the master matches the EPS");
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn pair_status(_: &mut Session, p: &Value) -> Result<Value> {
    let eps = str_param(p, "path").filter(|p| !p.trim().is_empty()).ok_or_else(|| bad(STATUS, "give `path`: the EPS file"))?;
    let companion = str_param(p, "companion").filter(|c| !c.trim().is_empty()).map_or_else(|| default_companion(eps), str::to_string);
    Ok(pair_status_of(eps, &companion))
}

/// `file.openPair`.
#[cfg(not(target_arch = "wasm32"))]
fn open_pair(s: &mut Session, p: &Value) -> Result<Value> {
    let eps = str_param(p, "path").filter(|p| !p.trim().is_empty()).ok_or_else(|| bad(OPEN, "give `path`: the EPS file"))?.to_string();
    let companion = str_param(p, "companion").filter(|c| !c.trim().is_empty()).map_or_else(|| default_companion(&eps), str::to_string);
    let status = pair_status_of(&eps, &companion);
    let state = status["status"].as_str().unwrap_or("unknown").to_string();
    let wanted = str_param(p, "use").unwrap_or("auto");
    let master = match (wanted, state.as_str()) {
        ("auto", "linked") | ("master", _) => true,
        ("eps", _) => false,
        ("auto", _) => {
            let reason = status["reason"].as_str().unwrap_or_default();
            return Err(bad(
                OPEN,
                format!("the native master is {state} ({reason}): give use: \"eps\" to open the EPS, or use: \"master\" to open the master anyway"),
            ));
        }
        (other, _) => return Err(bad(OPEN, format!("use: `{other}` is not one of auto, master, eps"))),
    };
    let mut reply = if master {
        if !std::path::Path::new(&companion).is_file() {
            return Err(bad(OPEN, format!("{companion}: there is no native master to open")));
        }
        let r = s.execute("document.open", &json!({ "path": companion }))?;
        // The master stands in for the EPS: saving it writes the pair again.
        let st = s.doc_mut()?;
        st.path = Some(eps.clone());
        st.format = "eps";
        r
    } else {
        s.execute("document.open", &json!({ "path": eps }))?
    };
    reply["opened"] = json!(if master { "master" } else { "eps" });
    reply["pair"] = status;
    reply["path"] = json!(eps);
    Ok(reply)
}

/// Does `document.save` with `p` save the active document as a pair? Its own file is an EPS it
/// was paired with, and `p` names no other path or format.
pub(crate) fn routes_save(s: &Session, p: &Value) -> bool {
    let Some(st) = s.active() else { return false };
    let own = st.path.as_deref();
    let other_path = str_param(p, "path").is_some_and(|given| !given.is_empty() && own.is_none_or(|own| given != own));
    let other_format = str_param(p, "format").is_some_and(|f| f != "eps");
    st.format == "eps" && own.is_some() && linkage(&st.doc).is_some() && !other_path && !other_format
}

/// `file.epsPreflight`: what an EPS of the active document leaves behind, before it is written.
fn eps_preflight(s: &mut Session, p: &Value) -> Result<Value> {
    let mut options = p.as_object().cloned().unwrap_or_default();
    options.remove("path");
    options.insert("format".into(), json!("eps"));
    let r = s.execute("document.export", &Value::Object(options))?;
    let doc = s.doc()?.doc.clone();
    let mut issues = vec![];
    for w in r["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let lower = w.to_ascii_lowercase();
        let kind = if lower.contains("transparen") {
            "transparency"
        } else if lower.contains("raster effect") || lower.contains("rasteriz") {
            "rasterEffects"
        } else if lower.contains("font") || lower.contains("type ") {
            "type"
        } else {
            "other"
        };
        issues.push(json!({ "kind": kind, "message": w }));
    }
    let mut has_text = false;
    doc.walk(|n| has_text |= matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)));
    if has_text {
        issues.push(json!({ "kind": "outlinedText", "message": "type is written as outlines in the EPS; the native master keeps it editable" }));
    }
    let boards = doc.artboards.len();
    let use_artboards = bool_or(p, "useArtboards", false);
    if boards > 1 && !use_artboards {
        issues.push(json!({
            "kind": "artboards",
            "message": format!("{boards} artboards: the EPS holds the visible art of all of them on one page (useArtboards writes a file per artboard, which a paired save doesn't take)")
        }));
    }
    Ok(json!({
        "issues": issues,
        "artboards": boards,
        "ok": issues.is_empty(),
        "alternatives": ["document.export {format: \"pdf\"} keeps transparency, live type and every artboard"],
    }))
}

#[cfg(target_arch = "wasm32")]
fn no_files(cmd: &str) -> crate::EngineError {
    bad(cmd, "paired saves need files on disk: not available in the browser yet")
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn save_pair(_: &mut Session, _: &Value) -> Result<Value> {
    Err(no_files(SAVE))
}

#[cfg(target_arch = "wasm32")]
fn pair_status(_: &mut Session, _: &Value) -> Result<Value> {
    Err(no_files(STATUS))
}

#[cfg(target_arch = "wasm32")]
fn open_pair(_: &mut Session, _: &Value) -> Result<Value> {
    Err(no_files(OPEN))
}

fn always_ok(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.savePair",
            "Save EPS with Native Master",
            [],
            None,
            "{path?: the EPS file (default: the document's own, when it is an EPS saved as a pair), companion?: the native master (default: the EPS's name with .vectorcraft), replaceCompanion?: false (overwrite a .vectorcraft file that is not this document's master), …the EPS options of document.save (useArtboards with several artboards is refused: one EPS file)} write an EPS and a native .vectorcraft master from the same snapshot: both are encoded and checked first, a transaction record (.<eps name>.pairtxn) is written, then the EPS, then the master, then the record is removed. The document takes on the EPS as its own file; document.save then saves the pair again. If the master can't be written after the EPS was, the reply is an error saying so, the record stays (file.pairStatus: interrupted) and the document stays modified → {path, companion, format: \"eps\", bytes, companionBytes, sha256 (the EPS's), documentId, revision, warnings}",
            has_doc,
            save_pair
        ),
        cmd!(
            query "file.pairStatus",
            "EPS and Master Status",
            [],
            None,
            "{path: the EPS file, companion?: (default: the EPS's name with .vectorcraft)} → {path, companion, status: linked | stale (the EPS changed since the master was saved) | missing (no master) | foreign (not a paired master, or another document's) | unreadable | missingEps | interrupted (a paired save didn't finish: transaction {path, state}), reason, documentId?, revision?}",
            always_ok,
            pair_status
        ),
        cmd!(
            "file.openPair",
            "Open EPS with Native Master",
            [],
            None,
            "{path: the EPS file, companion?, use?: auto (default: the master when file.pairStatus says linked, else an error naming the status) | master (the master even when stale) | eps (the EPS)} open the editable master in place of its EPS (the document's own file stays the EPS, so saving writes the pair again), or the EPS → {…as document.open, opened: master|eps, pair: file.pairStatus's reply, path: the EPS}",
            always_ok,
            open_pair
        ),
        cmd!(
            query "file.epsPreflight",
            "EPS Preflight",
            [],
            None,
            "{…the EPS options of document.export} what an EPS of the document leaves behind, before anything is written → {issues: [{kind: transparency | rasterEffects | type | outlinedText | artboards | other, message}], artboards, ok (no issues), alternatives}",
            has_doc,
            eps_preflight
        ),
    ]
}
