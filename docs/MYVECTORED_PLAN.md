# myVectorEd implementation contract

## Current status

Bootstrap tooling and a narrow desktop title overlay are prepared. This is not a completed application upgrade. Build results must be checked in Actions. No 90/100 score has been earned.

Base: storytold/vectorcraft commit 9c61c0b66e39082b1c0c448cbf59fd323736ff9c.
Keep upstream MIT/Apache licenses, NOTICE and contributor attribution.
Use published documentation as behavior reference; do not redistribute the supplied books or copy their artwork.

## Stage 1: safe independent application

- Import source at the pinned revision, preserving provenance.
- Rename customer-facing app, window, About, menus and package identity to myVectorEd.
- Original app icon; record attribution. Do not ship upstream trademark assets as the new identity.
- Separate profile, recovery, single-instance and installation identities.
- Disable upstream self-update; only offer an update mechanism after myVectorEd releases exist.
- Preserve .vectorcraft and legacy reading compatibility; do not blindly rename Rust identifiers or format identifiers.
- Build/test Windows first. Then macOS and web regression checks.
- Development preview is not an installer or production release.

## Stage 2: EPS-first saving with editable master

New documents suggest EPS. Explicit formats, existing native files, Save Copy and templates keep their intended semantics.

A save produces an EPS deliverable and companion native master from the same document revision. Stage and validate bytes before replacing files. Use a recoverable transaction record because replacing two files is not a single atomic operation. Do not overwrite unrelated companion files without confirmation. Preserve recovery data until both outputs are confirmed. Distinguish partial failures, never mark unsaved edits clean on failure, and preserve existing read-only destination handling.

Store document identity, revision and EPS content hash with the companion linkage. On reopen, use the native master only after linkage validation. Missing or stale companions require an explicit choice. Do not silently ignore external EPS edits.

EPS preflight must identify transparency flattening, outlined text, rasterized effects and multi-artboard handling. Permit cancel or PDF export. Native master retains original editable objects. Browser behavior needs an explicit paired download/archive flow.

Tests: canceled dialogs; permission failures; first/second write failure; process interruption and restart; name collisions; uppercase extensions; existing native saves; recovered documents; Save Copy; templates; multi-artboards; external EPS edits; missing companion; repeated save/reopen with editable type and effects.

### Stage 2 progress

Slice 1 (engine commands, `crates/engine/src/cmd/fileio/pair.rs`; tests in `fileio/tests_pair.rs`):

- `file.savePair`: encodes the EPS and the `.vectorcraft` master from one snapshot, checks both in memory, writes a transaction record (`.<eps name>.pairtxn`), then the EPS, then the master, then removes the record. Each file is replaced atomically through the existing save writer, so read-only and automation-root refusals behave as for any save. A master that can't be written after its EPS leaves the record marked `partial`, an error saying so, and the document modified. The document takes on the EPS as its own file only after both are written.
- Linkage in `Document::extra["myvectoredPair"]`: `documentId` and `revision` in the document and its EPS; the master also records the EPS's name, size and SHA-256.
- `document.save` on a document saved as a pair writes the pair again (next revision, same identity). Save As, Save a Copy, templates and EPS files never saved as a pair are unchanged.
- A `.vectorcraft` file at the master's path that belongs to another document is not overwritten unless `replaceCompanion: true`.
- `file.pairStatus`: `linked`, `stale` (EPS edited elsewhere), `missing`, `foreign`, `unreadable`, `missingEps` or `interrupted`. `file.openPair` opens the master only when linked, unless told `use: "master"` or `use: "eps"`; an opened master keeps the EPS as its file, so saving writes the pair again.
- `file.epsPreflight`: transparency flattening, rasterized effects, type written as outlines and several artboards, before anything is written, with PDF export named as the alternative.

Not done yet: the desktop UI (Save dialog defaulting to EPS for new documents, the stale/missing choice dialog, the preflight sheet with Cancel and Export PDF), Data Recovery keeping its copy until both files are confirmed, recovery of an interrupted record at start-up, a paired download for the browser build, and the dialog-cancel and restart tests that need the UI.

## Stage 3: customer workflow improvements

Original interface informed by Illustrator training and CorelDRAW documentation:
- Predictable click, Shift selection, marquee and node editing.
- Clear snapping feedback and precise dimension entry.
- Useful properties panel, keyboard shortcuts and discoverable tool options.
- Layers/artboards with legible names and selection state.
- English/Gujarati typography and fonts.
- Print dimensions, bleed, image resolution and export preflight.

## Acceptance score (target, not measured)

Drawing/editing 25; typography/layout 20; save/print fidelity 25; usability 15; reliability/performance 15.

Use original test documents representing a bilingual poster, two-sided card, large banner, vector cutting outline and effect-rich illustration. Score only demonstrated behavior. Record operating system, build commit, inputs and result. Correctness of save/recovery is mandatory even if the weighted score reaches 90.

## Remaining delivery gates

- Run upstream required checks and new targeted regressions.
- Inspect UI on a real presented frame.
- Validate EPS/PDF using independent permitted readers and print dimensions.
- No copied proprietary UI assets; preserve licenses.
- Deliver signed installers only after a separate signing setup; never imply unsigned previews are signed.
