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
    wrap_database,
)


def local(path, embedder=None):
    """Open a local on-disk database.

    When `embedder` is given (an `Embedder`, a `list[str] -> list[list[float]]` callable,
    or any object with `embed_documents`/`embed_query`), text is auto-embedded on ingest
    and search so dense/hybrid retrieval works without supplying vectors. With no embedder
    the raw database is returned unchanged.
    """
    return wrap_database(_local(path), embedder)


def connect(*args, embedder=None, **kwargs):
    """Connect to a database (see `local`). Accepts an optional `embedder`."""
    return wrap_database(_connect(*args, **kwargs), embedder)


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
]
