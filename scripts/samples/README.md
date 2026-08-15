# Sample libraries

Altium-authored reference libraries — the ground truth for the reader and round-trip
tests. **Generated on-site, not hand-edited** (one exception, in `manual/` — see below): run `scripts\Generate-Samples.ps1`,
which drives a real Altium Designer (via `altium\generate\GenerateSamples.pas`) to
author the libraries, then moves them here to be committed.

Committed as binaries (like [AltiumSharp](https://github.com/issus/AltiumSharp)'s `TestData`)
so CI can read them without Altium. Regenerate and re-commit whenever the authoring script's coverage grows.

> Building these is iterative — generate, read back with the Rust tests, extend the
> primitive set, regenerate. Coverage grows component by component.

## `manual/` — hand-authored, do NOT regenerate

`Generate-Samples.ps1` cannot produce everything: a few properties exist only in Altium's
UI and are not exposed on the scripting interfaces, so no DelphiScript can author them.
Those live in `manual/`, made by hand and committed as-is.

**`Generate-Samples.ps1` never touches this folder** — it only copies its own outputs over
the two top-level libraries. Equally, nothing regenerates these files: if one is deleted,
it has to be rebuilt by hand from the recipe below.

### `manual/parameters.SchLib`

One component, `PARAMPROPS`, carrying three `RECORD=41` parameters that between them cover
the parameter properties the generated golden cannot reach:

| Parameter | Carries | Why it is here |
|-----------|---------|----------------|
| `TestParam` = `123` | `Justification=7`, `NotAutoPosition=T` | the generated golden omits both, because Altium omits a property left at its default |
| `Rule` | `Text=UNIONINDEX=0¦RULEKIND=Width¦…`, `Description`, `IsHidden=T` | a PCB design-rule directive parameter — proves a rule is identified by `Name=Rule` plus that payload, **not** by an `IsRule` flag |
| `Comment` = `*` | the default set only | the control: it shows which keys Altium omits when nothing is changed |

**To rebuild it:**

1. **File → New → Library → Schematic Library**, save as `parameters.SchLib`.
2. Rename the component to `PARAMPROPS` and draw anything (a rectangle and one pin);
   the graphics are irrelevant.
3. Add a parameter `TestParam` = `123`, **visible**. In its Properties:
   - **untick Autoposition** — ticked is the default and Altium then writes nothing;
   - set **Justification** to top-centre (the up arrow), which stores `Justification=7`.
4. Add a second parameter via the parameter list's **Add → Rule**, choose a
   *Max-Min Width* rule, leave the widths at 10 mil, and click **OK** (not Cancel — a
   cancelled dialog writes nothing).
5. Save, and copy the file here.

**To check it before committing** — prints every key Altium actually wrote per parameter:

```powershell
python -c "import olefile,re,sys;f=olefile.OleFileIO(sys.argv[1]);d=b''.join(f.openstream('/'.join(e)).read() for e in f.listdir() if e[-1]=='Data');r=[x for x in re.split(rb'(?=\|RECORD=)',d) if b'RECORD=41' in x];[print('---',sorted(set(k.decode() for k in re.findall(rb'\|([A-Za-z0-9._%]+)=',x)))) for x in r]" scripts\samples\manual\parameters.SchLib
```

`NotAutoPosition` and `Justification` must both appear on `TestParam`, or step 3 did not
take.

## Contents

Each component groups primitives that share one feature axis, so a failing read test
pinpoints the feature. Tests live in [`tests/samples_pcblib.rs`](../../tests/samples_pcblib.rs)
and [`tests/samples_schlib.rs`](../../tests/samples_schlib.rs).

| Library | Component | Exercises |
|---------|-----------|-----------|
| `footprints.PcbLib` | `PAD_SHAPES` | Four SMD pads, one per pad shape: Round, Rectangle, Octagonal, RoundedRectangle |
| `footprints.PcbLib` | `PAD_HOLES` | Three through-hole pads, one per hole shape: round, square, slot (square/slot exercise the 651-byte size/shape block) |
| `footprints.PcbLib` | `VIAS` | Two simple through-vias (Top to Bottom), different pad/hole sizes |
| `footprints.PcbLib` | `PAD_STACK` | A multi-layer through-hole pad stack (top/mid/bottom shapes and sizes differ) |
| `footprints.PcbLib` | `TRACKS` | Five tracks: a 4-segment silk box + a wider copper track |
| `footprints.PcbLib` | `ARCS` | A full circle and a quarter arc |
| `footprints.PcbLib` | `REGIONS` | A copper box and a mechanical box (filled regions) |
| `footprints.PcbLib` | `FILLS` | Two top-layer copper fills, one axis-aligned and one rotated 45 degrees |
| `footprints.PcbLib` | `BODY3D` | A simple extruded 3D component body (rectangular outline + height) |
| `footprints.PcbLib` | `TEXT_STROKE` | Stroke-font strings, including a 90° rotation |
| `footprints.PcbLib` | `TEXT_WIN1252` | Stroke text with non-ASCII Windows-1252 glyphs (micro sign, plus-minus) that round-trip to UTF-8 |
| `footprints.PcbLib` | `EDGE` | Boundary-case pads: a 45° rotated rectangle, plus negative and large coordinates |
| `symbols.SchLib` | `PINS_ETYPE` | Eight pins, one per electrical type: input, bidirectional, output, open-collector, passive, hi-z, open-emitter, power |
| `symbols.SchLib` | `PINS_ORIENT` | Four pins, one per orientation: right, up, left, down |
| `symbols.SchLib` | `PINS_VIS` | Pins covering show-name/show-designator combinations plus a hidden pin |
| `symbols.SchLib` | `PINS_DECOR` | A clock or dot on each of the four IEEE decoration slots (inner/outer edge, inside, outside) |
| `symbols.SchLib` | `LINES` | Horizontal, vertical and diagonal lines |
| `symbols.SchLib` | `ARCS` | A full circle and a quarter arc |
| `symbols.SchLib` | `LABELS` | Free-text labels with different justifications and a rotation |
| `symbols.SchLib` | `PARAMS` | A visible and a hidden component parameter |
| `symbols.SchLib` | `DUALPART` | A two-part symbol; pins split across part 1 and part 2 |
| `symbols.SchLib` | `RECTS` | A filled and an unfilled rectangle |
| `symbols.SchLib` | `ELLIPSES` | A circle and an ellipse |
| `symbols.SchLib` | `POLYLINES` | A three-point open polyline |
| `symbols.SchLib` | `ROUNDRECTS` | A filled rounded rectangle |
| `symbols.SchLib` | `POLYGONS` | Two filled four-vertex polygon boxes |
| `symbols.SchLib` | `EDGE` | Boundary-case pins: large and negative coordinates, and a 35-character pin name |
