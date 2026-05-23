# ToraDB Python examples

## Setup

From the repo root:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install maturin
maturin develop
```

## Run

```bash
python examples/full_example.py
```

Optional: `pip install pandas pyarrow` for dataframe / Arrow ingest (`add_arrow` uses zero-copy PyCapsule import in Rust).

CLI after install:

```bash
toradb smoke
toradb query ./examples/_demo_db articles "Nikola Tesla motor"
toradb sql ./examples/_demo_db "SELECT tag, COUNT(*) FROM articles GROUP BY tag"
toradb tables ./examples/_demo_db
toradb reindex ./examples/_demo_db articles --using BM25
```

SQL catalog helpers (return analytics dict or text):

```bash
toradb sql ./examples/_demo_db "SHOW TABLES"
toradb sql ./examples/_demo_db "DESCRIBE articles"
toradb sql ./examples/_demo_db "DROP TABLE logs"
toradb sql ./examples/_demo_db "CREATE INDEX text_idx ON articles (text) USING BM25"
toradb sql ./examples/_demo_db "CREATE INDEX ann_idx ON papers (embedding) USING DISKANN"
```

Use `db.table("articles")` (not `create_table`) to query tables already on disk.

Rebuild indexes from Python: `db.reindex("articles", using="BM25")`.

Vector ANN on disk (needs enough embedded docs for graph build, typically 32+):

```python
table.search("query", strategy="diskann", query_vector=[...], top_k=10)
db.reindex("emb", using="DISKANN", column="embedding")
```

`DESCRIBE table` lists row count, vector dimension, segment count, and on-disk index sidecars (`bm25`, `vectors`, `hnsw`, `diskann`, `segment_*`).
