"""
ToraDB demo API — search, SQL, and catalog over a local on-disk database.

Environment:
  TORADB_DB_PATH   Path to database directory (default: ../../examples/_demo_db)
  TORADB_CORS      Comma-separated origins (default: *)
  TORADB_HOST      Bind host (default: 127.0.0.1)
  TORADB_PORT      Bind port (default: 8787)
  TORADB_DEBUG     Set to 1/true/yes to print bottleneck timings to stderr
"""

from __future__ import annotations

import json
import logging
import os
import re
import sys
import threading
import time
from contextlib import asynccontextmanager, contextmanager
from pathlib import Path
from typing import Any, Iterator, Literal

from fastapi import FastAPI, HTTPException, Query
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel, Field

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DB = REPO_ROOT / "examples" / "_demo_db"

DB_PATH = Path(os.environ.get("TORADB_DB_PATH", str(DEFAULT_DB))).resolve()
DOC_CACHE: dict[tuple[str, int], dict[str, Any]] = {}
MAX_DOC_CACHE = 50_000
_APP_DB: Any = None
_INIT_LOCK = threading.Lock()

_DEBUG_LOG = logging.getLogger("toradb.demo")
if not _DEBUG_LOG.handlers:
    _h = logging.StreamHandler(sys.stderr)
    _h.setFormatter(logging.Formatter("[toradb-demo] %(message)s"))
    _DEBUG_LOG.addHandler(_h)
    _DEBUG_LOG.propagate = False


def _debug_enabled() -> bool:
    return os.environ.get("TORADB_DEBUG", "").lower() in ("1", "true", "yes")


def _debug(msg: str, **fields: Any) -> None:
    if not _debug_enabled():
        return
    if fields:
        extra = " ".join(f"{k}={v}" for k, v in fields.items())
        _DEBUG_LOG.info("%s %s", msg, extra)
    else:
        _DEBUG_LOG.info("%s", msg)


@contextmanager
def _debug_span(label: str, **fields: Any) -> Iterator[None]:
    if not _debug_enabled():
        yield
        return
    t0 = time.perf_counter()
    try:
        yield
    finally:
        ms = (time.perf_counter() - t0) * 1000
        _debug(f"{label}", ms=f"{ms:.2f}", **fields)


def _debug_env_snapshot() -> dict[str, str]:
    keys = (
        "TORADB_DB_PATH",
        "TORADB_CACHE_INDEX_BYTES",
        "TORADB_CACHE_SEGMENT_ENTRIES",
        "TORADB_WARMUP_ON_START",
        "TORADB_LIGHTWEIGHT",
    )
    return {k: os.environ.get(k, "") for k in keys if os.environ.get(k)}


def _ensure_toradb() -> None:
    try:
        import toradb  # noqa: F401
    except ImportError as e:
        raise RuntimeError(
            "toradb is not installed. From repo root: maturin develop"
        ) from e


def _read_build_status(table: str) -> dict[str, Any] | None:
    path = DB_PATH / table / "indexes" / "build_status.json"
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def _scan_indexing_tables() -> list[str]:
    if not DB_PATH.is_dir():
        return []
    out: list[str] = []
    for child in DB_PATH.iterdir():
        if not child.is_dir():
            continue
        status = _read_build_status(child.name)
        if status and status.get("state") == "building":
            out.append(child.name)
    return sorted(out)


def _tables_on_disk() -> list[str]:
    if not DB_PATH.is_dir():
        return []
    return sorted(
        p.name
        for p in DB_PATH.iterdir()
        if p.is_dir() and (p / "manifest.json").is_file()
    )


def _read_table_manifest(table: str) -> dict[str, Any] | None:
    path = DB_PATH / table / "manifest.json"
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def _is_segment_only_table(table: str) -> bool:
    manifest = _read_table_manifest(table)
    return bool(manifest and manifest.get("index_mode") == "segment_only")


def _bm25_sidecar_is_tbm3(path: Path) -> bool:
    try:
        with path.open("rb") as f:
            return f.read(4) == b"TBM3"
    except OSError:
        return False


