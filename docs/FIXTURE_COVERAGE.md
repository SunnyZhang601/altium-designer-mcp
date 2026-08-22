# Golden-fixture coverage map

Goal: the committed Altium-authored fixtures (`scripts/samples/symbols.SchLib`,
`scripts/samples/footprints.PcbLib`) should exercise **every** property each primitive can
carry, so the library-reading tests (`tests/samples_schlib.rs`, `tests/samples_pcblib.rs`)
reach 100% *real* read coverage.

## Why this matters — the circularity problem

Our test pyramid has three tiers, and only one of them is true ground truth for a
**populated** (non-default) field:

| Tier | Proves | Blind spot |
|------|--------|------------|
| Readability **oracle** (`test_altium_readability.py`) | our *default* output opens in pyaltiumlib | only exercises from-scratch defaults; says nothing about non-default field values |
| **Self-round-trip** (write→read→assert) | our writer and our reader agree | **circular** — a field read wrong *and* written wrong the same way still passes |
| **Golden fixtures** (Altium-authored) | we read a real Altium file correctly | only as good as the values the fixture actually contains |

Every "self-round-trip only" caveat traces to the golden fixtures not yet carrying the
field. The fix is always to enrich them (see the workflow below).

## How the fixtures are produced (fully automated, on this PC)

- `scripts/Generate-Samples.ps1` — launches Altium headless (RunScript CLI), runs the
  DelphiScript, copies the authored libraries into `scripts/samples/`.
- `scripts/Watch-AltiumDialog.ps1` — run alongside the generator. A compile error or
  a native crash opens a modal dialog that, headless, would just sit there until the
  7-minute timeout; this catches it, prints the offending identifier and kills
  Altium. It is what makes batching several unproven names into one run safe.
- `scripts/altium/generate/GenerateSamples.pas` — the **authoring logic** (editable here;
  DelphiScript). Header declares it *iterative by design*: generate → read back → add the next
  feature → regenerate, until coverage is complete.
- **Standing workflow:** when a read test needs a feature the goldens don't carry, extend the
  `.pas`, run `python scripts/altium/generate/preflight_names.py` and reduce it to **one**
  unproven interface (one bad identifier aborts the whole script compile), kill any stale
  `X2` process, regenerate locally, commit the binaries, then write **exact** (non-guarded)
  assertions against the authored values. No tolerant or skipping tests.
