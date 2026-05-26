"""
ToraDB demo API — search, SQL, and catalog over a local on-disk database.

Environment:
  TORADB_DB_PATH   Path to database directory (default: ../../examples/_demo_db)
  TORADB_CORS      Comma-separated origins (default: *)
  TORADB_HOST      Bind host (default: 127.0.0.1)
  TORADB_PORT      Bind port (default: 8787)
"""

from __future__ import annotations

import json
import os
import re
import sys
import threading
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any, Literal

from fastapi import FastAPI, HTTPException, Query
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel, Field

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DB = REPO_ROOT / "examples" / "_demo_db"

DB_PATH = Path(os.environ.get("TORADB_DB_PATH", str(DEFAULT_DB))).resolve()
DOC_CACHE: dict[tuple[str, int], dict[str, Any]] = {}
MAX_DOC_CACHE = 50_000
_APP_DB: Any = None


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


def _use_lightweight_tables() -> bool:
    if os.environ.get("TORADB_LIGHTWEIGHT", "").lower() in ("1", "true", "yes"):
        return True
    return bool(_scan_indexing_tables())


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
    return toradb.local(str(DB_PATH), reload=reload)


def _init_app_db() -> None:
    global _APP_DB
    if _scan_indexing_tables():
        _APP_DB = None
        return
    _APP_DB = _open_db(reload=False)


def _warmup_searchable_tables() -> None:
    if os.environ.get("TORADB_WARMUP_ON_START", "").lower() not in ("1", "true", "yes"):
        return
    indexing = set(_scan_indexing_tables())
    try:
        db = _db()
    except HTTPException:
        return
    for name in _tables_on_disk():
        if name in indexing:
            continue
        try:
            db.table(name).search("warmup", top_k=1)
        except Exception:
            pass


def _db(*, reload: bool = False) -> Any:
    global _APP_DB
    if reload:
        _APP_DB = _open_db(reload=True)
        return _APP_DB
    if _APP_DB is None:
        _init_app_db()
    if _APP_DB is None:
        return _open_db(reload=False)
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
    _init_app_db()
    threading.Thread(target=_warmup_searchable_tables, daemon=True).start()
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
    toradb_ok = True
    try:
        _ensure_toradb()
    except RuntimeError:
        toradb_ok = False

    exists = DB_PATH.is_dir()
    indexing_tables = _scan_indexing_tables() if exists else []
    tables: list[str] = _tables_on_disk() if exists else []
    hint = None
    if not exists:
        hint = "Run: python examples/full_example.py"
    elif toradb_ok and not indexing_tables:
        try:
            tables = _db().list_tables()
        except Exception:
            hint = "Database path exists but could not be opened."
    elif indexing_tables:
        hint = "Index build in progress — search is disabled until finish completes."

    status: Literal["ok", "degraded"] = (
        "ok" if exists and toradb_ok and (tables or indexing_tables) else "degraded"
    )
    cache_hits: int | None = None
    cache_misses: int | None = None
    if toradb_ok and _APP_DB is not None and not indexing_tables:
        try:
            hits, misses = _APP_DB.cache_stats()
            cache_hits = int(hits)
            cache_misses = int(misses)
        except Exception:
            pass
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
    if _use_lightweight_tables():
        return [
            TableInfo(name=name, rows=0, describe=None) for name in _tables_on_disk()
        ]

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
    _invalidate_app_db()
    _init_app_db()
    return {"status": "ok", "path": str(DB_PATH)}


@app.post("/api/search", response_model=SearchResponse)
def search(req: SearchRequest) -> SearchResponse:
    _require_searchable(req.table)
    t_total = time.perf_counter()

    t_open = time.perf_counter()
    db = _db()
    try:
        table = db.table(req.table)
    except Exception as e:
        raise HTTPException(status_code=404, detail=f"Table not found: {req.table}") from e
    open_ms = (time.perf_counter() - t_open) * 1000

    kwargs: dict[str, Any] = {
        "top_k": req.top_k,
        "offset": req.offset,
        "explain": req.explain,
    }
    if req.strategy:
        kwargs["strategy"] = req.strategy
    if req.graph_expand:
        kwargs["graph_expand"] = True

    t_search = time.perf_counter()
    try:
        results = table.search(req.query, **kwargs)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    search_ms = (time.perf_counter() - t_search) * 1000

    _, rows = _sql_result_to_table(results.to_pandas())
    ids = [int(r["id"]) for r in rows]
    scores = [float(r["score"]) for r in rows]

    warm_cache: dict[int, dict[str, Any]] = {}
    missing_ids: list[int] = []
    for doc_id in ids:
        cached = _cache_get(req.table, doc_id)
        if cached:
            warm_cache[doc_id] = cached
        else:
            missing_ids.append(doc_id)
    t_fetch = time.perf_counter()
    fetched: dict[int, Any] = {}
    if missing_ids:
        fetched = db.fetch_documents(req.table, missing_ids)
    fetch_ms = (time.perf_counter() - t_fetch) * 1000
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

    explain_text = None
    if req.explain:
        try:
            explain_text = results.explain()
        except Exception:
            explain_text = None
        try:
            hits, misses = db.cache_stats()
            cache_line = f"cache_hits={hits} cache_misses={misses}"
            explain_text = f"{explain_text}\n{cache_line}" if explain_text else cache_line
        except Exception:
            pass

    total_ms = (time.perf_counter() - t_total) * 1000
    return SearchResponse(
        table=req.table,
        query=req.query,
        strategy=req.strategy,
        hits=hits,
        explain=explain_text,
        latency_ms=round(search_ms, 2),
        open_ms=round(open_ms, 2),
        search_ms=round(search_ms, 2),
        fetch_ms=round(fetch_ms, 2),
        total_ms=round(total_ms, 2),
    )


@app.post("/api/sql")
def run_sql(req: SqlRequest) -> dict[str, Any]:
    db = _db()
    t0 = time.perf_counter()
    try:
        out = db.sql(req.query)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    latency_ms = (time.perf_counter() - t0) * 1000

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