def _indexes_need_rebuild() -> list[str]:
    """Tables with missing or non-TBM3 segment BM25 sidecars."""
    need: list[str] = []
    for table in _tables_on_disk():
        indexes = DB_PATH / table / "indexes"
        if not indexes.is_dir():
            continue
        bins = list(indexes.glob("seg_*.bm25.bin"))
        if not bins:
            continue
        lex_count = len(list(indexes.glob("*.bm25.lex.bin")))
        tbm3_count = sum(1 for p in bins if _bm25_sidecar_is_tbm3(p))
        if tbm3_count < len(bins) or lex_count < len(bins):
            need.append(table)
    return need


def _manifest_row_count(table: str) -> int | None:
    manifest = _read_table_manifest(table)
    if not manifest:
        return None
    ranges = manifest.get("segment_id_ranges") or []
    if ranges:
        return sum(
            int(r["max_id"]) - int(r["min_id"]) + 1
            for r in ranges
            if "min_id" in r and "max_id" in r
        )
    return None


def _use_lightweight_tables() -> bool:
    if os.environ.get("TORADB_LIGHTWEIGHT", "").lower() in ("1", "true", "yes"):
        return True
    if _scan_indexing_tables():
        return True
    tables = _tables_on_disk()
    return bool(tables) and all(_is_segment_only_table(t) for t in tables)


def _open_db(*, reload: bool) -> Any:
    import toradb

    if not DB_PATH.is_dir():
        raise HTTPException(
            status_code=503,
            detail={
                "error": "database_not_found",
                "path": str(DB_PATH),
                "hint": "Run: python examples/full_example.py",
            },
        )
    with _debug_span("toradb.local", reload=reload, path=str(DB_PATH)):
        return toradb.local(str(DB_PATH), reload=reload)


def _init_app_db() -> None:
    global _APP_DB
    with _INIT_LOCK:
        if _APP_DB is not None:
            _debug("init_app_db", status="already_open")
            return
        if _scan_indexing_tables():
            _APP_DB = None
            _debug("init_app_db", status="skipped_index_building")
            return
        with _debug_span("init_app_db"):
            _APP_DB = _open_db(reload=False)


def _warmup_searchable_tables() -> None:
    if os.environ.get("TORADB_WARMUP_ON_START", "").lower() not in ("1", "true", "yes"):
        return
    _debug("warmup_start", env=_debug_env_snapshot())
    indexing = set(_scan_indexing_tables())
    try:
        with _debug_span("warmup_open_db"):
            db = _open_db(reload=False)
    except HTTPException:
        _debug("warmup_open_db", status="failed")
        return
    for name in _tables_on_disk():
        if name in indexing:
            continue
        try:
            with _debug_span("warmup_search", table=name):
                db.table(name).search("warmup", top_k=1)
        except Exception as exc:
            _debug("warmup_search", table=name, error=str(exc))
    _debug("warmup_done")


def _db(*, reload: bool = False) -> Any:
    global _APP_DB
    if reload:
        with _debug_span("_db", mode="reload"):
            _APP_DB = _open_db(reload=True)
        return _APP_DB
    if _APP_DB is None:
        with _debug_span("_db", mode="init_app_db"):
            _init_app_db()
    if _APP_DB is None:
        _debug("_db", mode="ephemeral_open")
        return _open_db(reload=False)
    _debug("_db", mode="cached")
    return _APP_DB


def _invalidate_app_db() -> None:
    global _APP_DB
    _APP_DB = None


def _require_searchable(table: str) -> None:
    status = _read_build_status(table)
    if status and status.get("state") == "building":
        raise HTTPException(
            status_code=503,
            detail={
                "error": "index_building",
                "table": table,
                "phase": status.get("phase"),
                "segments_done": status.get("segments_done", 0),
                "segments_total": status.get("segments_total", 0),
                "message": status.get("message"),
            },
        )
    if status and status.get("state") == "failed":
        raise HTTPException(
            status_code=503,
            detail={
                "error": "index_build_failed",
                "table": table,
                "message": status.get("message"),
            },
        )


def _cache_get(table: str, doc_id: int) -> dict[str, Any] | None:
    return DOC_CACHE.get((table, doc_id))


