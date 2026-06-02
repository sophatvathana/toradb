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


def test_faceted_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_facets_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla AC motor", "category": "electronics"},
                {"text": "Nikola Tesla wireless power", "category": "electronics"},
                {"text": "Marie Curie radioactivity", "category": "books"},
            ]
        )
        r = t.search("Nikola Tesla Marie Curie", top_k=5, facets=["category"])
        f = r.facets
        assert f["category"]["electronics"] == 2
        assert f["category"]["books"] == 1
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_facets_clause():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_facets_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "Nikola Tesla AC motor", "category": "electronics"},
                {"text": "Nikola Tesla wireless power", "category": "electronics"},
                {"text": "Marie Curie radioactivity", "category": "books"},
            ]
        )
        # LIMIT 1 returns a single hit, but facets count the full matched set.
        r = db.sql(
            "SELECT id FROM docs SPARSE SEARCH body "
            "BM25('Nikola Tesla Marie Curie') FACETS (category) LIMIT 1"
        )
        assert len(r.to_pandas()["id"]) == 1
        f = r.facets
        assert f["category"]["electronics"] == 2
        assert f["category"]["books"] == 1
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_learned_sparse_splade_encoder():
    import shutil

    from toradb.embeddings import SparseEncoder

    enc = SparseEncoder(
        lambda texts: [
            {tok.lower(): float(len(tok)) for tok in t.split()} for t in texts
        ]
    )
    path = Path(tempfile.mkdtemp(prefix="toradb_splade_"))
    try:
        db = toradb.local(str(path), sparse_encoder=enc)
        t = db.create_table("docs", mode="text")
        t.add(["tesla alternating", "tesla ac"])
        r = t.search("tesla alternating")
        ids = list(r.to_pandas()["id"])
        assert ids[0] == 0, f"SPLADE should rank the high-weight doc first, got {ids}"
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_learned_sparse_explicit_sparse_kwarg():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_splade_kwarg_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "tesla alternating", "sparse": {"tesla": 5.0, "alternating": 11.0}},
                {"text": "tesla ac", "sparse": {"tesla": 5.0, "ac": 2.0}},
            ]
        )
        r = t.search(
            "tesla alternating",
            strategy="splade",
            sparse={"tesla": 5.0, "alternating": 11.0},
        )
        ids = list(r.to_pandas()["id"])
        assert ids[0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_splade_falls_back_to_bm25_in_explain():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_splade_explain_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(["Nikola Tesla alternating current motor"])
        r = t.search("Nikola Tesla", top_k=5, strategy="splade", explain=True)
        text = r.explain()
        assert "sparse_backend=splade(fallback=bm25)" in text, text
        assert len(r.to_pandas()["id"]) > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_ranking_field_boost():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_boost_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "tesla tesla motor"},  # higher BM25
                {"text": "tesla motor", "editor_pick": "yes"},  # boosted
            ]
        )
        base = list(t.search("tesla motor").to_pandas()["id"])
        assert base[0] == 0
        boosted = list(
            t.search("tesla motor", boosts={"editor_pick": 5.0}).to_pandas()["id"]
        )
        assert boosted[0] == 1, f"boost should promote the editor pick, got {boosted}"
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_ranking_temporal_decay_and_breakdown():
    import json
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_decay_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "tesla motor", "published": "2020-01-01"},
                {"text": "tesla motor", "published": "2999-01-01"},
            ]
        )
        r = t.search("tesla motor", decay=("published", 30.0), explain=True)
        ids = list(r.to_pandas()["id"])
        assert ids[0] == 1, f"recent doc should win under decay, got {ids}"
        prov = json.loads(r.provenance)
        assert prov.get("score_breakdown"), "provenance should include a score breakdown"
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_ranking_bm25_params():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_k1b_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(["tesla tesla tesla tesla motor", "tesla motor"])
        # Just exercise the kwargs end-to-end; high k1 favors the high-tf doc.
        ids = list(t.search("tesla", k1=5.0, b=0.75).to_pandas()["id"])
        assert ids[0] == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_ranking_clauses():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_rank_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                {"text": "tesla tesla motor"},
                {"text": "tesla motor", "editor_pick": "yes"},
            ]
        )
        frame = db.sql(
            "SELECT id FROM docs SPARSE SEARCH body BM25('tesla motor', k1=1.5) "
            "BOOST(editor_pick, 5.0) LIMIT 5"
        ).to_pandas()
        assert list(frame["id"])[0] == 1
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_hybrid_schema_builder():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_hybrid_schema_"))
    try:
        db = toradb.connect(str(path))
        papers = db.create_table(
            "papers",
            mode="hybrid",
            schema={"id": "uuid", "title": "text", "embedding": "vector[768]"},
        )
        assert papers is not None
    finally:
        shutil.rmtree(path, ignore_errors=True)


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


def _unit_vector(doc_id: int, dim: int = 8) -> list[float]:
    v = [0.0] * dim
    v[doc_id % dim] = 1.0
    return v


