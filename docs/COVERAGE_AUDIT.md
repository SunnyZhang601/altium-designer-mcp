# MCP Feature-Coverage Audit

<!-- markdownlint-disable MD013 -->

> What this server reads, writes and exposes, versus what an Altium library can actually
> carry. The goal is parity: nothing an Altium `.PcbLib` / `.SchLib` can store should be
> lost on read or unreachable on write.
>
> A worklist, not a history. Prune an entry the moment it ships — git log is the record.

## Outstanding

Read-modify-write fidelity is enforced by `tests/golden_fidelity.rs`, which reads each
golden, writes it back and diffs the OLE streams and every parameter block. Anything it
still loses is listed in that test's `KNOWN_DEFECTS` and described here; the two lists are
the same list, so an entry leaves both together.

**Five i18n fixture symbols are internally inconsistent** (`_JV`, `_BN`, `_CR`, `_IU`,
`_SB`) — and every scripted route is a **verified negative** (four runs, 2026-08-16). The
root cause is **AD's reader**: it cannot losslessly decode these five byte sequences even
from its own file. Run 1, source literals: storage correct, records shifted. Run 2, wide
`Chr($A997)`: truncates to the low byte — the engine's strings are ANSI. Run 3, UTF-8 byte
`Chr($EA)+…`: storage and `SectionKeys` byte-perfect, text records double-widened. Run 4,
open+resave through AD itself: a fourth variant, *worse* than the input (replacement
characters), proving the decode itself is the broken part — the script engine feeds
literals through the same path. The cure is typing the five names once in the AD UI, which
bypasses the decode entirely (UI input → real wide string; the writer is faithful, as the
48 working symbols prove); the repo never re-opens goldens in AD afterwards. Until then
`tests/golden_fidelity.rs` excuses exactly these five, by suffix (`FIXTURE_INCONSISTENT`).

**Identity streams are keyed by ordinal, not attached to the primitive (`PcbLib`).** A
footprint's `PrimitiveGuids` records and its unique ids both name a primitive by its
position among all the footprint's primitives. That position is preserved across a
read-modify-write, so both survive one — but a *structural* edit (deleting a pad, inserting
a region) renumbers everything after it and silently re-points every later identity.
Attaching the GUID to the primitive it names would fix it, and touches eight primitive
structs.

## How to re-verify before trusting this

An earlier edition went badly stale — 103 of its 129 entries named fields that had since
been implemented — so treat "nothing outstanding" as a claim to re-check, not a fact:

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
(`IsKeepout` and `IsTentingTop` live in `PcbFlags`, not in fields of their own) — so
confirm each survivor by reading the parser. Three cross-checks:

- **modelled?** a field on the primitive struct;
- **parsed and written?** `src/altium/{pcblib,schlib}/`;
- **reachable?** an entry in `src/mcp/tool_definitions.rs`.

All three to count as supported; a golden fixture to count as *proven*.

To locate an unknown byte offset, author two primitives differing in exactly one field and
diff their record blocks — the offset carrying the authored value is the field. That found
the pad's mask-from-hole-edge flag at `@125` and all six barcode sizing offsets.

## Verified negatives — do not retry

Properties AD24 accepts without error but does not persist in a library, each confirmed by
authoring it and reading the saved bytes back.