def _cache_put(table: str, docs: dict[int, dict[str, Any]]) -> None:
    if len(DOC_CACHE) > MAX_DOC_CACHE:
        DOC_CACHE.clear()
    for doc_id, doc in docs.items():
        DOC_CACHE[(table, doc_id)] = doc


def _scalar(val: Any) -> Any:
    if hasattr(val, "item"):
        return val.item()
    return val


def _sql_result_to_table(frame: Any) -> tuple[list[str], list[dict[str, Any]]]:
    """ToraDB to_pandas() returns a column dict, not a pandas DataFrame."""
    if isinstance(frame, dict):
        columns = list(frame.keys())
        if not columns:
            return [], []
        lengths = [len(frame[c]) if isinstance(frame[c], (list, tuple)) else 1 for c in columns]
        n = max(lengths) if lengths else 0
        rows: list[dict[str, Any]] = []
        for i in range(n):
            row: dict[str, Any] = {}
            for col in columns:
                col_val = frame[col]
                if isinstance(col_val, (list, tuple)):
                    row[col] = _scalar(col_val[i]) if i < len(col_val) else None
                else:
                    row[col] = _scalar(col_val)
            rows.append(row)
        return columns, rows

    if hasattr(frame, "columns") and hasattr(frame, "to_dict"):
        columns = [str(c) for c in frame.columns]
        rows = frame.to_dict(orient="records")
        for row in rows:
            for key, val in list(row.items()):
                row[key] = _scalar(val)
        return columns, rows

    raise TypeError(f"unsupported SQL result frame type: {type(frame)!r}")


@asynccontextmanager
async def lifespan(_app: FastAPI):
    _ensure_toradb()
    if _debug_enabled():
        _DEBUG_LOG.setLevel(logging.INFO)
        _debug("api_start", env=_debug_env_snapshot(), db_path=str(DB_PATH))
    rebuild = _indexes_need_rebuild()
    if rebuild:
        _DEBUG_LOG.warning(
            "indexes need rebuild (TBM3 .bm25.bin + .bm25.lex.bin): %s — "
            "run: cargo build -p toradb-cli --release && "
            "./target/release/toradb-ingest resume --db %s --table %s",
            ", ".join(rebuild),
            DB_PATH,
            rebuild[0],
        )
    if os.environ.get("TORADB_WARMUP_ON_START", "").lower() in ("1", "true", "yes"):
        # Separate process: toradb.local()/search hold the GIL and would block /api/health.
        import multiprocessing

        multiprocessing.Process(target=_warmup_searchable_tables, daemon=True).start()
    yield
    _invalidate_app_db()


app = FastAPI(title="ToraDB Demo API", version="0.1.0", lifespan=lifespan)

