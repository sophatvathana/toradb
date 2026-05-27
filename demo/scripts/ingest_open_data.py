#!/usr/bin/env python3
"""
Bulk-ingest open text corpora into a local ToraDB database (1M+ documents).

Designed for offline builds used by the demo website (TORADB_DB_PATH).

Examples (from repo root, after maturin develop):

  pip install -r demo/scripts/requirements-ingest.txt

  # MS MARCO passages (~8.8M available; default limit 1M)
  python demo/scripts/ingest_open_data.py \\
    --db ./data/msmarco_1m \\
    --source hf \\
    --dataset Tevatron/msmarco-passage-corpus \\
    --limit 1000000

  # Local JSONL (one JSON object per line, needs "text" field)
  python demo/scripts/ingest_open_data.py \\
    --db ./data/custom \\
    --source jsonl \\
    --path /path/to/corpus.jsonl \\
    --limit 2000000

  # Local Parquet shards (directory of *.parquet files)
  python demo/scripts/ingest_open_data.py \\
    --db ./data/from_parquet \\
    --source parquet \\
    --path ./downloads/msmarco-shards \\
    --text-column text \\
    --id-column docid

Then point the demo API at the database:

  export TORADB_DB_PATH=./data/msmarco_1m
  ./demo/run.sh
"""

from __future__ import annotations

import argparse
import gzip
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Iterator

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import pyarrow as pa
import toradb
from toradb.ingest import add_arrow


def _pick(row: dict[str, Any], *keys: str) -> Any:
    for key in keys:
        if key in row and row[key] not in (None, ""):
            return row[key]
    return None


def _text_from_row(row: dict[str, Any], text_column: str) -> str | None:
    val = _pick(row, text_column, "text", "passage", "content", "body")
    if val is None:
        return None
    if isinstance(val, list):
        val = val[0] if val else None
    text = str(val).strip()
    return text or None


def _id_from_row(row: dict[str, Any], id_column: str, fallback: int) -> str:
    val = _pick(row, id_column, "docid", "doc_id", "passage_id", "id", "_id")
    if val is None:
        return str(fallback)
    if isinstance(val, list):
        val = val[0] if val else fallback
    return str(val)


def _flush_batch(
    table,
    texts: list[str],
    doc_ids: list[str],
    extra_meta: dict[str, list[str]] | None,
) -> int:
    if not texts:
        return 0
    columns: dict[str, Any] = {"text": texts, "doc_id": doc_ids}
    if extra_meta:
        for key, values in extra_meta.items():
            if key in ("text", "id", "embedding", "vector"):
                continue
            columns[key] = values
    batch = pa.table(columns)
    return add_arrow(table, batch)


def validate_input_path(source: str, path: Path) -> None:
    """Fail fast with a helpful message before DROP TABLE / ingest."""
    resolved = path.expanduser().resolve()
    placeholder = "/path/to" in str(path) or str(path).startswith("/path/")

    if source == "jsonl":
        if not resolved.is_file():
            msg = f"JSONL file not found: {resolved}"
            if placeholder:
                msg += "\n  (--path was a README placeholder; use a real .jsonl file)"
            raise SystemExit(msg)
        return

    if source == "parquet":
        if not resolved.exists():
            msg = f"Parquet path not found: {resolved}"
            if placeholder:
                msg += (
                    "\n  (--path was a README placeholder, e.g. /path/to/shards/)"
                    "\n  Use a real directory of .parquet files or a single .parquet file."
                )
            raise SystemExit(msg)
        if resolved.is_dir():
            shards = sorted(resolved.glob("*.parquet"))
            if not shards:
                raise SystemExit(
                    f"No .parquet files in directory: {resolved}\n"
                    "  Put shard files there or pass a single .parquet file with --path."
                )
            return
        if resolved.suffix.lower() != ".parquet":
            raise SystemExit(f"Expected a .parquet file, got: {resolved}")
        return

    raise ValueError(f"unknown source: {source}")


