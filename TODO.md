# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |

## A. Findings deferred from the bug sweep

Found while fixing something else; each needs its own verification or a fixture first.

- [ ] **`Text` (RECORD=3) is almost certainly the IEEE symbol record, not a text**
      annotation: Altium's record table has 3 = IEEE Symbol (4 = the text string this
      crate calls `Label`), and the format RE archive flagged the same. Our `Text` type
      writes RECORD=3 with label keys, which Altium would read as a symbol with missing
      keys. No fixture can settle it — AD24's scripting API cannot place an IEEE symbol
      (`TIeeeSymbol` exists only as a pin decoration), so it needs a hand-authored
      library. Until then: model RECORD=3 as an IEEE symbol carried verbatim, and route
      text authoring to RECORD=4.

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
