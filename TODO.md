# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |

## A. Findings deferred from the bug sweep

Found while fixing something else; each needs its own verification or a fixture first.

- [ ] **`EllipticalArc` and `Text` (RECORD=11 / RECORD=3) carry no display flags**
      (`graphically_locked`, `disabled`, `dimmed`, `owner_part_display_mode`) in the model,
      while the other 13 graphics do — almost certainly a gap, but no golden record exists
      to verify the keys AD24 emits. Blocked on the fixture (see `docs/FIXTURE_COVERAGE.md`).
- [ ] **Mechanical 17-32 on pads, arcs, text, fills, regions and bodies** is stored the way
      hand-authored *tracks* store it (byte 72 + V7 id `0x010200nn`, `MECHANICAL{nn}` token
      for regions/bodies) — inferred from the one kind seen, not verified per kind. Blocked on
      the fixture (`docs/FIXTURE_COVERAGE.md`, PcbLib backlog).

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
