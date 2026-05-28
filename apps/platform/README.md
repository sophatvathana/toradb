# ToraDB Platform Dashboard

`apps/platform` is the dashboard UI served by `toradb-api` through `toradb-ingest platform serve`.

## Routes

| Path | Module |
|------|--------|
| `/overview` | Metrics, latency trend, MV snippet, active tasks |
| `/query` | SQL workbench, saved queries, EXPLAIN, metrics, export CSV/JSON |
| `/query-log` | Filterable/searchable query history, export CSV |
| `/catalog` | Table list, link to create table |
| `/catalog/{table}` | Tabs: overview, sample, indexes, sidecars; compact/finish/drop |
| `/schema` | Create table wizard, quick DDL (SHOW TABLES / SHOW MVs) |
| `/views` | Materialized views: create, refresh, drop, sample |
| `/ingest` | File or HF ingest as background jobs with live progress (polls every 1s) |
| `/jobs` | Index jobs with segment progress bars, API tasks |

State: [Zustand](https://github.com/pmndrs/zustand) (`stores/platform-store.ts`). SQL text and saved queries persist in localStorage.

## API surface (via `toradb-api`)

Read: `/api/health`, `/api/tables`, `/api/tables/{name}`, `/api/tables/{name}/sample`, `/api/tables/{name}/ddl`, `/api/tables/{name}/indexes`, `/api/materialized-views`, `/api/materialized-views/{name}`, `/api/metrics`, `/api/jobs`, `/api/tasks`, `/api/ingest/jobs`, `/api/ingest/jobs/{id}`, `/api/sql`, `/api/query-preview`, `/api/query-history`

Write: `/api/tables/{name}/finish`, `/resume`, `/drop`, `/compact`, `/api/materialized-views` (create), `/api/materialized-views/{name}/refresh`, `/drop`, `/api/ingest/begin`, `/api/ingest/upload`, `/api/ingest/hf` (returns `job_id`), `/api/ingest/jobs/{id}/cancel`, `/api/ingest/finish`

`/api/sql` supports SELECT plus DDL: `CREATE TABLE`, `SHOW TABLES`, `SHOW INDEXES`, `SHOW CREATE TABLE`, materialized view statements, `COMPACT TABLE`, `DROP TABLE`, etc.

### Hugging Face ingest

`POST /api/ingest/hf` and `POST /api/ingest/upload` start background jobs and return `{ job_id }`. Poll `GET /api/ingest/jobs/{id}` for `phase` (e.g. `resolving`, `downloading 1/3`, `ingesting`), `progress` (0–100), and `rows_ingested`. HF downloads use parallel range requests ([hf_transfer](https://github.com/huggingface/hf_transfer), vendored under `crates/toradb-engine/third_party/hf_transfer_lib.rs`). Heavy ingest runs on a blocking thread pool so the API stays responsive.

Optional env:

- `HF_TOKEN` or `HUGGING_FACE_HUB_TOKEN` — gated datasets
- `HF_TRANSFER_MAX_CONCURRENCY` — parallel connections (default 8)
- `HF_TRANSFER_CHUNK_SIZE` — range chunk bytes (default 10MB)

**Security:** No authentication. Bind to localhost only (`127.0.0.1:8787`) in untrusted environments.

## Build Dashboard Assets

```bash
cd apps/platform
pnpm install
pnpm build
```

Static output: `apps/platform/out`.

## Run Single-Process Platform Serve

From repo root:

```bash
cargo run -p toradb-cli --bin toradb-ingest -- platform serve --db examples/_demo_db --static-dir apps/platform/out --addr 127.0.0.1:8787
```

Open `http://127.0.0.1:8787/overview`.

## Local development (split process)

Terminal 1:

```bash
cargo run -p toradb-cli --bin toradb-ingest -- platform serve --db examples/_demo_db --static-dir apps/platform/out --addr 127.0.0.1:8787
```

Terminal 2:

```bash
cd apps/platform && pnpm dev
```

`next.config.ts` rewrites `/api/*` to port 8787 in development only.

## Test checklist

1. `pnpm build` and `cargo check -p toradb-engine -p toradb-api -p toradb-cli`
2. Create table via `/schema`, appears in catalog
3. Create + refresh MV via `/views`, row count updates
4. HF ingest shows job progress then rows in catalog
5. Compact table from catalog detail; task appears on `/jobs`
6. Saved query survives reload; query-log search filters entries
