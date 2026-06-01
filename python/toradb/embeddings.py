"""Pluggable text embedding for ToraDB.

Register an embedder once and ToraDB auto-computes vectors on ingest (when a document
has no ``vector``/``embedding``) and on search (auto-filling ``query_vector``), so dense
and hybrid retrieval work without bringing your own vectors on every call::

    import toradb
    from toradb.embeddings import SentenceTransformerEmbedder

    db = toradb.local("./db", embedder=SentenceTransformerEmbedder("all-MiniLM-L6-v2"))
    papers = db.create_table("papers", mode="hybrid")
    papers.add(["Attention is all you need", "BERT pre-training"])  # vectors auto-computed
    papers.search("transformer architecture")                       # query auto-embedded

Without an embedder, ``toradb.local(path)`` returns the raw database unchanged.
"""

from typing import Any, Callable, List, Optional, Sequence, Union

# An embedder may be supplied as:
#   * an Embedder instance,
#   * a plain callable list[str] -> list[list[float]], or
#   * any object exposing embed_documents()/embed_query() (e.g. a LangChain embeddings obj).
EmbedderLike = Union["Embedder", Callable[[List[str]], List[List[float]]], Any]


class Embedder:
    """Wraps a text->vectors function with dimension validation.

    `func` takes a list of strings and returns a list of equal-length float vectors.
    `dim` is optional; when omitted it is learned from the first batch and then enforced.
    """

    def __init__(
        self,
        func: Callable[[List[str]], Sequence[Sequence[float]]],
        dim: Optional[int] = None,
    ):
        if not callable(func):
            raise TypeError("Embedder func must be callable: list[str] -> list[list[float]]")
        self._func = func
        self.dim = dim

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        if not texts:
            return []
        raw = self._func(list(texts))
        vecs = [list(map(float, v)) for v in raw]
        if len(vecs) != len(texts):
            raise ValueError(
                f"embedder returned {len(vecs)} vectors for {len(texts)} texts"
            )
        for v in vecs:
            if self.dim is None:
                self.dim = len(v)
            elif len(v) != self.dim:
                raise ValueError(
                    f"embedder produced a {len(v)}-dim vector but dim is {self.dim}; "
                    "all vectors in a table must share one dimension"
                )
        return vecs

    def embed_query(self, text: str) -> List[float]:
        return self.embed_documents([text])[0]

    @classmethod
    def coerce(cls, embedder: Optional[EmbedderLike]) -> Optional["Embedder"]:
        """Normalize any accepted form into an `Embedder` (or None)."""
        if embedder is None or isinstance(embedder, cls):
            return embedder
        # LangChain-style object: embed_documents + embed_query.
        if hasattr(embedder, "embed_documents") and hasattr(embedder, "embed_query"):
            obj = embedder

            def _fn(texts: List[str]) -> List[List[float]]:
                return [list(v) for v in obj.embed_documents(texts)]

            return cls(_fn)
        if callable(embedder):
            return cls(embedder)
        raise TypeError(
            "embedder must be an Embedder, a callable list[str]->list[list[float]], "
            "or an object with embed_documents()/embed_query()"
        )


def _has_vector(doc: dict) -> bool:
    return doc.get("vector") is not None or doc.get("embedding") is not None