_origins = os.environ.get("TORADB_CORS", "*").split(",")
app.add_middleware(
    CORSMiddleware,
    allow_origins=[o.strip() for o in _origins if o.strip()] or ["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


class SearchRequest(BaseModel):
    table: str = "articles"
    query: str = Field(min_length=1, max_length=2000)
    top_k: int = Field(default=10, ge=1, le=100)
    offset: int = Field(default=0, ge=0)
    strategy: str | None = None
    explain: bool = False
    graph_expand: bool = False


class SqlRequest(BaseModel):
    query: str = Field(min_length=1, max_length=8000)


class SearchHit(BaseModel):
    id: int
    score: float
    text: str | None = None
    metadata: dict[str, str] = Field(default_factory=dict)


class SearchResponse(BaseModel):
    table: str
    query: str
    strategy: str | None
    hits: list[SearchHit]
    explain: str | None = None
    latency_ms: float
    open_ms: float = 0.0
    search_ms: float = 0.0
    fetch_ms: float = 0.0
    total_ms: float = 0.0
    debug: str | None = None


class TableInfo(BaseModel):
    name: str
    rows: int
    describe: str | None = None


class HealthResponse(BaseModel):
    status: Literal["ok", "degraded"]
    db_path: str
    db_exists: bool
    tables: list[str]
    indexing_tables: list[str] = Field(default_factory=list)
    toradb_importable: bool
    hint: str | None = None
    cache_hits: int | None = None
    cache_misses: int | None = None


class IndexStatusResponse(BaseModel):
    table: str
    state: Literal["building", "ready", "failed"]
    phase: str | None = None
    segments_done: int = 0
    segments_total: int = 0
    message: str | None = None
    updated_unix_secs: int | None = None


@app.get("/api/health", response_model=HealthResponse)
def health() -> HealthResponse:
    t0 = time.perf_counter()
    toradb_ok = True
    try:
        _ensure_toradb()
    except RuntimeError:
        toradb_ok = False

    exists = DB_PATH.is_dir()
    with _debug_span("health_scan_indexing"):
        indexing_tables = _scan_indexing_tables() if exists else []
    with _debug_span("health_tables_on_disk"):
        tables: list[str] = _tables_on_disk() if exists else []
    hint = None
    if not exists:
        hint = "Run: python examples/full_example.py"
    elif indexing_tables:
        hint = "Index build in progress — search is disabled until finish completes."
    elif toradb_ok and exists and not tables:
        hint = "Database path exists but no tables were found."

    status: Literal["ok", "degraded"] = (
        "ok" if exists and toradb_ok and (tables or indexing_tables) else "degraded"
    )
    cache_hits: int | None = None
    cache_misses: int | None = None
    if toradb_ok and _APP_DB is not None and not indexing_tables:
        try:
            with _debug_span("health_cache_stats"):
                hits, misses = _APP_DB.cache_stats()
            cache_hits = int(hits)
            cache_misses = int(misses)
        except Exception:
            pass
    _debug("health", total_ms=f"{(time.perf_counter() - t0) * 1000:.2f}", app_db_open=_APP_DB is not None)
    return HealthResponse(
        status=status,
        db_path=str(DB_PATH),
        db_exists=exists,
        tables=tables,
        indexing_tables=indexing_tables,
        toradb_importable=toradb_ok,
        hint=hint,
        cache_hits=cache_hits,
        cache_misses=cache_misses,
    )


@app.get("/api/index-status", response_model=IndexStatusResponse)
def index_status(table: str = Query(..., min_length=1)) -> IndexStatusResponse:
    status = _read_build_status(table)
    if status:
        state = status.get("state", "building")
        if state not in ("building", "ready", "failed"):
            state = "building"
        return IndexStatusResponse(
            table=table,
            state=state,  # type: ignore[arg-type]
            phase=status.get("phase"),
            segments_done=int(status.get("segments_done", 0)),
            segments_total=int(status.get("segments_total", 0)),
            message=status.get("message"),
            updated_unix_secs=status.get("updated_unix_secs"),
        )
    indexes = DB_PATH / table / "indexes"
    if (indexes / "bm25.bin").is_file():
        return IndexStatusResponse(table=table, state="ready")
    return IndexStatusResponse(table=table, state="ready")


@app.get("/api/tables", response_model=list[TableInfo])
def list_tables() -> list[TableInfo]:
    t0 = time.perf_counter()
    if _use_lightweight_tables():
        names = _tables_on_disk()
        out = []
        for name in names:
            rows = _manifest_row_count(name) or 0
            describe = None
            if _debug_enabled():
                manifest = _read_table_manifest(name) or {}
                describe = (
                    f"index_mode={manifest.get('index_mode', '?')} "
                    f"query_mode={manifest.get('query_mode', '?')} "
                    f"segments={len(manifest.get('segments') or [])}"
                )
            out.append(TableInfo(name=name, rows=rows, describe=describe))
        _debug("list_tables", mode="lightweight", count=len(out), ms=f"{(time.perf_counter() - t0) * 1000:.2f}")
        return out

    with _debug_span("list_tables_open_db"):
        db = _db()
    row_counts: dict[str, int] = {}
    try:
        agg = db.sql("SHOW TABLES").to_pandas()
        cols, rows = _sql_result_to_table(agg)
        if "table" in cols and "rows" in cols:
            ti = cols.index("table")
            ri = cols.index("rows")
            for row in rows:
                row_counts[str(row[cols[ti]])] = int(row[cols[ri]])
    except Exception:
        pass

    out: list[TableInfo] = []
    for name in db.list_tables():
        rows = row_counts.get(name, 0)
        describe = None
        try:
            describe = str(db.sql(f"DESCRIBE {name}"))
        except Exception:
            describe = None
        out.append(TableInfo(name=name, rows=rows, describe=describe))
    _debug("list_tables", mode="full", count=len(out), ms=f"{(time.perf_counter() - t0) * 1000:.2f}")
    return out


@app.get("/api/tables/{table_name}/sample-queries")
def sample_queries(table_name: str) -> dict[str, list[str]]:
    return {
        "search": [
            "Nikola Tesla alternating current",
            "Nikola Tesla wireless",
            "year:1888",
            "tag:patent",
        ],
        "sql": [
            f"SELECT id FROM {table_name} SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
            f"SELECT id FROM {table_name} DISTRIBUTED SPARSE SEARCH body BM25('wireless') LIMIT 10",
            f"SELECT tag, COUNT(*) FROM {table_name} GROUP BY tag",
            f"DESCRIBE {table_name}",
            "SHOW TABLES",
        ],
    }


@app.post("/api/db/reload")
def reload_database() -> dict[str, str]:
    """Reload on-disk tables into the cached Database (after index build)."""
    with _debug_span("db_reload"):
        _invalidate_app_db()
        _init_app_db()
    return {"status": "ok", "path": str(DB_PATH)}


@app.post("/api/search", response_model=SearchResponse)
def search(req: SearchRequest) -> SearchResponse:
    _require_searchable(req.table)
    t_total = time.perf_counter()
    timings: dict[str, float] = {}

    t_open = time.perf_counter()
    db = _db()
    timings["db_ms"] = (time.perf_counter() - t_open) * 1000
    try:
        t_table = time.perf_counter()
        table = db.table(req.table)
        timings["table_ms"] = (time.perf_counter() - t_table) * 1000
    except Exception as e:
        raise HTTPException(status_code=404, detail=f"Table not found: {req.table}") from e
    open_ms = timings["db_ms"] + timings.get("table_ms", 0.0)

    kwargs: dict[str, Any] = {
        "top_k": req.top_k,
        "offset": req.offset,
        "explain": req.explain,
    }
    strategy = req.strategy
    if not strategy and _is_segment_only_table(req.table):
        strategy = "sparse"
    if strategy:
        kwargs["strategy"] = strategy
    if req.graph_expand:
        kwargs["graph_expand"] = True

    t_search = time.perf_counter()
    try:
        results = table.search(req.query, **kwargs)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    search_ms = (time.perf_counter() - t_search) * 1000
    timings["search_ms"] = search_ms

    t_pandas = time.perf_counter()
    _, rows = _sql_result_to_table(results.to_pandas())
    timings["to_pandas_ms"] = (time.perf_counter() - t_pandas) * 1000
    ids = [int(r["id"]) for r in rows]
    scores = [float(r["score"]) for r in rows]

    warm_cache: dict[int, dict[str, Any]] = {}
    missing_ids: list[int] = []
    t_cache = time.perf_counter()
    for doc_id in ids:
        cached = _cache_get(req.table, doc_id)
        if cached:
            warm_cache[doc_id] = cached
        else:
            missing_ids.append(doc_id)
    timings["doc_cache_lookup_ms"] = (time.perf_counter() - t_cache) * 1000
    timings["doc_cache_hits"] = float(len(warm_cache))
    timings["doc_cache_misses"] = float(len(missing_ids))

    t_fetch = time.perf_counter()
    fetched: dict[int, Any] = {}
    if missing_ids:
        fetched = db.fetch_documents(req.table, missing_ids)
    fetch_ms = (time.perf_counter() - t_fetch) * 1000
    timings["fetch_ms"] = fetch_ms
    docs: dict[int, dict[str, Any]] = {}
    for doc_id in ids:
        if doc_id in warm_cache:
            docs[doc_id] = warm_cache[doc_id]
        elif doc_id in fetched:
            row = fetched[doc_id]
            docs[doc_id] = {
                "id": int(row["id"]),
                "text": row.get("text") or "",
                "metadata": dict(row.get("metadata") or {}),
            }
    _cache_put(req.table, docs)

    hits: list[SearchHit] = []
    for doc_id, score in zip(ids, scores):
        doc = docs.get(doc_id) or _cache_get(req.table, doc_id)
        hits.append(
            SearchHit(
                id=doc_id,
                score=score,
                text=doc["text"] if doc else None,
                metadata=doc.get("metadata", {}) if doc else {},
            )
        )

    t_assemble = time.perf_counter()
    explain_text = None
    if req.explain:
        try:
            explain_text = results.explain()
        except Exception:
            explain_text = None
        try:
            index_hits, index_misses = db.cache_stats()
            cache_line = f"cache_hits={index_hits} cache_misses={index_misses}"
            explain_text = f"{explain_text}\n{cache_line}" if explain_text else cache_line
            timings["index_cache_hits"] = float(index_hits)
            timings["index_cache_misses"] = float(index_misses)
        except Exception:
            pass
    timings["assemble_ms"] = (time.perf_counter() - t_assemble) * 1000

    total_ms = (time.perf_counter() - t_total) * 1000
    timings["total_ms"] = total_ms
    debug_line = (
        " ".join(f"{k}={v:.2f}" if k.endswith("_ms") else f"{k}={int(v)}"
                 for k, v in sorted(timings.items()))
    )
    _debug(
        "search_bottleneck",
        table=req.table,
        query_len=len(req.query),
        strategy=req.strategy or "default",
        hits=len(hits),
        **{k: (f"{v:.2f}" if k.endswith("_ms") else str(int(v))) for k, v in timings.items()},
    )

    return SearchResponse(
        table=req.table,
        query=req.query,
        strategy=strategy,
        hits=hits,
        explain=explain_text,
        latency_ms=round(search_ms, 2),
        open_ms=round(open_ms, 2),
        search_ms=round(search_ms, 2),
        fetch_ms=round(fetch_ms, 2),
        total_ms=round(total_ms, 2),
        debug=debug_line if _debug_enabled() else None,
    )


@app.post("/api/sql")
def run_sql(req: SqlRequest) -> dict[str, Any]:
    t_total = time.perf_counter()
    t_db = time.perf_counter()
    db = _db()
    db_ms = (time.perf_counter() - t_db) * 1000
    t0 = time.perf_counter()
    try:
        out = db.sql(req.query)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    sql_ms = (time.perf_counter() - t0) * 1000
    latency_ms = (time.perf_counter() - t_total) * 1000
    _debug(
        "sql_bottleneck",
        db_ms=f"{db_ms:.2f}",
        sql_ms=f"{sql_ms:.2f}",
        total_ms=f"{latency_ms:.2f}",
        query_preview=req.query[:80],
    )

    if hasattr(out, "to_pandas"):
        try:
            columns, rows = _sql_result_to_table(out.to_pandas())
        except TypeError as e:
            raise HTTPException(status_code=500, detail=str(e)) from e
        return {
            "kind": "frame",
            "columns": columns,
            "rows": rows,
            "latency_ms": round(latency_ms, 2),
        }

    return {
        "kind": "message",
        "text": str(out),
        "latency_ms": round(latency_ms, 2),
    }


@app.get("/api/cli-hint")
def cli_hint(
    table: str = Query("articles"),
    query: str = Query("Nikola Tesla motor"),
) -> dict[str, str]:
    q = query.replace('"', '\\"')
    return {
        "query": f'toradb query "{DB_PATH}" {table} "{q}"',
        "sql": f'toradb sql "{DB_PATH}" "SELECT id FROM {table} SPARSE SEARCH body BM25(\'{q}\') LIMIT 10"',
    }


def main() -> None:
    host = os.environ.get("TORADB_HOST", "127.0.0.1")
    port = int(os.environ.get("TORADB_PORT", "8787"))
    import uvicorn

    uvicorn.run(
        "main:app",
        host=host,
        port=port,
        reload=bool(os.environ.get("TORADB_RELOAD")),
    )


if __name__ == "__main__":
    if str(REPO_ROOT) not in sys.path:
        sys.path.insert(0, str(REPO_ROOT))
    main()
