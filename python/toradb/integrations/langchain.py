"""LangChain-compatible vector store stub."""


class ToraDBVectorStore:
    """Minimal vector store adapter; requires optional `langchain_core` at runtime."""

    def __init__(self, table, embedding=None):
        self.table = table
        self.embedding = embedding

    def add_texts(self, texts, metadatas=None, **kwargs):
        _ = (metadatas, kwargs)
        return self.table.add(list(texts))

    def similarity_search(self, query, k=4, **kwargs):
        _ = kwargs
        query_vector = None
        strategy = None
        if self.embedding is not None:
            emb = self.embedding.embed_query(query)
            query_vector = list(emb) if not isinstance(emb, list) else emb
            strategy = "dense"
        results = self.table.search(
            query, top_k=k, strategy=strategy, query_vector=query_vector
        )
        frame = results.to_pandas()
        return [{"id": i, "score": s} for i, s in zip(frame["id"], frame["score"])]

    @classmethod
    def from_table(cls, table, embedding=None):
        return cls(table, embedding=embedding)
