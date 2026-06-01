"""Tests for the auto-embedding layer (toradb.embeddings).

The Embedder/coercion/dim-guard tests are pure Python and run without the Rust
extension. The end-to-end add/search test requires `maturin develop`.
"""

import sys
import tempfile
from pathlib import Path

import pytest

# Import the module directly so the unit tests run even if the compiled
# extension (_toradb_sdk) isn't built in this environment.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
from toradb import embeddings as emb  # noqa: E402


def fake_embedder(dim=3):
    # Deterministic, dependency-free: vector depends on text length.
    return emb.Embedder(lambda texts: [[float(len(t)), 1.0, 0.0] for t in texts], dim=dim)


def test_embed_documents_and_query():
    e = fake_embedder()
    vecs = e.embed_documents(["ab", "abcd"])
    assert vecs == [[2.0, 1.0, 0.0], [4.0, 1.0, 0.0]]
    assert e.embed_query("xyz") == [3.0, 1.0, 0.0]


def test_dim_learned_when_unspecified():
    e = emb.Embedder(lambda texts: [[0.0, 0.0]] * len(texts))
    assert e.dim is None
    e.embed_documents(["a"])
    assert e.dim == 2


def test_ragged_or_mismatched_dim_raises():
    e = emb.Embedder(lambda texts: [[0.0, 0.0, 0.0]] * len(texts), dim=4)
    with pytest.raises(ValueError):
        e.embed_documents(["a"])


def test_coerce_callable_and_langchain_object():
    # plain callable
    e1 = emb.Embedder.coerce(lambda texts: [[1.0]] * len(texts))
    assert isinstance(e1, emb.Embedder)
    # langchain-style object
    class LC:
        def embed_documents(self, texts):
            return [[7.0, 8.0] for _ in texts]

        def embed_query(self, text):
            return [7.0, 8.0]

    e2 = emb.Embedder.coerce(LC())
    assert e2.embed_query("q") == [7.0, 8.0]
    # None passes through
    assert emb.Embedder.coerce(None) is None


class _FakeTable:
    """Stand-in for the Rust Table; records what add/search received."""

    def __init__(self, vector_dim=None):
        self.added = None
        self.search_kwargs = None
        self._vector_dim = vector_dim

    def add(self, docs):
        self.added = docs
        return len(docs)

    def search(self, query, **kwargs):
        self.search_kwargs = kwargs
        return {"query": query, **kwargs}

    def vector_dim(self):
        return self._vector_dim


def test_add_fills_only_missing_vectors():
    t = emb.EmbeddingTable(_FakeTable(), fake_embedder())
    t.add([
        "hello",                                  # needs embedding (len 5)
        {"text": "world", "tag": "x"},            # needs embedding (len 5)
        {"text": "given", "vector": [9.0, 9.0]},  # already has a vector — untouched
    ])
    added = t._inner.added
    assert added[0]["embedding"] == [5.0, 1.0, 0.0]
    assert added[1]["embedding"] == [5.0, 1.0, 0.0]
    assert added[1]["tag"] == "x"
    assert "embedding" not in added[2]  # pre-supplied vector preserved
    assert added[2]["vector"] == [9.0, 9.0]


def test_search_injects_query_vector_and_default_strategy():
    t = emb.EmbeddingTable(_FakeTable(), fake_embedder())
    t.search("transformer")  # len 11
    assert t._inner.search_kwargs["query_vector"] == [11.0, 1.0, 0.0]
    assert t._inner.search_kwargs["strategy"] == "hybrid"

    # Caller-supplied query_vector/strategy are respected.
    t.search("x", query_vector=[1.0, 2.0, 3.0], strategy="dense")
    assert t._inner.search_kwargs["query_vector"] == [1.0, 2.0, 3.0]
    assert t._inner.search_kwargs["strategy"] == "dense"


def test_add_dim_mismatch_against_declared_table_dim_raises():
    t = emb.EmbeddingTable(_FakeTable(vector_dim=384), fake_embedder(dim=3))
    with pytest.raises(ValueError):
        t.add(["hello"])


def test_no_embedder_passthrough():
    inner = _FakeTable()
    t = emb.EmbeddingTable(inner, None)
    t.add(["a", "b"])
    assert inner.added == ["a", "b"]  # untouched


def test_wrap_database_none_returns_inner():
    class FakeDb:
        pass

    db = FakeDb()
    assert emb.wrap_database(db, None) is db


# --- end-to-end (requires the compiled extension) ----------------------------

def test_end_to_end_auto_embed():
    toradb = pytest.importorskip("toradb._toradb_sdk")  # ensure extension is built
    import toradb as td

    path = Path(tempfile.mkdtemp(prefix="toradb_emb_"))
    try:
        db = td.local(str(path), embedder=fake_embedder())
        table = db.create_table("docs", mode="hybrid")
        # No vectors supplied — they are auto-computed.
        table.add(["alpha alpha", "beta", "gamma gamma gamma"])
        results = table.search("beta")
        frame = results.to_pandas()
        assert len(frame["id"]) > 0
    finally:
        import shutil

        shutil.rmtree(path, ignore_errors=True)
