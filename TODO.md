# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |
| Test-coverage climb + 99% gate | issue #302 (status comments, kept current in place) |

## A. Format / fidelity residue

- [ ] **Footprint-model record fidelity (`SchLib` RECORD=45)** — three defects in
      `encode_footprint_model`: `IsCurrent` is written as `index == 0` instead of the read
      `model.is_current` (a symbol whose current footprint is the second model gets flipped);
      `DatafileCount=1` is hardcoded (multi-datafile models unsupported); `UniqueID` is
      regenerated every save instead of round-tripped.
- [ ] **Audit `VOLATILE_KEYS` in `tests/golden_fidelity.rs`** — every entry excuses a value
      from comparison; several may be identities Altium actually keeps stable across saves
      (component `UniqueID`, `ITEMGUID`/`REVISIONGUID`) that we regenerate instead of
      preserving. Settle each with a double-resave observation, then preserve the stable ones.
- [ ] **Pad binary-block fidelity** — the golden fidelity diff compares parameter text, not
      the pad's binary geometry blocks. Byte-diff the golden's pads against a rewrite to
      settle the two old RE findings: oblong/oval SMD pads routing to the 651 size/shape
      block, and the multi-entry full-stack tail (count > 1).
- [ ] **`IDENTIFIER` codepoint list (`ComponentBody`)** — written empty; a non-empty value
      (comma-separated codepoints) has no fixture. UI-only; candidate for the next
      `manual/` authoring session.

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
