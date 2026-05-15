#!/usr/bin/env python3
"""
Full ToraDB Python SDK walkthrough (local mode).

Sample corpus theme: Nikola Tesla (AC power, motors, Wardenclyffe, Tesla coil).

Run from repo root after: maturin develop
  python examples/full_example.py
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import toradb
from toradb.ingest import add_file
from toradb.integrations import ToraDBLlamaIndexStore, ToraDBVectorStore
from toradb.table import stream_search

DB_PATH = ROOT / "examples" / "_demo_db"
DATA_FILE = Path(__file__).resolve().parent / "data" / "sample_tesla.txt"


def section(title: str) -> None:
    print()
    print("=" * 60)
    print(title)
    print("=" * 60)


def show_results(label: str, results) -> None:
    frame = results.to_pandas()
    print(f"{label}:")
    for doc_id, score in zip(frame["id"], frame["score"]):
        print(f"  id={doc_id}  score={score:.4f}")
    print(f"  explain: {results.explain()}")


def main() -> None:
    section("1. Open local database")
    db = toradb.local(str(DB_PATH))
    print(db)

    section("2. Create text table and ingest Nikola Tesla documents")
    articles = db.create_table("articles", mode="text")

    n1 = articles.add(
        [
            {
                "text": "Nikola Tesla pioneered alternating current systems and the induction motor",
                "tag": "history",
            },
            {
                "text": "Thomas Edison promoted direct current for early electric lighting",
                "tag": "history",
            },
            "Nikola Tesla demonstrated wireless transmission experiments in Colorado Springs",
        ]
    )
    print(f"Added {n1} string documents")

    n2 = articles.add(
        [
            {
                "text": "Nikola Tesla filed patents in 1888 for polyphase alternating current",
                "year": "1888",
                "tag": "patent",
            },
            {
                "text": "Guide to visiting Niagara Falls and the surrounding parks",
                "year": "2024",
                "tag": "travel",
            },
        ]
    )
    print(f"Added {n2} documents with metadata")

    section("3. Basic search")
    show_results(
        "Query: Nikola Tesla alternating current",
        articles.search("Nikola Tesla alternating current", top_k=3),
    )

    section("4. Metadata filter in query (year:1888)")
    show_results(
        "Query: year:1888",
        articles.search("year:1888", top_k=5),
    )

    section("5. Advanced search — explain + graph strategy")
    show_results(
        "Query: Nikola Tesla wireless (hybrid, explain=True)",
        articles.search(
            "Nikola Tesla wireless",
            top_k=5,
            strategy="hybrid",
            explain=True,
            graph_expand=True,
            depth=2,
        ),
    )

    section("6. HYDE and CRAG strategies")
    show_results(
        "HYDE strategy",
        articles.search("Tesla coil high voltage", top_k=3, strategy="hyde"),
    )
    show_results(
        "CRAG strategy",
        articles.search("Nikola Tesla motor", top_k=5, strategy="crag"),
    )

    section("7. Stream search")
    for batch in stream_search(articles, "Nikola Tesla", batch_size=2):
        frame = batch.to_pandas()
        print(f"  batch ids={list(frame['id'])}")

    section("8. Ingest from file (paragraph chunks)")
    files_table = db.create_table("files", mode="text")
    count = add_file(files_table, DATA_FILE, chunk_by="paragraph")
    print(f"Ingested {count} chunks from {DATA_FILE.name}")
    show_results(
        "Query: Wardenclyffe Tower",
        files_table.search("Wardenclyffe Tower", top_k=2),
    )

    section("9. Hybrid table + vectors (small demo embeddings)")
    papers = db.create_table(
        "papers",
        mode="hybrid",
        schema={
            "id": "uuid",
            "title": "text",
            "embedding": "vector[4]",
        },
    )
    papers.add(
        [
            {
                "text": "Nikola Tesla high frequency resonant transformer known as the Tesla coil",
                "embedding": [0.9, 0.1, 0.0, 0.0],
                "tag": "tesla",
            },
            {
                "text": "George Westinghouse commercialized AC power using Tesla patents",
                "embedding": [0.1, 0.9, 0.0, 0.0],
                "tag": "business",
            },
        ]
    )
    show_results(
        "Lexical: Tesla coil",
        papers.search("Tesla coil resonant", top_k=2, strategy="dense"),
    )

    section("10. SQL retrieval")
    show_results(
        "SQL SPARSE SEARCH",
        db.sql(
            "SELECT id FROM articles SPARSE SEARCH body BM25('Nikola Tesla alternating current') LIMIT 3"
        ),
    )

    section("11. PyArrow ingest (zero-copy Rust path)")
    try:
        import pyarrow as pa
        from toradb.ingest import add_arrow

        batch = pa.table(
            {
                "text": ["Nikola Tesla wireless energy vision"],
                "tag": ["vision"],
                "score": [42],
            }
        )
        n_arrow = add_arrow(articles, batch)
        print(f"add_arrow ingested {n_arrow} rows")
    except ImportError:
        print("skip add_arrow (install pyarrow)")

    section("12. SQL analytics (GROUP BY, WHERE, SUM)")
    analytics = db.sql(
        "SELECT tag, COUNT(*) FROM articles GROUP BY tag"
    ).to_pandas()
    print("GROUP BY tag:", dict(zip(analytics["tag"], analytics["count"])))

    where_only = db.sql(
        "SELECT tag, COUNT(*) FROM articles WHERE tag = 'patent' GROUP BY tag"
    ).to_pandas()
    print("WHERE tag=patent:", dict(zip(where_only["tag"], where_only["count"])))

    try:
        sum_scores = db.sql(
            "SELECT tag, SUM(score) FROM articles GROUP BY tag"
        ).to_pandas()
        if "sum_score" in sum_scores:
            print("SUM(score) by tag:", dict(zip(sum_scores["tag"], sum_scores["sum_score"])))
    except Exception as exc:
        print(f"SUM(score) skipped: {exc}")

    section("13. SQL retrieval + analytics")
    hybrid = db.sql(
        "SELECT tag, COUNT(*) FROM articles "
        "SPARSE SEARCH body BM25('Nikola Tesla') GROUP BY tag"
    ).to_pandas()
    print("Search then GROUP BY tag:", dict(zip(hybrid["tag"], hybrid["count"])))

    section("14. SQL DDL")
    print(db.sql("CREATE TABLE logs USING text"))
    print(db.sql("SHOW TABLES"))

    section("15. Export results (pandas-style dict)")
    results = articles.search("Nikola Tesla", top_k=3)
    print(results.to_pandas())
    print(results.to_polars())

    section("16. LangChain adapter")
    store = ToraDBVectorStore.from_table(articles)
    store.add_texts(
        ["Nikola Tesla envisioned worldwide wireless power distribution"]
    )
    hits = store.similarity_search("wireless power Tesla", k=2)
    print("LangChain hits:", hits)

    section("17. LlamaIndex-style adapter")
    li_store = ToraDBLlamaIndexStore.from_table(articles)

    class SimpleNode:
        def __init__(self, text: str):
            self.text = text

    li_store.add(
        [SimpleNode("Summary of Nikola Tesla AC motor patents and demonstrations")]
    )
    li_result = li_store.query("AC motor Tesla", similarity_top_k=2)
    show_results("LlamaIndex query", li_result)

    section("Done")
    print(f"Database path: {DB_PATH}")


if __name__ == "__main__":
    main()