| Property | Evidence |
|----------|----------|
| Via tenting (`IsTenting_Top`/`_Bottom`) | authors fine; saved flag word is empty |
| Assembly test points (`IsAssyTestPoint_Top`/`_Bottom`) | flag word comes back a plain `0x000C` |
| `TearDrop`, `UserRouted` | same `0x000C`, identical to an untouched pad |
| `DrillType` | saved pad is byte-identical to a plain TH pad apart from coordinates |
| `IsBackDrill`, `IsCounterHole`, `IsPreRoute` | derived board state; no per-pad property |
| Barcode `MinWidth` | `@153` reads 39604/88235 against an authored 5 mil — Altium computes it |
| Barcode `RenderMode` | moves no byte; `@115` is a creation-order ordinal, not the property |
| PCB text justification | `TextJustification` does not exist on `IPCB_Text` |
| Net index (any primitive) | a `PcbLib` has no net table, so it is always `0xFFFF` |
| SchLib `IsRule` | AD24 marks a rule by `Name=Rule` plus a `RULEKIND=…` payload in `Text`, not by a flag |
| SchLib `IsSystemParameter` | absent even on `Comment`; not written into a library |
| SchLib `IsConfigurable` | absent from every parameter record in an authored library |
| SchLib `TextHorzAnchor` / `TextVertAnchor` | absent from every parameter record in an authored library |
| SchLib `IsNotAccesible` = false (Altium's spelling) | every graphic record in a library carries `=T`; no library case omits it |
| SchLib arc fill (`IsSolid` / `AreaColor`) | `Arc.IsSolid` does not compile — an `ISch_Arc` is a stroked shape with no fill |
| SchLib pie `Transparent` | `Pie.Transparent` does not compile — real on rectangle/round-rect/ellipse/polygon, absent from `ISch_Pie` |
| SchLib round-rect `LineStyle` | accepted without error, but the saved `RECORD=10` carries no `LineStyle` key — `ISch_Line` and `ISch_Polyline` both persist it |
| SchLib text-frame `Orientation` | not on `ISch_TextFrame` — `Frm.Orientation` does not compile, though it is real on label/parameter/pin |
| SchLib image `ShowBorder` | not on `ISch_Image` — `Img.ShowBorder` does not compile, though it is real on `ISch_TextFrame` |
| SchLib polyline fill (`AreaColor`/`IsSolid`/`Transparent`) | all three compile on `ISch_Polyline` and none is written; the rectangle and polygon records do persist theirs |
| SchLib parameter area colour | there is no such property: `parse_parameter` reads no colour-fill key and no authored parameter record carries one |
| SchLib text-frame `Transparent` | accepted, then not written — the saved `RECORD=28` has no `Transparent` key |
| PcbLib region `ArcResolution` | not on `IPCB_Region` — `Rgn.ArcResolution` does not compile, though the name is real elsewhere in the identifier table |
| PcbLib via mask expansion via DIRECT setters | `Via.SolderMaskExpansion*` / `.PasteMaskExpansion*` compile, then crash AD24 with a native access violation in `ScriptingSystem.DLL`. Set them through `TPadCache` (`GetState_Cache` → `SetState_Cache`) instead, which works |

### Deciding whether a property is settable at all

Two separate questions, and answering only the first is what makes a run fail.

**Does the name exist?** The DelphiScript engine's identifier table lives in
**`ScriptingSystem.dll`** as **UTF-16LE** strings — not in `Advpcb.dll` / `AdvSch.dll`, and
not in the `Altium.*.dll` .NET assemblies. The test is presence, and only presence:
absence means the name does not resolve at all (`TextJustification`, `eJustify_CenterCenter`),
while a hit means it is worth trying. Enum literals are in the same table, so check those
too — `eJustify_Center` is there, `eJustify_CenterCenter` is not.

Do **not** read anything more into the `SetState_` / `GetState_` prefixes. They are absent
for plenty of settable properties: `StandoffHeight`, `BodyColor3D` and `BodyOpacity3D` have
neither and all three set and persist. A missing `Set*` method proves nothing either —
`SolderMaskExpansionFromHoleEdge` and `BarCodeKind` both lack one and both set fine.

```python
import re
b = open(r'C:\Program Files\Altium\AD24\System\ScriptingSystem.dll', 'rb').read()
ids = set()
for m in re.finditer(rb'(?:[\x20-\x7e]\x00){3,}', b):
    ids |= set(re.findall(r'[A-Za-z_][A-Za-z0-9_]{2,}', m.group().decode('utf-16-le')))
'SetState_CavityHeight' in ids     # True -> settable
```

**Does this interface carry it?** The table is global, so a hit only says the name is real
*somewhere*. Resolution is per-interface and happens at compile time: `IsSolid` is in the
table and genuine on a rectangle, yet `Arc.IsSolid` still fails with `Undeclared
identifier: IsSolid`. Only a run settles this, so `scripts/altium/generate/preflight_names.py`
lists every `(interface, property)` pair with no precedent in the committed `.pas` — keep
it to **one unproven interface per run**, or a failure will not say which name was at
fault. An unresolved identifier aborts the whole compile and takes every other footprint
in that run with it; `try/except` cannot help, because nothing has run yet.

A third failure mode is worse than either: some properties compile and then take AD24 down
with a native access violation. Both known cases — pad thermal relief and via mask
expansion — sit on the pad cache, and the via one is avoidable by going through
`TPadCache` rather than the direct setters. Watch the run for the crash dialog instead of
waiting out the timeout, or a single bad line costs seven minutes and a hung Altium.
