# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |
| Test-coverage climb + 99% gate | issue #302 (status comments, kept current in place) |

## A. Format / fidelity residue

*(empty — §A completed 2026-08-16: every footprint Data stream in the golden is
byte-identical through a read-modify-write, enforced by `golden_fidelity`. The two
old RE findings dissolved: no 651 mis-routing exists, and the multi-entry
full-stack tail is unexercised rather than broken — a fixture would be needed to
claim more.)*

## B. Coverage (issue #302 owns the detail)

*(empty — §B completed 2026-08-17. #379 took the tool layer past 98%; the
guard-then-read rework landed as #383, so a truncated block is now refused by the
field that could not be read rather than by an upfront length constant; and #385
plus #387 cleared the 99% gate. Clearing it left only five lines of margin, so the
last three constant guards in `pcblib/reader/parsers.rs` were folded into their
own reads, taking that file to 100% — 99.07% overall on the production metric,
`main.rs` excluded, 390 uncovered lines of 41,892, about 28 clear of the gate.
The cheapest headroom left is `mcp/server.rs` (83 uncovered) and
`mcp/tools/library_ops.rs` (40). Note the total moves by a few lines run to run —
`mcp/server.rs` alone varied by one across consecutive runs — so a gate this
tight can flap without anyone changing a thing.)*

## C. On-site Altium tooling

- [ ] *(Optional)* extend `Verify-Libraries.ps1` to assert primitive counts / specific
      properties, not just "opened".

## D. Release & distribution (no release exists yet)

- [ ] Cut **v0.1.0 as a pre-release** once §A above is complete (decision 2026-08-16) —
      **maintainer triggers it personally**. Tag-day steps: stamp the changelog
      heading/date, `git tag v0.1.0`, push, watch the Release workflow, verify artefacts.
- [ ] **v1.0.0 is the real release**, gated on ALL features built and **99% test
      coverage** (production metric, #381). The climb between the two happens calmly —
      neither gate blocks the other's work.
- [ ] Consider a `.dxt` Claude Desktop extension for one-click install (pattern from
      coffeenmusic/altium-mcp).

## E. Docs / AI workflow

- [ ] Enrich `docs/AI_WORKFLOW.md` with symbol pin-placement guidance (idea from
      coffeenmusic's `symbol_placement_rules.txt`).
