# Quickstart

This guide gets you from install to your first retrieval query.

## 1) Install locally

Follow [INSTALL.md](INSTALL.md), then return to repo root.

## 2) Run the bundled example

```bash
python examples/full_example.py
```

## 3) Use CLI against demo DB

```bash
toradb query ./examples/_demo_db articles "Nikola Tesla motor"
toradb sql ./examples/_demo_db "SHOW TABLES"
toradb sql ./examples/_demo_db "SELECT tag, COUNT(*) FROM articles GROUP BY tag"
```

## 4) Minimal Python flow

```python
import toradb

db = toradb.local("./quickstart_db")
docs = db.create_table("docs", mode="text")
docs.add([
    "Nikola Tesla invented the alternating current induction motor",
    "Marie Curie studied radioactivity",
])

results = docs.search("Nikola Tesla alternating current motor", top_k=3)
print(results.to_pandas())
```

## 5) Reindex (optional)

```bash
toradb reindex ./quickstart_db docs --using BM25
```

## Next steps

- Read [Contributing](CONTRIBUTING.md) if you want to contribute.
- Review [Security Policy](SECURITY.md) for responsible disclosure.