class EmbeddingTable:
    """Wraps a Rust `Table`, auto-embedding on `add()`/`search()`. Delegates all else."""

    def __init__(self, inner, embedder: Optional[Embedder]):
        self._inner = inner
        self._embedder = embedder
        self._ingested_dim: Optional[int] = None

    # -- ingest ---------------------------------------------------------------
    def add(self, docs):
        if self._embedder is None:
            return self._inner.add(docs)

        normalized = []
        to_embed: List[str] = []
        embed_targets = []  # indices into `normalized` needing a vector
        for doc in docs:
            d = {"text": doc} if isinstance(doc, str) else dict(doc)
            if not _has_vector(d):
                text = d.get("text")
                if not isinstance(text, str):
                    raise ValueError(
                        "auto-embedding requires a string 'text' field on each document "
                        "(or supply 'vector'/'embedding' explicitly)"
                    )
                embed_targets.append(len(normalized))
                to_embed.append(text)
            normalized.append(d)

        if to_embed:
            vecs = self._embedder.embed_documents(to_embed)
            dim = len(vecs[0]) if vecs else None
            self._check_table_dim(dim)
            for idx, vec in zip(embed_targets, vecs):
                normalized[idx]["embedding"] = vec
        return self._inner.add(normalized)

    def _check_table_dim(self, dim):
        if dim is None:
            return
        if self._ingested_dim is None:
            self._ingested_dim = dim
        elif dim != self._ingested_dim:
            raise ValueError(
                f"embedder dim {dim} != previously ingested dim {self._ingested_dim}"
            )
        # If the table was created with an explicit vector(D), surface a clear error.
        declared = self._declared_vector_dim()
        if declared is not None and dim != declared:
            raise ValueError(
                f"embedder dim {dim} != table vector({declared}); "
                "create the table without a fixed vector dim or match the embedder"
            )

    def _declared_vector_dim(self) -> Optional[int]:
        getter = getattr(self._inner, "vector_dim", None)
        try:
            return getter() if callable(getter) else getter
        except Exception:
            return None

    # -- search ---------------------------------------------------------------
    def search(self, query, **kwargs):
        if (
            self._embedder is not None
            and kwargs.get("query_vector") is None
            and isinstance(query, str)
        ):
            kwargs["query_vector"] = self._embedder.embed_query(query)
            kwargs.setdefault("strategy", "hybrid")
        return self._inner.search(query, **kwargs)

    # -- everything else delegates -------------------------------------------
    def __getattr__(self, name):
        return getattr(self._inner, name)


class EmbeddingDatabase:
    """Wraps a Rust `Database`; hands out `EmbeddingTable`s. Delegates all else."""

    def __init__(self, inner, embedder: Optional[Embedder]):
        self._inner = inner
        self._embedder = embedder

    def table(self, name):
        return EmbeddingTable(self._inner.table(name), self._embedder)

    def create_table(self, name, mode=None, schema=None):
        inner = self._inner.create_table(name, mode, schema)
        return EmbeddingTable(inner, self._embedder)

    def __getattr__(self, name):
        return getattr(self._inner, name)


def wrap_database(inner, embedder: Optional[EmbedderLike]):
    """Return `inner` unchanged when no embedder, else an `EmbeddingDatabase`."""
    emb = Embedder.coerce(embedder)
    if emb is None:
        return inner
    return EmbeddingDatabase(inner, emb)


# --- built-in adapters (deps imported lazily; optional) ----------------------

def SentenceTransformerEmbedder(model: str = "all-MiniLM-L6-v2", **encode_kwargs) -> Embedder:
    """Embedder backed by a `sentence-transformers` model. Requires `toradb[embeddings]`."""
    try:
        from sentence_transformers import SentenceTransformer
    except ImportError as e:  # pragma: no cover - exercised only without the dep
        raise ImportError(
            "SentenceTransformerEmbedder requires sentence-transformers "
            "(pip install 'toradb[embeddings]')"
        ) from e
    st_model = SentenceTransformer(model)
    dim = st_model.get_sentence_embedding_dimension()

    def _fn(texts: List[str]) -> List[List[float]]:
        return st_model.encode(texts, **encode_kwargs).tolist()

    return Embedder(_fn, dim=dim)


def OpenAIEmbedder(
    model: str = "text-embedding-3-small",
    api_key: Optional[str] = None,
    dim: Optional[int] = None,
) -> Embedder:
    """Embedder backed by the OpenAI embeddings API. Requires `toradb[openai]`."""
    try:
        from openai import OpenAI
    except ImportError as e:  # pragma: no cover
        raise ImportError(
            "OpenAIEmbedder requires the openai package (pip install 'toradb[openai]')"
        ) from e
    client = OpenAI(api_key=api_key) if api_key else OpenAI()

    def _fn(texts: List[str]) -> List[List[float]]:
        resp = client.embeddings.create(model=model, input=list(texts))
        return [d.embedding for d in resp.data]

    return Embedder(_fn, dim=dim)
