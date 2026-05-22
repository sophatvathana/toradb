def sql(db, statement):
    return db.sql(statement)


def sql_stream(db, statement, batch_size=128):
    """Yield search result pages for a retrieval SELECT (OFFSET/LIMIT paging)."""
    pages = db.sql_stream(statement, batch_size=batch_size)
    yield from pages
