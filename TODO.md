# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment | `docs/FIXTURE_COVERAGE.md` |
| Format parity + verified negatives | `docs/COVERAGE_AUDIT.md` § Outstanding |

## A. On-site Altium tooling

- [ ] *(Optional)* extend `Verify-Libraries.ps1` to assert primitive counts / specific
      properties, not just "opened".

## B. Release & distribution

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
