# Scripting: the document object model for scripts

**Status (2026-11-19):** the script engine (`crates/js`) runs ECMAScript 5.1 with E4X, the
preprocessor lines and `File`/`Folder`; `script.run` gives scripts the `vc` object (any command,
a document snapshot, the log) and `$`. This page tracks the work on a document object model in the
style of the reference app's public scripting reference, so that common scripts written for it run
here: `app`, `documents`, layers, path, text and group items, colours, the selection, artboards,
export and save. Each behaviour cites the public page it follows; nothing is copied from the
reference app, its sample scripts or its documentation text.

Sources (public documentation only):

- Scripting reference, object model: <https://ai-scripting.docsforadobe.dev/> and
  <https://helpx.adobe.com/illustrator/using/automation-with-scripts.html>
- File › Scripts: <https://helpx.adobe.com/illustrator/using/automation-with-scripts.html>

## How host objects work

A script reaches the document through **host classes** (`vectorcraft_js::HostClass`, defined with
`Realm::define_host_class`). A host class is a global constructor whose prototype has properties
and methods that the app answers: reading `item.name` or calling `item.remove()` calls into the
engine with the object the script holds.

- An object of the model is a `Data::Host(HostRef { class, id, items })` on the host side: the class
  name and the app's id for the thing it stands for. In the script it is an object with the class's
  prototype, so `instanceof` and `constructor` work, and when the script passes it back (as an
  argument, inside an array or object, or as `this`) the app gets the same reference.
- A reference with `items` is also a list (a collection such as `pathItems`): the items are at
  indices `0 … length-1` and `length` is set; none of them can be changed by the script.
- The class and id live in two hidden, read-only properties; `for … in`, `JSON.stringify` and plain
  data reads do not see them, and a script cannot change them.
- Each read gives a new object, so two objects for one item are not `===`; compare them through the
  model (for example by `uuid`), as the app compares them by id.
- A class with a constructor makes plain values with its prototype (`new RGBColor()`); a class
  without one throws a `TypeError` when a script tries to make one.
- Passed to `vc.run`, a host object becomes the app's id string for it.

## Checklist

| Area | State | Evidence |
|---|---|---|
| Host classes in the script engine | done | `crates/js/src/host_class_tests.rs` |
| `app`, `app.activeDocument`, `documents.add` | to do | |
| Layers, `pathItems` (rectangle, ellipse, polygon, star, add, setEntirePath) | to do | |
| `fillColor` / `strokeColor` with `RGBColor`, `CMYKColor`, `GrayColor`, `NoColor` | to do | |
| `textFrames` (contents, character size and font) | to do | |
| `groupItems`, `selection`, `artboards` | to do | |
| `exportFile` (SVG, PDF, PNG options), `saveAs` | to do | |
| Batch over a folder with an Action | to do | |
| Data sets (CSV) and image and graph variables | to do | |

## Revisions

| Date | Change |
|---|---|
| 2026-11-19 | Host classes: `Data::Host`, `HostClass`, `Realm::define_host_class`; objects that stand for something the app keeps, with properties and methods the app answers. |
