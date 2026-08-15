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

One fixture gap remains, tracked in [FIXTURE_COVERAGE.md](FIXTURE_COVERAGE.md): the SchLib
Parameter display properties (`AUTOPOSITION`, `ISRULE`, `ISSYSTEMPARAMETER`,
`TEXTHORZANCHOR` / `TEXTVERTANCHOR`) are modelled and round-tripped, but AD24 does not
expose them on `ISch_Parameter`, so no script can author a golden for them. Moving them
from round-trip coverage to fixture coverage needs a hand-authored file.

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

> **Deciding whether a property is settable at all.** Check whether the name appears in
> **`Advpcb.dll`** (PCB) or `AdvSch.dll` (schematic) — the native Delphi engines. A missing
> `Set*` counterpart proves nothing: `SolderMaskExpansionFromHoleEdge` and `BarCodeKind`
> both lack one and both set fine. Names found only in the `Altium.*.dll` **.NET**
> assemblies do not resolve in DelphiScript — `TextJustification` is one — and an
> unresolved identifier aborts the entire script compile, taking every other footprint in
> that run with it.
