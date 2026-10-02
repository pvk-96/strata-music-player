#!/usr/bin/env python3
"""Regenerate the Strata icon family from the master artwork.

The master ``assets/branding/strata-icon.svg`` is the only hand-edited source. Everything
else in this repository is derived from it:

* ``assets/branding/windows/strata.ico`` - multi-resolution Windows executable icon.
* ``data/icons/hicolor/<size>x<size>/apps/dev.strata.Strata.png`` and
  ``data/icons/hicolor/scalable/apps/dev.strata.Strata.svg`` - the installed Linux theme.

Every generated PNG is committed, because packaging installs them straight from the tree and
neither the Debian, RPM nor Flatpak build regenerates them; ``--check`` is what keeps them honest.

Usage::

    python3 scripts/generate-icons.py           # write every derived file
    python3 scripts/generate-icons.py --check   # fail if the committed files are stale

Rendering needs one of ``rsvg-convert``, ``resvg`` or the ``cairosvg`` module; the check is
skipped when no renderer is available so that source-only checkouts stay usable.
"""

from __future__ import annotations

import argparse
import io
import shutil
import struct
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ElementTree
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MASTER = ROOT / "assets" / "branding" / "strata-icon.svg"
BRANDING_ICO = ROOT / "assets" / "branding" / "windows" / "strata.ico"
HICOLOR_DIR = ROOT / "data" / "icons" / "hicolor"
HICOLOR_SCALABLE = HICOLOR_DIR / "scalable" / "apps" / "dev.strata.Strata.svg"

# The hicolor sizes a panel, launcher or file manager may ask for; 22 covers panels. 1024 is
# not a hicolor directory - scalable themes answer that request from the SVG instead.
PNG_SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)
# Windows picks an entry from the executable icon, so ship every shell size too.
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


def find_renderer() -> tuple[str, list[str]]:
    """Return (name, argv prefix) for the first available SVG rasteriser."""
    if shutil.which("rsvg-convert"):
        return "rsvg-convert", ["rsvg-convert"]
    if shutil.which("resvg"):
        return "resvg", ["resvg"]
    try:  # cairosvg is a pure-python fallback, handy on minimal systems
        import cairosvg  # noqa: F401
    except ImportError:
        return "", []
    return "cairosvg", []


def render(master: Path, size: int, renderer: tuple[str, list[str]]) -> bytes:
    name, argv = renderer
    if name == "cairosvg":
        import cairosvg

        return cairosvg.svg2png(url=str(master), output_width=size, output_height=size)
    command = [*argv, "-w", str(size), "-h", str(size)]
    if name == "rsvg-convert":
        command += [str(master)]
    else:
        command += [str(master)]
    done = subprocess.run(command, check=True, capture_output=True)
    return done.stdout


def ico_bytes(frames: list[tuple[int, bytes]]) -> bytes:
    """Pack PNG frames into an .ico container (Vista and newer read PNG frames)."""
    header = struct.pack("<HHH", 0, 1, len(frames))
    directory = bytearray()
    payload = bytearray()
    offset = 6 + 16 * len(frames)
    for size, data in frames:
        directory += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset
        )
        payload += data
        offset += len(data)
    return header + bytes(directory) + bytes(payload)


def png_size(data: bytes) -> tuple[int, int]:
    """Read the pixel size out of a PNG header."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    width, height = struct.unpack(">II", data[16:24])
    return width, height


def same_pixels(left: bytes, right: bytes) -> bool:
    """Compare decoded pixels so a different rasteriser version cannot fail the check."""
    try:
        from PIL import Image
    except ImportError:
        return left == right
    try:
        with Image.open(io.BytesIO(left)) as a, Image.open(io.BytesIO(right)) as b:
            return a.convert("RGBA").tobytes() == b.convert("RGBA").tobytes()
    except Exception:
        return left == right


def validate_master(master: Path) -> None:
    root = ElementTree.parse(master).getroot()
    if not root.tag.endswith("svg"):
        raise SystemExit(f"{master} is not an SVG document")
    if root.get("viewBox") != "0 0 1024 1024":
        raise SystemExit(f"{master} must use a 0 0 1024 1024 viewBox")
    text = master.read_text(encoding="utf-8")
    for banned in ("<image", "<filter", "<linearGradient", "<radialGradient", "<text"):
        if banned in text:
            raise SystemExit(f"{master} must stay flat and vector-only (found {banned})")


def build(renderer: tuple[str, list[str]], out: Path) -> dict[Path, Path]:
    """Stage every derived artefact in ``out`` and return ``{destination: staged}``."""
    produced: dict[Path, Path] = {}
    png = {size: render(MASTER, size, renderer) for size in PNG_SIZES}
    for size, data in png.items():
        if png_size(data) != (size, size):
            raise SystemExit(f"{size}x{size} render produced {png_size(data)}")
        destination = HICOLOR_DIR / f"{size}x{size}" / "apps" / "dev.strata.Strata.png"
        staged = out / destination.relative_to(ROOT).as_posix()
        staged.parent.mkdir(parents=True, exist_ok=True)
        staged.write_bytes(data)
        produced[destination] = staged
    frames = [(size, png[size]) for size in ICO_SIZES]
    staged_ico = out / BRANDING_ICO.relative_to(ROOT).as_posix()
    staged_ico.parent.mkdir(parents=True, exist_ok=True)
    staged_ico.write_bytes(ico_bytes(frames))
    produced[BRANDING_ICO] = staged_ico
    staged_svg = out / HICOLOR_SCALABLE.relative_to(ROOT).as_posix()
    staged_svg.parent.mkdir(parents=True, exist_ok=True)
    staged_svg.write_bytes(MASTER.read_bytes())
    produced[HICOLOR_SCALABLE] = staged_svg
    return produced


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed files match the master instead of writing them",
    )
    args = parser.parse_args()
    validate_master(MASTER)
    renderer = find_renderer()
    if not renderer:
        print("no SVG rasteriser found (install librsvg or resvg); nothing to do")
        return 0 if args.check else 1

    with tempfile.TemporaryDirectory() as tmp:
        produced = build(renderer, Path(tmp))
        stale = []
        for destination, staged in sorted(produced.items()):
            data = staged.read_bytes()
            if args.check:
                if not destination.exists() or not same_pixels(destination.read_bytes(), data):
                    stale.append(destination.relative_to(ROOT).as_posix())
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                print(f"wrote {destination.relative_to(ROOT)}")
    if args.check:
        if stale:
            print("stale icon files, run python3 scripts/generate-icons.py:", file=sys.stderr)
            for name in stale:
                print(f"  {name}", file=sys.stderr)
            return 1
        print("icon assets are up to date")
    return 0


if __name__ == "__main__":
    sys.exit(main())