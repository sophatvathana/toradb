"""Optional framework adapters (thin wrappers over Table.search)."""

from toradb.integrations.langchain import ToraDBVectorStore
from toradb.integrations.llamaindex import ToraDBLlamaIndexStore

__all__ = ["ToraDBVectorStore", "ToraDBLlamaIndexStore"]
