# TODO — Working Notebook

Outstanding work only — shipped items are deleted, not struck through (git log is the
record). The specialised worklists stay the single source of truth for their areas:

| Area | Worklist |
|------|----------|
| Golden-fixture enrichment and verified negatives | `scripts/samples/COVERAGE.md` |

## B. The climb to v1.0.0 (active)

- [ ] **Polish everything to perfection first** — a full pass over README, `docs/`, the
      bundled release README, tool descriptions and CHANGELOG for anything stale,
      pre-1.0-flavoured or unclear, before the version is bumped. Every feature the
      1.0 line promises is built; the remaining gate is polish.
- [ ] **Release dry run** (`workflow_dispatch` on `release.yml`) proving the full
      pipeline, including the new extension `bundle` job, before any tag —
      [`docs/RELEASING.md`](docs/RELEASING.md) is the proven runbook.
- [ ] **v1.0.0**: stamp `CHANGELOG.md` (with a short lead paragraph on what 1.0 means),
      bump the version in `Cargo.toml` / `Cargo.lock` / `package.json` / `package-lock.json`,
      refresh the supported-versions table and date in `SECURITY.md`, dry-run once more,
      tag, review the draft (install the `.mcpb` once), publish.

## After v1.0.0 (v1.1.0)

- [ ] **Streamable HTTP transport** alongside stdio, so web-only assistants (claude.ai in
      the browser, ChatGPT) can connect as a remote server — today they cannot
      (`docs/CLIENT_SETUP.md` § Web-only assistants). Deliberately after 1.0.

## C. Maintenance & waiting

- [ ] **Drop the `cfb` git pin** (`[patch.crates-io]`, rev `8c1ec76`) as soon as rust-cfb
      publishes a release newer than v0.14.0 — check
      [rust-cfb releases](https://github.com/mdsteele/rust-cfb/releases) at session start.
