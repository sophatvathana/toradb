"""LlamaIndex-compatible vector store stub."""


class ToraDBLlamaIndexStore:
    """Minimal vector store adapter for LlamaIndex-style retrieval pipelines."""

    def __init__(self, table):
        self.table = table

    def add(self, nodes, **kwargs):
        _ = kwargs
        texts = [getattr(n, "text", str(n)) for n in nodes]
        return self.table.add(texts)

    def query(self, query_str, similarity_top_k=5, **kwargs):
        _ = kwargs
        return self.table.search(query_str, top_k=similarity_top_k)

    @classmethod
    def from_table(cls, table):
        return cls(table)
