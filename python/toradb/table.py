"""Table helpers."""


def create_table(db, name, schema=None, mode="text"):
    return db.create_table(name, mode, schema)


def stream_search(table, query, batch_size=128, strategy=None, query_vector=None):
    """Yield search result pages until fewer than batch_size hits are returned."""
    offset = 0
    while True:
        kwargs = {"top_k": batch_size, "offset": offset}
        if strategy is not None:
            kwargs["strategy"] = strategy
        if query_vector is not None:
            kwargs["query_vector"] = query_vector
        results = table.search(query, **kwargs)
        frame = results.to_pandas()
        n = len(frame["id"])
        if n == 0:
            break
        yield results
        offset += n
        if n < batch_size:
            break
