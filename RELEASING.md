# Releasing ToraDB

Releases are driven by **git tags**. Pushing a `v*` tag runs
[`.github/workflows/release.yml`](.github/workflows/release.yml), which gates on the test
suite, builds Python wheels + an sdist + standalone `toradb-ingest` CLI binaries, creates a
GitHub Release with those assets, and publishes the wheels to PyPI.

## One-time setup: PyPI Trusted Publishing (OIDC)

The release workflow publishes with **no stored API token** using PyPI's trusted publishing.
Configure it once on PyPI (https://pypi.org/manage/account/publishing/):

- **PyPI project name:** `toradb`
- **Owner:** `sophatvathana`
- **Repository:** `toradb`
- **Workflow filename:** `release.yml`
- **Environment name:** `pypi`

Then create a GitHub Actions environment named `pypi` in repo settings (Settings →
Environments) — optionally with required reviewers to gate publishing.

> For the very first upload, PyPI requires either a "pending publisher" (configure the
> trusted publisher before the project exists) or an initial manual upload. The pending
> publisher flow is recommended.

## Cutting a release

1. Ensure `main` is green (the `CI` workflow: fmt, clippy, Rust tests, Python tests).
2. Bump the version in `Cargo.toml` (`[workspace.package] version`) if needed — this is the
   single source of truth for the wheel/CLI/crate version.
3. Update [`CHANGELOG.md`](CHANGELOG.md): move items from `Unreleased` into a new
   `## [X.Y.Z] - YYYY-MM-DD` section and update the compare links at the bottom.
4. Commit, then tag and push:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

   Use a pre-release suffix (e.g. `v0.2.0-rc1`) to have the GitHub Release marked as a
   pre-release automatically.
5. Watch the `Release` workflow. On success it produces:
   - a **GitHub Release** with wheels, the sdist, and `toradb-ingest-*` CLI archives, and
     auto-generated notes;
   - the package on **PyPI** (`pip install toradb`).

## Dry run (no publish)

Trigger `release.yml` via **workflow_dispatch** (Actions tab → Release → Run workflow). It
builds wheels and CLI binaries without a tag; the GitHub Release and PyPI steps are
tag-gated, so they are skipped — letting you exercise the build matrix before tagging.

## Verifying locally

The CI steps reproduce locally:

```sh
cargo fmt --all --check
cargo clippy --workspace --exclude toradb-sdk --all-targets
cargo test --workspace --exclude toradb-sdk     # toradb-sdk is a PyO3 cdylib; build via maturin
maturin develop && pytest tests/ -q
cargo build --release -p toradb-cli             # produces target/release/toradb-ingest
```
