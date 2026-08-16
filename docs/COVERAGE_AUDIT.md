# MCP Feature-Coverage Audit

<!-- markdownlint-disable MD013 -->

> What this server reads, writes and exposes, versus what an Altium library can actually
> carry. The goal is parity: nothing an Altium `.PcbLib` / `.SchLib` can store should be
> lost on read or unreachable on write.
>
> A worklist, not a history. Prune an entry the moment it ships — git log is the record.

## Outstanding

Nothing in the format layer. No field an Altium library can store is currently known to be
lost on read or unreachable on write.

The SchLib Parameter display properties are settled: `NotAutoPosition` and
`Justification` are covered by a hand-authored fixture
(`scripts/samples/manual/parameters.SchLib` — see that folder's README to rebuild it).
`IsRule`, `IsSystemParameter` and `TextHorzAnchor`/`TextVertAnchor` are listed among the
negatives below.

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
| SchLib `IsConfigurable` | read-only — the identifier table has `GetState_` but no `SetState_` |
| SchLib `TextHorzAnchor` / `TextVertAnchor` | absent from every parameter record in an authored library |
| SchLib `IsNotAccesible` = false (Altium's spelling) | every graphic record in a library carries `=T`; no library case omits it |
| SchLib arc fill (`IsSolid` / `AreaColor`) | `Arc.IsSolid` does not compile — an `ISch_Arc` is a stroked shape with no fill |
| SchLib pie `Transparent` | `Pie.Transparent` does not compile — real on rectangle/round-rect/ellipse/polygon, absent from `ISch_Pie` |

### Deciding whether a property is settable at all

Two separate questions, and answering only the first is what makes a run fail.

**Does the name exist?** The DelphiScript engine's identifier table lives in
**`ScriptingSystem.dll`** as **UTF-16LE** strings — not in `Advpcb.dll` / `AdvSch.dll`, and
not in the `Altium.*.dll` .NET assemblies. A `SetState_<Name>` entry means settable, a
`GetState_` without one means read-only, and absence means the name does not exist at all
(`TextJustification`). Enum literals are in the same table: `eJustify_Center` is there,
`eJustify_CenterCenter` is not. A missing `Set*` *method* proves nothing —
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
