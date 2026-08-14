# Releasing

How a tagged release of `altium-designer-mcp` is cut, and the order the safety
nets are meant to catch mistakes in.

A release is effectively permanent. The tag can be force-moved in principle, but
by then people have pinned it, mirrors have copied it and the download links are
in someone's notes; and a published release is public the instant it exists. The
pipeline is therefore built so that **every irreversible step happens last, and
only after a human has looked at the artefacts**.

## What is automated

`.github/workflows/release.yml` runs in three jobs:

| Job | What it does | Can it publish? |
|-----|--------------|-----------------|
| `validate` | Tag format, `Cargo.toml` version match, tagged commit is on `main`, CHANGELOG entry exists, no release already exists | no |
| `build` | Builds and tests on Linux / macOS / Windows, packages each archive, **unpacks it again and runs the packaged binary** | no |
| `release` | Verifies all three artefacts arrived, generates and re-checks `SHA256SUMS.txt`, attests SLSA build provenance, creates a **draft** release | draft only |

The final publish is a manual click. Nothing in CI makes a release public.

## Dry run — do this first

The whole pipeline can be exercised without a tag:

```bash
gh workflow run release.yml --ref main
gh run watch
```

This builds, packages, smoke-tests and checksums all three platforms, then stops
before the `release` job. Artefacts are kept for 7 days on the workflow run page,
so you can download the real archives and try them on a real machine.

Do this **before** stamping the changelog or tagging. It is the only way to find a
packaging problem that costs nothing to fix.

Two notes on dry runs:

- The version used is whatever `Cargo.toml` currently declares.
- A missing CHANGELOG section is a warning here, not a failure, because dry runs
  normally happen before the heading is stamped. It is a hard failure for a real
  tag.

## Cutting the release

1. **Confirm main is releasable.** Green CI on the head commit, no open PR you
   meant to include, coverage where you want it.

   ```bash
   gh run list --workflow ci_main.yml --limit 1
   ```

2. **Stamp the CHANGELOG.** Replace the `## [Unreleased]` heading with
   `## [X.Y.Z] - YYYY-MM-DD`. The release notes published on GitHub are exactly
   the text between that heading and the next `## [`, so read it once as a
   stranger would. Add a fresh empty `## [Unreleased]` above it for the next
   cycle.

3. **Set the version in `Cargo.toml`** if it is not already `X.Y.Z`. The tag and
   the `[package]` version must agree or `validate` fails.

4. **Merge those to main** through a PR, and let CI go green.

5. **Dry-run once more** on that exact commit (see above), now that the changelog
   heading exists. This is the last free rehearsal.

6. **Tag and push.**

   ```bash
   git switch main && git pull
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin v0.1.0
   ```

7. **Watch the workflow.**

   ```bash
   gh run watch
   ```

8. **Review the draft.** Download the three archives from the draft release page,
   check `SHA256SUMS.txt`, and run at least one binary on a real machine:

   ```bash
   gh release view v0.1.0
   sha256sum -c SHA256SUMS.txt
   ```

9. **Publish.**

   ```bash
   gh release edit v0.1.0 --draft=false
   ```

10. **Announce** — including a note on the tracking issue if one is open.

## Supply chain

Each archive — and `SHA256SUMS.txt` itself — gets a signed
[SLSA build provenance](https://slsa.dev/) attestation, binding the artefact's
digest to this repository, the workflow that built it and the commit it was built
from. The attestation lives in GitHub's attestation store, not in the release, so
it cannot be swapped out by editing release assets.

Verify any published artefact (this works for anyone, not just maintainers):

```bash
gh attestation verify altium-designer-mcp-linux-x86_64.tar.gz \
    --repo embedded-society/altium-designer-mcp
```

The release body carries these instructions automatically — the workflow appends
them to the changelog-derived notes, so there is nothing to remember at tag time.

Worth checking once on the draft before publishing: download one archive and run
the verify command against it. If provenance is broken, it is better found on a
draft than after the release is public.

## If something is wrong

- **Before publishing** — delete the draft, fix, and re-tag. Nothing was public.

  ```bash
  gh release delete v0.1.0 --yes
  git push --delete origin v0.1.0
  git tag -d v0.1.0
  ```

- **After publishing** — do not delete or move the tag. Ship `v0.1.1`. A version
  someone already downloaded should keep meaning what it meant.

## Known gaps

- **`Cargo.lock` is gitignored**, so release builds resolve dependencies fresh
  and the three platform binaries are not guaranteed to be built against
  identical dependency versions. Committing the lockfile and building with
  `--locked` is the usual practice for a distributed binary; worth deciding
  before the first release rather than after.
- **No code signing.** Windows SmartScreen and macOS Gatekeeper will warn on
  first run; macOS users need right-click → Open. The generated release notes say
  so, and point at the provenance attestation as the stronger check. Proper
  signing needs a paid Apple Developer account and a Windows certificate, so it
  is a cost decision rather than a technical one.
