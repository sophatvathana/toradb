# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Faceted search** — `search(facets=[...])` and `SELECT ... FACETS (col, ...)` return
  per-field value counts over the full matched result set (independent of `LIMIT`/`OFFSET`
  paging). Exposed via `SearchResults.facets` (dict-of-dicts), the `/api/search` REST
  response, and persisted into the per-table search log alongside the provenance trace.

## [0.1.0] - 2026-06-01

First public release.

### Added

- **Local retrieval engine** — on-disk Parquet segments with index sidecars, WAL replay,
  and tiered compaction.
- **Text, vector, and hybrid search** — BM25 (TBM3 block-max WAND, lexicon pruning, segment
  routing), dense ANN (HNSW / DiskANN, IVF, TurboQuant compression), and RRF fusion.
- **Retrieval SQL dialect** — `SELECT ... SPARSE SEARCH BM25(...)` / `VECTOR SEARCH ANN(...)`,
  `GROUP BY` / `HAVING` aggregates, CTEs, materialized views, and non-search scan
  `SELECT ... WHERE id = N` / metadata.
- **Typed columns** — declare `int` / `float` / `bool` / `date` / `timestamp` / `json` /
  `vector(N)` in `CREATE TABLE`; type-aware `WHERE` with `=`, `!=`, `<`, `>`, `BETWEEN`,
  `IN`, `LIKE`, `AND`/`OR`, plus `ORDER BY <column>` and `DISTINCT`.
- **Soft delete** — `DELETE FROM t WHERE id = N | id IN (...)` with per-table tombstones,
  read-path filtering, and physical reclamation on compaction.
- **Retrieval provenance** — `search(explain=True).provenance` exposes per-tier candidates,
  drops, and latency; persisted to a per-table search log; surfaced in the dashboard.
- **Auto-embedding** — register an embedder (`sentence-transformers`, OpenAI, or any
  callable) so text is embedded automatically on ingest and query; available via
  `toradb.local(path, embedder=...)`.
- **Python SDK** (PyO3, GIL released during heavy ops), **`toradb-ingest` CLI** (bulk ingest,
  index build, distributed worker, `platform serve`), and a **Next.js dashboard**.
- **Integrations** — LangChain and LlamaIndex vector-store adapters.

[Unreleased]: https://github.com/sophatvathana/toradb/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/sophatvathana/toradb/releases/tag/v0.1.0
