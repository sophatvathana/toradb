"""Smoke tests for toradb Python SDK (requires maturin develop)."""

import tempfile
from pathlib import Path

import pytest

toradb = pytest.importorskip("toradb")


def test_local_text_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_smoke_"))
    try:
        db = toradb.local(str(path))
        docs = db.create_table("docs", mode="text")
        docs.add(
            [
                "Nikola Tesla invented the alternating current induction motor",
                "Marie Curie studied radioactivity and Nobel prizes",
            ]
        )
        results = docs.search("Nikola Tesla alternating current motor", top_k=5)
        assert results is not None
        frame = results.to_pandas()
        assert len(frame["id"]) > 0
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_hybrid_schema_builder():
    db = toradb.connect("./test_db_hybrid")
    papers = db.create_table(
        "papers",
        mode="hybrid",
        schema={"id": "uuid", "title": "text", "embedding": "vector[768]"},
    )
    assert papers is not None


def test_stream_search_pagination():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_stream_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                "Nikola Tesla alternating current motor",
                "Nikola Tesla wireless power",
                "Nikola Tesla coil invention",
                "Marie Curie radioactivity",
                "Marie Curie Nobel prize",
            ]
        )
        from toradb.table import stream_search

        batches = list(stream_search(t, "Nikola Tesla", batch_size=2))
        assert len(batches) >= 2
        ids = []
        for batch in batches:
            ids.extend(list(batch.to_pandas()["id"]))
        assert len(ids) >= 3
        assert len(ids) == len(set(ids))
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_dense_vector_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_dense_"))
    try:
        db = toradb.local(str(path))
        papers = db.create_table("papers", mode="hybrid")
        papers.add(
            [
                {
                    "text": "Nikola Tesla coil",
                    "embedding": [1.0, 0.0, 0.0, 0.0],
                },
                {
                    "text": "Marie Curie radiation",
                    "embedding": [0.0, 1.0, 0.0, 0.0],
                },
            ]
        )
        frame = papers.search(
            "query",
            top_k=1,
            strategy="dense",
            query_vector=[0.95, 0.05, 0.0, 0.0],
        ).to_pandas()
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_search_with_strategy_and_explain():
    import re
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_explain_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(["machine learning retrieval vector database"])
        r = t.search("machine learning retrieval", top_k=10, strategy="hybrid", explain=True)
        text = r.explain()
        assert "tier1=" in text
        assert "graph_expand=" in text
        tier1 = int(re.search(r"tier1=(\d+)", text).group(1))
        assert tier1 > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_add_arrow_ingest():
    pa = pytest.importorskip("pyarrow")
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_arrow_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("arrow", mode="text")
        table = pa.table(
            {
                "text": [
                    "Nikola Tesla alternating current motor",
                    "Marie Curie radioactivity research",
                ],
                "tag": ["patent", "science"],
                "score": [10, 5],
            }
        )
        from toradb.ingest import add_arrow

        n = add_arrow(t, table)
        assert n == 2
        frame = t.search("Nikola Tesla motor", top_k=3).to_pandas()
        assert len(frame["id"]) > 0
        agg = db.sql("SELECT tag, SUM(score) FROM arrow GROUP BY tag").to_pandas()
        assert dict(zip(agg["tag"], agg["sum_score"]))["patent"] == 10.0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_add_file_ingest():
    db = toradb.local("./test_db_ingest")
    t = db.create_table("files", mode="text")
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False, encoding="utf-8") as f:
        f.write("alpha\n\nbeta\n\ngamma")
        path = f.name
    try:
        from toradb.ingest import add_file

        n = add_file(t, path)
        assert n == 3
    finally:
        Path(path).unlink(missing_ok=True)


