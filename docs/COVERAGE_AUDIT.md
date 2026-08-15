# MCP Feature-Coverage Audit

<!-- markdownlint-disable MD013 -->

> What this server reads, writes and exposes, versus what an Altium library can actually
> carry. The goal is parity: nothing an Altium `.PcbLib` / `.SchLib` can store should be
> lost on read or unreachable on write.
>
> This is a worklist, not a history. Prune an entry the moment it ships.

## How to re-verify this list

Every entry below was checked against the source rather than carried forward on trust. The
previous edition had drifted badly — 103 of its 129 feature-loss entries named fields that
had since been implemented — so re-verify before acting on any of it:

```python
# every backticked Altium property claimed missing, vs every field we model
import io, os, re
audit = io.open('docs/COVERAGE_AUDIT.md', encoding='utf-8').read()
claims = re.findall(r'\*\*\[gap \| ([a-z]+)\]\*\*\s*`([^`]+)`', audit)
src = ''.join(io.open(os.path.join(r, f), encoding='utf-8', errors='replace').read()
              for r, _, fs in os.walk('src') for f in fs if f.endswith('.rs'))
fields = set(re.findall(r'pub ([a-z_0-9]+)\s*:', src))
```

Name matching only narrows the candidates — a field can be modelled under a different name
(`IsKeepout` and `IsTentingTop` live in `PcbFlags`, not in fields of their own), so confirm
each survivor by reading the parser. The three cross-checks that matter:

- **modelled?** a field on the primitive struct;
- **parsed and written?** `src/altium/{pcblib,schlib}/`;
- **reachable?** an entry in `src/mcp/tool_definitions.rs`.

A feature needs all three to count as supported, and a golden fixture to count as *proven*
— see [FIXTURE_COVERAGE.md](FIXTURE_COVERAGE.md).

## Status

Geometry and the properties that decide how a footprint or symbol **looks** are complete
across every primitive, in both directions, and pinned to Altium-authored fixtures.

What remains is fabrication and authoring **metadata**: fields Altium stores that never
change the rendering, which is why they outlasted everything else. Thirteen groups, listed
below.

## 1. PcbLib format gaps

Each of these is unmodelled: no struct field, nothing parsed, nothing written, nothing
exposed. A golden fixture cannot cover them until the field exists, so the reader work
leads and the fixture follows.

- **[gap | read]** `JumperID` — `AltiumSharp` reads an i16 at Pad offset 110 into
  `PcbPad.JumperID`; `PcbPadDto` exposes `JUMPERID`. Groups pads as a jumper / 0-ohm link
  and feeds test-point identification.
- **[gap | read]** `DrillType` (0=Simple, 1=Pressfit) — `AltiumSharp` `PcbPad.DrillType`
  separates a plated drilled hole from a press-fit one. Meaningful for connectors.
- **[gap | read]** `SolderMaskExpansionFromHoleEdge` on **Via** (`SubRecord-1` offset 258)
  — the Pad equivalent ships (main-block bool @125); the Via byte is still unread.
- **[gap | read]** `DrillLayerPairType` (Via `SubRecord-1` offset 312) — `AltiumSharp`
  reads `B(312)` (0=Through, 1=BlindBuriedStart, 2=Mid, 3=End). We infer span from
  `from_layer` / `to_layer` and have no explicit drill-pair type, so the blind/buried
  classification is lost.
- **[gap | read]** Text barcode sizing block — we model `kind = BarCode` and a golden
  covers it, but none of the block's own keys: `BarCodeKind@157`,
  `BarCodeRenderMode@158`, `BarCodeInverted@159`, `BarCodeFontName@161-224`,
  `BarCodeShowText@225`, `BarCodeFullWidth/Height@137/141`, `BarCodeXMargin/YMargin@145/149`,
  `BarCodeMinWidth@153`. A barcode round-trips as a barcode but loses its sizing.
- **[gap | read]** `model_2d_location` (`MODEL.2D.X` / `MODEL.2D.Y`) on ComponentBody —
  `model_2d_rotation` is modelled but the position is not: the reader drops both keys and
  the writer always emits `MODEL.2D.X=0mil|MODEL.2D.Y=0mil`. A body whose model is offset
  in the 2D plane loses that offset.

## 2. SchLib format gaps

None outstanding. The Parameter display properties (`AUTOPOSITION`, `ISRULE`,
`ISSYSTEMPARAMETER`, `TEXTHORZANCHOR` / `TEXTVERTANCHOR`) are modelled, read, written and
exposed. They have **no golden fixture**: AD24 does not expose them on `ISch_Parameter`,
so they cannot be authored by script, and the golden library does not contain them
naturally. Coverage is a write-readback round-trip until a hand-authored file provides
one — a fixture gap, not a modelling gap. See [FIXTURE_COVERAGE.md](FIXTURE_COVERAGE.md).

## 3. Tool-schema gaps

Modelled and round-tripped, but not reachable through `write_pcblib`:

- **[gap | tool]** Pad per-layer stack arrays — `stack_mode` is in the schema, but
  `per_layer_sizes` / `per_layer_shapes` / `per_layer_corner_radii` / `per_layer_offsets`
  are not, so a non-Simple stack can be requested and then not described.

## 4. Round-trip fidelity

Verified present, kept here only because fidelity regressions are easy to reintroduce:

| Item | State |
|------|-------|
| `unique_id` preservation across PcbLib primitives | modelled (8 primitives) and read |
| SchLib `*_FRAC` sub-coordinates | shipped, golden-covered (`FRACPINS`, `FRACSHAPES`) |
| SchLib Pin auxiliary streams (`PinFrac`, `PinSymbolLineWidth`) | shipped, golden-covered |
| PcbLib net / polygon / component indices | shipped; net index is [structurally absent from a library](FIXTURE_COVERAGE.md) |
| `IsNotAccessible` round-trip (SchLib) | shipped |

Region and ComponentBody already carry an `additional_parameters` passthrough: every key
the typed model does not consume is captured in read order and re-emitted verbatim, so an
unmodelled key survives a read-modify-write. Covered end to end by
`write_pcblib_additional_parameters_roundtrip`.

## Ordering

Rough value order, highest first:

1. **`DrillLayerPairType`, `DrillType`** — classification metadata for blind/buried and
   press-fit.
2. **Via `SolderMaskExpansionFromHoleEdge`** — the Pad half shipped; this is the
   remaining byte.
3. **Barcode sizing block** — ten keys, one primitive, rarely used.
4. **`model_2d_location`, `JumperID`, per-layer stack arrays** — small and isolated.
