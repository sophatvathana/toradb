# ToraDB

Retrieval-focused database for local text, vector, and hybrid search — with a Python SDK, CLI, and retrieval SQL.

## Features

- **Local on-disk tables** — Parquet segments, index sidecars, WAL replay
- **Text, vector, hybrid** — BM25, dense ANN (HNSW / DiskANN), fusion
- **SQL + SDK** — `SELECT` with sparse/vector search, GROUP BY, materialized views
- **CLI** — ingest, query, reindex, catalog helpers

## Quick start

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install maturin
maturin develop

toradb smoke
python examples/full_example.py
```

See [docs/QUICKSTART.md](docs/QUICKSTART.md) for a full walkthrough.

## Documentation

| Topic | Link |
|-------|------|
| Overview | [docs/README.md](docs/README.md) |
| Install | [docs/INSTALL.md](docs/INSTALL.md) |
| Quickstart | [docs/QUICKSTART.md](docs/QUICKSTART.md) |
| Contributing | [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) |
| Code of Conduct | [docs/CODE_OF_CONDUCT.md](docs/CODE_OF_CONDUCT.md) |
| Security | [docs/SECURITY.md](docs/SECURITY.md) |

## Development

```bash
cargo test
pytest tests/python_smoke.py -q
```

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
