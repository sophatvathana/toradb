import toradb

def test_local_text_search():
    db = toradb.local("./test_db")
    docs = db.create_table("docs", "text")
    assert docs is not None