def iter_jsonl(
    path: Path,
    *,
    limit: int | None,
    text_column: str,
    id_column: str,
) -> Iterator[dict[str, Any]]:
    use_gzip = str(path).endswith(".gz")
    opener = gzip.open if use_gzip else open
    mode = "rt" if use_gzip else "r"
    count = 0
    with opener(path, mode, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            yield json.loads(line)
            count += 1
            if limit is not None and count >= limit:
                break


def iter_parquet_dir(
    path: Path,
    *,
    limit: int | None,
    text_column: str,
    id_column: str,
    batch_size: int,
) -> Iterator[dict[str, Any]]:
    files = sorted(path.glob("*.parquet")) if path.is_dir() else [path]
    if not files:
        raise FileNotFoundError(f"No parquet files under {path}")

    seen = 0
    for file_path in files:
        import pyarrow.parquet as pq

        pf = pq.ParquetFile(file_path)
        cols = [c for c in (text_column, id_column) if c]
        for record_batch in pf.iter_batches(batch_size=batch_size, columns=cols or None):
            table = pa.Table.from_batches([record_batch])
            col_names = table.column_names
            text_idx = col_names.index(text_column) if text_column in col_names else None
            id_idx = col_names.index(id_column) if id_column in col_names else None
            for i in range(table.num_rows):
                row: dict[str, Any] = {}
                if text_idx is not None:
                    row[text_column] = table.column(text_idx)[i].as_py()
                if id_idx is not None:
                    row[id_column] = table.column(id_idx)[i].as_py()
                yield row
                seen += 1
                if limit is not None and seen >= limit:
                    return


def iter_hf(
    *,
    dataset: str,
    config: str | None,
    split: str,
    limit: int | None,
    text_column: str,
    id_column: str,
) -> Iterator[dict[str, Any]]:
    try:
        from datasets import load_dataset
    except ImportError as e:
        raise SystemExit(
            "Hugging Face ingest requires: pip install datasets"
        ) from e

    kwargs: dict[str, Any] = {"split": split, "streaming": True}
    if config:
        kwargs["name"] = config
    ds = load_dataset(dataset, **kwargs)
    count = 0
    for row in ds:
        yield row
        count += 1
        if limit is not None and count >= limit:
            break


def ingest_stream(
    table,
    rows: Iterator[dict[str, Any]],
    *,
    batch_size: int,
    text_column: str,
    id_column: str,
    log_every: int,
) -> int:
    texts: list[str] = []
    doc_ids: list[str] = []
    total = 0
    skipped = 0
    t0 = time.perf_counter()
    next_id = 0

    def report(force: bool = False) -> None:
        if not force and total % log_every != 0 and total != 0:
            return
        elapsed = time.perf_counter() - t0
        rate = total / elapsed if elapsed > 0 else 0.0
        print(
            f"  ingested {total:,} docs ({skipped:,} skipped) "
            f"@ {rate:,.0f} docs/s",
            flush=True,
        )

    for row in rows:
        text = _text_from_row(row, text_column)
        if not text:
            skipped += 1
            continue
        texts.append(text)
        doc_ids.append(_id_from_row(row, id_column, next_id))
        next_id += 1

        if len(texts) >= batch_size:
            batch_no = total // batch_size + 1
            print(
                f"  flushing batch {batch_no} ({len(texts):,} docs to disk)…",
                flush=True,
            )
            t_flush = time.perf_counter()
            n = _flush_batch(table, texts, doc_ids, None)
            total += n
            texts, doc_ids = [], []
            print(
                f"  batch {batch_no} flushed in {time.perf_counter() - t_flush:.1f}s",
                flush=True,
            )
            report(force=True)

    if texts:
        total += _flush_batch(table, texts, doc_ids, None)
        report(force=True)

    return total


def open_table(db_path: Path, table_name: str, *, drop: bool) -> tuple[Any, Any]:
    db_path.mkdir(parents=True, exist_ok=True)
    table_dir = db_path / table_name
    if drop and table_dir.is_dir():
        print(f"Removing existing table on disk (avoids loading into RAM): {table_dir}")
        t0 = time.perf_counter()
        shutil.rmtree(table_dir)
        print(f"  removed in {time.perf_counter() - t0:.1f}s")
    db = toradb.local(str(db_path))
    if drop:
        try:
            db.sql(f"DROP TABLE {table_name}")
        except Exception:
            pass
    names = set(db.list_tables())
    if drop or table_name not in names:
        print(f"Creating table {table_name} (mode=text)")
        table = db.create_table(table_name, mode="text")
    else:
        print(f"Appending to existing table {table_name}")
        table = db.table(table_name)
    return db, table


def post_process(db: Any, table_name: str, *, compact: bool, reindex: bool) -> None:
    if compact:
        print("Compacting segments (FULL)…")
        t0 = time.perf_counter()
        db.sql(f"COMPACT TABLE {table_name} FULL")
        print(f"  done in {time.perf_counter() - t0:.1f}s")
    if reindex:
        print("Reindexing BM25…")
        t0 = time.perf_counter()
        db.reindex(table_name, using="BM25")
        print(f"  done in {time.perf_counter() - t0:.1f}s")
    try:
        describe = db.sql(f"DESCRIBE {table_name}")
        print(describe)
    except Exception:
        pass


def _toradb_ingest_bin() -> str:
    env = os.environ.get("TORADB_INGEST_BIN")
    if env:
        return env
    found = shutil.which("toradb-ingest")
    if found:
        return found
    return str(ROOT / "target" / "debug" / "toradb-ingest")


def run_rust_ingest(args: argparse.Namespace, limit: int | None) -> int:
    """Delegate parquet/jsonl fast-bulk to the native toradb-ingest binary."""
    bin_path = _toradb_ingest_bin()
    if not Path(bin_path).is_file():
        print(
            f"error: toradb-ingest not found at {bin_path}\n"
            "  Build: cargo build -p toradb-cli --release\n"
            "  Or set TORADB_INGEST_BIN",
            file=sys.stderr,
        )
        return 1

    cmd = [
        bin_path,
        "bulk",
        "--db",
        str(args.db.resolve()),
        "--table",
        args.table,
        "--source",
        args.source,
        "--batch-size",
        str(args.batch_size),
    ]
    if args.path is not None:
        cmd.extend(["--path", str(args.path)])
    if limit is not None:
        cmd.extend(["--limit", str(limit)])
    if args.drop:
        cmd.append("--drop-table")

    print(f"  native ingest: {' '.join(cmd)}")
    subprocess.run(cmd, check=True)
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description="Ingest 1M+ open-text documents into ToraDB (offline demo / benchmark DB).",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    p.add_argument(
        "--db",
        type=Path,
        required=True,
        help="Database directory (created if missing)",
    )
    p.add_argument("--table", default="passages", help="Table name")
    p.add_argument(
        "--source",
        choices=("hf", "jsonl", "parquet"),
        default="hf",
        help="Input source",
    )
    p.add_argument(
        "--path",
        type=Path,
        help="Path to .jsonl(.gz) file, .parquet file, or directory of parquet shards",
    )
    p.add_argument(
        "--dataset",
        default="Tevatron/msmarco-passage-corpus",
        help="Hugging Face dataset id (source=hf)",
    )
    p.add_argument("--config", default=None, help="HF dataset config/name")
    p.add_argument("--split", default="train", help="HF split")
    p.add_argument("--text-column", default="text", help="Text field name")
    p.add_argument("--id-column", default="docid", help="Id field → metadata doc_id")
    p.add_argument(
        "--batch-size",
        type=int,
        default=25_000,
        help="Rows per add_arrow() call",
    )
    p.add_argument(
        "--limit",
        type=int,
        default=1_000_000,
        help="Max documents to ingest (0 = no limit)",
    )
    p.add_argument(
        "--log-every",
        type=int,
        default=100_000,
        help="Print progress every N documents",
    )
    p.add_argument(
        "--drop",
        action="store_true",
        help="DROP TABLE before ingest",
    )
    p.add_argument(
        "--no-compact",
        action="store_true",
        help="Skip COMPACT TABLE FULL after ingest",
    )
    p.add_argument(
        "--no-reindex",
        action="store_true",
        help="Skip BM25 reindex after ingest",
    )
    p.add_argument(
        "--dry-run",
        action="store_true",
        help="Count / validate input only; do not write",
    )
    p.add_argument(
        "--fast-bulk",
        action="store_true",
        help="Use db.begin_bulk_ingest / finish_bulk_ingest (recommended for 100k+ rows)",
    )
    return p


def main() -> int:
    args = build_parser().parse_args()
    limit = None if args.limit == 0 else args.limit

    if args.source in ("jsonl", "parquet") and not args.path:
        print("error: --path is required for jsonl/parquet sources", file=sys.stderr)
        return 2

    if args.path is not None and args.source in ("jsonl", "parquet"):
        validate_input_path(args.source, args.path)
        args.path = args.path.expanduser().resolve()

    print("ToraDB bulk ingest")
    print(f"  db:          {args.db.resolve()}")
    print(f"  table:       {args.table}")
    print(f"  source:      {args.source}")
    print(f"  batch_size:  {args.batch_size:,}")
    print(f"  limit:       {'∞' if limit is None else f'{limit:,}'}")

    if args.source == "hf":
        print(f"  dataset:     {args.dataset} ({args.split})")
        print("  opening HuggingFace stream (first batch may wait on download)…", flush=True)
        row_iter: Iterator[dict[str, Any]] = iter_hf(
            dataset=args.dataset,
            config=args.config,
            split=args.split,
            limit=limit,
            text_column=args.text_column,
            id_column=args.id_column,
        )
    elif args.source == "jsonl":
        assert args.path is not None
        print(f"  path:        {args.path}")
        row_iter = iter_jsonl(
            args.path,
            limit=limit,
            text_column=args.text_column,
            id_column=args.id_column,
        )
    else:
        assert args.path is not None
        print(f"  path:        {args.path}")
        row_iter = iter_parquet_dir(
            args.path,
            limit=limit,
            text_column=args.text_column,
            id_column=args.id_column,
            batch_size=args.batch_size,
        )

    if args.dry_run:
        n = 0
        for row in row_iter:
            if _text_from_row(row, args.text_column):
                n += 1
        print(f"Dry run: {n:,} documents with non-empty text")
        return 0

    if args.fast_bulk and args.source in ("parquet", "jsonl"):
        return run_rust_ingest(args, limit)

    if args.fast_bulk and args.batch_size == 25_000:
        args.batch_size = 200_000
        print(f"  fast-bulk: batch_size raised to {args.batch_size:,}")

    db, table = open_table(args.db, args.table, drop=args.drop)
    if args.fast_bulk:
        print("  fast-bulk: begin_bulk_ingest (deferred per-batch index rebuilds)")
        db.begin_bulk_ingest(args.table)

    if args.fast_bulk:
        args.log_every = min(args.log_every, args.batch_size)

    print("  streaming documents…", flush=True)
    t0 = time.perf_counter()
    total = ingest_stream(
        table,
        row_iter,
        batch_size=args.batch_size,
        text_column=args.text_column,
        id_column=args.id_column,
        log_every=max(1, args.log_every),
    )
    elapsed = time.perf_counter() - t0
    print(f"Ingest finished: {total:,} documents in {elapsed:.1f}s")

    if args.fast_bulk:
        print()
        print(
            "You can start the demo API now (./demo/run.sh); "
            "search stays disabled until finish_bulk_ingest completes."
        )
        print(
            "If finish is interrupted, resume with: "
            f"db.resume_index_build({args.table!r}) on the same db path."
        )
        print("  fast-bulk: finish_bulk_ingest")
        t_finish = time.perf_counter()
        db.finish_bulk_ingest(
            args.table,
            compact=not args.no_compact,
            reindex_bm25=not args.no_reindex,
        )
        print(f"  finish_bulk_ingest: {time.perf_counter() - t_finish:.1f}s")
        if not args.no_reindex or not args.no_compact:
            try:
                print(db.sql(f"DESCRIBE {args.table}"))
            except Exception:
                pass
    elif not args.no_compact or not args.no_reindex:
        post_process(
            db,
            args.table,
            compact=not args.no_compact,
            reindex=not args.no_reindex,
        )

    print()
    print("Demo website:")
    print(f"  export TORADB_DB_PATH={args.db.resolve()}")
    print("  ./demo/run.sh")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