def test_sql_search_group_by_analytics():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_sg_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla AC motor", "tag": "patent"},
                {"text": "Marie Curie radiation", "tag": "science"},
            ]
        )
        frame = db.sql(
            "SELECT tag, COUNT(*) FROM docs "
            "SPARSE SEARCH body BM25('Nikola Tesla') GROUP BY tag"
        ).to_pandas()
        counts = dict(zip(frame["tag"], frame["count"]))
        assert counts.get("patent") == 1
        assert "science" not in counts or counts.get("science", 0) == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_where_group_by_analytics():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_where_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla AC motor", "tag": "patent"},
                {"text": "Marie Curie radiation", "tag": "science"},
            ]
        )
        frame = db.sql(
            "SELECT tag, COUNT(*) FROM docs WHERE tag = 'science' GROUP BY tag"
        ).to_pandas()
        counts = dict(zip(frame["tag"], frame["count"]))
        assert counts == {"science": 1}
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_sum_group_by_analytics():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_sum_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "a", "tag": "patent", "score": "10"},
                {"text": "b", "tag": "patent", "score": "20"},
                {"text": "c", "tag": "science", "score": "5"},
            ]
        )
        frame = db.sql("SELECT tag, SUM(score) FROM docs GROUP BY tag").to_pandas()
        sums = dict(zip(frame["tag"], frame["sum_score"]))
        assert sums["patent"] == 30.0
        assert sums["science"] == 5.0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_where_in_group_by_analytics():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_in_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "a", "tag": "patent"},
                {"text": "b", "tag": "science"},
                {"text": "c", "tag": "other"},
            ]
        )
        frame = db.sql(
            "SELECT tag, COUNT(*) FROM docs WHERE tag IN ('patent', 'science') GROUP BY tag"
        ).to_pandas()
        tags = set(frame["tag"])
        assert tags == {"patent", "science"}
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_group_by_analytics():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_olap_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla AC motor", "tag": "patent"},
                {"text": "Nikola Tesla coil", "tag": "patent"},
                {"text": "Marie Curie radiation", "tag": "science"},
            ]
        )
        frame = db.sql("SELECT tag, COUNT(*) FROM docs GROUP BY tag").to_pandas()
        counts = dict(zip(frame["tag"], frame["count"]))
        assert counts["patent"] == 2
        assert counts["science"] == 1
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_vector_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_vec_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("papers", mode="hybrid")
        t.add(
            [
                {"text": "Nikola Tesla coil", "embedding": [1.0, 0.0, 0.0, 0.0]},
                {"text": "Marie Curie radiation", "embedding": [0.0, 1.0, 0.0, 0.0]},
            ]
        )
        results = db.sql(
            "SELECT id FROM papers VECTOR SEARCH embedding ANN([0.95, 0.05, 0.0, 0.0]) LIMIT 1"
        )
        frame = results.to_pandas()
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_sparse_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                "Nikola Tesla alternating current induction motor",
                "Marie Curie studied radioactivity",
            ]
        )
        results = db.sql(
            "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5"
        )
        frame = results.to_pandas()
        assert len(frame["id"]) > 0
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_list_tables_and_table_accessor():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_tables_"))
    try:
        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["Nikola Tesla motor"])
        assert "docs" in db.list_tables()

        del db
        db2 = toradb.local(str(path))
        frame = db2.table("docs").search("Nikola Tesla", top_k=3).to_pandas()
        assert len(frame["id"]) > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_describe_table():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_describe_"))
    try:
        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["one", "two"])
        out = db.sql("DESCRIBE docs")
        assert isinstance(out, str)
        assert "table: docs" in out
        assert "rows: 2" in out
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_create_index():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_create_index_"))
    try:
        db = toradb.local(str(path))
        db.create_table("papers", mode="text").add(["Nikola Tesla motor patents"])
        msg = db.sql("CREATE INDEX text_idx ON papers (text) USING BM25")
        assert isinstance(msg, str)
        assert "created index TEXT_IDX" in msg
        assert "USING BM25" in msg
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_drop_table():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_drop_table_"))
    try:
        db = toradb.local(str(path))
        db.create_table("gone", mode="text").add(["bye"])
        assert "gone" in db.list_tables()
        msg = db.sql("DROP TABLE gone")
        assert isinstance(msg, str)
        assert "dropped table gone" in msg
        assert "gone" not in db.list_tables()
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_show_tables():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_show_tables_"))
    try:
        db = toradb.local(str(path))
        db.create_table("alpha", mode="text").add(["one"])
        db.create_table("beta", mode="text").add(["two", "three"])
        frame = db.sql("SHOW TABLES").to_pandas()
        assert set(frame["table"]) == {"alpha", "beta"}
        rows = dict(zip(frame["table"], frame["rows"]))
        assert rows["alpha"] == 1.0
        assert rows["beta"] == 2.0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_persist_reload_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_reload_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(["Nikola Tesla alternating current induction motor"])
        del db

        db2 = toradb.local(str(path))
        results = db2.table("docs").search("Nikola Tesla alternating current", top_k=5)
        frame = results.to_pandas()
        assert len(frame["id"]) > 0
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_cli_sql_show_tables_prints_text():
    import shutil

    from toradb.cli import cmd_sql

    path = Path(tempfile.mkdtemp(prefix="toradb_cli_show_"))
    try:
        import toradb

        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["hello"])
        assert cmd_sql(str(path), "SHOW TABLES") == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_database_reindex():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_db_reindex_"))
    try:
        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["Nikola Tesla motor"])
        msg = db.reindex("docs", using="BM25", column="text")
        assert "reindex" in msg.lower() or "created index" in msg.lower()
        frame = db.table("docs").search("Nikola Tesla", top_k=3).to_pandas()
        assert len(frame["id"]) > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_cli_reindex_command():
    import shutil

    from toradb.cli import cmd_reindex

    path = Path(tempfile.mkdtemp(prefix="toradb_cli_reindex_"))
    try:
        import toradb

        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["Nikola Tesla motor"])
        assert cmd_reindex(str(path), "docs", "BM25", "text") == 0
        results = db.table("docs").search("Nikola Tesla", top_k=3)
        assert len(results.to_pandas()["id"]) > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_cli_smoke_command():
    from toradb.cli import cmd_smoke

    assert cmd_smoke() == 0


