# Contributing

> **Published docs:** [Mintlify site](https://toradb.mintlify.app/community/contributing) — source in [`mdx/community/contributing.mdx`](../mdx/community/contributing.mdx).

Thanks for your interest in contributing to ToraDB.

## Development setup

1. Complete [INSTALL.md](INSTALL.md) — use **Build from source** for development (PyPI is for end users).
2. Create a feature branch from `main`.

```bash
git checkout -b feat/your-change
```

## Workflow

- Keep changes scoped and reviewable.
- Add or update tests for behavior changes.
- Update docs for user-facing changes.
- Keep commit messages clear and imperative (`feat:`, `fix:`, `test:`, `docs:` style is preferred).

## Before opening a PR

Run:

```bash
cargo test
pytest tests/python_smoke.py -q
```

If relevant, also run:

```bash
cargo bench -p toradb-storage --bench segment_read
```

## Pull request checklist

- [ ] Change is explained clearly in PR description
- [ ] Tests added/updated and passing
- [ ] No unrelated files/noise in diff
- [ ] Docs updated where needed
- [ ] Security/privacy impact considered

## Reporting bugs

Please include:
- environment (OS, Rust version, Python version)
- exact command(s) run
- expected vs actual behavior
- minimal reproducible input/data
