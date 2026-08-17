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

*(empty — §B completed 2026-08-17: the 99% gate is met on the production metric
(#381) via #379/#383/#385/#387, with #388's guard folding taking
`pcblib/reader/parsers.rs` to 100% and leaving ~28 lines of margin. Two facts
worth keeping: the measured total flaps by a few lines between identical runs, so
read near-gate figures accordingly; and the cheapest remaining headroom sits in
`mcp/server.rs` and `mcp/tools/library_ops.rs`. Holding the line is CI's job now.)*

## C. On-site Altium tooling

- [ ] *(Optional)* extend `Verify-Libraries.ps1` to assert primitive counts / specific
      properties, not just "opened".

## D. Release & distribution (no release exists yet)

- [ ] Cut **v0.1.0 as a pre-release** — all gates met 2026-08-17 (§A and §B above are
      empty); **maintainer triggers it personally**, following
      [`docs/RELEASING.md`](docs/RELEASING.md) step by step. Abbreviated: dry-run the
      pipeline first (`gh workflow run release.yml --ref main` — it has never had a green
      run), stamp the changelog heading/date, tag **signed** (`git tag -s v0.1.0 -m "v0.1.0"`
      — the tag ruleset rejects unsigned tags), push, watch the workflow, then review the
      **draft** release it creates and publish with
      `gh release edit v0.1.0 --draft=false --prerelease` (the `--prerelease` flag is
      manual here: the workflow only sets it for suffixed tags like `v0.1.0-rc1`).
- [ ] **v1.0.0 is the real release**, gated on ALL features built and **99% test
      coverage** (production metric, #381). The climb between the two happens calmly —
      neither gate blocks the other's work.
- [ ] Consider a `.dxt` Claude Desktop extension for one-click install (pattern from
      coffeenmusic/altium-mcp).

## E. Docs / AI workflow

- [ ] Enrich `docs/AI_WORKFLOW.md` with symbol pin-placement guidance (idea from
      coffeenmusic's `symbol_placement_rules.txt`).
