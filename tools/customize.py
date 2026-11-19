#!/usr/bin/env python3
"""Apply the initial, deliberately narrow myVectorEd display-name overlay."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = (
    "apps/vectorcraft/src/main.rs",
    "crates/ui-egui/src/chrome.rs",
)
def main():
    changes = []
    for name in FILES:
        path = ROOT / name
        text = path.read_text(encoding="utf-8")
        if "VectorCraft" not in text:
            raise SystemExit(f"Expected pinned branding not found: {name}")
        changes.append((path, text.replace("VectorCraft", "myVectorEd")))
    for path, text in changes:
        path.write_text(text, encoding="utf-8")
    print("Updated desktop title and app chrome. Internal format and crate names preserved.")
    print("Development only: settings isolation, icons, updater, packaging and EPS pair-save remain pending.")

if __name__ == "__main__":
    main()
