#!/usr/bin/env python3
"""Writes THIRD-PARTY.md: every crate in the macOS build and its licence.

Run it after changing dependencies. The notices at the top are by hand, for what the licences ask to be
named specifically.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

HEADER = """# Third-party software

Reel is licensed under the AGPL-3.0 (see LICENSE). It's a rewrite of [Recordly](https://github.com/webadderallorg/Recordly), which is AGPL-3.0 too, and parts of its behaviour closely follow Recordly's.

## Notices

- **Rust/UI** (<https://rust-ui.com/>), MIT. Its components are vendored in `crates/reel-app/src/ui`, and listed with what's changed in `VENDORED.md`.
- **whisper.cpp** and **ggml** (<https://github.com/ggml-org/whisper.cpp>), MIT. Built into the app through `whisper-rs`. Speech models are downloaded separately, only when asked for.
- **Mozilla's root certificates**, through `webpki-roots`, CDLA-Permissive-2.0. Used to check the download of a speech model.
- **FFmpeg** isn't part of Reel on macOS, which uses Apple's frameworks. The Windows and Linux plans would use an LGPL build, and would need its notice added here.

## Crates

"""


def main() -> int:
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", "aarch64-apple-darwin"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    metadata = json.loads(output)
    resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
    packages = {
        (package["name"], package["version"], package.get("license") or "See the crate")
        for package in metadata["packages"]
        if package["id"] in resolved and package.get("source")
    }
    lines = [HEADER, "| Crate | Version | Licence |\n|---|---|---|\n"]
    lines += [f"| {name} | {version} | {licence} |\n" for name, version, licence in sorted(packages)]
    (ROOT / "THIRD-PARTY.md").write_text("".join(lines))
    print(f"{len(packages)} crates")
    return 0


if __name__ == "__main__":
    sys.exit(main())
