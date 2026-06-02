"""ToraDB Python SDK"""
from toradb._toradb_sdk import (
    local as _local,
    connect as _connect,
    Database,
    Table,
    SearchResults,
)
from toradb.embeddings import (
    Embedder,
    EmbeddingDatabase,
    EmbeddingTable,
    OpenAIEmbedder,
    SentenceTransformerEmbedder,
    SparseEncoder,
    SpladeEncoder,
    wrap_database,
)


def local(path, embedder=None, sparse_encoder=None):
    """Open a local on-disk database.

    When `embedder` is given (an `Embedder`, a `list[str] -> list[list[float]]` callable,
    or any object with `embed_documents`/`embed_query`), text is auto-embedded on ingest
    and search so dense/hybrid retrieval works without supplying vectors.

    When `sparse_encoder` is given (a `SparseEncoder`, a `list[str] -> list[dict[str,float]]`
    callable, or an object with `encode_documents`/`encode_query`), text is auto-encoded to
    learned-sparse `{token: weight}` maps for SPLADE/Seismic retrieval. With neither, the raw
    database is returned unchanged.
    """
    return wrap_database(_local(path), embedder, sparse_encoder)


def connect(*args, embedder=None, sparse_encoder=None, **kwargs):
    """Connect to a database (see `local`). Accepts optional `embedder`/`sparse_encoder`."""
    return wrap_database(_connect(*args, **kwargs), embedder, sparse_encoder)


__all__ = [
    "local",
    "connect",
    "Database",
    "Table",
    "SearchResults",
    "Embedder",
    "EmbeddingDatabase",
    "EmbeddingTable",
    "SentenceTransformerEmbedder",
    "OpenAIEmbedder",
    "SparseEncoder",
    "SpladeEncoder",
]
