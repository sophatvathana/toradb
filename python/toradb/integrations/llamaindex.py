"""LlamaIndex-compatible vector store adapter."""


class ToraDBLlamaIndexStore:
    """Minimal vector store adapter for LlamaIndex-style retrieval pipelines."""

    def __init__(self, table, embedding=None):
        self.table = table
        self.embedding = embedding

    def add(self, nodes, **kwargs):
        _ = kwargs
        docs = []
        for node in nodes:
            text = getattr(node, "text", None) or getattr(node, "get_content", lambda: None)()
            if text is None:
                text = str(node)
            metadata = getattr(node, "metadata", None)
            if metadata:
                doc = {"text": text}
                doc.update(metadata)
                docs.append(doc)
            else:
                docs.append(text)
        return self.table.add(docs)

    def query(self, query_str, similarity_top_k=5, **kwargs):
        _ = kwargs
        query_vector = None
        strategy = None
        if self.embedding is not None:
            emb = self.embedding.embed_query(query_str)
            query_vector = list(emb) if not isinstance(emb, list) else emb
            strategy = "dense"
        return self.table.search(
            query_str,
            top_k=similarity_top_k,
            strategy=strategy,
            query_vector=query_vector,
        )

    @classmethod
    def from_table(cls, table, embedding=None):
        return cls(table, embedding=embedding)
