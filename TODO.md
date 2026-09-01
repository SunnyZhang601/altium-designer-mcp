# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment and verified negatives | `scripts/samples/COVERAGE.md` |

## B. Release & distribution

- [ ] **v1.0.0 is the real release**, gated on ALL features built (the 99% production
      coverage gate is already met). The climb happens calmly;
      [`docs/RELEASING.md`](docs/RELEASING.md) is the proven runbook.
- [ ] **Streamable HTTP transport** alongside stdio, so web-only assistants (claude.ai in
      the browser, ChatGPT) can connect as a remote server — today they cannot
      (`docs/CLIENT_SETUP.md` § Web-only assistants).

## C. Quality & coverage

- [ ] **Test-coverage push** once the extension bundle ships: expand tests toward full
      coverage again (baseline 99.32%; the remainder is mostly defensive/unreachable
      branches, so each gain needs a deliberate fixture or restructure).

## D. Maintenance & waiting

- [ ] **Drop the `cfb` git pin** (`[patch.crates-io]`, rev `8c1ec76`) as soon as rust-cfb
      publishes a release newer than v0.14.0 — check
      [rust-cfb releases](https://github.com/mdsteele/rust-cfb/releases) at session start.
- [ ] **Dismiss the recurring GitHub "AI findings"** (maintainer-only, in the repo's
      Security → Quality UI): the same three findings keep regenerating autofix PRs
      (#415/#416, #469/#470 all closed as verified non-bugs) until they are dismissed
      there — the API cannot do it.
