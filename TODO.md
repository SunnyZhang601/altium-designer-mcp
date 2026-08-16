# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |
| Test-coverage climb + 99% gate | issue #302 (status comments, kept current in place) |

## A. Format / fidelity residue

- [ ] **Pad binary-block fidelity** — the golden fidelity diff compares parameter text, not
      the pad's binary geometry blocks. Byte-diff the golden's pads against a rewrite to
      settle the two old RE findings: oblong/oval SMD pads routing to the 651 size/shape
      block, and the multi-entry full-stack tail (count > 1). **Last §A item.**

## B. Coverage (issue #302 owns the detail)

- [ ] Land #379 (tool layer to ~98%+ on the production metric).
- [ ] **Guard-then-read rework** (offered to ande2407): drop the redundant upfront length
      guards in the `PcbLib` parsers so each read's `ok_or_else` arm becomes live, reachable
      error handling — same behaviour, simpler, and the arms become testable.
- [ ] Close the remainder to the 99% gate (production metric, `main.rs` excluded — see #381).

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