def test_cli_sql_command():
    import shutil

    from toradb.cli import cmd_sql

    path = Path(tempfile.mkdtemp(prefix="toradb_cli_sql_"))
    try:
        import toradb

        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla motor", "tag": "patent"},
                {"text": "Marie Curie radiation", "tag": "science"},
            ]
        )
        sql = "SELECT tag, COUNT(*) FROM docs GROUP BY tag"
        assert cmd_sql(str(path), sql) == 0
        # argv-style: quoted SQL is parsed into the `table` slot by argparse
        import sys
        from toradb.cli import main

        assert (
            main(["sql", str(path), sql])
            == 0
        )
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_llamaindex_adapter_hybrid():
    import shutil

    from toradb.integrations import ToraDBLlamaIndexStore

    path = Path(tempfile.mkdtemp(prefix="toradb_li_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("papers", mode="hybrid")

        class Node:
            def __init__(self, text, metadata=None):
                self.text = text
                self.metadata = metadata or {}

        store = ToraDBLlamaIndexStore.from_table(t)
        store.add(
            [
                Node("Nikola Tesla coil", {"tag": "patent", "embedding": [1.0, 0.0]}),
                Node("Marie Curie radiation", {"tag": "science", "embedding": [0.0, 1.0]}),
            ]
        )
        result = store.query("coil", similarity_top_k=1)
        frame = result.to_pandas()
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_add_arrow_hybrid_embeddings():
    pa = pytest.importorskip("pyarrow")
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_arrow_hybrid_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("hybrid", mode="hybrid")
        table = pa.table(
            {
                "text": ["Nikola Tesla coil", "Marie Curie radiation"],
                "embedding": [[1.0, 0.0], [0.0, 1.0]],
                "tag": ["patent", "science"],
            }
        )
        from toradb.ingest import add_arrow

        assert add_arrow(t, table) == 2
        frame = t.search(
            "query",
            top_k=1,
            strategy="dense",
            query_vector=[0.95, 0.05],
        ).to_pandas()
        assert frame["id"][0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_langchain_metadata():
    import shutil

    from toradb.integrations import ToraDBVectorStore

    path = Path(tempfile.mkdtemp(prefix="toradb_lc_meta_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("lc", mode="text")
        store = ToraDBVectorStore.from_table(t)
        store.add_texts(
            ["Nikola Tesla motors"],
            metadatas=[{"tag": "patent"}],
        )
        agg = db.sql("SELECT tag, COUNT(*) FROM lc GROUP BY tag").to_pandas()
        assert dict(zip(agg["tag"], agg["count"]))["patent"] == 1.0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_langchain_adapter():
    import shutil

    from toradb.integrations import ToraDBVectorStore

    path = Path(tempfile.mkdtemp(prefix="toradb_lc_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("lc", mode="text")
        store = ToraDBVectorStore.from_table(t)
        store.add_texts(["doc one about Tesla motors", "doc two about Curie radiation"])
        hits = store.similarity_search("Tesla motors", k=2)
        assert len(hits) >= 1
        assert hits[0]["id"] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)
