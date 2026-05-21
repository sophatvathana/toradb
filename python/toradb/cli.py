"""ToraDB command-line helpers."""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path


def _print_usage() -> None:
    print(
        """toradb — local retrieval database CLI

Commands:
  smoke              Quick ingest + search sanity check
  query PATH TABLE Q Run BM25 search and print ranked ids
  sql PATH QUERY     Run a SQL statement (search or analytics)
  tables PATH        List tables with on-disk manifests
  reindex PATH TABLE Rebuild indexes (CREATE INDEX)

Examples:
  toradb smoke
  toradb query ./my_db docs "Nikola Tesla motor"
  toradb sql ./my_db "SELECT tag, COUNT(*) FROM articles GROUP BY tag"
  toradb tables ./examples/_demo_db
"""
    )


def cmd_smoke() -> int:
    import toradb

    path = Path(tempfile.mkdtemp(prefix="toradb_cli_smoke_"))
    try:
        db = toradb.local(str(path))
        table = db.create_table("docs", mode="text")
        table.add(
            [
                {
                    "text": "Nikola Tesla alternating current induction motor",
                    "tag": "history",
                },
                {"text": "Marie Curie studied radioactivity", "tag": "science"},
            ]
        )
        results = table.search("Nikola Tesla alternating current", top_k=3)
        frame = results.to_pandas()
        if len(frame["id"]) == 0 or frame["id"][0] != 0:
            print("smoke failed: expected ranked hit on doc 0", file=sys.stderr)
            return 1
        counts = db.sql("SELECT tag, COUNT(*) FROM docs GROUP BY tag").to_pandas()
        by_tag = dict(zip(counts["tag"], counts["count"]))
        if by_tag.get("history") != 1:
            print(f"smoke failed: unexpected GROUP BY counts {by_tag}", file=sys.stderr)
            return 1
        print("smoke ok: search + sql analytics")
        print(f"  search top id={frame['id'][0]} score={frame['score'][0]:.4f}")
        print(f"  group-by: {by_tag}")
        return 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def cmd_query(db_path: str, table: str, query: str, top_k: int) -> int:
    import toradb

    db = toradb.local(db_path)
    results = db.table(table).search(query, top_k=top_k)
    frame = results.to_pandas()
    if len(frame["id"]) == 0:
        print("no results")
        return 0
    for doc_id, score in zip(frame["id"], frame["score"]):
        print(f"{doc_id}\t{score:.6f}")
    return 0


def _join_positional_tail(args, rest: list[str]) -> str:
    """Join extra positionals (sql/query text may land in table, query, or rest)."""
    parts = [p for p in (args.table, args.query) if p]
    parts.extend(rest)
    return " ".join(parts).strip()


def cmd_sql(db_path: str, query: str) -> int:
    import toradb

    db = toradb.local(db_path)
    out = db.sql(query)
    if isinstance(out, str):
        print(out)
        return 0
    frame = out.to_pandas()
    if not frame:
        print("(empty)")
        return 0
    cols = list(frame.keys())
    print("\t".join(cols))
    n = len(frame[cols[0]])
    for i in range(n):
        print("\t".join(str(frame[c][i]) for c in cols))
    return 0


def cmd_reindex(db_path: str, table: str, using: str, column: str) -> int:
    import toradb

    db = toradb.local(db_path)
    sql = f"CREATE INDEX cli_reindex ON {table} ({column}) USING {using}"
    out = db.sql(sql)
    print(out if isinstance(out, str) else "ok")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="toradb", add_help=False)
    parser.add_argument("command", nargs="?", default="help")
    parser.add_argument("path", nargs="?")
    parser.add_argument("table", nargs="?")
    parser.add_argument("query", nargs="?", default="")
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--using", default="BM25")
    parser.add_argument("--column", default="text")
    parser.add_argument("-h", "--help", action="store_true")
    args, rest = parser.parse_known_args(argv)

    if args.help or args.command in ("help", "-h", "--help"):
        _print_usage()
        return 0

    if args.command == "smoke":
        return cmd_smoke()

    if args.command == "query":
        if not args.path or not args.table:
            _print_usage()
            return 2
        q = (args.query or " ".join(rest)).strip()
        if not q:
            print("toradb query requires a query string", file=sys.stderr)
            return 2
        return cmd_query(args.path, args.table, q, args.top_k)

    if args.command == "tables":
        if not args.path:
            _print_usage()
            return 2
        import toradb

        for name in toradb.local(args.path).list_tables():
            print(name)
        return 0

    if args.command == "sql":
        if not args.path:
            _print_usage()
            return 2
        q = _join_positional_tail(args, rest)
        if not q:
            print("toradb sql requires a SQL string", file=sys.stderr)
            return 2
        return cmd_sql(args.path, q)

    if args.command == "reindex":
        if not args.path or not args.table:
            _print_usage()
            return 2
        return cmd_reindex(args.path, args.table, args.using, args.column)

    _print_usage()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
