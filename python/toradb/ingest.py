"""Batch ingestion helpers."""


def add_dataframe(table, df):
    records = df.to_dict(orient="records")
    return table.add(records)


def add_arrow(table, arrow_table):
    try:
        import pyarrow as pa
    except ImportError as e:
        raise ImportError("pyarrow is required for add_arrow") from e
    return add_dataframe(table, arrow_table.to_pandas())


def add_file(table, path, chunk_by="paragraph"):
    """Ingest a UTF-8 text file as document chunks (stub tokenizer)."""
    with open(path, encoding="utf-8") as f:
        text = f.read()
    if chunk_by == "line":
        chunks = [ln.strip() for ln in text.splitlines() if ln.strip()]
    else:
        chunks = [p.strip() for p in text.split("\n\n") if p.strip()]
        if not chunks and text.strip():
            chunks = [text.strip()]
    return table.add(chunks)
