"""Batch ingestion helpers."""


def add_dataframe(table, df):
    records = df.to_dict(orient="records")
    return table.add(records)


def _arrow_value_at(column, row_index):
    value = column[row_index]
    if hasattr(value, "as_py"):
        return value.as_py()
    return value


def add_arrow(table, arrow_table):
    """Ingest a PyArrow table without converting through pandas."""
    try:
        import pyarrow as pa  # noqa: F401
    except ImportError as e:
        raise ImportError("pyarrow is required for add_arrow") from e

    if arrow_table.num_rows == 0:
        return 0

    columns = {name: arrow_table[name] for name in arrow_table.column_names}
    records = []
    for row in range(arrow_table.num_rows):
        record = {name: _arrow_value_at(col, row) for name, col in columns.items()}
        if "text" not in record:
            for key, value in record.items():
                if isinstance(value, str) and key != "id":
                    record["text"] = value
                    break
        if "text" not in record:
            continue
        records.append(record)

    return table.add(records)


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