def test_diskann_strategy_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_diskann_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {
                    "text": f"doc {i}",
                    "embedding": _unit_vector(i),
                }
                for i in range(40)
            ]
        )
        sidecar = path / "emb" / "indexes" / "diskann.bin"
        assert sidecar.is_file(), "diskann.bin should exist after flush"
        frame = emb.search(
            "query",
            top_k=5,
            strategy="diskann",
            query_vector=_unit_vector(39),
        ).to_pandas()
        assert len(frame["id"]) > 0
        assert 39 in list(frame["id"][:5])
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_create_index_diskann():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_create_index_diskann_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {
                    "text": f"doc {i}",
                    "embedding": _unit_vector(i),
                }
                for i in range(40)
            ]
        )
        msg = db.sql("CREATE INDEX ann_idx ON emb (embedding) USING DISKANN")
        assert isinstance(msg, str)
        assert "DISKANN" in msg
        assert (path / "emb" / "indexes" / "diskann.bin").is_file()
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_reindex_diskann():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_reindex_diskann_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {
                    "text": f"doc {i}",
                    "embedding": _unit_vector(i),
                }
                for i in range(40)
            ]
        )
        sidecar = path / "emb" / "indexes" / "diskann.bin"
        sidecar.unlink(missing_ok=True)
        msg = db.reindex("emb", using="DISKANN", column="embedding")
        assert "DISKANN" in msg
        assert sidecar.is_file()
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_describe_lists_index_sidecars():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_describe_idx_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {
                    "text": f"doc {i}",
                    "embedding": _unit_vector(i),
                }
                for i in range(40)
            ]
        )
        out = db.sql("DESCRIBE emb")
        assert isinstance(out, str)
        assert "indexes:" in out
        assert "diskann" in out
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


def test_dense_strategy_uses_diskann_when_sidecar_present():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_dense_diskann_auto_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {"text": f"doc {i}", "embedding": _unit_vector(i)}
                for i in range(40)
            ]
        )
        r = emb.search(
            "query",
            top_k=5,
            strategy="dense",
            query_vector=_unit_vector(39),
            explain=True,
        )
        assert "dense_backend=diskann" in r.explain()
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_search_explain_reports_dense_backend_diskann():
    import re
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_explain_diskann_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {"text": f"doc {i}", "embedding": _unit_vector(i)}
                for i in range(40)
            ]
        )
        r = emb.search(
            "query",
            top_k=3,
            strategy="diskann",
            query_vector=_unit_vector(39),
            explain=True,
        )
        assert re.search(r"dense_backend=diskann", r.explain())
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_cli_tables_prints_describe():
    import shutil

    from toradb.cli import cmd_tables

    path = Path(tempfile.mkdtemp(prefix="toradb_cli_tables_"))
    try:
        import toradb

        db = toradb.local(str(path))
        db.create_table("docs", mode="text").add(["hello"])
        assert cmd_tables(str(path)) == 0
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
    import shutil

    db_path = Path(tempfile.mkdtemp(prefix="toradb_ingest_"))
    db = toradb.local(str(db_path))
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
        shutil.rmtree(db_path, ignore_errors=True)


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


