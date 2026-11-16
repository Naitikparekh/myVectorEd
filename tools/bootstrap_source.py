#!/usr/bin/env python3
"""Import the pinned VectorCraft source without replacing repository-owned files."""
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

UPSTREAM = "https://github.com/storytold/vectorcraft.git"
REVISION = "9c61c0b66e39082b1c0c448cbf59fd323736ff9c"
ROOT = Path(__file__).resolve().parents[1]

def run(*args, cwd):
    return subprocess.run(args, cwd=cwd, check=True, text=True, capture_output=True).stdout.strip()

def main():
    if (ROOT / "Cargo.toml").exists():
        raise SystemExit("Source already present; refusing to overwrite it.")
    with tempfile.TemporaryDirectory(prefix="myvectored-upstream-") as directory:
        source = Path(directory)
        run("git", "init", cwd=source)
        run("git", "remote", "add", "origin", UPSTREAM, cwd=source)
        run("git", "fetch", "--depth=1", "origin", REVISION, cwd=source)
        run("git", "checkout", "--detach", "FETCH_HEAD", cwd=source)
        if run("git", "rev-parse", "HEAD", cwd=source) != REVISION:
            raise SystemExit("Upstream revision mismatch")
        excluded = {".git", ".github", "README.md", "AGENTS.md"}
        items = [p for p in source.iterdir() if p.name not in excluded]
        collisions = [p.name for p in items if (ROOT / p.name).exists()]
        if collisions:
            raise SystemExit("Import would overwrite: " + ", ".join(collisions))
        for item in items:
            destination = ROOT / item.name
            if item.is_dir():
                shutil.copytree(item, destination, symlinks=True)
            else:
                shutil.copy2(item, destination)
        archive = ROOT / "docs" / "upstream-reference"
        archive.mkdir(parents=True, exist_ok=True)
        for name in ("README.md", "AGENTS.md"):
            shutil.copy2(source / name, archive / name)
    (ROOT / "UPSTREAM.json").write_text(json.dumps({
        "repository": UPSTREAM, "commit": REVISION,
        "excluded": sorted(excluded),
        "status": "Unmodified engine; myVectorEd implementation pending",
    }, indent=2) + "\n", encoding="utf-8")
    print("Imported pinned VectorCraft source. No feature parity or build claim made.")

if __name__ == "__main__":
    main()
