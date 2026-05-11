"""Smoke tests for toradb Python SDK (requires maturin develop)."""

import tempfile
from pathlib import Path

import pytest

toradb = pytest.importorskip("toradb")


def test_local_text_search():
    db = toradb.local("./test_db_smoke")
    docs = db.create_table("docs", mode="text")
    docs.add(
        [
            "Nikola Tesla invented the alternating current induction motor",
            "Marie Curie studied radioactivity and Nobel prizes",
        ]
    )
    results = docs.search("Nikola Tesla alternating current motor", top_k=5)
    assert results is not None
    frame = results.to_pandas()
    assert len(frame["id"]) > 0
    assert frame["id"][0] == 0


def test_hybrid_schema_builder():
    db = toradb.connect("./test_db_hybrid")
    papers = db.create_table(
        "papers",
        mode="hybrid",
        schema={"id": "uuid", "title": "text", "embedding": "vector[768]"},
    )
    assert papers is not None


def test_search_with_strategy_and_explain():
    db = toradb.local("./test_db_strategy")
    t = db.create_table("docs", mode="text")
    r = t.search("machine learning retrieval", top_k=10, strategy="hybrid", explain=True)
    text = r.explain()
    assert "tier1=" in text
    assert "graph_expand=" in text


def test_add_file_ingest():
    db = toradb.local("./test_db_ingest")
    t = db.create_table("files", mode="text")
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8") as f:
        f.write("alpha\n\nbeta\n\ngamma")
        path = f.name
    try:
        from toradb.ingest import add_file

        n = add_file(t, path)
        assert n == 3
    finally:
        Path(path).unlink(missing_ok=True)


def test_langchain_adapter():
    from toradb.integrations import ToraDBVectorStore

    db = toradb.local("./test_db_lc")
    t = db.create_table("lc", mode="text")
    store = ToraDBVectorStore.from_table(t)
    store.add_texts(["doc one about Tesla motors", "doc two about Curie radiation"])
    hits = store.similarity_search("Tesla motors", k=2)
    assert len(hits) >= 1
    assert hits[0]["id"] == 0
