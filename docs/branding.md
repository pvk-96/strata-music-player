# Branding

The mark is a vinyl record seen edge on, cut into strata: six warm-cream wave strokes bend
around a small drilled hole on a rounded blue square. Everything in the repository is derived
from one file.

## Source of truth

| File | Role |
| --- | --- |
| `assets/branding/strata-icon.svg` | **The master.** Edit this, never a generated file |
| `data/icons/hicolor/…/dev.strata.Strata.png` | Generated Linux size family, installed by every package |
| `data/icons/hicolor/scalable/apps/dev.strata.Strata.svg` | Copy of the master, installed for GTK |
| `assets/branding/windows/strata.ico` | Generated Windows executable icon |

The approved design raster is deliberately **not** committed: it is a design input, not a build
input, and the SVG carries everything the build needs. If you change the mark, compare against
that raster before committing (see the last section).

Geometry, in the master's 1024×1024 viewBox:

| Element | Value |
| --- | --- |
| Canvas | `1024×1024`, transparent outside the square |
| Square | `x=109 y=109 width=806 height=806 rx=196` |
| Blue | `#1056E5` |
| Cream | `#F3F0EE` |
| Stroke height | `32` (`HALF = 16`), drawn as `16`-radius end caps at `x = ±226` |
| Bar centres | `y = -214`, `-158`, `-102` and their 180° rotations |
| Disc | `r = 82` centred on `512,512`, with an `r = 17` hole |

The artwork is deliberately 180°-rotationally symmetric: each upper stroke is defined once and
its lower partner is the same path mirrored through the centre. Only the disc is a compound path
(`fill-rule="evenodd"`) so the hole punches through.

## Rules

* Flat and vector-only: no gradients, shadows, filters, embedded bitmaps or text in the SVG.
* Keep the padding: the square occupies 79% of the canvas, so the icon keeps its shape in a
  panel, in a launcher grid and in a file manager at any size.
* Do not add new symbols — no note, headphone, play button or extra waveform.
* Use `dev.strata.Strata` as the icon name everywhere. It is the application id, so the desktop
  entry, the AppStream metadata, the Flatpak manifest and hicolor cannot drift apart.

## Regenerating

```sh
python3 scripts/generate-icons.py           # rewrite every derived file
python3 scripts/generate-icons.py --check   # fail if the committed files are stale
```

The script needs one of `rsvg-convert` (librsvg), `resvg`, or the `cairosvg` module. It renders
16, 22, 24, 32, 48, 64, 128, 256 and 512 px PNGs with a transparent background, and packs
16, 24, 32, 48, 64, 128 and 256 px frames into the `.ico`. `--check` compares decoded pixels, so a
different rasteriser version cannot cause a false failure.

The PNGs and the `.ico` **are** committed, because the Debian, RPM and Flatpak builds install
them straight out of the tree and none of them regenerates them. `tests/packaging.rs` asserts
that the master stays flat, that every size in the family exists at the right dimensions, that
the `.ico` carries all seven frames, and that the desktop entry, AppStream metadata, hicolor
theme and packaging scripts still agree on one icon name. Run `cargo test` after changing
anything in `assets/` or `data/`.

## Checking a change against the approved raster

Point the comparison at your copy of the approved reference; it is not in the repository. The
approved master scores **0.993** on this measure, so anything below about 0.99 means geometry
drifted.

```sh
REFERENCE=/path/to/reference.png
rsvg-convert -w 1254 -h 1254 assets/branding/strata-icon.svg -o /tmp/strata-1254.png
python3 - "$REFERENCE" <<'PY'
import sys, numpy as np
from PIL import Image
ref = np.array(Image.open(sys.argv[1]).convert('RGB')).astype(int)
shot = Image.open('/tmp/strata-1254.png').convert('RGBA')
flat = Image.new('RGBA', shot.size, (255, 255, 255, 255)); flat.alpha_composite(shot)
shot = np.array(flat.convert('RGB')).astype(int)
mark = lambda a: ((a.max(axis=2) - a.min(axis=2)) > 12) | (a.mean(axis=2) < 250)
print('IoU', (mark(ref) & mark(shot)).sum() / (mark(ref) | mark(shot)).sum())
PY
```