def test_sql_vector_search_prefers_diskann_sidecar():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_vec_diskann_"))
    try:
        db = toradb.local(str(path))
        emb = db.create_table("emb", mode="hybrid")
        emb.add(
            [
                {"text": f"doc {i}", "embedding": _unit_vector(i)}
                for i in range(40)
            ]
        )
        (path / "emb" / "indexes" / "hnsw.bin").unlink(missing_ok=True)
        del db

        db2 = toradb.local(str(path))
        q = _unit_vector(39)
        ann = "[" + ", ".join(f"{x:.1f}" for x in q) + "]"
        frame = db2.sql(
            f"SELECT id FROM emb VECTOR SEARCH embedding ANN({ann}) LIMIT 5"
        ).to_pandas()
        assert len(frame["id"]) > 0
        assert 39 in list(frame["id"][:5])
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_alter_segment_workers():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_segment_workers_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        for i in range(20):
            t.add([f"Nikola Tesla item {i} motor"])
        msg = db.sql("ALTER TABLE docs SET SEGMENT_WORKERS = 2")
        assert "segment_workers=2" in str(msg)
        desc = db.sql("DESCRIBE docs")
        assert "segment_workers: 2" in str(desc)
        explain = t.search(
            "Nikola Tesla motor", top_k=3, strategy="distributed", explain=True
        ).explain()
        assert "segment_workers=2" in explain
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_explain_select():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_explain_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(["Nikola Tesla alternating current motor"])
        plan = db.sql(
            "EXPLAIN SELECT id FROM docs SPARSE SEARCH body BM25('Tesla') LIMIT 5"
        ).explain()
        assert "RetrievalScan" in plan
        assert "sparse=true" in plan
        assert "table=docs" in plan
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_show_materialized_views():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_show_mv_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                "Nikola Tesla alternating current motor",
                "Marie Curie studied radioactivity",
            ]
        )
        db.sql(
            "CREATE MATERIALIZED VIEW hot AS "
            "SELECT id FROM docs SPARSE SEARCH body BM25('Tesla') LIMIT 10"
        )
        frame = db.sql("SHOW MATERIALIZED VIEWS").to_pandas()
        assert "hot" in list(frame["view"])
        assert list(frame["rows"])[list(frame["view"]).index("hot")] >= 1
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_distributed_segment_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_distributed_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        for i in range(40):
            t.add([f"Nikola Tesla item {i} motor"])
        frame = db.sql(
            "SELECT id FROM docs DISTRIBUTED SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5"
        ).to_pandas()
        assert len(frame["id"]) > 0
        explain = t.search(
            "Nikola Tesla motor", top_k=3, strategy="distributed", explain=True
        ).explain()
        assert "distributed=true" in explain
        assert "segment_workers=" in explain
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_materialized_view():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_mv_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                "Nikola Tesla alternating current motor",
                "Marie Curie studied radioactivity",
            ]
        )
        msg = db.sql(
            "CREATE MATERIALIZED VIEW top_docs AS "
            "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5"
        )
        assert "created materialized view" in str(msg)
        cached = db.sql("SELECT id FROM top_docs LIMIT 10").to_pandas()["id"]
        assert len(cached) >= 1
        t.add(["Nikola Tesla high voltage coil"])
        refresh = db.sql("REFRESH MATERIALIZED VIEW top_docs")
        assert "refreshed materialized view" in str(refresh)
        drop = db.sql("DROP MATERIALIZED VIEW top_docs")
        assert "dropped materialized view" in str(drop)
        tables = db.list_tables()
        assert "top_docs" not in tables
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_stream_search_pagination():
    import shutil

    from toradb.sql import sql_stream

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_stream_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        for i in range(6):
            t.add([f"Nikola Tesla item {i} motor"])
        pages = list(
            sql_stream(
                db,
                "STREAM SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 2",
                batch_size=2,
            )
        )
        assert len(pages) >= 2
        total = sum(len(p.to_pandas()["id"]) for p in pages)
        assert total >= 2
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_search_join_metadata():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_join_"))
    try:
        db = toradb.local(str(path))
        papers = db.create_table("papers", mode="text")
        citations = db.create_table("citations", mode="text")
        papers.add(
            [
                {
                    "text": "Nikola Tesla alternating current motor",
                    "paper_id": "p1",
                },
                {
                    "text": "Nikola Tesla wireless power",
                    "paper_id": "p2",
                },
            ]
        )
        citations.add([{"text": "cites p1", "paper_id": "p1"}])
        all_ids = db.sql(
            "SELECT id FROM papers SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5"
        ).to_pandas()["id"]
        joined = db.sql(
            "SELECT id FROM papers JOIN citations "
            "ON papers.paper_id = citations.paper_id "
            "SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5"
        ).to_pandas()["id"]
        assert len(joined) < len(all_ids)
        assert int(joined[0]) == 0
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_search_order_by_score():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_order_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        t.add(
            [
                "unrelated topic",
                "Nikola Tesla Nikola Tesla alternating current motor",
                "Nikola Tesla once",
            ]
        )
        raw = db.sql(
            "SELECT id, score FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 10"
        ).to_pandas()
        expected_ids = [
            int(doc_id)
            for doc_id, _ in sorted(
                zip(raw["id"], raw["score"]), key=lambda row: row[1], reverse=True
            )
        ]
        frame = db.sql(
            "SELECT id, score FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') "
            "ORDER BY score DESC LIMIT 3"
        ).to_pandas()
        scores = list(frame["score"])
        assert scores == sorted(scores, reverse=True)
        want = min(3, len(expected_ids))
        assert [int(x) for x in frame["id"]] == expected_ids[:want]
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_sql_search_limit_offset():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_sql_offset_"))
    try:
        db = toradb.local(str(path))
        t = db.create_table("docs", mode="text")
        for i in range(5):
            t.add([f"Nikola Tesla item {i}"])
        all_ids = db.sql(
            "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla') LIMIT 5 OFFSET 0"
        ).to_pandas()["id"]
        page1 = db.sql(
            "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla') LIMIT 2 OFFSET 2"
        ).to_pandas()["id"]
        assert list(page1) == list(all_ids[2:4])
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


def test_bulk_ingest_finish_and_search():
    import shutil

    path = Path(tempfile.mkdtemp(prefix="toradb_bulk_smoke_"))
    try:
        db = toradb.local(str(path))
        db.begin_bulk_ingest("docs")
        t = db.create_table("docs", mode="text")
        for i in range(5):
            t.add([f"Nikola Tesla bulk document {i} alternating current motor"])
        assert db.bulk_ingest_active("docs")
        db.finish_bulk_ingest("docs", compact=False, reindex_bm25=True)
        assert not db.bulk_ingest_active("docs")
        frame = t.search("Nikola Tesla alternating current", top_k=3).to_pandas()
        assert len(frame["id"]) > 0
    finally:
        shutil.rmtree(path, ignore_errors=True)