- **Documented negatives:** when Altium does not persist an authored property, record the
  negative in the `.pas` next to the helper so it is not retried blindly, and mark the row
  below 🚫. The evidence for each lives once, in
  [COVERAGE_AUDIT.md](COVERAGE_AUDIT.md#verified-negatives--do-not-retry).

## Coverage map

Legend: ✅ authored + asserted · ❌ not exercised (self-round-trip only) · 🚫 documented
negative — the evidence for each lives once, in [COVERAGE_AUDIT.md](COVERAGE_AUDIT.md#verified-negatives--do-not-retry) · 🔒 structurally absent from a
library (see below — no fixture is possible, and none is needed).

### Structurally absent: the net index

`net_index` (common header @3) indexes a **board's** net list. A `PcbLib` has no net
table — the golden's 24 top-level OLE entries are `FileHeader`, `FileVersionInfo`,
`Library` and the footprint storages, with nothing for the index to point at — so every
primitive in every library reads the `0xFFFF` "no net" sentinel. All 33 pads, tracks and
vias in the golden do.

No golden can therefore exercise a non-sentinel value, and none needs to: the reader does
not branch on it beyond the short-header fallback, which a well-formed file never takes.
Both paths are covered by `common_indices_decode_values_sentinels_and_short_headers` (real
values, the sentinel, and every truncation point) and by `binary_roundtrip_common_indices`
for the board-context encode/decode. The rows below are marked 🔒 rather than ❌ so the
distinction stays visible: this is not an authoring gap waiting on an Altium run.

### PcbLib (`footprints.PcbLib`)

| Primitive | Exercised today | Not exercised (❌) |
|-----------|-----------------|--------------------|
| Pad | shape (round/rect/oct/rrect), TH holes (round/square/slot), local stack, rotation, negative/far coords, ✅ rotated unplated slot (`PRIMPROPS` S1: a 20×40 mil slot at 30°, `Plated=False`), ✅ manual paste/solder-mask expansion (`PADMASK`, authored via the pad cache); ✅ mask expansion from the hole edge (`PADMASK` pad 3, main-block bool @125 — offset derived by byte-diffing against pad 1), ✅ locked + keepout flags, ✅ drill tolerances (`LOCKFLAGS_PCB`); ✅ jumper group (`LOCKFLAGS_PCB` pads 7-8 share id 4, `samples_pcblib_jumper_group`); ✅ fabrication test points top/bottom (`LOCKFLAGS_PCB` pads 5-6, `samples_pcblib_testpoint_flags` — Altium also locks a test-point pad, so both read LOCKED) | 🚫 assembly test points; 🚫 DrillType; 🚫 fabrication flags; 🚫 corner-radius `CRPercentage` (crashes on a fresh Simple pad — needs correct pad-stack init first); 🚫 **FINAL** thermal-relief / power-plane setters (`PowerPlaneConnectStyle` / `ReliefConductorWidth` / `ReliefEntries` / `ReliefAirGap` / `PowerPlaneClearance` crash AD24's ScriptingSystem.DLL with a native access violation in **every** scripted sequence tried — pre- and post-registration, with and without the `GetState_Cache` block; batch 4a + 4b bisects. `PAD_THERMAL` cannot be authored by script in AD24 and stays disabled in the `.pas`) |
| Via | simple TH, two pad/hole sizes; ✅ mask-expansion cache state + 4 mil template expansion (`samples_pcblib_via_mask_state_is_altium_factory_default` — an Altium via carries byte @66 = 0, `eCacheInvalid`); ✅ manual solder + paste mask expansion (`PRIMPROPS`, set through `TPadCache`) | thermal-relief, power-plane, GUID; 🔒 mask-from-hole-edge (@258) + drill-pair type (@312) — modelled and round-tripped, but AD24 exposes no setter for either, so no script can author a non-default value; 🔒 net index; 🚫 tenting flags |
| Track | silk box + copper track, two widths, two layers; ✅ multi-layer spread (`MULTILAYER`: six tracks on Mechanical 2 / Mid-Layer 5 / Drill Guide / Drill Drawing / Internal Plane 1 / Keep-Out — real golden coverage for `layer_from_id`'s exotic arms; ID 58 reads as the documented `TopAssembly` alias, `samples_pcblib_multilayer`); ✅ locked + keepout (`LOCKFLAGS_PCB`) | 🔒 net index |
| Arc | full circle + quarter arc; ✅ locked + keepout (`LOCKFLAGS_PCB`) | fill/area colour; 🔒 net index |
| Region | copper box + mechanical box; ✅ board-cutout representation (`ISBOARDCUTOUT=TRUE` + `KEEPOUT=TRUE`, relocated to the keep-out layer — `samples_pcblib_region_cutout`); ✅ every `TRegionKind` (`PRIMPROPS`: KIND=2 NamedRegion, KIND=4 Cavity, KIND=1 Cutout — a board cutout has no KIND of its own); ✅ name + union index | net, cavity/subpoly params; 🚫 arc resolution (not on `IPCB_Region`) |
| Fill | axis-aligned + 45°-rotated copper; ✅ locked + keepout (`LOCKFLAGS_PCB`) | 🔒 net index |
| Text | stroke text, Win-1252 chars, vertical (90°); ✅ TrueType `font_name`='Arial' + bold + italic + mirror (`TEXT_STYLE`); ✅ kind=BarCode (`TEXT_SPECIAL` 'BC128'); ✅ inverted (knockout) text + inverted-rect descriptor (`TEXT_SPECIAL` 'INV': `is_inverted`, `use_inverted_rectangle`, `inverted_border`=10 mil, auto-computed rect width/height exact-asserted); ✅ barcode sizing block (`TEXT_SPECIAL` 'BC2': full width/height, X/Y margins, symbology and UTF-16LE font name); ✅ barcode inverted + show-text (`BC3`/`BC4`, each differing from `BC2` in exactly one field, pinning @159 and @225); ✅ stroke-font variants + 12 mil stroke width (`PRIMPROPS`: FontID 2 Sans Serif, 3 Serif — the reader only surfaces a font above id 1) ; ✅ stroke-font variants + non-default stroke width (`PRIMPROPS`: FontID 2 Sans Serif and 3 Serif, 12 mil width — the reader only surfaces a stroke font above id 1) | 🚫 justification (`TextJustification` is not on `IPCB_Text`); 🚫 barcode MinWidth and RenderMode |
| ComponentBody | one extruded box (Mechanical); ✅ embedded STEP model (`EMBSTEP`: `MODELID`/`MODEL.CHECKSUM`/`MODEL.NAME` on the body + zlib model stream in `/Library/Models/0`, decompressed `ISO-10303-21` bytes exact-asserted — `samples_pcblib_embstep`); ✅ standoff + cavity height + 3D colour + opacity (`PRIMPROPS` — `cavity_height` was not modelled at all until this fixture exposed it) | model 2D location/rotation, raw-outline precision |

### SchLib (`symbols.SchLib`)

| Primitive | Exercised today | Not exercised (❌) |
|-----------|-----------------|--------------------|
| Pin | electrical types (all 8), orientations (0/90/180/270), name/designator visibility, edge decorations, dual-part `owner_part_id`; ✅ PinFrac off-grid coords (`FRACPINS`), ✅ PinSymbolLineWidth (`Symbol_LineWidth=eLarge`); ✅ swap-id tail (`SWAPPIN`: `SwapId_Pin`→`swap_id_group`='A', `SwapId_Part`→`part_and_sequence`='1', `DefaultValue`→`default_value`='3V3') | owner_part_display_mode (non-default), graphically_locked |
| Line | plain segments; ✅ line_style dashed + dotted (`SHAPESTYLE`); ✅ non-default colour (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | 🔒 is_not_accessible=false |
| Arc | plain arcs; ✅ `_Frac` coords (`FRACSHAPES`: centre (0.05, 0.05), radius 4.05 — AD24 omits the zero integer keys and stores frac-only); ✅ non-default colour + non-zero `StartAngle` (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | 🚫 fill/area colour (an `ISch_Arc` has no fill — `Arc.IsSolid` does not compile); 🔒 is_not_accessible=false |
| Rectangle | plain rects; ✅ transparent (`SHAPESTYLE`), ✅ GraphicallyLocked (`LOCKFLAGS`); ✅ `_Frac` coords incl. negatives (`FRACSHAPES`: (-5.45, -2.45)–(5.55, 2.55)); ✅ non-default border + fill colour (`SHAPECOLOR`) | line_style; 🚫 Disabled/Dimmed (authored but not persisted by AD24) |
| RoundRect | plain rounded rects; ✅ non-default border colour (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | 🚫 line_style and 🚫 transparent — both accepted by AD24 and neither written to a library round-rect |
| Ellipse | plain ellipses; ✅ transparent (batch 3); ✅ non-default border colour (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | — |
| Polyline | plain polylines; ✅ non-default colour (`SHAPECOLOR`); ✅ line_style + start/end shapes + shape size (`SHAPESTYLE2`: dashed, arrow → solid arrow, eLarge — all four persist); ✅ display flags (`LOCKFLAGS2`) | 🚫 fill/transparency (AreaColor/IsSolid/Transparent compile on `ISch_Polyline` and none is written) |
| Polygon | plain polygons; ✅ transparent (`SHAPESTYLE` triangle); ✅ non-default border colour (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | 🔒 is_not_accessible=false (line_style: N/A — `ISch_Polygon` has no LineStyle in AD24); 🔒 is_not_accessible=false |
| Pie | ✅ authored (`PIESYM`: 30–210°, radius 5 units, yellow fill, exact-asserted); ✅ non-default border + fill colour (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | `_Frac` coords; 🚫 transparent (`ISch_Pie` has none — `Pie.Transparent` does not compile) |
| Image | ✅ authored (`IMAGESYM`: bounding box, `logo.bmp`, KeepAspect, non-embedded); ✅ embedded image bytes in the `/Storage` stream (`EMBIMGSYM`, exact-asserted against the committed `embed.bmp`); ✅ display flags (`LOCKFLAGS2`) | 🚫 show_border (not on `ISch_Image` — does not compile) |
| Bezier | ✅ authored (`BEZIERSYM`, four control points exact-asserted); ✅ non-default colour + eMedium width (`SHAPECOLOR`); ✅ display flags (`LOCKFLAGS2`) | — |
| Label | plain labels; ✅ justification variants + rotation (`JUSTIFY`); ✅ non-default colour (`SHAPECOLOR`); ✅ mirrored (`SHAPESTYLE2`); ✅ display flags (`LOCKFLAGS2`) | — |
| Parameter | Value etc.; ✅ justification + orientation (`JUSTIFY`: `Justification=8` on Value, `Justification=4` + `Orientation=1` on the hidden Tol); ✅ autoposition + justification from the hand-authored `manual/parameters.SchLib`; ✅ show_name + read_only_state + is_mirrored + param_type (`SHAPESTYLE2` — `is_mirrored` was not modelled at all until this fixture exposed it) | 🚫 is_rule / is_system_parameter / is_configurable / text anchors — read-only or never written into a library |

### Cross-cutting (both formats)

- **Universal display/lock flags** — `GraphicallyLocked` is golden-covered on Rectangle
  (`LOCKFLAGS`); `Disabled`/`Dimmed` are 🚫 documented AD24 negatives (not persisted on
  library shapes); `OwnerPartDisplayMode` at a non-default value is now ✅ golden-covered
  (`DISPMODE`: a `DisplayModeCount=2` symbol whose mode-1 rectangle carries
  `OwnerPartDisplayMode=1` — `samples_schlib_dispmode`; the pin-record byte remains
  self-round-trip only).
- **`unique_id`** — present in fixtures, so identity read is covered; but per-primitive GUID
  streams for populated cases are thin.
- **Fractional coordinates** — the Pin `_Frac` path is golden-covered via the `PinFrac` aux
  stream (`FRACPINS`); the text-record `*_Frac` key path on graphic shapes is now ✅
  golden-covered too (`FRACSHAPES`, batch 4a). The golden exposed a real convention:
  AD24 stores negative off-grid coordinates as **truncation toward zero with a SIGNED
  `_Frac`** (`Location.X=-5|Location.X_Frac=-45000` = −5.45) and **omits a zero integer
  key** when only the fraction is non-zero (`Location.X_Frac=5000` with no `Location.X`).
  The reader previously parsed `_Frac` as unsigned and silently truncated every negative
  off-grid coordinate; reader and writer now follow the signed toward-zero convention
  (see `docs/SCHLIB_FORMAT.md` § Fractional coordinates).

## Remaining enrichment backlog

Each batch: extend the `.pas` → run `preflight_names.py` → regenerate locally → commit
binaries → exact assertions.

An unproven `(interface, property)` pair is only a *compile* risk, and a failure names the
identifier in a modal dialog, so several may go in one run provided that dialog is read
rather than waited out — otherwise keep it to one unproven interface, or a timeout will
not say which name was at fault.

**SchLib, not yet attempted:** `*_Frac` coordinates on the shapes that still lack
them. (Parameter "area colour" appeared here in error — `parse_parameter` reads no
such key, and no authored parameter record carries one.) **Record kinds the golden
does not contain at all** — the tool-layer replay tests can only hold these to the
structs, not to Altium: an elliptical arc (RECORD=11; also the reason `EllipticalArc`
carries no display flags in the model — nothing to verify them against), a text
annotation (RECORD=3; same for `Text`), and a footprint model link (RECORD=45, with
its `ImplementationList`/`MapDefiner` children) — the `FootprintModel` replay of
`unique_id`/`is_current`/`raw_params` is unit-tested only, and its UI-authored form
(`IntegratedModel=T|DatabaseModel=T`, no empty `Description`) is known from a hand-authored
library, not a golden.

**PcbLib:** region net and the cavity/subpoly params, raw-outline precision for ComponentBody.
A **primitive of every kind on Mechanical 17-32** (`eMechanical20` or
`LayerUtils.MechanicalLayer(20)` if the AD24 API names it): the header byte 72 + V7 id
`0x01020014` pair is settled by hand-authored tracks, but the same pair on a pad, arc,
text, fill, region and body — and the `MECHANICAL20` token on the last two — is inferred
from the track, not seen. A **via block longer than the 321-byte template** (an older
Altium's 351-byte vias are known only from a hand-authored library) so the golden pins
that the extra bytes go back verbatim.
A **non-embedded STEP reference** in the *generated* golden — the form itself is settled
by a UI-authored library (`MODELID=` empty, `MODEL.EMBED=FALSE`, `MODEL.NAME`, the full
group; `/Library/ModelsNoEmbed` stays empty) and `write_pcblib` follows it. A **text beyond U+00FF** (Ω, CJK) so a golden pins `WideStrings` as UTF-16
code units (today only AltiumSharp and the Latin-1 `10µF` of `TEXT_WIN1252` do).
Pad thermal-relief / power-plane is
🚫 **FINAL** on the scripting side (native crash on a fresh library pad in every sequence
tried — see the Pad row); a golden would need a non-scripted authoring route.
