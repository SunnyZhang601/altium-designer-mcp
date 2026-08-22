# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |

## A. Findings deferred from the bug sweep

Found while fixing something else; each needs its own verification or a fixture first.

- [ ] **SchLib header `UniqueID` (RECORD=1) is dropped on write** — Altium stores one per
      component (`|RECORD=1|...|UniqueID=PMHDDPDX`); `Symbol` has no field for it, so the
      writer omits it and Altium presumably re-generates it. Excused today via
      `VOLATILE_KEYS` in `tests/golden_fidelity.rs`. Carry it like `designator_unique_id`.
- [ ] **Shapes the golden stores without a `UniqueID` (pies) get a fresh random one per
      save** — `encode_pie` always emits `UniqueID=`, so two saves of the same library
      differ. Either Altium accepts an absent key (omit when `None`, matching the golden)
      or the fixture is an outlier; settle against Altium, then make saves deterministic.
- [ ] **`EllipticalArc` and `Text` (RECORD=11 / RECORD=3) carry no display flags**
      (`graphically_locked`, `disabled`, `dimmed`, `owner_part_display_mode`) in the model,
      while the other 13 graphics do — almost certainly a gap, but no golden record exists
      to verify the keys AD24 emits. Blocked on the fixture (see `docs/FIXTURE_COVERAGE.md`).

## B. On-site Altium tooling

- [ ] *(Optional)* extend `Verify-Libraries.ps1` to assert primitive counts / specific
      properties, not just "opened".

## C. Release & distribution

- [ ] **v1.0.0 is the real release**, gated on ALL features built (the 99% coverage
      gate — production metric, #381 — is already met). The climb happens calmly;
      [`docs/RELEASING.md`](docs/RELEASING.md) is the proven runbook.
- [ ] **Streamable HTTP transport** alongside stdio, so web-only assistants (claude.ai in
      the browser, ChatGPT) can connect as a remote server — today they cannot
      (`docs/CLIENT_SETUP.md` § Web-only assistants).
- [ ] **Claude Desktop extension** for one-click install, published as **both `.mcpb` and
      `.dxt`** — the same bundle format under its new and old names, so older Claude Desktop
      builds install it too. Claude Desktop now steers users to extensions over hand-edited
      JSON (pattern from coffeenmusic/altium-mcp).
