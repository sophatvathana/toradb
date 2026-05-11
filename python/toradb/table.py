"""Table helpers."""

def create_table(db, name, schema=None, mode="text"):
    return db.create_table(name, mode, schema)


def stream_search(table, query, batch_size=128):
    results = table.search(query, top_k=batch_size)
    yield results
