# myVectorEd

A developing desktop vector editor based on VectorCraft. Intended focus: practical drawing and layout, English/Gujarati typography, clear UI interactions, and reliable EPS/PDF output.

## Status

**Early development, not a finished upgrade or a measured 90/100 replacement.**

The development branch imports VectorCraft at the revision recorded in [UPSTREAM.json](UPSTREAM.json) and changes the initial desktop window/app chrome title to myVectorEd. Internal Rust crate names and the native .vectorcraft format remain compatible.

EPS-first saving with an editable companion, a full independent application identity, and the planned UX improvements are **not implemented yet**. Upstream icons, profile behavior and update integration still require work; do not use the upstream updater in a development build.

See [implementation and acceptance criteria](docs/MYVECTORED_PLAN.md).

## Build

The [GitHub Actions workflow](../../actions) imports the pinned source onto the development branch and checks/builds the Windows desktop. A successful run attaches an unsigned development preview, not a production installer. An in-progress or failed run does not mean a usable build exists.

Locally, after source import, install the Rust version declared in Cargo.toml and run:

```sh
cargo check --locked -p vectorcraft -p vectorcraft-cli
cargo build --locked -p vectorcraft
```

The executable retains its internal vectorcraft build target. CI names the preview executable myVectorEd.exe.

## Source and licensing

Based on [VectorCraft](https://github.com/storytold/vectorcraft), MIT OR Apache-2.0. Preserve LICENSE-MIT, LICENSE-APACHE, NOTICE and upstream contributor attribution. Original upstream documentation is under docs/upstream-reference. Reference books are not distributed with this project.
