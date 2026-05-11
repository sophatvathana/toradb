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
        results = self.table.search(query, top_k=k)
        frame = results.to_pandas()
        return [{"id": i, "score": s} for i, s in zip(frame["id"], frame["score"])]

    @classmethod
    def from_table(cls, table, embedding=None):
        return cls(table, embedding=embedding)
